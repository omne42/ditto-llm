use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use async_trait::async_trait;
use futures_util::StreamExt;
use futures_util::stream;
use serde::Deserialize;
use serde_json::{Value, json};

#[cfg(feature = "embeddings")]
use crate::embedding::EmbeddingModel;
use crate::model::{LanguageModel, StreamResult};
use crate::profile::{
    Env, HttpAuth, ProviderAuth, ProviderConfig, RequestAuth, apply_http_query_params,
    resolve_request_auth_with_default_keys,
};
#[cfg(feature = "rerank")]
use crate::rerank::RerankModel;
use crate::types::{
    ContentPart, FinishReason, GenerateRequest, GenerateResponse, Message, RerankDocument,
    RerankRequest, RerankResponse, RerankResult, Role, StreamChunk, Tool, ToolChoice, Usage,
    Warning,
};
use crate::{DittoError, Result};

const DEFAULT_BASE_URL: &str = "https://api.cohere.com/v2";

#[derive(Clone)]
pub struct Cohere {
    http: reqwest::Client,
    base_url: String,
    auth: Option<RequestAuth>,
    default_model: String,
    http_query_params: BTreeMap<String, String>,
}

impl Cohere {
    pub fn new(api_key: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        let api_key = api_key.into();
        let auth = if api_key.trim().is_empty() {
            None
        } else {
            HttpAuth::header_value("authorization", Some("Bearer "), &api_key)
                .ok()
                .map(RequestAuth::Http)
        };

        Self {
            http,
            base_url: DEFAULT_BASE_URL.to_string(),
            auth,
            default_model: String::new(),
            http_query_params: BTreeMap::new(),
        }
    }

    pub fn with_http_client(mut self, http: reqwest::Client) -> Self {
        self.http = http;
        self
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.default_model = model.into();
        self
    }

