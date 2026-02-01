use std::collections::{BTreeMap, HashMap, HashSet};

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
        const DEFAULT_KEYS: &[&str] = &["COHERE_API_KEY"];
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
