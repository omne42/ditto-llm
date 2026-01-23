use std::collections::VecDeque;

use async_trait::async_trait;
use futures_util::StreamExt;
use futures_util::stream;
use serde::Deserialize;
use serde_json::{Map, Value};

#[cfg(feature = "embeddings")]
use crate::embedding::EmbeddingModel;
use crate::model::{LanguageModel, StreamResult};
use crate::profile::{Env, ProviderAuth, ProviderConfig, resolve_auth_token_with_default_keys};
use crate::types::{
    ContentPart, FinishReason, GenerateRequest, GenerateResponse, ImageSource, Message, Role,
    StreamChunk, Tool, ToolChoice, Usage, Warning,
};
use crate::{DittoError, Result};

#[derive(Clone)]
pub struct OpenAI {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    default_model: String,
}

impl OpenAI {
    pub fn new(api_key: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .expect("reqwest client build should not fail");

        Self {
            http,
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: api_key.into(),
            default_model: String::new(),
        }
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
        const DEFAULT_KEYS: &[&str] = &["OPENAI_API_KEY", "CODE_PM_OPENAI_API_KEY"];
        let auth = config
            .auth
            .clone()
            .unwrap_or(ProviderAuth::ApiKeyEnv { keys: Vec::new() });
        let api_key = resolve_auth_token_with_default_keys(&auth, env, DEFAULT_KEYS).await?;

        let mut out = Self::new(api_key);
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

    fn responses_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/responses") {
            base.to_string()
        } else {
            format!("{base}/responses")
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
            "openai model is not set (set request.model or OpenAI::with_model)".to_string(),
        ))
    }

    fn tool_to_openai(tool: &Tool) -> Value {
        let mut out = Map::<String, Value>::new();
        out.insert("type".to_string(), Value::String("function".to_string()));
        out.insert("name".to_string(), Value::String(tool.name.clone()));
        if let Some(description) = &tool.description {
            out.insert(
                "description".to_string(),
                Value::String(description.clone()),
            );
        }
        out.insert("parameters".to_string(), tool.parameters.clone());
        if let Some(strict) = tool.strict {
            out.insert("strict".to_string(), Value::Bool(strict));
        }
        Value::Object(out)
    }

    fn tool_choice_to_openai(choice: &ToolChoice) -> Value {
        match choice {
            ToolChoice::Auto => Value::String("auto".to_string()),
            ToolChoice::None => Value::String("none".to_string()),
            ToolChoice::Required => Value::String("required".to_string()),
            ToolChoice::Tool { name } => serde_json::json!({ "type": "function", "name": name }),
        }
    }

    fn messages_to_input(messages: &[Message]) -> (Vec<Value>, Vec<Warning>) {
        let mut input = Vec::<Value>::new();
        let mut warnings = Vec::<Warning>::new();

        for message in messages {
            match message.role {
                Role::System => {
                    let mut text = String::new();
                    for part in &message.content {
                        match part {
                            ContentPart::Text { text: chunk } => text.push_str(chunk),
                            other => warnings.push(Warning::Unsupported {
                                feature: "system_content_part".to_string(),
                                details: Some(format!(
                                    "unsupported system content part: {other:?}"
                                )),
                            }),
                        }
                    }
                    if text.trim().is_empty() {
                        continue;
                    }
                    input.push(serde_json::json!({ "role": "system", "content": text }));
                }
                Role::User => {
                    let mut content = Vec::<Value>::new();
                    for part in &message.content {
                        match part {
                            ContentPart::Text { text } => {
                                if text.is_empty() {
                                    continue;
                                }
                                content.push(
                                    serde_json::json!({ "type": "input_text", "text": text }),
                                );
                            }
                            ContentPart::Image { source } => {
                                let image_url = match source {
                                    ImageSource::Url { url } => url.clone(),
                                    ImageSource::Base64 { media_type, data } => {
                                        format!("data:{media_type};base64,{data}")
                                    }
                                };
                                content.push(serde_json::json!({ "type": "input_image", "image_url": image_url }));
                            }
                            other => warnings.push(Warning::Unsupported {
                                feature: "user_content_part".to_string(),
                                details: Some(format!("unsupported user content part: {other:?}")),
                            }),
                        }
                    }
                    if content.is_empty() {
                        continue;
                    }
                    input.push(serde_json::json!({ "role": "user", "content": content }));
                }
                Role::Assistant => {
                    let mut content = Vec::<Value>::new();
                    for part in &message.content {
                        match part {
                            ContentPart::Text { text } => {
                                if text.is_empty() {
                                    continue;
                                }
                                content.push(
                                    serde_json::json!({ "type": "output_text", "text": text }),
                                );
                            }
                            ContentPart::ToolCall {
                                id,
                                name,
                                arguments,
                            } => {
                                if !content.is_empty() {
                                    input.push(serde_json::json!({ "role": "assistant", "content": content }));
                                    content = Vec::new();
                                }
                                input.push(serde_json::json!({
                                    "type": "function_call",
                                    "call_id": id,
                                    "name": name,
                                    "arguments": arguments.to_string(),
                                }));
                            }
                            ContentPart::Reasoning { .. } => warnings.push(Warning::Unsupported {
                                feature: "reasoning".to_string(),
                                details: Some(
                                    "reasoning parts are not sent to OpenAI input".to_string(),
                                ),
                            }),
                            other => warnings.push(Warning::Unsupported {
                                feature: "assistant_content_part".to_string(),
                                details: Some(format!(
                                    "unsupported assistant content part: {other:?}"
                                )),
                            }),
                        }
                    }
                    if !content.is_empty() {
                        input.push(serde_json::json!({ "role": "assistant", "content": content }));
                    }
                }
                Role::Tool => {
                    for part in &message.content {
                        match part {
                            ContentPart::ToolResult {
                                tool_call_id,
                                content,
                                ..
                            } => {
                                input.push(serde_json::json!({
                                    "type": "function_call_output",
                                    "call_id": tool_call_id,
                                    "output": content,
                                }));
                            }
                            other => warnings.push(Warning::Unsupported {
                                feature: "tool_content_part".to_string(),
                                details: Some(format!("unsupported tool content part: {other:?}")),
                            }),
                        }
                    }
                }
            }
        }

        (input, warnings)
    }

    fn parse_usage(value: &Value) -> Usage {
        let mut usage = Usage::default();
        if let Some(obj) = value.as_object() {
            usage.input_tokens = obj.get("input_tokens").and_then(Value::as_u64);
            usage.output_tokens = obj.get("output_tokens").and_then(Value::as_u64);
            usage.total_tokens = obj.get("total_tokens").and_then(Value::as_u64);
        }
        usage.merge_total();
        usage
    }
}