    pub async fn from_config(config: &ProviderConfig, env: &Env) -> Result<Self> {
        const DEFAULT_KEYS: &[&str] = &["COHERE_API_KEY", "CODE_PM_COHERE_API_KEY"];
        let auth = config
            .auth
            .clone()
            .unwrap_or(ProviderAuth::ApiKeyEnv { keys: Vec::new() });
        let auth_header = resolve_request_auth_with_default_keys(
            &auth,
            env,
            DEFAULT_KEYS,
            "authorization",
            Some("Bearer "),
        )
        .await?;

        let mut out = Self::new("");
        out.auth = Some(auth_header);
        out.http_query_params = config.http_query_params.clone();
        if !config.http_headers.is_empty() {
            out = out.with_http_client(crate::profile::build_http_client(
                std::time::Duration::from_secs(300),
                &config.http_headers,
            )?);
        }
        if let Some(base_url) = config.base_url.as_deref().filter(|s| !s.trim().is_empty()) {
            out = out.with_base_url(base_url);
        }
        if let Some(model) = config
            .default_model
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            out = out.with_model(model);
        }
        Ok(out)
    }

    fn apply_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let req = match self.auth.as_ref() {
            Some(auth) => auth.apply(req),
            None => req,
        };
        apply_http_query_params(req, &self.http_query_params)
    }

    fn chat_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/chat") {
            base.to_string()
        } else {
            format!("{base}/chat")
        }
    }

    fn resolve_model<'a>(&'a self, request: &'a GenerateRequest) -> Result<&'a str> {
        if let Some(model) = request.model.as_deref().filter(|m| !m.trim().is_empty()) {
            return Ok(model);
        }
        if !self.default_model.trim().is_empty() {
            return Ok(self.default_model.as_str());
        }
        Err(DittoError::InvalidResponse(
            "cohere chat model is not set (set request.model or Cohere::with_model)".to_string(),
        ))
    }

    fn sanitize_temperature(
        temperature: f32,
        warnings: &mut Vec<Warning>,
    ) -> Option<serde_json::Number> {
        if !temperature.is_finite() {
            warnings.push(Warning::Compatibility {
                feature: "temperature".to_string(),
                details: "temperature must be a finite number; dropping invalid value".to_string(),
            });
            return None;
        }
        serde_json::Number::from_f64(f64::from(temperature))
    }

    fn map_finish_reason(reason: Option<&str>, has_tool_calls: bool) -> FinishReason {
        if has_tool_calls {
            return FinishReason::ToolCalls;
        }

        match reason.unwrap_or("").trim().to_uppercase().as_str() {
            "COMPLETE" => FinishReason::Stop,
            "STOP_SEQUENCE" => FinishReason::Stop,
            "MAX_TOKENS" => FinishReason::Length,
            "ERROR" => FinishReason::Error,
            _ => FinishReason::Unknown,
        }
    }

    fn parse_usage(value: &Value) -> Usage {
        let mut usage = Usage::default();
        let tokens = value.get("tokens").unwrap_or(value);
        if let Some(obj) = tokens.as_object() {
            usage.input_tokens = obj.get("input_tokens").and_then(Value::as_u64);
            usage.output_tokens = obj.get("output_tokens").and_then(Value::as_u64);
        }
        usage.merge_total();
        usage
    }

    fn tool_call_arguments_to_json_string(arguments: &Value) -> String {
        match arguments {
            Value::String(raw) => raw.to_string(),
            _ => arguments.to_string(),
        }
    }

    fn messages_to_cohere_messages(messages: &[Message]) -> (Vec<Value>, Vec<Warning>) {
        let mut out = Vec::<Value>::new();
        let mut warnings = Vec::<Warning>::new();

        for message in messages {
            match message.role {
                Role::System | Role::User => {
                    let mut text = String::new();
                    for part in &message.content {
                        match part {
                            ContentPart::Text { text: t } => text.push_str(t),
                            ContentPart::Reasoning { .. } => warnings.push(Warning::Unsupported {
                                feature: "cohere.message.reasoning".to_string(),
                                details: Some(
                                    "dropping reasoning content for non-assistant roles"
                                        .to_string(),
                                ),
                            }),
                            other => warnings.push(Warning::Unsupported {
                                feature: "cohere.message.content_part".to_string(),
                                details: Some(format!(
                                    "dropping unsupported content part for cohere: {other:?}"
                                )),
                            }),
                        }
                    }

                    out.push(json!({
                        "role": match message.role { Role::System => "system", Role::User => "user", _ => unreachable!() },
                        "content": text,
                    }));
                }
                Role::Assistant => {
                    let mut content_blocks = Vec::<Value>::new();
                    let mut tool_calls = Vec::<Value>::new();

                    for part in &message.content {
                        match part {
                            ContentPart::Text { text } => {
                                if !text.is_empty() {
                                    content_blocks.push(json!({ "type": "text", "text": text }));
                                }
                            }
                            ContentPart::ToolCall {
                                id,
                                name,
                                arguments,
                            } => {
                                if id.trim().is_empty() || name.trim().is_empty() {
                                    warnings.push(Warning::Compatibility {
                                        feature: "cohere.tool_call".to_string(),
                                        details: "assistant tool_call is missing id or name; dropping tool call".to_string(),
                                    });
                                    continue;
                                }

                                tool_calls.push(json!({
                                    "id": id,
                                    "type": "tool_call",
                                    "function": {
                                        "name": name,
                                        "arguments": Self::tool_call_arguments_to_json_string(arguments),
                                    }
                                }));
                            }
                            ContentPart::Reasoning { .. } => warnings.push(Warning::Unsupported {
                                feature: "cohere.message.reasoning".to_string(),
                                details: Some(
                                    "dropping reasoning content in assistant message".to_string(),
                                ),
                            }),
                            other => warnings.push(Warning::Unsupported {
                                feature: "cohere.message.content_part".to_string(),
                                details: Some(format!(
                                    "dropping unsupported content part for cohere: {other:?}"
                                )),
                            }),
                        }
                    }

                    let mut obj = serde_json::Map::<String, Value>::new();
                    obj.insert("role".to_string(), Value::String("assistant".to_string()));
                    obj.insert("content".to_string(), Value::Array(content_blocks));
                    if !tool_calls.is_empty() {
                        obj.insert("tool_calls".to_string(), Value::Array(tool_calls));
                    }
                    out.push(Value::Object(obj));
                }
                Role::Tool => {
                    for part in &message.content {
                        match part {
                            ContentPart::ToolResult {
                                tool_call_id,
                                content,
                                ..
                            } => {
                                out.push(json!({
                                    "role": "tool",
                                    "tool_call_id": tool_call_id,
                                    "content": content,
                                }));
                            }
                            other => warnings.push(Warning::Unsupported {
                                feature: "cohere.message.tool".to_string(),
                                details: Some(format!(
                                    "dropping non-tool_result content part: {other:?}"
                                )),
                            }),
                        }
                    }
                }
            }
        }

        (out, warnings)
    }

    fn cohere_param_type_from_schema(
        schema: &Value,
        tool_name: &str,
        param_name: &str,
        warnings: &mut Vec<Warning>,
    ) -> String {
        let mut raw = schema
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_string);
        if raw.is_none() {
            if let Some(types) = schema.get("type").and_then(Value::as_array) {
                raw = types
                    .iter()
                    .filter_map(Value::as_str)
                    .find(|t| *t != "null")
                    .map(str::to_string);
            }
        }

        match raw.as_deref().unwrap_or("") {
            "string" => "str".to_string(),
            "integer" => "int".to_string(),
            "number" => "float".to_string(),
            "boolean" => "bool".to_string(),
            "array" => "list".to_string(),
            "object" => "dict".to_string(),
            other => {
                warnings.push(Warning::Compatibility {
                    feature: "cohere.tools.parameter_definitions".to_string(),
                    details: format!(
                        "tool {tool_name} param {param_name} has unsupported JSON schema type {other:?}; defaulting to str"
                    ),
                });
                "str".to_string()
            }
        }
    }

    fn tool_to_cohere(tool: &Tool, warnings: &mut Vec<Warning>) -> Value {
        let mut out = serde_json::Map::<String, Value>::new();
        out.insert("name".to_string(), Value::String(tool.name.clone()));
        if let Some(desc) = tool.description.as_deref().filter(|s| !s.trim().is_empty()) {
            out.insert("description".to_string(), Value::String(desc.to_string()));
        }

        let mut required = HashSet::<String>::new();
        if let Some(req) = tool.parameters.get("required").and_then(Value::as_array) {
            for item in req {
                if let Some(name) = item.as_str().filter(|s| !s.trim().is_empty()) {
                    required.insert(name.to_string());
                }
            }
        }

        let mut defs = serde_json::Map::<String, Value>::new();
        if let Some(props) = tool.parameters.get("properties").and_then(Value::as_object) {
            for (name, schema) in props {
                let ty = Self::cohere_param_type_from_schema(schema, &tool.name, name, warnings);
                let mut def = serde_json::Map::<String, Value>::new();
                if let Some(desc) = schema.get("description").and_then(Value::as_str) {
                    if !desc.trim().is_empty() {
                        def.insert("description".to_string(), Value::String(desc.to_string()));
                    }
                }
                def.insert("type".to_string(), Value::String(ty));
                def.insert("required".to_string(), Value::Bool(required.contains(name)));
                defs.insert(name.to_string(), Value::Object(def));
            }
        } else if tool.parameters != Value::Null && tool.parameters != json!({}) {
            warnings.push(Warning::Compatibility {
                feature: "cohere.tools.parameter_definitions".to_string(),
                details: format!(
                    "tool {} parameters are not an object schema with properties; sending empty parameter_definitions",
                    tool.name
                ),
            });
        }

        out.insert("parameter_definitions".to_string(), Value::Object(defs));
        Value::Object(out)
    }

    fn normalize_tool_choice(
        tool_choice: &ToolChoice,
        tools: Option<&[Tool]>,
        warnings: &mut Vec<Warning>,
    ) -> (Option<Value>, Option<Vec<Tool>>) {
        match tool_choice {
            ToolChoice::Auto => (None, None),
            ToolChoice::None => (Some(Value::String("NONE".to_string())), None),
            ToolChoice::Required => (Some(Value::String("REQUIRED".to_string())), None),
            ToolChoice::Tool { name } => {
                warnings.push(Warning::Compatibility {
                    feature: "tool_choice".to_string(),
                    details: "cohere only supports tool_choice REQUIRED or NONE; approximating named tool choice by filtering tools list".to_string(),
                });

                let Some(tools) = tools else {
                    return (Some(Value::String("REQUIRED".to_string())), None);
                };

                let filtered: Vec<Tool> =
                    tools.iter().filter(|t| t.name == *name).cloned().collect();
                if filtered.is_empty() {
                    return (Some(Value::String("REQUIRED".to_string())), None);
                }
                (Some(Value::String("REQUIRED".to_string())), Some(filtered))
            }
        }
    }
}

