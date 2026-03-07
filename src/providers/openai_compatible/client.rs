use async_trait::async_trait;
#[cfg(feature = "streaming")]
use futures_util::StreamExt;
#[cfg(feature = "streaming")]
use futures_util::stream;
use serde::Deserialize;
use serde_json::{Map, Value};

use super::openai_like;

#[cfg(feature = "embeddings")]
use crate::embedding::EmbeddingModel;
use crate::file::{FileContent, FileDeleteResponse, FileObject};
use crate::model::{LanguageModel, StreamResult};
use crate::profile::{Env, OpenAiProviderFamily, ProviderConfig, infer_openai_provider_quirks};
#[cfg(feature = "streaming")]
use crate::types::StreamChunk;
use crate::types::{
    ContentPart, FileSource, FinishReason, GenerateRequest, GenerateResponse, ImageSource, Message,
    Role, Tool, ToolChoice, Usage, Warning,
};
use crate::{DittoError, Result};

#[derive(Clone)]
pub struct OpenAICompatible {
    client: openai_like::OpenAiLikeClient,
    request_quirks: OpenAiCompatibleRequestQuirks,
}

#[derive(Clone, Copy, Debug)]
struct OpenAiCompatibleRequestQuirks {
    family: OpenAiProviderFamily,
    assistant_tool_call_requires_reasoning_content: bool,
    assistant_tool_call_requires_thought_signature: bool,
    allow_prompt_cache_key: bool,
}

impl Default for OpenAiCompatibleRequestQuirks {
    fn default() -> Self {
        Self::from_base_url("https://api.openai.com/v1")
    }
}

impl OpenAiCompatibleRequestQuirks {
    fn from_base_url(base_url: &str) -> Self {
        let family = infer_openai_provider_quirks("", base_url).family;
        let assistant_tool_call_requires_reasoning_content =
            matches!(family, OpenAiProviderFamily::Kimi);
        let base_url_lower = base_url.to_ascii_lowercase();
        let assistant_tool_call_requires_thought_signature = base_url_lower.contains("litellm");
        let allow_prompt_cache_key = false;
        Self {
            family,
            assistant_tool_call_requires_reasoning_content,
            assistant_tool_call_requires_thought_signature,
            allow_prompt_cache_key,
        }
    }

    fn should_send_prompt_cache_key(self) -> bool {
        self.allow_prompt_cache_key
            || env_flag_is_true("DITTO_OPENAI_COMPAT_SEND_PROMPT_CACHE_KEY")
            || env_flag_is_true("OMNE_OPENAI_COMPAT_SEND_PROMPT_CACHE_KEY")
    }

    fn should_send_assistant_tool_call_thought_signature(self) -> bool {
        self.assistant_tool_call_requires_thought_signature
            || env_flag_is_true("DITTO_OPENAI_COMPAT_SEND_TOOL_CALL_THOUGHT_SIGNATURE")
            || env_flag_is_true("OMNE_OPENAI_COMPAT_SEND_TOOL_CALL_THOUGHT_SIGNATURE")
    }
}