fn apply_provider_options(
    body: &mut Map<String, Value>,
    provider_options: &crate::types::ProviderOptions,
) -> Result<()> {
    if let Some(effort) = provider_options.reasoning_effort {
        body.insert(
            "reasoning".to_string(),
            serde_json::json!({ "effort": effort }),
        );
    }
    if let Some(response_format) = provider_options.response_format.as_ref() {
        body.insert(
            "response_format".to_string(),
            serde_json::to_value(response_format)?,
        );
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct ResponsesApiResponse {
    id: String,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    incomplete_details: Option<Value>,
    #[serde(default)]
    output: Vec<Value>,
    #[serde(default)]
    usage: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ResponsesStreamEvent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    response: Option<Value>,
    #[serde(default)]
    item: Option<Value>,
    #[serde(default)]
    delta: Option<String>,
}

fn map_responses_finish_reason(
    status: Option<&str>,
    incomplete_reason: Option<&str>,
    has_tool_calls: bool,
) -> FinishReason {
    match status {
        Some("completed") | Some("done") => {
            if has_tool_calls {
                FinishReason::ToolCalls
            } else {
                FinishReason::Stop
            }
        }
        Some("incomplete") => match incomplete_reason {
            Some("max_output_tokens") | Some("max_tokens") => FinishReason::Length,
            Some("content_filter") | Some("content_filtered") => FinishReason::ContentFilter,
            Some("tool_calls") => FinishReason::ToolCalls,
            _ => FinishReason::Length,
        },
        Some("failed") | Some("cancelled") | Some("canceled") | Some("error") => {
            FinishReason::Error
        }
        _ => FinishReason::Unknown,
    }
}

fn finish_reason_for_final_event(
    event_kind: &str,
    response: Option<&Value>,
    has_tool_calls: bool,
) -> FinishReason {
    let response_status = response.and_then(|resp| resp.get("status").and_then(Value::as_str));
    let response_incomplete_reason = response
        .and_then(|resp| resp.get("incomplete_details"))
        .and_then(|details| details.get("reason"))
        .and_then(Value::as_str);

    let status = response_status.or(match event_kind {
        "response.incomplete" => Some("incomplete"),
        "response.completed" | "response.done" => Some("completed"),
        _ => None,
    });

    map_responses_finish_reason(status, response_incomplete_reason, has_tool_calls)
}

fn parse_openai_output(output: &[Value]) -> Vec<ContentPart> {
    let mut content = Vec::<ContentPart>::new();

    for item in output {
        let Some(kind) = item.get("type").and_then(Value::as_str) else {
            continue;
        };
        match kind {
            "message" => {
                let Some(parts) = item.get("content").and_then(Value::as_array) else {
                    continue;
                };
                for part in parts {
                    if part.get("type").and_then(Value::as_str) != Some("output_text") {
                        continue;
                    }
                    let Some(text) = part.get("text").and_then(Value::as_str) else {
                        continue;
                    };
                    if text.is_empty() {
                        continue;
                    }
                    content.push(ContentPart::Text {
                        text: text.to_string(),
                    });
                }
            }
            "function_call" => {
                let Some(call_id) = item.get("call_id").and_then(Value::as_str) else {
                    continue;
                };
                let Some(name) = item.get("name").and_then(Value::as_str) else {
                    continue;
                };
                let arguments_raw = item.get("arguments").and_then(Value::as_str).unwrap_or("");
                let arguments = serde_json::from_str::<Value>(arguments_raw)
                    .unwrap_or_else(|_| Value::String(arguments_raw.to_string()));
                content.push(ContentPart::ToolCall {
                    id: call_id.to_string(),
                    name: name.to_string(),
                    arguments,
                });
            }
            _ => {}
        }
    }

    content
}

#[async_trait]
impl LanguageModel for OpenAI {
    fn provider(&self) -> &str {
        "openai"
    }

    fn model_id(&self) -> &str {
        self.default_model.as_str()
    }

    async fn generate(&self, request: GenerateRequest) -> Result<GenerateResponse> {
        let model = self.resolve_model(&request)?;
        let provider_options = request.parsed_provider_options()?.unwrap_or_default();
        let (input, mut warnings) = Self::messages_to_input(&request.messages);

        if request.stop_sequences.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "stop_sequences".to_string(),
                details: Some("OpenAI Responses API stop sequences are not supported".to_string()),
            });
        }

        let mut body = Map::<String, Value>::new();
        body.insert("model".to_string(), Value::String(model.to_string()));
        body.insert("input".to_string(), Value::Array(input));
        body.insert("stream".to_string(), Value::Bool(false));
        body.insert("store".to_string(), Value::Bool(false));

        if let Some(temperature) = request.temperature {
            body.insert(
                "temperature".to_string(),
                Value::Number(
                    serde_json::Number::from_f64(temperature as f64).unwrap_or_else(|| 0.into()),
                ),
            );
        }
        if let Some(max_tokens) = request.max_tokens {
            body.insert(
                "max_output_tokens".to_string(),
                Value::Number(max_tokens.into()),
            );
        }
        if let Some(top_p) = request.top_p {
            body.insert(
                "top_p".to_string(),
                Value::Number(
                    serde_json::Number::from_f64(top_p as f64).unwrap_or_else(|| 0.into()),
                ),
            );
        }

        if let Some(tools) = request.tools {
            if cfg!(feature = "tools") {
                let mapped = tools
                    .into_iter()
                    .map(|t| Self::tool_to_openai(&t))
                    .collect();
                body.insert("tools".to_string(), Value::Array(mapped));
            } else {
                warnings.push(Warning::Unsupported {
                    feature: "tools".to_string(),
                    details: Some("ditto-llm built without tools feature".to_string()),
                });
            }
        }
        if let Some(tool_choice) = request.tool_choice {
            if cfg!(feature = "tools") {
                body.insert(
                    "tool_choice".to_string(),
                    Self::tool_choice_to_openai(&tool_choice),
                );
            } else {
                warnings.push(Warning::Unsupported {
                    feature: "tool_choice".to_string(),
                    details: Some("ditto-llm built without tools feature".to_string()),
                });
            }
        }

        apply_provider_options(&mut body, &provider_options)?;

        let url = self.responses_url();
        let response = self
            .http
            .post(url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(DittoError::Api { status, body: text });
        }

        let parsed = response.json::<ResponsesApiResponse>().await?;
        let content = parse_openai_output(&parsed.output);
        let has_tool_calls = content
            .iter()
            .any(|part| matches!(part, ContentPart::ToolCall { .. }));
        let usage = parsed
            .usage
            .as_ref()
            .map(Self::parse_usage)
            .unwrap_or_default();
        let finish_reason = map_responses_finish_reason(
            parsed.status.as_deref(),
            parsed
                .incomplete_details
                .as_ref()
                .and_then(|details| details.get("reason"))
                .and_then(Value::as_str),
            has_tool_calls,
        );

        Ok(GenerateResponse {
            content,
            finish_reason,
            usage,
            warnings,
            provider_metadata: Some(serde_json::json!({ "id": parsed.id })),
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
            let provider_options = request.parsed_provider_options()?.unwrap_or_default();
            let (input, mut warnings) = Self::messages_to_input(&request.messages);

            let mut body = Map::<String, Value>::new();
            body.insert("model".to_string(), Value::String(model.to_string()));
            body.insert("input".to_string(), Value::Array(input));
            body.insert("stream".to_string(), Value::Bool(true));
            body.insert("store".to_string(), Value::Bool(false));

            if request.stop_sequences.is_some() {
                warnings.push(Warning::Unsupported {
                    feature: "stop_sequences".to_string(),
                    details: Some(
                        "OpenAI Responses API stop sequences are not supported".to_string(),
                    ),
                });
            }

            if let Some(temperature) = request.temperature {
                body.insert(
                    "temperature".to_string(),
                    Value::Number(
                        serde_json::Number::from_f64(temperature as f64)
                            .unwrap_or_else(|| 0.into()),
                    ),
                );
            }
            if let Some(max_tokens) = request.max_tokens {
                body.insert(
                    "max_output_tokens".to_string(),
                    Value::Number(max_tokens.into()),
                );
            }
            if let Some(top_p) = request.top_p {
                body.insert(
                    "top_p".to_string(),
                    Value::Number(
                        serde_json::Number::from_f64(top_p as f64).unwrap_or_else(|| 0.into()),
                    ),
                );
            }

            if let Some(tools) = request.tools {
                if cfg!(feature = "tools") {
                    let mapped = tools
                        .into_iter()
                        .map(|t| Self::tool_to_openai(&t))
                        .collect();
                    body.insert("tools".to_string(), Value::Array(mapped));
                } else {
                    warnings.push(Warning::Unsupported {
                        feature: "tools".to_string(),
                        details: Some("ditto-llm built without tools feature".to_string()),
                    });
                }
            }
            if let Some(tool_choice) = request.tool_choice {
                if cfg!(feature = "tools") {
                    body.insert(
                        "tool_choice".to_string(),
                        Self::tool_choice_to_openai(&tool_choice),
                    );
                } else {
                    warnings.push(Warning::Unsupported {
                        feature: "tool_choice".to_string(),
                        details: Some("ditto-llm built without tools feature".to_string()),
                    });
                }
            }

            apply_provider_options(&mut body, &provider_options)?;

            let url = self.responses_url();
            let response = self
                .http
                .post(url)
                .bearer_auth(&self.api_key)
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
                (data_stream, buffer, false, false),
                |(mut data_stream, mut buffer, mut done, mut has_tool_calls)| async move {
                    loop {
                        if let Some(item) = buffer.pop_front() {
                            return Some((item, (data_stream, buffer, done, has_tool_calls)));
                        }

                        if done {
                            return None;
                        }

                        let next = data_stream.next().await;
                        match next {
                            Some(Ok(data)) => {
                                match serde_json::from_str::<ResponsesStreamEvent>(&data) {
                                    Ok(event) => match event.kind.as_str() {
                                        "response.output_text.delta" => {
                                            if let Some(delta) = event.delta {
                                                buffer.push_back(Ok(StreamChunk::TextDelta {
                                                    text: delta,
                                                }));
                                            }
                                        }
                                        "response.reasoning_text.delta" => {
                                            if let Some(delta) = event.delta {
                                                buffer.push_back(Ok(StreamChunk::ReasoningDelta {
                                                    text: delta,
                                                }));
                                            }
                                        }
                                        "response.output_item.done" => {
                                            let Some(item) = event.item else {
                                                continue;
                                            };
                                            if item.get("type").and_then(Value::as_str)
                                                != Some("function_call")
                                            {
                                                continue;
                                            }
                                            has_tool_calls = true;
                                            let Some(call_id) =
                                                item.get("call_id").and_then(Value::as_str)
                                            else {
                                                continue;
                                            };
                                            let Some(name) =
                                                item.get("name").and_then(Value::as_str)
                                            else {
                                                continue;
                                            };
                                            let arguments = item
                                                .get("arguments")
                                                .and_then(Value::as_str)
                                                .unwrap_or("")
                                                .to_string();
                                            buffer.push_back(Ok(StreamChunk::ToolCallStart {
                                                id: call_id.to_string(),
                                                name: name.to_string(),
                                            }));
                                            buffer.push_back(Ok(StreamChunk::ToolCallDelta {
                                                id: call_id.to_string(),
                                                arguments_delta: arguments,
                                            }));
                                        }
                                        "response.failed" => {
                                            done = true;
                                            buffer.push_back(Err(DittoError::InvalidResponse(
                                                event
                                                    .response
                                                    .map(|v| v.to_string())
                                                    .unwrap_or_else(|| {
                                                        "openai response.failed".to_string()
                                                    }),
                                            )));
                                        }
                                        "response.completed"
                                        | "response.done"
                                        | "response.incomplete" => {
                                            done = true;
                                            if let Some(resp) = event.response {
                                                if let Some(usage) = resp.get("usage") {
                                                    buffer.push_back(Ok(StreamChunk::Usage(
                                                        Self::parse_usage(usage),
                                                    )));
                                                }
                                                let finish_reason = finish_reason_for_final_event(
                                                    &event.kind,
                                                    Some(&resp),
                                                    has_tool_calls,
                                                );
                                                buffer.push_back(Ok(StreamChunk::FinishReason(
                                                    finish_reason,
                                                )));
                                            } else {
                                                let finish_reason = finish_reason_for_final_event(
                                                    &event.kind,
                                                    None,
                                                    has_tool_calls,
                                                );
                                                buffer.push_back(Ok(StreamChunk::FinishReason(
                                                    finish_reason,
                                                )));
                                            }
                                        }
                                        _ => {}
                                    },
                                    Err(err) => {
                                        done = true;
                                        buffer.push_back(Err(err.into()));
                                    }
                                }
                            }
                            Some(Err(err)) => {
                                done = true;
                                buffer.push_back(Err(err));
                            }
                            None => return None,
                        }
                    }
                },
            );

            Ok(Box::pin(stream))
        }
    }
}