#[derive(Debug, Deserialize, Default)]
struct CohereChatResponse {
    #[serde(default)]
    id: String,
    #[serde(default)]
    finish_reason: Option<String>,
    #[serde(default)]
    message: CohereAssistantMessage,
    #[serde(default)]
    usage: Option<Value>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct CohereAssistantMessage {
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    content: Vec<CohereContentBlock>,
    #[serde(default)]
    tool_plan: Option<String>,
    #[serde(default)]
    tool_calls: Vec<CohereToolCall>,
    #[serde(default)]
    citations: Option<Value>,
}

#[derive(Debug, Deserialize, Default)]
struct CohereContentBlock {
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct CohereToolCall {
    #[serde(default)]
    id: String,
    #[serde(default)]
    function: CohereToolFunction,
}

#[derive(Debug, Deserialize, Default)]
struct CohereToolFunction {
    #[serde(default)]
    name: String,
    #[serde(default)]
    arguments: String,
}

#[async_trait]
impl LanguageModel for Cohere {
    fn provider(&self) -> &str {
        "cohere"
    }

    fn model_id(&self) -> &str {
        self.default_model.as_str()
    }

    async fn generate(&self, request: GenerateRequest) -> Result<GenerateResponse> {
        let model = self.resolve_model(&request)?;
        let selected_provider_options = request.provider_options_value_for(self.provider())?;

        let (messages, mut warnings) = Self::messages_to_cohere_messages(&request.messages);

        let mut body = serde_json::Map::<String, Value>::new();
        body.insert("model".to_string(), Value::String(model.to_string()));
        body.insert("messages".to_string(), Value::Array(messages));

        if let Some(temperature) = request.temperature {
            if let Some(value) = Self::sanitize_temperature(temperature, &mut warnings) {
                body.insert("temperature".to_string(), Value::Number(value));
            }
        }
        if let Some(max_tokens) = request.max_tokens {
            body.insert("max_tokens".to_string(), Value::Number(max_tokens.into()));
        }
        if let Some(top_p) = request.top_p {
            if let Some(value) = crate::utils::params::clamped_number_from_f32(
                "top_p",
                top_p,
                0.01,
                0.99,
                &mut warnings,
            ) {
                body.insert("p".to_string(), Value::Number(value));
            }
        }
        if let Some(stops) = request.stop_sequences.as_ref() {
            let stops = crate::utils::params::sanitize_stop_sequences(stops, None, &mut warnings);
            if !stops.is_empty() {
                body.insert(
                    "stop_sequences".to_string(),
                    Value::Array(stops.into_iter().map(Value::String).collect()),
                );
            }
        }

        let mut tools = request.tools.unwrap_or_default();
        if !tools.is_empty() {
            if cfg!(feature = "tools") {
                let tool_choice = request.tool_choice.unwrap_or(ToolChoice::Auto);
                let (mapped_choice, filtered_tools) =
                    Self::normalize_tool_choice(&tool_choice, Some(&tools), &mut warnings);
                if let Some(filtered) = filtered_tools {
                    tools = filtered;
                }

                let mapped = tools
                    .iter()
                    .map(|t| Self::tool_to_cohere(t, &mut warnings))
                    .collect::<Vec<_>>();
                body.insert("tools".to_string(), Value::Array(mapped));
                if let Some(choice) = mapped_choice {
                    body.insert("tool_choice".to_string(), choice);
                }
            } else {
                warnings.push(Warning::Unsupported {
                    feature: "tools".to_string(),
                    details: Some("ditto-llm built without tools feature".to_string()),
                });
            }
        } else if let Some(tool_choice) = request.tool_choice {
            warnings.push(Warning::Unsupported {
                feature: "tool_choice".to_string(),
                details: Some(format!(
                    "cohere requires tools to be provided when tool_choice is set (got {tool_choice:?})"
                )),
            });
        }

        crate::types::merge_provider_options_into_body(
            &mut body,
            selected_provider_options.as_ref(),
            &["reasoning_effort", "response_format", "parallel_tool_calls"],
            "cohere.provider_options",
            &mut warnings,
        );

        let url = self.chat_url();
        let req = self.http.post(url);
        let response = self.apply_auth(req).json(&body).send().await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(DittoError::Api { status, body: text });
        }

        let parsed = response.json::<CohereChatResponse>().await?;

        let mut content = Vec::<ContentPart>::new();
        for block in &parsed.message.content {
            if block.kind.as_deref() != Some("text") {
                continue;
            }
            let Some(text) = block.text.as_deref().filter(|t| !t.is_empty()) else {
                continue;
            };
            content.push(ContentPart::Text {
                text: text.to_string(),
            });
        }

        if let Some(plan) = parsed
            .message
            .tool_plan
            .as_deref()
            .filter(|t| !t.trim().is_empty())
        {
            content.push(ContentPart::Reasoning {
                text: plan.to_string(),
            });
        }