fn env_flag_is_true(name: &str) -> bool {
    let Ok(raw) = std::env::var(name) else {
        return false;
    };
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

const TOOL_CALL_THOUGHT_SIGNATURE_SEPARATOR: &str = "__gts_";
const OPENAI_COMPAT_DUMMY_THOUGHT_SIGNATURE: &str = "skip_thought_signature_validator";

fn split_tool_call_id_and_thought_signature(id: &str) -> (String, Option<String>) {
    let Some((base_id, hex)) = id.rsplit_once(TOOL_CALL_THOUGHT_SIGNATURE_SEPARATOR) else {
        return (id.to_string(), None);
    };
    if hex.len() % 2 != 0 {
        return (id.to_string(), None);
    }
    let mut bytes = Vec::<u8>::with_capacity(hex.len() / 2);
    for chunk in hex.as_bytes().chunks_exact(2) {
        let hex_pair = match std::str::from_utf8(chunk) {
            Ok(value) => value,
            Err(_) => return (id.to_string(), None),
        };
        let byte = match u8::from_str_radix(hex_pair, 16) {
            Ok(value) => value,
            Err(_) => return (id.to_string(), None),
        };
        bytes.push(byte);
    }
    let signature = match String::from_utf8(bytes) {
        Ok(value) => value,
        Err(_) => return (id.to_string(), None),
    };
    if signature.trim().is_empty() {
        return (base_id.to_string(), None);
    }
    (base_id.to_string(), Some(signature))
}

fn encode_tool_call_id_with_thought_signature(id: &str, thought_signature: Option<&str>) -> String {
    let (base_id, _) = split_tool_call_id_and_thought_signature(id);
    let Some(signature) = thought_signature.map(str::trim).filter(|s| !s.is_empty()) else {
        return base_id;
    };
    let mut hex = String::with_capacity(signature.len() * 2);
    for byte in signature.as_bytes() {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    format!("{base_id}{TOOL_CALL_THOUGHT_SIGNATURE_SEPARATOR}{hex}")
}

const OPENAI_COMPAT_RESERVED_PROVIDER_OPTION_KEYS: &[&str] = &[
    "reasoning_effort",
    "response_format",
    "parallel_tool_calls",
    "prompt_cache_key",
];

impl OpenAICompatible {
    pub fn new(api_key: impl Into<String>) -> Self {
        let client = openai_like::OpenAiLikeClient::new(api_key);
        let request_quirks = OpenAiCompatibleRequestQuirks::from_base_url(&client.base_url);
        Self {
            client,
            request_quirks,
        }
    }

    pub fn with_http_client(mut self, http: reqwest::Client) -> Self {
        self.client = self.client.with_http_client(http);
        self
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        let base_url = base_url.into();
        self.client = self.client.with_base_url(base_url.clone());
        self.request_quirks = OpenAiCompatibleRequestQuirks::from_base_url(&base_url);
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
        const DEFAULT_KEYS: &[&str] = &["OPENAI_COMPAT_API_KEY", "OPENAI_API_KEY"];
        let client =
            openai_like::OpenAiLikeClient::from_config_optional(config, env, DEFAULT_KEYS).await?;
        let request_quirks = OpenAiCompatibleRequestQuirks::from_base_url(&client.base_url);
        Ok(Self {
            client,
            request_quirks,
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

    fn messages_to_chat_messages(
        messages: &[Message],
        quirks: OpenAiCompatibleRequestQuirks,
    ) -> (Vec<Value>, Vec<Warning>) {
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
                    let mut reasoning = String::new();
                    let mut tool_calls = Vec::<Value>::new();
                    for part in &message.content {
                        match part {
                            ContentPart::Text { text: chunk } => text.push_str(chunk),
                            ContentPart::Reasoning { text: chunk } => reasoning.push_str(chunk),
                            ContentPart::ToolCall {
                                id,
                                name,
                                arguments,
                            } => {
                                let (tool_call_id, thought_signature) =
                                    split_tool_call_id_and_thought_signature(id);
                                let mut function = Map::<String, Value>::new();
                                function.insert("name".to_string(), Value::String(name.clone()));
                                function.insert(
                                    "arguments".to_string(),
                                    Value::String(Self::tool_call_arguments_to_openai_string(
                                        arguments,
                                    )),
                                );
                                if quirks.should_send_assistant_tool_call_thought_signature() {
                                    let thought_signature = thought_signature
                                        .as_deref()
                                        .unwrap_or(OPENAI_COMPAT_DUMMY_THOUGHT_SIGNATURE);
                                    function.insert(
                                        "thought_signature".to_string(),
                                        Value::String(thought_signature.to_string()),
                                    );
                                }
                                let mut tool_call = Map::<String, Value>::new();
                                tool_call.insert("id".to_string(), Value::String(tool_call_id));
                                tool_call.insert(
                                    "type".to_string(),
                                    Value::String("function".to_string()),
                                );
                                tool_call.insert("function".to_string(), Value::Object(function));
                                tool_calls.push(Value::Object(tool_call));
                            }
                            other => warnings.push(Warning::Unsupported {
                                feature: "assistant_content_part".to_string(),
                                details: Some(format!(
                                    "unsupported assistant content part: {other:?}"
                                )),
                            }),
                        }
                    }

                    if text.trim().is_empty()
                        && tool_calls.is_empty()
                        && reasoning.trim().is_empty()
                    {
                        continue;
                    }
                    let has_tool_calls = !tool_calls.is_empty();

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
                    if !reasoning.trim().is_empty()
                        || (has_tool_calls && quirks.assistant_tool_call_requires_reasoning_content)
                    {
                        // Some OpenAI-compatible providers require reasoning_content to exist
                        // on assistant tool-call messages in follow-up turns. If reasoning text
                        // is absent, send an empty string to satisfy strict schema checks.
                        msg.insert("reasoning_content".to_string(), Value::String(reasoning));
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
                                let (tool_call_id, _) =
                                    split_tool_call_id_and_thought_signature(tool_call_id);
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
        quirks: OpenAiCompatibleRequestQuirks,
        provider_options: &crate::types::ProviderOptions,
        selected_provider_options: Option<&Value>,
        stream: bool,
        provider_options_context: &'static str,
    ) -> Result<(Map<String, Value>, Vec<Warning>)> {
        let (messages, mut warnings) = Self::messages_to_chat_messages(&request.messages, quirks);

        let mut body = Map::<String, Value>::new();
        body.insert("model".to_string(), Value::String(model.to_string()));
        body.insert("messages".to_string(), Value::Array(messages));
        body.insert("stream".to_string(), Value::Bool(stream));
        if stream {
            body.insert(
                "stream_options".to_string(),
                serde_json::json!({ "include_usage": true }),
            );
        }

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
        if let Some(user) = request
            .user
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
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
                    details: format!(
                        "top_logprobs must be between 1 and 20 (got {top_logprobs}); dropping"
                    ),
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
            let stops =
                crate::utils::params::sanitize_stop_sequences(stops, Some(4), &mut warnings);
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
        if matches!(quirks.family, OpenAiProviderFamily::DeepSeek)
            && provider_options.reasoning_effort.is_some()
            && !body.contains_key("thinking")
        {
            body.remove("reasoning_effort");
            body.insert(
                "thinking".to_string(),
                serde_json::json!({ "type": "enabled" }),
            );
            warnings.push(Warning::Compatibility {
                feature: "reasoning_effort".to_string(),
                details: "mapped to thinking.type=enabled for deepseek compatibility".to_string(),
            });
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
            OPENAI_COMPAT_RESERVED_PROVIDER_OPTION_KEYS,
            provider_options_context,
            &mut warnings,
        );

        if let Some(prompt_cache_key) = provider_options
            .prompt_cache_key
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if quirks.should_send_prompt_cache_key() {
                body.insert(
                    "prompt_cache_key".to_string(),
                    Value::String(prompt_cache_key.to_string()),
                );
            }
        }

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
            let deepseek_cache_hit_tokens =
                obj.get("prompt_cache_hit_tokens").and_then(Value::as_u64);
            let deepseek_cache_miss_tokens =
                obj.get("prompt_cache_miss_tokens").and_then(Value::as_u64);

            usage.input_tokens =
                obj.get("prompt_tokens")
                    .and_then(Value::as_u64)
                    .or_else(
                        || match (deepseek_cache_hit_tokens, deepseek_cache_miss_tokens) {
                            (Some(hit), Some(miss)) => Some(hit.saturating_add(miss)),
                            _ => None,
                        },
                    );
            usage.cache_input_tokens = obj
                .get("cached_tokens")
                .and_then(Value::as_u64)
                .or_else(|| {
                    obj.get("prompt_tokens_details")
                        .and_then(|details| details.get("cached_tokens"))
                        .and_then(Value::as_u64)
                })
                .or_else(|| {
                    obj.get("prompt_tokens_details")
                        .and_then(|details| details.get("cache_read_input_tokens"))
                        .and_then(Value::as_u64)
                })
                .or_else(|| obj.get("cache_read_input_tokens").and_then(Value::as_u64))
                .or_else(|| obj.get("cache_input_tokens").and_then(Value::as_u64))
                .or(deepseek_cache_hit_tokens);
            usage.cache_creation_input_tokens = obj
                .get("cache_creation_input_tokens")
                .and_then(Value::as_u64)
                .or_else(|| {
                    obj.get("prompt_tokens_details")
                        .and_then(|details| details.get("cache_creation_input_tokens"))
                        .and_then(Value::as_u64)
                })
                .or_else(|| obj.get("cache_write_input_tokens").and_then(Value::as_u64));
            usage.output_tokens = obj.get("completion_tokens").and_then(Value::as_u64);
            usage.total_tokens = obj.get("total_tokens").and_then(Value::as_u64);
        }
        usage.merge_total();
        usage
    }
}

#[cfg(test)]
mod client_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_usage_reads_cached_tokens_top_level() {
        let usage = OpenAICompatible::parse_usage(&json!({
            "prompt_tokens": 10,
            "completion_tokens": 2,
            "total_tokens": 12,
            "cached_tokens": 7,
        }));
        assert_eq!(usage.cache_input_tokens, Some(7));
    }

    #[test]
    fn parse_usage_reads_cached_tokens_nested_prompt_tokens_details() {
        let usage = OpenAICompatible::parse_usage(&json!({
            "prompt_tokens": 10,
            "completion_tokens": 2,
            "total_tokens": 12,
            "prompt_tokens_details": { "cached_tokens": 5 },
        }));
        assert_eq!(usage.cache_input_tokens, Some(5));
    }

    #[test]
    fn parse_usage_reads_cache_read_input_tokens_alias() {
        let usage = OpenAICompatible::parse_usage(&json!({
            "prompt_tokens": 10,
            "completion_tokens": 2,
            "total_tokens": 12,
            "cache_read_input_tokens": 4,
        }));
        assert_eq!(usage.cache_input_tokens, Some(4));
    }

    #[test]
    fn parse_usage_reads_cache_write_input_tokens_alias() {
        let usage = OpenAICompatible::parse_usage(&json!({
            "prompt_tokens": 10,
            "completion_tokens": 2,
            "total_tokens": 12,
            "cache_write_input_tokens": 3,
        }));
        assert_eq!(usage.cache_creation_input_tokens, Some(3));
    }

    #[test]
    fn parse_usage_reads_deepseek_prompt_cache_hit_tokens_alias() {
        let usage = OpenAICompatible::parse_usage(&json!({
            "prompt_tokens": 10,
            "completion_tokens": 2,
            "total_tokens": 12,
            "prompt_cache_hit_tokens": 6,
            "prompt_cache_miss_tokens": 4
        }));
        assert_eq!(usage.cache_input_tokens, Some(6));
        assert_eq!(usage.input_tokens, Some(10));
    }

    #[test]
    fn parse_usage_derives_prompt_tokens_from_deepseek_hit_miss_when_missing_prompt_tokens() {
        let usage = OpenAICompatible::parse_usage(&json!({
            "completion_tokens": 2,
            "total_tokens": 12,
            "prompt_cache_hit_tokens": 7,
            "prompt_cache_miss_tokens": 3
        }));
        assert_eq!(usage.cache_input_tokens, Some(7));
        assert_eq!(usage.input_tokens, Some(10));
    }

    #[test]
    fn build_body_suppresses_prompt_cache_key_by_default_and_keeps_stream_usage() -> Result<()> {
        let request = GenerateRequest::from(vec![Message::user("hi")]);
        let provider_options = crate::types::ProviderOptions {
            prompt_cache_key: Some("thread-123".to_string()),
            ..Default::default()
        };
        let selected = serde_json::to_value(&provider_options)?;

        let (body, _warnings) = OpenAICompatible::build_chat_completions_body(
            &request,
            "gpt-4.1",
            OpenAiCompatibleRequestQuirks::default(),
            &provider_options,
            Some(&selected),
            true,
            "test.provider_options",
        )?;

        assert!(
            body.get("prompt_cache_key").is_none(),
            "prompt_cache_key should be suppressed by default for compatibility"
        );
        assert_eq!(
            body.get("stream_options")
                .and_then(Value::as_object)
                .and_then(|opts| opts.get("include_usage"))
                .and_then(Value::as_bool),
            Some(true)
        );

        Ok(())
    }

    #[test]
    fn build_body_includes_prompt_cache_key_when_quirk_enables_it() -> Result<()> {
        let request = GenerateRequest::from(vec![Message::user("hi")]);
        let provider_options = crate::types::ProviderOptions {
            prompt_cache_key: Some("thread-123".to_string()),
            ..Default::default()
        };
        let selected = serde_json::to_value(&provider_options)?;

        let (body, _warnings) = OpenAICompatible::build_chat_completions_body(
            &request,
            "gpt-4.1",
            OpenAiCompatibleRequestQuirks {
                allow_prompt_cache_key: true,
                ..Default::default()
            },
            &provider_options,
            Some(&selected),
            false,
            "test.provider_options",
        )?;

        assert_eq!(
            body.get("prompt_cache_key").and_then(Value::as_str),
            Some("thread-123")
        );
        assert_eq!(body.get("stream").and_then(Value::as_bool), Some(false));
        assert!(
            body.get("stream_options").is_none(),
            "non-streaming request should not include stream_options"
        );

        Ok(())
    }

    #[test]
    fn provider_options_schema_drops_unknown_keys_and_keeps_known_openai_fields() {
        let selected = serde_json::json!({
            "stream": true,
            "unknown_private_flag": true
        });
        let schema = apply_openai_compatible_provider_options_schema(
            OpenAiProviderFamily::Kimi,
            Some(selected),
            OPENAI_COMPAT_RESERVED_PROVIDER_OPTION_KEYS,
            "test.provider_options",
        );

        assert_eq!(
            schema.selected_provider_options,
            Some(serde_json::json!({ "stream": true }))
        );
        assert!(schema.warnings.iter().any(|warning| matches!(
            warning,
            Warning::Unsupported { feature, details }
                if feature == "test.provider_options"
                    && details.as_deref().is_some_and(|msg| msg.contains("unknown_private_flag"))
        )));
    }

    #[test]
    fn provider_options_schema_maps_minimax_reasoning_split_alias() {
        let selected = serde_json::json!({
            "reasoningSplit": true
        });
        let schema = apply_openai_compatible_provider_options_schema(
            OpenAiProviderFamily::MiniMax,
            Some(selected),
            OPENAI_COMPAT_RESERVED_PROVIDER_OPTION_KEYS,
            "test.provider_options",
        );

        assert_eq!(
            schema.selected_provider_options,
            Some(serde_json::json!({ "reasoning_split": true }))
        );
        assert!(schema.warnings.iter().any(|warning| matches!(
            warning,
            Warning::Compatibility { feature, details }
                if feature == "test.provider_options"
                    && details.contains("\"reasoningSplit\"")
                    && details.contains("reasoning_split")
        )));
    }

    #[test]
    fn provider_options_schema_keeps_openrouter_provider_object() {
        let selected = serde_json::json!({
            "provider": {
                "order": ["Google AI Studio"],
                "allow_fallbacks": false
            }
        });
        let schema = apply_openai_compatible_provider_options_schema(
            OpenAiProviderFamily::OpenRouter,
            Some(selected),
            OPENAI_COMPAT_RESERVED_PROVIDER_OPTION_KEYS,
            "test.provider_options",
        );

        assert_eq!(
            schema.selected_provider_options,
            Some(serde_json::json!({
                "provider": {
                    "order": ["Google AI Studio"],
                    "allow_fallbacks": false
                }
            }))
        );
        assert!(schema.warnings.is_empty());
    }

    #[test]
    fn provider_options_schema_drops_non_object_openrouter_provider() {
        let selected = serde_json::json!({
            "provider": "Google AI Studio"
        });
        let schema = apply_openai_compatible_provider_options_schema(
            OpenAiProviderFamily::OpenRouter,
            Some(selected),
            OPENAI_COMPAT_RESERVED_PROVIDER_OPTION_KEYS,
            "test.provider_options",
        );

        assert_eq!(schema.selected_provider_options, None);
        assert!(schema.warnings.iter().any(|warning| matches!(
            warning,
            Warning::Compatibility { feature, details }
                if feature == "test.provider_options"
                    && details.contains("\"provider\" for openrouter expects a JSON object")
        )));
    }

    #[test]
    fn build_body_maps_deepseek_reasoning_effort_to_thinking() -> Result<()> {
        let request = GenerateRequest::from(vec![Message::user("hi")]);
        let provider_options = crate::types::ProviderOptions {
            reasoning_effort: Some(crate::types::ReasoningEffort::High),
            ..Default::default()
        };
        let selected = serde_json::to_value(&provider_options)?;

        let (body, warnings) = OpenAICompatible::build_chat_completions_body(
            &request,
            "deepseek-chat",
            OpenAiCompatibleRequestQuirks {
                family: OpenAiProviderFamily::DeepSeek,
                ..Default::default()
            },
            &provider_options,
            Some(&selected),
            false,
            "test.provider_options",
        )?;

        assert!(
            body.get("reasoning_effort").is_none(),
            "deepseek requests should not send reasoning_effort directly"
        );
        assert_eq!(
            body.get("thinking"),
            Some(&serde_json::json!({ "type": "enabled" }))
        );
        assert!(warnings.iter().any(|warning| matches!(
            warning,
            Warning::Compatibility { feature, details }
                if feature == "reasoning_effort"
                    && details.contains("deepseek")
        )));
        Ok(())
    }

    #[test]
    fn messages_to_chat_messages_preserves_assistant_reasoning_for_tool_calls() {
        let assistant = Message {
            role: Role::Assistant,
            content: vec![
                ContentPart::Reasoning {
                    text: "chain-of-thought".to_string(),
                },
                ContentPart::ToolCall {
                    id: "call_1".to_string(),
                    name: "thread".to_string(),
                    arguments: json!({"op":"state"}),
                },
            ],
        };

        let (messages, warnings) =
            OpenAICompatible::messages_to_chat_messages(&[assistant], Default::default());
        assert_eq!(messages.len(), 1);
        assert!(warnings
                .iter()
                .all(|warning| !matches!(warning, Warning::Unsupported { feature, .. } if feature == "reasoning")));

        let msg = messages[0].as_object().expect("assistant message object");
        assert_eq!(msg.get("role").and_then(Value::as_str), Some("assistant"));
        assert_eq!(
            msg.get("reasoning_content").and_then(Value::as_str),
            Some("chain-of-thought")
        );
        assert_eq!(msg.get("content"), Some(&Value::Null));
        assert!(msg.get("tool_calls").is_some());
    }

    #[test]
    fn messages_to_chat_messages_keeps_reasoning_only_assistant_message() {
        let assistant = Message {
            role: Role::Assistant,
            content: vec![ContentPart::Reasoning {
                text: "thinking-only".to_string(),
            }],
        };

        let (messages, warnings) =
            OpenAICompatible::messages_to_chat_messages(&[assistant], Default::default());
        assert_eq!(messages.len(), 1);
        assert!(warnings.is_empty());

        let msg = messages[0].as_object().expect("assistant message object");
        assert_eq!(msg.get("role").and_then(Value::as_str), Some("assistant"));
        assert_eq!(
            msg.get("reasoning_content").and_then(Value::as_str),
            Some("thinking-only")
        );
        assert_eq!(msg.get("content"), Some(&Value::Null));
        assert!(msg.get("tool_calls").is_none());
    }

    #[test]
    fn messages_to_chat_messages_skips_empty_reasoning_for_tool_calls_by_default() {
        let assistant = Message {
            role: Role::Assistant,
            content: vec![ContentPart::ToolCall {
                id: "call_1".to_string(),
                name: "workspace".to_string(),
                arguments: json!({"op":"help"}),
            }],
        };

        let (messages, warnings) =
            OpenAICompatible::messages_to_chat_messages(&[assistant], Default::default());
        assert_eq!(messages.len(), 1);
        assert!(warnings.is_empty());

        let msg = messages[0].as_object().expect("assistant message object");
        assert_eq!(msg.get("role").and_then(Value::as_str), Some("assistant"));
        assert_eq!(msg.get("content"), Some(&Value::Null));
        assert!(msg.get("reasoning_content").is_none());
        assert!(msg.get("tool_calls").is_some());
    }

    #[test]
    fn messages_to_chat_messages_adds_empty_reasoning_for_kimi_tool_calls() {
        let assistant = Message {
            role: Role::Assistant,
            content: vec![ContentPart::ToolCall {
                id: "call_1".to_string(),
                name: "workspace".to_string(),
                arguments: json!({"op":"help"}),
            }],
        };

        let (messages, warnings) = OpenAICompatible::messages_to_chat_messages(
            &[assistant],
            OpenAiCompatibleRequestQuirks {
                family: OpenAiProviderFamily::Kimi,
                assistant_tool_call_requires_reasoning_content: true,
                assistant_tool_call_requires_thought_signature: false,
                allow_prompt_cache_key: false,
            },
        );
        assert_eq!(messages.len(), 1);
        assert!(warnings.is_empty());

        let msg = messages[0].as_object().expect("assistant message object");
        assert_eq!(msg.get("role").and_then(Value::as_str), Some("assistant"));
        assert_eq!(msg.get("content"), Some(&Value::Null));
        assert_eq!(
            msg.get("reasoning_content").and_then(Value::as_str),
            Some("")
        );
        assert!(msg.get("tool_calls").is_some());
    }

    #[test]
    fn messages_to_chat_messages_adds_dummy_thought_signature_for_litellm_tool_calls() {
        let assistant = Message {
            role: Role::Assistant,
            content: vec![ContentPart::ToolCall {
                id: "call_1".to_string(),
                name: "workspace".to_string(),
                arguments: json!({"op":"help"}),
            }],
        };

        let (messages, warnings) = OpenAICompatible::messages_to_chat_messages(
            &[assistant],
            OpenAiCompatibleRequestQuirks {
                assistant_tool_call_requires_thought_signature: true,
                ..Default::default()
            },
        );
        assert_eq!(messages.len(), 1);
        assert!(warnings.is_empty());

        let msg = messages[0].as_object().expect("assistant message object");
        let tool_calls = msg
            .get("tool_calls")
            .and_then(Value::as_array)
            .expect("tool calls array");
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(
            tool_calls[0].get("id").and_then(Value::as_str),
            Some("call_1")
        );
        assert_eq!(
            tool_calls[0]
                .get("function")
                .and_then(Value::as_object)
                .and_then(|function| function.get("thought_signature"))
                .and_then(Value::as_str),
            Some(OPENAI_COMPAT_DUMMY_THOUGHT_SIGNATURE)
        );
    }

    #[test]
    fn messages_to_chat_messages_replays_encoded_thought_signature_and_normalizes_ids() {
        let encoded_id = encode_tool_call_id_with_thought_signature("call_abc", Some("hi"));
        let assistant = Message {
            role: Role::Assistant,
            content: vec![ContentPart::ToolCall {
                id: encoded_id.clone(),
                name: "workspace".to_string(),
                arguments: json!({"op":"help"}),
            }],
        };
        let tool = Message {
            role: Role::Tool,
            content: vec![ContentPart::ToolResult {
                tool_call_id: encoded_id,
                content: "ok".to_string(),
                is_error: None,
            }],
        };

        let (messages, warnings) = OpenAICompatible::messages_to_chat_messages(
            &[assistant, tool],
            OpenAiCompatibleRequestQuirks {
                assistant_tool_call_requires_thought_signature: true,
                ..Default::default()
            },
        );
        assert_eq!(messages.len(), 2);
        assert!(warnings.is_empty());

        let assistant_msg = messages[0].as_object().expect("assistant message object");
        let tool_calls = assistant_msg
            .get("tool_calls")
            .and_then(Value::as_array)
            .expect("tool calls array");
        assert_eq!(
            tool_calls[0].get("id").and_then(Value::as_str),
            Some("call_abc")
        );
        assert_eq!(
            tool_calls[0]
                .get("function")
                .and_then(Value::as_object)
                .and_then(|function| function.get("thought_signature"))
                .and_then(Value::as_str),
            Some("hi")
        );

        let tool_msg = messages[1].as_object().expect("tool message object");
        assert_eq!(
            tool_msg.get("tool_call_id").and_then(Value::as_str),
            Some("call_abc")
        );
    }
}