#[cfg(feature = "embeddings")]
#[derive(Clone)]
pub struct OpenAIEmbeddings {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
}

#[cfg(feature = "embeddings")]
impl OpenAIEmbeddings {
    pub fn new(api_key: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .expect("reqwest client build should not fail");

        Self {
            http,
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: api_key.into(),
            model: String::new(),
        }
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
        const DEFAULT_KEYS: &[&str] = &["OPENAI_API_KEY", "CODE_PM_OPENAI_API_KEY"];
        let auth = config
            .auth
            .clone()
            .unwrap_or(ProviderAuth::ApiKeyEnv { keys: Vec::new() });
        let api_key = resolve_auth_token_with_default_keys(&auth, env, DEFAULT_KEYS).await?;

        let mut out = Self::new(api_key);
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

    fn resolve_model(&self) -> Result<&str> {
        if !self.model.trim().is_empty() {
            return Ok(self.model.as_str());
        }
        Err(DittoError::InvalidResponse(
            "openai embedding model is not set (set OpenAIEmbeddings::with_model)".to_string(),
        ))
    }
}

#[cfg(feature = "embeddings")]
#[derive(Debug, Deserialize)]
struct EmbeddingsResponse {
    #[serde(default)]
    data: Vec<EmbeddingsItem>,
}

#[cfg(feature = "embeddings")]
#[derive(Debug, Deserialize)]
struct EmbeddingsItem {
    embedding: Vec<f32>,
}

#[cfg(feature = "embeddings")]
#[async_trait]
impl EmbeddingModel for OpenAIEmbeddings {
    fn provider(&self) -> &str {
        "openai"
    }