        for call in &parsed.message.tool_calls {
            let id = call.id.trim();
            let name = call.function.name.trim();
            if id.is_empty() || name.is_empty() {
                warnings.push(Warning::Compatibility {
                    feature: "tool_call".to_string(),
                    details: "cohere response tool_call missing id or name; dropping tool call"
                        .to_string(),
                });
                continue;
            }

            let raw = call.function.arguments.trim();
            let raw_json = if raw.is_empty() { "{}" } else { raw };
            let arguments = serde_json::from_str::<Value>(raw_json).unwrap_or_else(|err| {
                warnings.push(Warning::Compatibility {
                    feature: "tool_call.arguments".to_string(),
                    details: format!(
                        "failed to parse tool_call arguments as JSON for id={id}: {err}; preserving raw string"
                    ),
                });
                Value::String(call.function.arguments.to_string())
            });

            content.push(ContentPart::ToolCall {
                id: id.to_string(),
                name: name.to_string(),
                arguments,
            });
        }

        let finish_reason = Self::map_finish_reason(
            parsed.finish_reason.as_deref(),
            !parsed.message.tool_calls.is_empty(),
        );

        let usage = parsed
            .usage
            .as_ref()
            .map(Self::parse_usage)
            .unwrap_or_default();

        let mut provider_metadata = serde_json::Map::<String, Value>::new();
        provider_metadata.insert("id".to_string(), Value::String(parsed.id.clone()));
        if let Some(model) = parsed.model.as_deref() {
            provider_metadata.insert("model".to_string(), Value::String(model.to_string()));
        }
        if let Some(role) = parsed.message.role.as_deref() {
            provider_metadata.insert("role".to_string(), Value::String(role.to_string()));
        }
        if let Some(citations) = parsed.message.citations.clone() {
            provider_metadata.insert("citations".to_string(), citations);
        }

        Ok(GenerateResponse {
            content,
            finish_reason,
            usage,
            warnings,
            provider_metadata: Some(Value::Object(provider_metadata)),
        })
    }

