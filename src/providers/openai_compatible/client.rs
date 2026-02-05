use async_trait::async_trait;
use futures_util::StreamExt;
use futures_util::stream;
use serde::Deserialize;
use serde_json::{Map, Value};

use super::openai_like;

#[cfg(feature = "embeddings")]
use crate::embedding::EmbeddingModel;
use crate::file::{FileContent, FileDeleteResponse, FileObject};
use crate::model::{LanguageModel, StreamResult};
use crate::profile::{Env, ProviderConfig};
use crate::types::{
    ContentPart, FileSource, FinishReason, GenerateRequest, GenerateResponse, ImageSource, Message,
    Role, StreamChunk, Tool, ToolChoice, Usage, Warning,
};
use crate::{DittoError, Result};

#[derive(Clone)]
pub struct OpenAICompatible {
    client: openai_like::OpenAiLikeClient,
}

impl OpenAICompatible {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: openai_like::OpenAiLikeClient::new(api_key),
        }
    }

    pub fn with_http_client(mut self, http: reqwest::Client) -> Self {
        self.client = self.client.with_http_client(http);
        self
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.client = self.client.with_base_url(base_url);
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.client = self.client.with_model(model);
        self
    }

    pub fn with_max_binary_response_bytes(mut self, max_bytes: usize) -> Self {
        self.client = self.client.with_max_binary_response_bytes(max_bytes);
        self
    }

    pub async fn from_config(config: &ProviderConfig, env: &Env) -> Result<Self> {
        const DEFAULT_KEYS: &[&str] = &[
            "OPENAI_COMPAT_API_KEY",
            "OPENAI_API_KEY",
        ];
        Ok(Self {
            client: openai_like::OpenAiLikeClient::from_config_optional(config, env, DEFAULT_KEYS)
                .await?,
        })
    }

    fn apply_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        self.client.apply_auth(req)
    }

    fn chat_completions_url(&self) -> String {
        self.client.endpoint("chat/completions")
    }

    pub async fn upload_file(&self, filename: impl Into<String>, bytes: Vec<u8>) -> Result<String> {
        self.upload_file_with_purpose(filename, bytes, "assistants", None)
            .await
    }

    pub async fn upload_file_with_purpose(
        &self,
        filename: impl Into<String>,
        bytes: Vec<u8>,
        purpose: impl Into<String>,
        media_type: Option<&str>,
    ) -> Result<String> {
        self.client
            .upload_file_with_purpose(crate::file::FileUploadRequest {
                filename: filename.into(),
                bytes,
                purpose: purpose.into(),
                media_type: media_type.map(|s| s.to_string()),
            })
            .await
    }

    pub async fn list_files(&self) -> Result<Vec<FileObject>> {
        self.client.list_files().await
    }

    pub async fn retrieve_file(&self, file_id: &str) -> Result<FileObject> {
        self.client.retrieve_file(file_id).await
    }

    pub async fn delete_file(&self, file_id: &str) -> Result<FileDeleteResponse> {
        self.client.delete_file(file_id).await
    }

    pub async fn download_file_content(&self, file_id: &str) -> Result<FileContent> {
        self.client.download_file_content(file_id).await
    }

    fn resolve_model<'a>(&'a self, request: &'a GenerateRequest) -> Result<&'a str> {
        if let Some(model) = request.model.as_deref().filter(|m| !m.trim().is_empty()) {
            return Ok(model);
        }
        if !self.client.model.trim().is_empty() {
            return Ok(self.client.model.as_str());
        }
        Err(DittoError::InvalidResponse(
            "openai-compatible model is not set (set request.model or OpenAICompatible::with_model)"
                .to_string(),
        ))
    }

    fn tool_to_openai(tool: &Tool, warnings: &mut Vec<Warning>) -> Value {
        if tool.strict.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "tool.strict".to_string(),
                details: Some("chat/completions does not support strict tool schemas".to_string()),
            });
        }

        let mut function = Map::<String, Value>::new();
        function.insert("name".to_string(), Value::String(tool.name.clone()));
        if let Some(description) = &tool.description {
            function.insert(
                "description".to_string(),
                Value::String(description.clone()),
            );
        }
        function.insert("parameters".to_string(), tool.parameters.clone());

        let mut out = Map::<String, Value>::new();
        out.insert("type".to_string(), Value::String("function".to_string()));
        out.insert("function".to_string(), Value::Object(function));
        Value::Object(out)
    }

    fn tool_choice_to_openai(choice: &ToolChoice) -> Value {
        match choice {
            ToolChoice::Auto => Value::String("auto".to_string()),
            ToolChoice::None => Value::String("none".to_string()),
            ToolChoice::Required => Value::String("required".to_string()),
            ToolChoice::Tool { name } => serde_json::json!({
                "type": "function",
                "function": { "name": name }
            }),
        }
    }

    fn tool_call_arguments_to_openai_string(arguments: &Value) -> String {
        match arguments {
            Value::String(raw) => {
                let raw = raw.trim();
                if raw.is_empty() {
                    "{}".to_string()
                } else {
                    raw.to_string()
                }
            }
            other => other.to_string(),
        }
    }

    fn messages_to_chat_messages(messages: &[Message]) -> (Vec<Value>, Vec<Warning>) {
        let mut out = Vec::<Value>::new();
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
                    out.push(serde_json::json!({ "role": "system", "content": text }));
                }
                Role::User => {
                    let mut texts = String::new();
                    let mut parts = Vec::<Value>::new();
                    let mut has_non_text = false;

                    for part in &message.content {
                        match part {
                            ContentPart::Text { text } => {
                                if text.is_empty() {
                                    continue;
                                }
                                texts.push_str(text);
                                parts.push(serde_json::json!({ "type": "text", "text": text }));
                            }
                            ContentPart::Image { source } => {
                                has_non_text = true;
                                let image_url = match source {
                                    ImageSource::Url { url } => url.clone(),
                                    ImageSource::Base64 { media_type, data } => {
                                        format!("data:{media_type};base64,{data}")
                                    }
                                };
                                parts.push(serde_json::json!({
                                    "type": "image_url",
                                    "image_url": { "url": image_url }
                                }));
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
                                            "unsupported file media type for openai-compatible: {media_type}"
                                        )),
                                    });
                                    continue;
                                }

                                has_non_text = true;
                                let part = match source {
                                    FileSource::Url { url } => {
                                        warnings.push(Warning::Unsupported {
                                            feature: "file_url".to_string(),
                                            details: Some(format!(
                                                "openai-compatible chat messages do not support file URLs (url={url})"
                                            )),
                                        });
                                        continue;
                                    }
                                    FileSource::Base64 { data } => serde_json::json!({
                                        "type": "file",
                                        "file": {
                                            "filename": filename.clone().unwrap_or_else(|| "file.pdf".to_string()),
                                            "file_data": format!("data:{media_type};base64,{data}"),
                                        }
                                    }),
                                    FileSource::FileId { file_id } => serde_json::json!({
                                        "type": "file",
                                        "file": { "file_id": file_id }
                                    }),
                                };
                                parts.push(part);
                            }
                            other => warnings.push(Warning::Unsupported {
                                feature: "user_content_part".to_string(),
                                details: Some(format!("unsupported user content part: {other:?}")),
                            }),
                        }
                    }

                    if parts.is_empty() {
                        continue;
                    }

                    if has_non_text {
                        out.push(serde_json::json!({ "role": "user", "content": parts }));
                    } else {
                        out.push(serde_json::json!({ "role": "user", "content": texts }));
                    }
                }
                Role::Assistant => {
                    let mut text = String::new();
                    let mut tool_calls = Vec::<Value>::new();
                    for part in &message.content {
                        match part {
                            ContentPart::Text { text: chunk } => text.push_str(chunk),
                            ContentPart::ToolCall {
                                id,
                                name,
                                arguments,
                            } => {
                                tool_calls.push(serde_json::json!({
                                    "id": id,
                                    "type": "function",
                                    "function": {
                                        "name": name,
                                        "arguments": Self::tool_call_arguments_to_openai_string(arguments),
                                    }
                                }));
                            }
                            ContentPart::Reasoning { .. } => warnings.push(Warning::Unsupported {
                                feature: "reasoning".to_string(),
                                details: Some(
                                    "reasoning parts are not sent to openai-compatible messages"
                                        .to_string(),
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

                    if text.trim().is_empty() && tool_calls.is_empty() {
                        continue;
                    }

                    let mut msg = Map::<String, Value>::new();
                    msg.insert("role".to_string(), Value::String("assistant".to_string()));
                    if text.trim().is_empty() {
                        msg.insert("content".to_string(), Value::Null);
                    } else {
                        msg.insert("content".to_string(), Value::String(text));
                    }
                    if !tool_calls.is_empty() {
                        msg.insert("tool_calls".to_string(), Value::Array(tool_calls));
                    }
                    out.push(Value::Object(msg));
                }
                Role::Tool => {
                    for part in &message.content {
                        match part {
                            ContentPart::ToolResult {
                                tool_call_id,
                                content,
                                ..
                            } => {
                                if content.trim().is_empty() {
                                    continue;
                                }
                                out.push(serde_json::json!({
                                    "role": "tool",
                                    "tool_call_id": tool_call_id,
                                    "content": content,
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

        (out, warnings)
    }

    fn build_chat_completions_body(
        request: &GenerateRequest,
        model: &str,
        provider_options: &crate::types::ProviderOptions,
        selected_provider_options: Option<&Value>,
        stream: bool,
        provider_options_context: &'static str,
    ) -> Result<(Map<String, Value>, Vec<Warning>)> {
        let (messages, mut warnings) = Self::messages_to_chat_messages(&request.messages);

        let mut body = Map::<String, Value>::new();
        body.insert("model".to_string(), Value::String(model.to_string()));
        body.insert("messages".to_string(), Value::Array(messages));
        body.insert("stream".to_string(), Value::Bool(stream));

        if let Some(temperature) = request.temperature {
            if let Some(value) = crate::utils::params::clamped_number_from_f32(
                "temperature",
                temperature,
                0.0,
                2.0,
                &mut warnings,
            ) {
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
                0.0,
                1.0,
                &mut warnings,
            ) {
                body.insert("top_p".to_string(), Value::Number(value));
            }
        }
        if let Some(seed) = request.seed {
            body.insert("seed".to_string(), Value::Number(seed.into()));
        }
        if let Some(presence_penalty) = request.presence_penalty {
            if let Some(value) = crate::utils::params::clamped_number_from_f32(
                "presence_penalty",
                presence_penalty,
                -2.0,
                2.0,
                &mut warnings,
            ) {
                body.insert("presence_penalty".to_string(), Value::Number(value));
            }
        }
        if let Some(frequency_penalty) = request.frequency_penalty {
            if let Some(value) = crate::utils::params::clamped_number_from_f32(
                "frequency_penalty",
                frequency_penalty,
                -2.0,
                2.0,
                &mut warnings,
            ) {
                body.insert("frequency_penalty".to_string(), Value::Number(value));
            }
        }
        if let Some(user) = request.user.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            body.insert("user".to_string(), Value::String(user.to_string()));
        }
        match request.logprobs {
            Some(true) => {
                body.insert("logprobs".to_string(), Value::Bool(true));
            }
            Some(false) => {
                body.insert("logprobs".to_string(), Value::Bool(false));
            }
            None => {}
        }
        if let Some(top_logprobs) = request.top_logprobs {
            if request.logprobs == Some(false) {
                warnings.push(Warning::Compatibility {
                    feature: "top_logprobs".to_string(),
                    details: "top_logprobs requires logprobs=true; dropping".to_string(),
                });
            } else if top_logprobs == 0 || top_logprobs > 20 {
                warnings.push(Warning::Compatibility {
                    feature: "top_logprobs".to_string(),
                    details: format!("top_logprobs must be between 1 and 20 (got {top_logprobs}); dropping"),
                });
            } else {
                body.insert("logprobs".to_string(), Value::Bool(true));
                body.insert(
                    "top_logprobs".to_string(),
                    Value::Number((top_logprobs as u64).into()),
                );
            }
        }
        if let Some(stops) = request.stop_sequences.as_ref() {
            let stops = crate::utils::params::sanitize_stop_sequences(stops, Some(4), &mut warnings);
            if !stops.is_empty() {
                body.insert(
                    "stop".to_string(),
                    Value::Array(stops.into_iter().map(Value::String).collect()),
                );
            }
        }

        if let Some(tools) = request.tools.as_ref() {
            if cfg!(feature = "tools") {
                let mapped = tools
                    .iter()
                    .map(|t| Self::tool_to_openai(t, &mut warnings))
                    .collect();
                body.insert("tools".to_string(), Value::Array(mapped));
            } else {
                warnings.push(Warning::Unsupported {
                    feature: "tools".to_string(),
                    details: Some("ditto-llm built without tools feature".to_string()),
                });
            }
        }
        if let Some(tool_choice) = request.tool_choice.as_ref() {
            if cfg!(feature = "tools") {
                body.insert(
                    "tool_choice".to_string(),
                    Self::tool_choice_to_openai(tool_choice),
                );
            } else {
                warnings.push(Warning::Unsupported {
                    feature: "tool_choice".to_string(),
                    details: Some("ditto-llm built without tools feature".to_string()),
                });
            }
        }

        if let Some(effort) = provider_options.reasoning_effort {
            body.insert(
                "reasoning_effort".to_string(),
                serde_json::to_value(effort)?,
            );
        }
        if let Some(response_format) = provider_options.response_format.as_ref() {
            body.insert(
                "response_format".to_string(),
                serde_json::to_value(response_format)?,
            );
        }
        if let Some(parallel_tool_calls) = provider_options.parallel_tool_calls {
            body.insert(
                "parallel_tool_calls".to_string(),
                Value::Bool(parallel_tool_calls),
            );
        }

        crate::types::merge_provider_options_into_body(
            &mut body,
            selected_provider_options,
            &["reasoning_effort", "response_format", "parallel_tool_calls"],
            provider_options_context,
            &mut warnings,
        );

        Ok((body, warnings))
    }

    fn parse_finish_reason(reason: Option<&str>) -> FinishReason {
        match reason {
            Some("stop") => FinishReason::Stop,
            Some("length") => FinishReason::Length,
            Some("tool_calls") => FinishReason::ToolCalls,
            Some("function_call") => FinishReason::ToolCalls,
            Some("content_filter") => FinishReason::ContentFilter,
            Some("error") => FinishReason::Error,
            _ => FinishReason::Unknown,
        }
    }

    fn parse_usage(value: &Value) -> Usage {
        let mut usage = Usage::default();
        if let Some(obj) = value.as_object() {
            usage.input_tokens = obj.get("prompt_tokens").and_then(Value::as_u64);
            usage.cache_input_tokens = obj
                .get("prompt_tokens_details")
                .and_then(|details| details.get("cached_tokens"))
                .and_then(Value::as_u64);
            usage.cache_creation_input_tokens = obj
                .get("cache_creation_input_tokens")
                .and_then(Value::as_u64);
            usage.output_tokens = obj.get("completion_tokens").and_then(Value::as_u64);
            usage.total_tokens = obj.get("total_tokens").and_then(Value::as_u64);
        }
        usage.merge_total();
        usage
    }
}