    fn model_id(&self) -> &str {
        self.model.as_str()
    }

    async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let model = self.resolve_model()?;

        let url = {
            let base = self.base_url.trim_end_matches('/');
            if base.ends_with("/embeddings") {
                base.to_string()
            } else {
                format!("{base}/embeddings")
            }
        };

        let response = self
            .http
            .post(url)
            .bearer_auth(&self.api_key)
            .json(&serde_json::json!({ "model": model, "input": texts }))
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(DittoError::Api { status, body: text });
        }

        let parsed = response.json::<EmbeddingsResponse>().await?;
        Ok(parsed.data.into_iter().map(|item| item.embedding).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn from_config_resolves_api_key_and_model() -> crate::Result<()> {
        let config = ProviderConfig {
            base_url: Some("http://localhost:1234/v1".to_string()),
            default_model: Some("test-model".to_string()),
            auth: Some(crate::ProviderAuth::ApiKeyEnv {
                keys: vec!["CODEPM_TEST_OPENAI_KEY".to_string()],
            }),
            ..ProviderConfig::default()
        };
        let env = Env {
            dotenv: std::collections::BTreeMap::from([(
                "CODEPM_TEST_OPENAI_KEY".to_string(),
                "sk-test".to_string(),
            )]),
        };

        let client = OpenAI::from_config(&config, &env).await?;
        assert_eq!(client.provider(), "openai");
        assert_eq!(client.model_id(), "test-model");
        Ok(())
    }

    #[test]
    fn converts_messages_to_responses_input() {
        let messages = vec![
            Message::system("sys"),
            Message::user("hi"),
            Message {
                role: Role::Assistant,
                content: vec![ContentPart::ToolCall {
                    id: "c1".to_string(),
                    name: "add".to_string(),
                    arguments: json!({ "a": 1, "b": 2 }),
                }],
            },
            Message {
                role: Role::Tool,
                content: vec![ContentPart::ToolResult {
                    tool_call_id: "c1".to_string(),
                    content: "{\"result\":3}".to_string(),
                    is_error: None,
                }],
            },
        ];

        let (input, warnings) = OpenAI::messages_to_input(&messages);
        assert!(warnings.is_empty());
        assert_eq!(input.len(), 4);
        assert_eq!(input[0].get("role").and_then(Value::as_str), Some("system"));
        assert_eq!(input[1].get("role").and_then(Value::as_str), Some("user"));
        assert_eq!(
            input[2].get("type").and_then(Value::as_str),
            Some("function_call")
        );
        assert_eq!(
            input[3].get("type").and_then(Value::as_str),
            Some("function_call_output")
        );
    }

    #[test]
    fn parses_function_call_from_output() {
        let output = vec![serde_json::json!({
            "type": "function_call",
            "call_id": "c1",
            "name": "add",
            "arguments": "{\"a\":1,\"b\":2}"
        })];

        let parsed = parse_openai_output(&output);
        assert_eq!(parsed.len(), 1);

        match &parsed[0] {
            ContentPart::ToolCall {
                id,
                name,
                arguments,
            } => {
                assert_eq!(id, "c1");
                assert_eq!(name, "add");
                assert_eq!(arguments.get("a").and_then(Value::as_i64), Some(1));
            }
            other => panic!("unexpected part: {other:?}"),
        }
    }

    #[test]
    fn apply_provider_options_maps_reasoning_and_response_format() -> crate::Result<()> {
        let mut body = Map::<String, Value>::new();
        let options = crate::ProviderOptions {
            reasoning_effort: Some(crate::ReasoningEffort::High),
            response_format: Some(crate::ResponseFormat::JsonSchema {
                json_schema: crate::JsonSchemaFormat {
                    name: "unit_test".to_string(),
                    schema: json!({ "type": "object" }),
                    strict: Some(true),
                },
            }),
        };

        apply_provider_options(&mut body, &options)?;

        assert_eq!(body.get("reasoning"), Some(&json!({ "effort": "high" })));
        assert_eq!(
            body.get("response_format"),
            Some(&json!({
                "type": "json_schema",
                "json_schema": {
                    "name": "unit_test",
                    "schema": { "type": "object" },
                    "strict": true
                }
            }))
        );
        Ok(())
    }

    #[test]
    fn maps_responses_finish_reason_completed_to_stop_or_tool_calls() {
        assert_eq!(
            map_responses_finish_reason(Some("completed"), None, false),
            FinishReason::Stop
        );
        assert_eq!(
            map_responses_finish_reason(Some("completed"), None, true),
            FinishReason::ToolCalls
        );
    }

    #[test]
    fn maps_responses_finish_reason_incomplete_reason() {
        assert_eq!(
            map_responses_finish_reason(Some("incomplete"), Some("max_output_tokens"), false),
            FinishReason::Length
        );
        assert_eq!(
            map_responses_finish_reason(Some("incomplete"), Some("content_filter"), false),
            FinishReason::ContentFilter
        );
    }

    #[test]
    fn finish_reason_for_final_event_prefers_response_payload() {
        let response = json!({
            "status": "completed",
            "incomplete_details": { "reason": "max_output_tokens" }
        });

        assert_eq!(
            finish_reason_for_final_event("response.incomplete", Some(&response), false),
            FinishReason::Stop
        );
    }

    #[test]
    fn finish_reason_for_final_event_marks_tool_calls() {
        let response = json!({ "status": "completed" });
        assert_eq!(
            finish_reason_for_final_event("response.completed", Some(&response), true),
            FinishReason::ToolCalls
        );
    }
}