    async fn stream(&self, request: GenerateRequest) -> Result<StreamResult> {
        #[cfg(not(feature = "streaming"))]
        {
            let _ = request;
            return Err(DittoError::InvalidResponse(
                "ditto-llm built without streaming feature".to_string(),
            ));
        }

        #[cfg(feature = "streaming")]
        {
            let model = self.resolve_model(&request)?;
            let selected_provider_options = request.provider_options_value_for(self.provider())?;

            let (messages, mut warnings) = Self::messages_to_cohere_messages(&request.messages);

            let mut body = serde_json::Map::<String, Value>::new();
            body.insert("model".to_string(), Value::String(model.to_string()));
            body.insert("messages".to_string(), Value::Array(messages));
            body.insert("stream".to_string(), Value::Bool(true));

            if let Some(temperature) = request.temperature {
                if let Some(value) = Self::sanitize_temperature(temperature, &mut warnings) {
                    body.insert("temperature".to_string(), Value::Number(value));
                }
            }
            if let Some(max_tokens) = request.max_tokens {
                body.insert("max_tokens".to_string(), Value::Number(max_tokens.into()));
            }
            if let Some(top_p) = request.top_p {
                if let Some(value) = crate::utils::params::clamped_number_from_f32(
                    "top_p",
                    top_p,
                    0.01,
                    0.99,
                    &mut warnings,
                ) {
                    body.insert("p".to_string(), Value::Number(value));
                }
            }
            if let Some(stops) = request.stop_sequences.as_ref() {
                let stops =
                    crate::utils::params::sanitize_stop_sequences(stops, None, &mut warnings);
                if !stops.is_empty() {
                    body.insert(
                        "stop_sequences".to_string(),
                        Value::Array(stops.into_iter().map(Value::String).collect()),
                    );
                }
            }

            let mut tools = request.tools.unwrap_or_default();
            if !tools.is_empty() {
                if cfg!(feature = "tools") {
                    let tool_choice = request.tool_choice.unwrap_or(ToolChoice::Auto);
                    let (mapped_choice, filtered_tools) =
                        Self::normalize_tool_choice(&tool_choice, Some(&tools), &mut warnings);
                    if let Some(filtered) = filtered_tools {
                        tools = filtered;
                    }

                    let mapped = tools
                        .iter()
                        .map(|t| Self::tool_to_cohere(t, &mut warnings))
                        .collect::<Vec<_>>();
                    body.insert("tools".to_string(), Value::Array(mapped));
                    if let Some(choice) = mapped_choice {
                        body.insert("tool_choice".to_string(), choice);
                    }
                } else {
                    warnings.push(Warning::Unsupported {
                        feature: "tools".to_string(),
                        details: Some("ditto-llm built without tools feature".to_string()),
                    });
                }
            } else if request.tool_choice.is_some() {
                warnings.push(Warning::Unsupported {
                    feature: "tool_choice".to_string(),
                    details: Some(
                        "cohere requires tools to be provided when tool_choice is set".to_string(),
                    ),
                });
            }

            crate::types::merge_provider_options_into_body(
                &mut body,
                selected_provider_options.as_ref(),
                &["reasoning_effort", "response_format", "parallel_tool_calls"],
                "cohere.provider_options",
                &mut warnings,
            );

            let url = self.chat_url();
            let req = self.http.post(url);
            let response = self
                .apply_auth(req)
                .header("Accept", "text/event-stream")
                .json(&body)
                .send()
                .await?;

            let status = response.status();
            if !status.is_success() {
                let text = response.text().await.unwrap_or_default();
                return Err(DittoError::Api { status, body: text });
            }

            let data_stream = crate::utils::sse::sse_data_stream_from_response(response);
            let mut buffer = VecDeque::<Result<StreamChunk>>::new();
            if !warnings.is_empty() {
                buffer.push_back(Ok(StreamChunk::Warnings { warnings }));
            }

            let stream = stream::unfold(
                (
                    data_stream,
                    buffer,
                    false,
                    false,
                    false,
                    false,
                    Vec::<String>::new(),
                    HashMap::<String, String>::new(),
                ),
                |(
                    mut data_stream,
                    mut buffer,
                    mut done,
                    mut has_tool_calls,
                    mut id_sent,
                    mut finish_sent,
                    mut tool_order,
                    mut tool_args,
                )| async move {
                    loop {
                        if let Some(item) = buffer.pop_front() {
                            return Some((
                                item,
                                (
                                    data_stream,
                                    buffer,
                                    done,
                                    has_tool_calls,
                                    id_sent,
                                    finish_sent,
                                    tool_order,
                                    tool_args,
                                ),
                            ));
                        }

                        if done {
                            return None;
                        }

                        let next = data_stream.next().await;
                        match next {
                            Some(Ok(data)) => {
                                let event = match serde_json::from_str::<Value>(&data) {
                                    Ok(event) => event,
                                    Err(err) => {
                                        buffer.push_back(Err(err.into()));
                                        continue;
                                    }
                                };
                                let kind = event.get("type").and_then(Value::as_str).unwrap_or("");

                                match kind {
                                    "message-start" => {
                                        if !id_sent {
                                            if let Some(id) = event
                                                .get("delta")
                                                .and_then(|v| v.get("message"))
                                                .and_then(|v| v.get("id"))
                                                .and_then(Value::as_str)
                                                .filter(|id| !id.trim().is_empty())
                                            {
                                                id_sent = true;
                                                buffer.push_back(Ok(StreamChunk::ResponseId {
                                                    id: id.to_string(),
                                                }));
                                            }
                                        }
                                    }
                                    "content-delta" => {
                                        if let Some(text) = event
                                            .get("delta")
                                            .and_then(|v| v.get("message"))
                                            .and_then(|v| v.get("content"))
                                            .and_then(|v| v.get("text"))
                                            .and_then(Value::as_str)
                                        {
                                            if !text.is_empty() {
                                                buffer.push_back(Ok(StreamChunk::TextDelta {
                                                    text: text.to_string(),
                                                }));
                                            }
                                        }
                                    }
                                    "tool-plan-delta" => {
                                        if let Some(text) = event
                                            .get("delta")
                                            .and_then(|v| v.get("tool_plan"))
                                            .and_then(Value::as_str)
                                        {
                                            if !text.is_empty() {
                                                buffer.push_back(Ok(StreamChunk::ReasoningDelta {
                                                    text: text.to_string(),
                                                }));
                                            }
                                        }
                                    }
                                    "tool-call-start" => {
                                        has_tool_calls = true;
                                        let tool_call =
                                            event.get("delta").and_then(|v| v.get("tool_call"));
                                        let Some(id) = tool_call
                                            .and_then(|v| v.get("id"))
                                            .and_then(Value::as_str)
                                            .filter(|id| !id.trim().is_empty())
                                        else {
                                            continue;
                                        };
                                        let name = tool_call
                                            .and_then(|v| v.get("function"))
                                            .and_then(|v| v.get("name"))
                                            .and_then(Value::as_str)
                                            .unwrap_or("")
                                            .to_string();

                                        if !name.trim().is_empty() {
                                            buffer.push_back(Ok(StreamChunk::ToolCallStart {
                                                id: id.to_string(),
                                                name,
                                            }));
                                        }

                                        let arguments = tool_call
                                            .and_then(|v| v.get("function"))
                                            .and_then(|v| v.get("arguments"))
                                            .and_then(Value::as_str)
                                            .unwrap_or("")
                                            .to_string();
                                        if !tool_args.contains_key(id) {
                                            tool_order.push(id.to_string());
                                        }
                                        tool_args.insert(id.to_string(), arguments);
                                    }
                                    "tool-call-delta" => {
                                        has_tool_calls = true;
                                        let tool_call =
                                            event.get("delta").and_then(|v| v.get("tool_call"));
                                        let Some(id) = tool_call
                                            .and_then(|v| v.get("id"))
                                            .and_then(Value::as_str)
                                            .filter(|id| !id.trim().is_empty())
                                        else {
                                            continue;
                                        };
                                        let arguments = tool_call
                                            .and_then(|v| v.get("function"))
                                            .and_then(|v| v.get("arguments"))
                                            .and_then(Value::as_str)
                                            .unwrap_or("")
                                            .to_string();
                                        if !tool_args.contains_key(id) {
                                            tool_order.push(id.to_string());
                                        }
                                        tool_args.insert(id.to_string(), arguments);
                                    }
                                    "message-end" => {
                                        let reason = event
                                            .get("delta")
                                            .and_then(|v| v.get("finish_reason"))
                                            .and_then(Value::as_str);

                                        for tool_call_id in std::mem::take(&mut tool_order) {
                                            let Some(arguments) = tool_args.remove(&tool_call_id)
                                            else {
                                                continue;
                                            };
                                            if arguments.is_empty() {
                                                continue;
                                            }
                                            buffer.push_back(Ok(StreamChunk::ToolCallDelta {
                                                id: tool_call_id,
                                                arguments_delta: arguments,
                                            }));
                                        }

                                        let finish_reason =
                                            Cohere::map_finish_reason(reason, has_tool_calls);
                                        buffer.push_back(Ok(StreamChunk::FinishReason(
                                            finish_reason,
                                        )));
                                        finish_sent = true;

                                        if let Some(usage) =
                                            event.get("delta").and_then(|v| v.get("usage"))
                                        {
                                            buffer.push_back(Ok(StreamChunk::Usage(
                                                Cohere::parse_usage(usage),
                                            )));
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            Some(Err(err)) => {
                                buffer.push_back(Err(err));
                            }
                            None => {
                                if !finish_sent {
                                    for tool_call_id in std::mem::take(&mut tool_order) {
                                        let Some(arguments) = tool_args.remove(&tool_call_id)
                                        else {
                                            continue;
                                        };
                                        if arguments.is_empty() {
                                            continue;
                                        }
                                        buffer.push_back(Ok(StreamChunk::ToolCallDelta {
                                            id: tool_call_id,
                                            arguments_delta: arguments,
                                        }));
                                    }
                                    buffer.push_back(Ok(StreamChunk::FinishReason(
                                        if has_tool_calls {
                                            FinishReason::ToolCalls
                                        } else {
                                            FinishReason::Unknown
                                        },
                                    )));
                                    finish_sent = true;
                                }
                                done = true;
                            }
                        }
                    }
                },
            )
            .boxed();

            Ok(Box::pin(stream))
        }
    }
}

#[cfg(feature = "embeddings")]
#[derive(Clone)]
pub struct CohereEmbeddings {
    http: reqwest::Client,
    base_url: String,
    auth: Option<RequestAuth>,
    model: String,
    http_query_params: BTreeMap<String, String>,
}

#[cfg(feature = "embeddings")]
impl CohereEmbeddings {
    pub fn new(api_key: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        let api_key = api_key.into();
        let auth = if api_key.trim().is_empty() {
            None
        } else {
            HttpAuth::header_value("authorization", Some("Bearer "), &api_key)
                .ok()
                .map(RequestAuth::Http)
        };

        Self {
            http,
            base_url: DEFAULT_BASE_URL.to_string(),
            auth,
            model: String::new(),
            http_query_params: BTreeMap::new(),
        }
    }

    pub fn with_http_client(mut self, http: reqwest::Client) -> Self {
        self.http = http;
        self
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    pub async fn from_config(config: &ProviderConfig, env: &Env) -> Result<Self> {
        const DEFAULT_KEYS: &[&str] = &["COHERE_API_KEY", "CODE_PM_COHERE_API_KEY"];
        let auth = config
            .auth
            .clone()
            .unwrap_or(ProviderAuth::ApiKeyEnv { keys: Vec::new() });
        let auth_header = resolve_request_auth_with_default_keys(
            &auth,
            env,
            DEFAULT_KEYS,
            "authorization",
            Some("Bearer "),
        )
        .await?;

        let mut out = Self::new("");
        out.auth = Some(auth_header);
        out.http_query_params = config.http_query_params.clone();
        if !config.http_headers.is_empty() {
            out = out.with_http_client(crate::profile::build_http_client(
                std::time::Duration::from_secs(300),
                &config.http_headers,
            )?);
        }
        if let Some(base_url) = config.base_url.as_deref().filter(|s| !s.trim().is_empty()) {
            out = out.with_base_url(base_url);
        }
        if let Some(model) = config
            .default_model
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            out = out.with_model(model);
        }
        Ok(out)
    }

    fn apply_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let req = match self.auth.as_ref() {
            Some(auth) => auth.apply(req),
            None => req,
        };
        apply_http_query_params(req, &self.http_query_params)
    }

    fn embed_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/embed") {
            base.to_string()
        } else {
            format!("{base}/embed")
        }
    }

    fn resolve_model(&self) -> Result<&str> {
        if !self.model.trim().is_empty() {
            return Ok(self.model.as_str());
        }
        Err(DittoError::InvalidResponse(
            "cohere embedding model is not set (set CohereEmbeddings::with_model)".to_string(),
        ))
    }
}

#[cfg(feature = "embeddings")]
#[async_trait]
impl EmbeddingModel for CohereEmbeddings {
    fn provider(&self) -> &str {
        "cohere"
    }

