use std::collections::{BTreeMap, HashMap, VecDeque};

use async_trait::async_trait;
use futures_util::StreamExt;
use futures_util::stream;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::model::{LanguageModel, StreamResult};
use crate::profile::{
    Env, HttpAuth, ProviderAuth, ProviderConfig, RequestAuth, apply_http_query_params,
    resolve_request_auth_with_default_keys,
};
use crate::types::{
    ContentPart, FileSource, FinishReason, GenerateRequest, GenerateResponse, ImageSource, Message,
    Role, StreamChunk, Tool, ToolChoice, Usage, Warning,
};
use crate::{DittoError, Result};

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com/v1";
const DEFAULT_VERSION: &str = "2023-06-01";
const BETA_PDFS_2024_09_25: &str = "pdfs-2024-09-25";

#[derive(Clone)]
pub struct Anthropic {
    http: reqwest::Client,
    base_url: String,
    auth: Option<RequestAuth>,
    default_model: String,
    version: String,
    http_query_params: BTreeMap<String, String>,
}

impl Anthropic {
    pub fn new(api_key: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        let api_key = api_key.into();
        let auth = if api_key.trim().is_empty() {
            None
        } else {
            HttpAuth::header_value("x-api-key", None, &api_key)
                .ok()
                .map(RequestAuth::Http)
        };

        Self {
            http,
            base_url: DEFAULT_BASE_URL.to_string(),
            auth,
            default_model: String::new(),
            version: DEFAULT_VERSION.to_string(),
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

    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }

    pub async fn from_config(config: &ProviderConfig, env: &Env) -> Result<Self> {
        const DEFAULT_KEYS: &[&str] = &["ANTHROPIC_API_KEY", "CODE_PM_ANTHROPIC_API_KEY"];
        let auth = config
            .auth
            .clone()
            .unwrap_or(ProviderAuth::ApiKeyEnv { keys: Vec::new() });
        let auth_header =
            resolve_request_auth_with_default_keys(&auth, env, DEFAULT_KEYS, "x-api-key", None)
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

    fn messages_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/messages") {
            base.to_string()
        } else {
            format!("{base}/messages")
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
            "anthropic model is not set (set request.model or Anthropic::with_model)".to_string(),
        ))
    }

    fn tool_to_anthropic(tool: &Tool, warnings: &mut Vec<Warning>) -> Value {
        if tool.strict.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "tool.strict".to_string(),
                details: Some(
                    "Anthropic strict tools require structured output betas; ignored".to_string(),
                ),
            });
        }

        let mut out = Map::<String, Value>::new();
        out.insert("name".to_string(), Value::String(tool.name.clone()));
        if let Some(description) = &tool.description {
            out.insert(
                "description".to_string(),
                Value::String(description.clone()),
            );
        }
        out.insert("input_schema".to_string(), tool.parameters.clone());
        Value::Object(out)
    }

    fn tool_choice_to_anthropic(choice: &ToolChoice) -> Option<Value> {
        match choice {
            ToolChoice::Auto => Some(serde_json::json!({ "type": "auto" })),
            ToolChoice::Required => Some(serde_json::json!({ "type": "any" })),
            ToolChoice::Tool { name } => Some(serde_json::json!({ "type": "tool", "name": name })),
            ToolChoice::None => None,
        }
    }

    fn extract_system_text(message: &Message, warnings: &mut Vec<Warning>) -> Option<String> {
        let mut out = String::new();
        for part in &message.content {
            match part {
                ContentPart::Text { text } => {
                    if !text.is_empty() {
                        out.push_str(text);
                    }
                }
                other => warnings.push(Warning::Unsupported {
                    feature: "system_content_part".to_string(),
                    details: Some(format!("unsupported system content part: {other:?}")),
                }),
            }
        }
        (!out.trim().is_empty()).then_some(out)
    }

    fn message_to_anthropic_blocks(
        message: &Message,
        tool_names: &HashMap<String, String>,
        warnings: &mut Vec<Warning>,
    ) -> Option<(String, Vec<Value>)> {
        match message.role {
            Role::System => None,
            Role::User => {
                let mut blocks = Vec::<Value>::new();
                for part in &message.content {
                    match part {
                        ContentPart::Text { text } => {
                            if text.is_empty() {
                                continue;
                            }
                            blocks.push(serde_json::json!({ "type": "text", "text": text }));
                        }
                        ContentPart::Image { source } => {
                            let src = match source {
                                ImageSource::Url { url } => serde_json::json!({
                                    "type": "url",
                                    "url": url,
                                }),
                                ImageSource::Base64 { media_type, data } => serde_json::json!({
                                    "type": "base64",
                                    "media_type": media_type,
                                    "data": data,
                                }),
                            };
                            blocks.push(serde_json::json!({ "type": "image", "source": src }));
                        }
                        ContentPart::File {
                            filename,
                            media_type,
                            source,
                        } => {
                            if media_type != "application/pdf" {
                                warnings.push(Warning::Unsupported {
                                    feature: "file".to_string(),
                                    details: Some(format!(
                                        "unsupported file media type for Anthropic Messages: {media_type}"
                                    )),
                                });
                                continue;
                            }

                            let src = match source {
                                FileSource::Url { url } => serde_json::json!({
                                    "type": "url",
                                    "url": url,
                                }),
                                FileSource::Base64 { data } => serde_json::json!({
                                    "type": "base64",
                                    "media_type": "application/pdf",
                                    "data": data,
                                }),
                                FileSource::FileId { file_id } => {
                                    warnings.push(Warning::Unsupported {
                                        feature: "file_id".to_string(),
                                        details: Some(format!(
                                            "Anthropic Messages does not support OpenAI file ids (file_id={file_id})"
                                        )),
                                    });
                                    continue;
                                }
                            };

                            let mut doc = serde_json::json!({ "type": "document", "source": src });
                            if let Some(title) = filename.clone().filter(|s| !s.trim().is_empty()) {
                                if let Some(obj) = doc.as_object_mut() {
                                    obj.insert("title".to_string(), Value::String(title));
                                }
                            }
                            blocks.push(doc);
                        }
                        other => warnings.push(Warning::Unsupported {
                            feature: "user_content_part".to_string(),
                            details: Some(format!("unsupported user content part: {other:?}")),
                        }),
                    }
                }
                if blocks.is_empty() {
                    None
                } else {
                    Some(("user".to_string(), blocks))
                }
            }
            Role::Assistant => {
                let mut blocks = Vec::<Value>::new();
                for part in &message.content {
                    match part {
                        ContentPart::Text { text } => {
                            if text.is_empty() {
                                continue;
                            }
                            blocks.push(serde_json::json!({ "type": "text", "text": text }));
                        }
                        ContentPart::ToolCall {
                            id,
                            name,
                            arguments,
                        } => {
                            blocks.push(serde_json::json!({
                                "type": "tool_use",
                                "id": id,
                                "name": name,
                                "input": arguments,
                            }));
                        }
                        ContentPart::Reasoning { .. } => warnings.push(Warning::Unsupported {
                            feature: "reasoning".to_string(),
                            details: Some(
                                "reasoning parts are not sent to Anthropic messages".to_string(),
                            ),
                        }),
                        other => warnings.push(Warning::Unsupported {
                            feature: "assistant_content_part".to_string(),
                            details: Some(format!("unsupported assistant content part: {other:?}")),
                        }),
                    }
                }
                if blocks.is_empty() {
                    None
                } else {
                    Some(("assistant".to_string(), blocks))
                }
            }
            Role::Tool => {
                let mut blocks = Vec::<Value>::new();
                for part in &message.content {
                    match part {
                        ContentPart::ToolResult {
                            tool_call_id,
                            content,
                            is_error,
                        } => {
                            let is_error = is_error.unwrap_or(false);
                            let tool_use_id = tool_call_id;
                            let mut block = Map::<String, Value>::new();
                            block.insert(
                                "type".to_string(),
                                Value::String("tool_result".to_string()),
                            );
                            block.insert(
                                "tool_use_id".to_string(),
                                Value::String(tool_use_id.clone()),
                            );
                            block.insert("content".to_string(), Value::String(content.clone()));
                            block.insert("is_error".to_string(), Value::Bool(is_error));

                            if tool_names.get(tool_call_id).is_none() {
                                warnings.push(Warning::Compatibility {
                                    feature: "tool_result".to_string(),
                                    details: format!(
                                        "tool_result references unknown tool_call_id={tool_call_id}; sending anyway"
                                    ),
                                });
                            }

                            blocks.push(Value::Object(block));
                        }
                        other => warnings.push(Warning::Unsupported {
                            feature: "tool_content_part".to_string(),
                            details: Some(format!("unsupported tool content part: {other:?}")),
                        }),
                    }
                }
                if blocks.is_empty() {
                    None
                } else {
                    Some(("user".to_string(), blocks))
                }
            }
        }
    }

    fn required_betas(messages: &[Message]) -> Vec<&'static str> {
        let has_pdf = messages.iter().any(|message| {
            message.content.iter().any(|part| {
                matches!(
                    part,
                    ContentPart::File { media_type, .. } if media_type == "application/pdf"
                )
            })
        });

        let mut out = Vec::<&'static str>::new();
        if has_pdf {
            out.push(BETA_PDFS_2024_09_25);
        }
        out
    }

    fn build_tool_name_map(messages: &[Message]) -> HashMap<String, String> {
        let mut map = HashMap::<String, String>::new();
        for message in messages {
            for part in &message.content {
                if let ContentPart::ToolCall { id, name, .. } = part {
                    map.insert(id.clone(), name.clone());
                }
            }
        }
        map
    }

    fn stop_reason_to_finish_reason(stop_reason: Option<&str>) -> FinishReason {
        match stop_reason {
            Some("end_turn") => FinishReason::Stop,
            Some("max_tokens") => FinishReason::Length,
            Some("tool_use") => FinishReason::ToolCalls,
            Some("stop_sequence") => FinishReason::Stop,
            Some("content_filtered") => FinishReason::ContentFilter,
            _ => FinishReason::Unknown,
        }
    }

    fn parse_usage(value: &Value) -> Usage {
        let mut usage = Usage::default();
        if let Some(obj) = value.as_object() {
            usage.input_tokens = obj.get("input_tokens").and_then(Value::as_u64);
            usage.output_tokens = obj.get("output_tokens").and_then(Value::as_u64);
        }
        usage.merge_total();
        usage
    }
}