    fn model_id(&self) -> &str {
        self.model.as_str()
    }

    async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        #[derive(Debug, Deserialize)]
        struct EmbedResponse {
            embeddings: EmbedEmbeddings,
        }

        #[derive(Debug, Deserialize)]
        struct EmbedEmbeddings {
            float: Vec<Vec<f32>>,
        }

        let model = self.resolve_model()?;
        let url = self.embed_url();
        let body = json!({
            "model": model,
            "embedding_types": ["float"],
            "texts": texts,
            "input_type": "search_query",
        });

        let mut req = self.http.post(url);
        req = self.apply_auth(req);
        let response = req.json(&body).send().await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(DittoError::Api { status, body: text });
        }

        let parsed = response.json::<EmbedResponse>().await?;
        Ok(parsed.embeddings.float)
    }
}

#[cfg(feature = "rerank")]
#[derive(Clone)]
pub struct CohereRerank {
    http: reqwest::Client,
    base_url: String,
    auth: Option<RequestAuth>,
    default_model: String,
    http_query_params: BTreeMap<String, String>,
}

#[cfg(feature = "rerank")]
impl CohereRerank {
    pub fn new(api_key: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        let api_key = api_key.into();
        let auth = if api_key.trim().is_empty() {
            None
        } else {
            HttpAuth::header_value("authorization", Some("Bearer "), &api_key)
                .ok()
                .map(RequestAuth::Http)
        };

        Self {
            http,
            base_url: DEFAULT_BASE_URL.to_string(),
            auth,
            default_model: String::new(),
            http_query_params: BTreeMap::new(),
        }
    }

    pub fn with_http_client(mut self, http: reqwest::Client) -> Self {
        self.http = http;
        self
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.default_model = model.into();
        self
    }

    pub async fn from_config(config: &ProviderConfig, env: &Env) -> Result<Self> {
        const DEFAULT_KEYS: &[&str] = &["COHERE_API_KEY", "CODE_PM_COHERE_API_KEY"];
        let auth = config
            .auth
            .clone()
            .unwrap_or(ProviderAuth::ApiKeyEnv { keys: Vec::new() });
        let auth_header = resolve_request_auth_with_default_keys(
            &auth,
            env,
            DEFAULT_KEYS,
            "authorization",
            Some("Bearer "),
        )
        .await?;

        let mut out = Self::new("");
        out.auth = Some(auth_header);
        out.http_query_params = config.http_query_params.clone();
        if !config.http_headers.is_empty() {
            out = out.with_http_client(crate::profile::build_http_client(
                std::time::Duration::from_secs(300),
                &config.http_headers,
            )?);
        }
        if let Some(base_url) = config.base_url.as_deref().filter(|s| !s.trim().is_empty()) {
            out = out.with_base_url(base_url);
        }
        if let Some(model) = config
            .default_model
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            out = out.with_model(model);
        }
        Ok(out)
    }

    fn apply_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let req = match self.auth.as_ref() {
            Some(auth) => auth.apply(req),
            None => req,
        };
        apply_http_query_params(req, &self.http_query_params)
    }

    fn rerank_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/rerank") {
            base.to_string()
        } else {
            format!("{base}/rerank")
        }
    }

    fn resolve_model<'a>(&'a self, request: &'a RerankRequest) -> Result<&'a str> {
        if let Some(model) = request.model.as_deref().filter(|m| !m.trim().is_empty()) {
            return Ok(model);
        }
        if !self.default_model.trim().is_empty() {
            return Ok(self.default_model.as_str());
        }
        Err(DittoError::InvalidResponse(
            "cohere rerank model is not set (set request.model or CohereRerank::with_model)"
                .to_string(),
        ))
    }
}

#[cfg(feature = "rerank")]
#[derive(Debug, Deserialize, Default)]
struct CohereRerankOptions {
    #[serde(default, alias = "maxTokensPerDoc", alias = "max_tokens_per_doc")]
    max_tokens_per_doc: Option<u32>,
    #[serde(default)]
    priority: Option<u32>,
}

#[cfg(feature = "rerank")]
impl CohereRerankOptions {
    fn from_value(value: &Value) -> Result<Self> {
        serde_json::from_value::<Self>(value.clone()).map_err(|err| {
            DittoError::InvalidResponse(format!(
                "invalid provider_options for cohere rerank: {err}"
            ))
        })
    }
}

#[cfg(feature = "rerank")]
#[async_trait]
impl RerankModel for CohereRerank {
    fn provider(&self) -> &str {
        "cohere"
    }

    fn model_id(&self) -> &str {
        self.default_model.as_str()
    }

    async fn rerank(&self, request: RerankRequest) -> Result<RerankResponse> {
        #[derive(Debug, Deserialize)]
        struct WireResponse {
            #[serde(default)]
            id: Option<String>,
            #[serde(default)]
            results: Vec<WireResult>,
            #[serde(default)]
            meta: Value,
        }

        #[derive(Debug, Deserialize)]
        struct WireResult {
            index: u32,
            relevance_score: f32,
        }

        let model = self.resolve_model(&request)?.to_string();

        let selected_provider_options = crate::types::select_provider_options_value(
            request.provider_options.as_ref(),
            self.provider(),
        )?;
        let options = selected_provider_options
            .as_ref()
            .map(CohereRerankOptions::from_value)
            .transpose()?
            .unwrap_or_default();

        let mut warnings = Vec::<Warning>::new();
        let has_object_documents = request
            .documents
            .iter()
            .any(|doc| !matches!(doc, RerankDocument::Text(_)));
        if has_object_documents {
            warnings.push(Warning::Compatibility {
                feature: "cohere.rerank.object_documents".to_string(),
                details: "object documents are converted to strings".to_string(),
            });
        }

        let documents = request
            .documents
            .into_iter()
            .map(|doc| match doc {
                RerankDocument::Text(text) => Ok(text),
                RerankDocument::Json(value) => Ok(serde_json::to_string(&value)?),
            })
            .collect::<Result<Vec<_>>>()?;

        let url = self.rerank_url();
        let body = json!({
            "model": model,
            "query": request.query,
            "documents": documents,
            "top_n": request.top_n,
            "max_tokens_per_doc": options.max_tokens_per_doc,
            "priority": options.priority,
        });

        let mut req = self.http.post(url);
        req = self.apply_auth(req);
        let response = req.json(&body).send().await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(DittoError::Api { status, body: text });
        }

        let parsed = response.json::<WireResponse>().await?;
        Ok(RerankResponse {
            ranking: parsed
                .results
                .into_iter()
                .map(|result| RerankResult {
                    index: result.index,
                    relevance_score: result.relevance_score,
                    provider_metadata: None,
                })
                .collect(),
            warnings,
            provider_metadata: Some(json!({ "id": parsed.id, "meta": parsed.meta })),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::{Method::POST, MockServer};

    #[tokio::test]
    async fn chat_posts_and_parses_text_and_tool_calls() -> Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v2/chat")
                    .header("authorization", "Bearer sk-test")
                    .body_includes("\"model\":\"command-r\"")
                    .body_includes("\"messages\"")
                    .body_includes("\"tools\"")
                    .body_includes("\"parameter_definitions\"")
                    .body_includes("\"tool_choice\":\"REQUIRED\"")
                    .body_includes("\"tool_call_id\":\"call_1\"")
                    .body_includes("\"tool_calls\"");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(
                        serde_json::json!({
                            "id": "chat_123",
                            "finish_reason": "COMPLETE",
                            "message": {
                                "role": "assistant",
                                "content": [
                                    { "type": "text", "text": "hello" }
                                ],
                                "tool_calls": [
                                    {
                                        "id": "call_2",
                                        "type": "tool_call",
                                        "function": { "name": "get_weather", "arguments": "{\"city\":\"sf\"}" }
                                    }
                                ]
                            },
                            "usage": { "tokens": { "input_tokens": 10, "output_tokens": 5 } }
                        })
                        .to_string(),
                    );
            })
            .await;

        let config = ProviderConfig {
            base_url: Some(server.url("/v2")),
            default_model: Some("command-r".to_string()),
            auth: Some(crate::ProviderAuth::ApiKeyEnv {
                keys: vec!["CODEPM_TEST_COHERE_KEY".to_string()],
            }),
            ..ProviderConfig::default()
        };
        let env = Env {
            dotenv: BTreeMap::from([("CODEPM_TEST_COHERE_KEY".to_string(), "sk-test".to_string())]),
        };

        let tool = Tool {
            name: "get_weather".to_string(),
            description: Some("get weather".to_string()),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "city": { "type": "string", "description": "city name" }
                },
                "required": ["city"]
            }),
            strict: None,
        };

        let client = Cohere::from_config(&config, &env).await?;
        let response = client
            .generate(GenerateRequest {
                messages: vec![
                    Message::user("hi"),
                    Message {
                        role: Role::Assistant,
                        content: vec![ContentPart::ToolCall {
                            id: "call_1".to_string(),
                            name: "get_weather".to_string(),
                            arguments: serde_json::json!({ "city": "sf" }),
                        }],
                    },
                    Message::tool_result("call_1", "sunny"),
                ],
                model: None,
                temperature: None,
                max_tokens: None,
                top_p: None,
                stop_sequences: None,
                tools: Some(vec![tool]),
                tool_choice: Some(ToolChoice::Required),
                provider_options: None,
            })
            .await?;

        mock.assert_async().await;
        assert!(matches!(response.finish_reason, FinishReason::ToolCalls));
        assert!(response.content.iter().any(|part| matches!(
            part,
            ContentPart::Text { text } if text == "hello"
        )));
        assert!(response.content.iter().any(|part| matches!(
            part,
            ContentPart::ToolCall { id, name, arguments } if id == "call_2" && name == "get_weather" && arguments.get("city").and_then(Value::as_str) == Some("sf")
        )));
        Ok(())
    }

    #[cfg(feature = "streaming")]
    #[tokio::test]
    async fn stream_parses_text_and_tool_call_deltas() -> Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;

        let sse = concat!(
            "data: {\"type\":\"message-start\",\"delta\":{\"message\":{\"id\":\"chat_123\"}}}\n\n",
            "data: {\"type\":\"content-delta\",\"delta\":{\"message\":{\"content\":{\"text\":\"Hello\"}}}}\n\n",
            "data: {\"type\":\"tool-call-start\",\"delta\":{\"tool_call\":{\"id\":\"call_1\",\"type\":\"tool_call\",\"function\":{\"name\":\"get_weather\",\"arguments\":\"{\\\"city\\\":\\\"sf\\\"}\"}}}}\n\n",
            "data: {\"type\":\"tool-call-delta\",\"delta\":{\"tool_call\":{\"id\":\"call_1\",\"type\":\"tool_call\",\"function\":{\"arguments\":\"{\\\"city\\\":\\\"sf\\\",\\\"unit\\\":\\\"c\\\"}\"}}}}\n\n",
            "data: {\"type\":\"message-end\",\"delta\":{\"finish_reason\":\"COMPLETE\",\"usage\":{\"tokens\":{\"input_tokens\":10,\"output_tokens\":5}}}}\n\n",
        );

        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v2/chat")
                    .header("authorization", "Bearer sk-test")
                    .body_includes("\"stream\":true");
                then.status(200)
                    .header("content-type", "text/event-stream")
                    .body(sse);
            })
            .await;

        let client = Cohere::new("sk-test")
            .with_base_url(server.url("/v2"))
            .with_model("command-r");

        let stream = client
            .stream(GenerateRequest::from(vec![Message::user("hi")]))
            .await?;
        let collected = crate::collect_stream(stream).await?;

        mock.assert_async().await;

        assert_eq!(collected.response_id.as_deref(), Some("chat_123"));
        assert!(matches!(
            collected.response.finish_reason,
            FinishReason::ToolCalls
        ));
        assert_eq!(collected.response.usage.input_tokens, Some(10));
        assert_eq!(collected.response.usage.output_tokens, Some(5));

        assert!(
            collected
                .response
                .content
                .iter()
                .any(|part| matches!(part, ContentPart::Text { text } if text == "Hello"))
        );
        assert!(collected.response.content.iter().any(|part| matches!(
            part,
            ContentPart::ToolCall { id, name, arguments } if id == "call_1" && name == "get_weather" && arguments.get("city").and_then(Value::as_str) == Some("sf") && arguments.get("unit").and_then(Value::as_str) == Some("c")
        )));

        Ok(())
    }

    #[cfg(feature = "rerank")]
    #[tokio::test]
    async fn rerank_posts_and_parses_results() -> Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v2/rerank")
                    .header("authorization", "Bearer sk-test")
                    .body_includes("\"model\":\"rerank-v3.5\"")
                    .body_includes("\"query\":\"hello\"")
                    .body_includes("\"top_n\":2")
                    .body_includes("\"documents\":[\"a\",\"b\"]");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(
                        serde_json::json!({
                            "id": "rr-123",
                            "results": [
                                { "index": 0, "relevance_score": 0.9 },
                                { "index": 1, "relevance_score": 0.1 }
                            ],
                            "meta": { "billed_units": { "search_units": 1 } }
                        })
                        .to_string(),
                    );
            })
            .await;

        let config = ProviderConfig {
            base_url: Some(server.url("/v2")),
            default_model: Some("rerank-v3.5".to_string()),
            auth: Some(crate::ProviderAuth::ApiKeyEnv {
                keys: vec!["CODEPM_TEST_COHERE_KEY".to_string()],
            }),
            ..ProviderConfig::default()
        };
        let env = Env {
            dotenv: BTreeMap::from([("CODEPM_TEST_COHERE_KEY".to_string(), "sk-test".to_string())]),
        };

        let client = CohereRerank::from_config(&config, &env).await?;
        let response = client
            .rerank(RerankRequest {
                query: "hello".to_string(),
                documents: vec![
                    RerankDocument::Text("a".to_string()),
                    RerankDocument::Text("b".to_string()),
                ],
                model: None,
                top_n: Some(2),
                provider_options: None,
            })
            .await?;

        mock.assert_async().await;
        assert_eq!(response.ranking.len(), 2);
        assert_eq!(response.ranking[0].index, 0);
        assert_eq!(response.ranking[0].relevance_score, 0.9);
        assert_eq!(response.ranking[1].index, 1);
        assert_eq!(response.ranking[1].relevance_score, 0.1);
        Ok(())
    }

    #[cfg(feature = "rerank")]
    #[tokio::test]
    async fn rerank_warns_on_object_documents() -> Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v2/rerank")
                    .body_includes("\\\"answer\\\":42");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(
                        serde_json::json!({
                            "id": "rr-123",
                            "results": [],
                            "meta": {}
                        })
                        .to_string(),
                    );
            })
            .await;

        let client = CohereRerank::new("sk-test")
            .with_base_url(server.url("/v2"))
            .with_model("rerank-v3.5");

        let response = client
            .rerank(RerankRequest {
                query: "hello".to_string(),
                documents: vec![RerankDocument::Json(serde_json::json!({ "answer": 42 }))],
                model: None,
                top_n: None,
                provider_options: None,
            })
            .await?;

        mock.assert_async().await;
        assert!(
            response
                .warnings
                .iter()
                .any(|warning| matches!(warning, Warning::Compatibility { feature, .. } if feature == "cohere.rerank.object_documents"))
        );
        Ok(())
    }
}
