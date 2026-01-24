use std::collections::VecDeque;

use async_trait::async_trait;
use futures_util::StreamExt;
use futures_util::stream;
use reqwest::multipart::{Form, Part};
use serde::Deserialize;
use serde_json::{Map, Value};

#[cfg(feature = "embeddings")]
use crate::embedding::EmbeddingModel;
use crate::model::{LanguageModel, StreamResult};
use crate::profile::{
    Env, HttpAuth, ProviderConfig, RequestAuth, resolve_request_auth_with_default_keys,
};
use crate::types::{
    ContentPart, FileSource, FinishReason, GenerateRequest, GenerateResponse, ImageSource, Message,
    Role, StreamChunk, Tool, ToolChoice, Usage, Warning,
};
use crate::{DittoError, Result};

#[derive(Clone)]
pub struct OpenAICompatible {
    http: reqwest::Client,
    base_url: String,
    auth: Option<RequestAuth>,
    default_model: String,
}

impl OpenAICompatible {
    pub fn new(api_key: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .expect("reqwest client build should not fail");

        let api_key = api_key.into();
        let auth = if api_key.trim().is_empty() {
            None
        } else {
            HttpAuth::bearer(&api_key).ok().map(RequestAuth::Http)
        };

        Self {
            http,
            base_url: "https://api.openai.com/v1".to_string(),
            auth,
            default_model: String::new(),
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
        const DEFAULT_KEYS: &[&str] = &[
            "OPENAI_COMPAT_API_KEY",
            "OPENAI_API_KEY",
            "CODE_PM_OPENAI_API_KEY",
        ];

        let auth = match config.auth.clone() {
            Some(auth) => Some(
                resolve_request_auth_with_default_keys(
                    &auth,
                    env,
                    DEFAULT_KEYS,
                    "authorization",
                    Some("Bearer "),
                )
                .await?,
            ),
            None => DEFAULT_KEYS
                .iter()
                .find_map(|key| env.get(key))
                .and_then(|token| HttpAuth::bearer(&token).ok().map(RequestAuth::Http)),
        };

        let mut out = Self::new("");
        out.auth = auth;
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
        match self.auth.as_ref() {
            Some(auth) => auth.apply(req),
            None => req,
        }
    }

    fn chat_completions_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/chat/completions") {
            base.to_string()
        } else {
            format!("{base}/chat/completions")
        }
    }

    fn files_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/files") {
            base.to_string()
        } else {
            format!("{base}/files")
        }
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
        #[derive(Deserialize)]
        struct FilesUploadResponse {
            id: String,
        }

        let filename = filename.into();
        let mut file_part = Part::bytes(bytes).file_name(filename);
        if let Some(media_type) = media_type {
            file_part = file_part.mime_str(media_type).map_err(|err| {
                DittoError::InvalidResponse(format!("invalid file upload media type: {err}"))
            })?;
        }

        let form = Form::new()
            .text("purpose", purpose.into())
            .part("file", file_part);

        let url = self.files_url();
        let mut req = self.http.post(url);
        req = self.apply_auth(req);
        let response = req.multipart(form).send().await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(DittoError::Api { status, body: text });
        }

        let parsed = response.json::<FilesUploadResponse>().await?;
        Ok(parsed.id)
    }

    fn resolve_model<'a>(&'a self, request: &'a GenerateRequest) -> Result<&'a str> {
        if let Some(model) = request.model.as_deref().filter(|m| !m.trim().is_empty()) {
            return Ok(model);
        }
        if !self.default_model.trim().is_empty() {
            return Ok(self.default_model.as_str());
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

    fn tool_choice_to_openai(choice: &ToolChoice, warnings: &mut Vec<Warning>) -> Value {
        match choice {
            ToolChoice::Auto => Value::String("auto".to_string()),
            ToolChoice::None => Value::String("none".to_string()),
            ToolChoice::Required => {
                warnings.push(Warning::Compatibility {
                    feature: "tool_choice.required".to_string(),
                    details: "chat/completions does not support `required`; using `auto`"
                        .to_string(),
                });
                Value::String("auto".to_string())
            }
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

    fn parse_finish_reason(reason: Option<&str>) -> FinishReason {
        match reason {
            Some("stop") => FinishReason::Stop,
            Some("length") => FinishReason::Length,
            Some("tool_calls") => FinishReason::ToolCalls,
            Some("content_filter") => FinishReason::ContentFilter,
            Some("error") => FinishReason::Error,
            _ => FinishReason::Unknown,
        }
    }

    fn parse_usage(value: &Value) -> Usage {
        let mut usage = Usage::default();
        if let Some(obj) = value.as_object() {
            usage.input_tokens = obj.get("prompt_tokens").and_then(Value::as_u64);
            usage.output_tokens = obj.get("completion_tokens").and_then(Value::as_u64);
            usage.total_tokens = obj.get("total_tokens").and_then(Value::as_u64);
        }
        usage.merge_total();
        usage
    }
}

#[derive(Debug, Deserialize)]
struct ChatCompletionsResponse {
    id: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    choices: Vec<ChatChoice>,
    #[serde(default)]
    usage: Option<Value>,
}

#[derive(Debug, Deserialize, Default)]
struct ChatChoice {
    #[serde(default)]
    message: ChatMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct ChatMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ChatToolCall>>,
}

#[derive(Debug, Deserialize, Default)]
struct ChatToolCall {
    #[serde(default)]
    id: String,
    #[serde(default)]
    function: ChatToolFunction,
}

#[derive(Debug, Deserialize, Default)]
struct ChatToolFunction {
    #[serde(default)]
    name: String,
    #[serde(default)]
    arguments: String,
}

#[cfg(feature = "streaming")]
#[derive(Debug, Deserialize, Default)]
struct ChatCompletionsChunk {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    choices: Vec<ChatChoiceChunk>,
    #[serde(default)]
    usage: Option<Value>,
}

#[cfg(feature = "streaming")]
#[derive(Debug, Deserialize, Default)]
struct ChatChoiceChunk {
    #[serde(default)]
    delta: ChatDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[cfg(feature = "streaming")]
#[derive(Debug, Deserialize, Default)]
struct ChatDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ChatToolCallDelta>>,
}

#[cfg(feature = "streaming")]
#[derive(Debug, Deserialize)]
struct ChatToolCallDelta {
    #[serde(default)]
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<ChatToolFunctionDelta>,
}

#[cfg(feature = "streaming")]
#[derive(Debug, Deserialize, Default)]
struct ChatToolFunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[cfg(feature = "streaming")]
#[derive(Debug, Default)]
struct StreamToolCallState {
    id: Option<String>,
    name: Option<String>,
    started: bool,
    pending_arguments: String,
}

#[cfg(feature = "streaming")]
#[derive(Debug, Default)]
struct StreamState {
    response_id: Option<String>,
    tool_calls: Vec<StreamToolCallState>,
}

#[cfg(feature = "streaming")]
fn finalize_stream_state(state: &mut StreamState) -> Vec<StreamChunk> {
    let mut out = Vec::<StreamChunk>::new();
    let mut warnings = Vec::<Warning>::new();

    for (idx, slot) in state.tool_calls.iter_mut().enumerate() {
        if slot.started {
            continue;
        }

        let name = slot.name.as_deref().unwrap_or("").trim();
        let has_any_data = slot.id.as_deref().is_some_and(|v| !v.trim().is_empty())
            || !name.is_empty()
            || !slot.pending_arguments.is_empty();

        if !has_any_data {
            continue;
        }

        let id = match slot.id.as_deref().filter(|v| !v.trim().is_empty()) {
            Some(id) => id.to_string(),
            None => {
                let synthesized = format!("call_{idx}");
                slot.id = Some(synthesized.clone());
                warnings.push(Warning::Compatibility {
                    feature: "tool_call.id".to_string(),
                    details: format!(
                        "stream ended before tool_call id was received; synthesizing {synthesized}"
                    ),
                });
                synthesized
            }
        };

        if name.is_empty() {
            warnings.push(Warning::Compatibility {
                feature: "tool_call.name".to_string(),
                details: format!(
                    "stream ended before tool_call name was received for id={id}; dropping tool call"
                ),
            });
            slot.pending_arguments.clear();
            continue;
        }

        out.push(StreamChunk::ToolCallStart {
            id: id.clone(),
            name: name.to_string(),
        });
        slot.started = true;

        if !slot.pending_arguments.is_empty() {
            out.push(StreamChunk::ToolCallDelta {
                id,
                arguments_delta: std::mem::take(&mut slot.pending_arguments),
            });
        }
    }

    if !warnings.is_empty() {
        out.insert(0, StreamChunk::Warnings { warnings });
    }

    out
}

#[cfg(feature = "streaming")]
fn parse_stream_data(state: &mut StreamState, data: &str) -> Result<(Vec<StreamChunk>, bool)> {
    let chunk = serde_json::from_str::<ChatCompletionsChunk>(data)?;
    let mut out = Vec::<StreamChunk>::new();
    let mut done = false;

    if state.response_id.is_none() {
        if let Some(id) = chunk.id.as_deref().filter(|id| !id.trim().is_empty()) {
            state.response_id = Some(id.to_string());
            out.push(StreamChunk::ResponseId { id: id.to_string() });
        }
    }

    if let Some(usage) = chunk.usage.as_ref() {
        out.push(StreamChunk::Usage(OpenAICompatible::parse_usage(usage)));
    }

    let Some(choice) = chunk.choices.first() else {
        return Ok((out, done));
    };

    if let Some(content) = choice.delta.content.as_deref() {
        if !content.is_empty() {
            out.push(StreamChunk::TextDelta {
                text: content.to_string(),
            });
        }
    }

    if let Some(tool_calls) = choice.delta.tool_calls.as_ref() {
        for tool_call in tool_calls {
            let idx = tool_call.index;
            while state.tool_calls.len() <= idx {
                state.tool_calls.push(StreamToolCallState::default());
            }
            let slot = &mut state.tool_calls[idx];

            if let Some(id) = tool_call.id.as_deref().filter(|v| !v.trim().is_empty()) {
                slot.id = Some(id.to_string());
            }
            if let Some(function) = tool_call.function.as_ref() {
                if let Some(name) = function.name.as_deref().filter(|v| !v.trim().is_empty()) {
                    slot.name = Some(name.to_string());
                }

                let arguments = function.arguments.as_deref().unwrap_or("");
                if !arguments.is_empty() {
                    if slot.started {
                        if let Some(id) = slot.id.as_deref() {
                            out.push(StreamChunk::ToolCallDelta {
                                id: id.to_string(),
                                arguments_delta: arguments.to_string(),
                            });
                        }
                    } else {
                        slot.pending_arguments.push_str(arguments);
                    }
                }
            }

            if !slot.started {
                if let (Some(id), Some(name)) = (slot.id.as_deref(), slot.name.as_deref()) {
                    out.push(StreamChunk::ToolCallStart {
                        id: id.to_string(),
                        name: name.to_string(),
                    });
                    slot.started = true;
                    if !slot.pending_arguments.is_empty() {
                        out.push(StreamChunk::ToolCallDelta {
                            id: id.to_string(),
                            arguments_delta: std::mem::take(&mut slot.pending_arguments),
                        });
                    }
                }
            }
        }
    }

    if let Some(reason) = choice.finish_reason.as_deref() {
        out.extend(finalize_stream_state(state));
        done = true;
        out.push(StreamChunk::FinishReason(
            OpenAICompatible::parse_finish_reason(Some(reason)),
        ));
    }

    Ok((out, done))
}

#[async_trait]
impl LanguageModel for OpenAICompatible {
    fn provider(&self) -> &str {
        "openai-compatible"
    }

    fn model_id(&self) -> &str {
        self.default_model.as_str()
    }

    async fn generate(&self, request: GenerateRequest) -> Result<GenerateResponse> {
        let model = self.resolve_model(&request)?;
        let provider_options = request.parsed_provider_options()?.unwrap_or_default();
        let (messages, mut warnings) = Self::messages_to_chat_messages(&request.messages);

        let mut body = Map::<String, Value>::new();
        body.insert("model".to_string(), Value::String(model.to_string()));
        body.insert("messages".to_string(), Value::Array(messages));

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
        if let Some(stops) = request.stop_sequences.as_ref() {
            let stops = stops
                .iter()
                .filter(|s| !s.trim().is_empty())
                .map(|s| Value::String(s.clone()))
                .collect::<Vec<_>>();
            if !stops.is_empty() {
                body.insert("stop".to_string(), Value::Array(stops));
            }
        }

        if let Some(tools) = request.tools {
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
        if let Some(tool_choice) = request.tool_choice {
            if cfg!(feature = "tools") {
                body.insert(
                    "tool_choice".to_string(),
                    Self::tool_choice_to_openai(&tool_choice, &mut warnings),
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

        let url = self.chat_completions_url();
        let mut req = self.http.post(url);
        req = self.apply_auth(req);
        let response = req.json(&body).send().await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(DittoError::Api { status, body: text });
        }

        let parsed = response.json::<ChatCompletionsResponse>().await?;
        let choice = parsed.choices.first().ok_or_else(|| {
            DittoError::InvalidResponse("chat/completions response has no choices".to_string())
        })?;

        let mut content = Vec::<ContentPart>::new();
        if let Some(text) = choice.message.content.as_deref().filter(|t| !t.is_empty()) {
            content.push(ContentPart::Text {
                text: text.to_string(),
            });
        }
        if let Some(tool_calls) = choice.message.tool_calls.as_ref() {
            for tool_call in tool_calls {
                let arguments_raw = tool_call.function.arguments.as_str();
                let raw = arguments_raw.trim();
                let raw_json = if raw.is_empty() { "{}" } else { raw };
                let arguments = serde_json::from_str::<Value>(raw_json).unwrap_or_else(|err| {
                    warnings.push(Warning::Compatibility {
                        feature: "tool_call.arguments".to_string(),
                        details: format!(
                            "failed to parse tool_call arguments as JSON for id={}: {err}; preserving raw string",
                            tool_call.id
                        ),
                    });
                    Value::String(arguments_raw.to_string())
                });
                content.push(ContentPart::ToolCall {
                    id: tool_call.id.clone(),
                    name: tool_call.function.name.clone(),
                    arguments,
                });
            }
        }

        let usage = parsed
            .usage
            .as_ref()
            .map(Self::parse_usage)
            .unwrap_or_default();

        let finish_reason = Self::parse_finish_reason(choice.finish_reason.as_deref());

        Ok(GenerateResponse {
            content,
            finish_reason,
            usage,
            warnings,
            provider_metadata: Some(serde_json::json!({ "id": parsed.id, "model": parsed.model })),
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
            let (messages, mut warnings) = Self::messages_to_chat_messages(&request.messages);

            let mut body = Map::<String, Value>::new();
            body.insert("model".to_string(), Value::String(model.to_string()));
            body.insert("messages".to_string(), Value::Array(messages));
            body.insert("stream".to_string(), Value::Bool(true));

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
            if let Some(stops) = request.stop_sequences.as_ref() {
                let stops = stops
                    .iter()
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| Value::String(s.clone()))
                    .collect::<Vec<_>>();
                if !stops.is_empty() {
                    body.insert("stop".to_string(), Value::Array(stops));
                }
            }

            if let Some(tools) = request.tools {
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
            if let Some(tool_choice) = request.tool_choice {
                if cfg!(feature = "tools") {
                    body.insert(
                        "tool_choice".to_string(),
                        Self::tool_choice_to_openai(&tool_choice, &mut warnings),
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

            let url = self.chat_completions_url();
            let req = self
                .http
                .post(url)
                .header("Accept", "text/event-stream")
                .json(&body);
            let response = self.apply_auth(req).send().await?;

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
                (data_stream, buffer, StreamState::default(), false),
                |(mut data_stream, mut buffer, mut state, mut done)| async move {
                    loop {
                        if let Some(item) = buffer.pop_front() {
                            return Some((item, (data_stream, buffer, state, done)));
                        }

                        if done {
                            return None;
                        }

                        let next = data_stream.next().await;
                        match next {
                            Some(Ok(data)) => match parse_stream_data(&mut state, &data) {
                                Ok((chunks, is_done)) => {
                                    for chunk in chunks {
                                        buffer.push_back(Ok(chunk));
                                    }
                                    if is_done {
                                        done = true;
                                    }
                                }
                                Err(err) => {
                                    done = true;
                                    buffer.push_back(Err(err));
                                }
                            },
                            Some(Err(err)) => {
                                done = true;
                                buffer.push_back(Err(err));
                            }
                            None => {
                                done = true;
                                for chunk in finalize_stream_state(&mut state) {
                                    buffer.push_back(Ok(chunk));
                                }
                                let has_tool_calls =
                                    state.tool_calls.iter().any(|slot| slot.started);
                                buffer.push_back(Ok(StreamChunk::FinishReason(
                                    if has_tool_calls {
                                        FinishReason::ToolCalls
                                    } else {
                                        FinishReason::Stop
                                    },
                                )));
                            }
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
pub struct OpenAICompatibleEmbeddings {
    http: reqwest::Client,
    base_url: String,
    auth: Option<RequestAuth>,
    model: String,
}

#[cfg(feature = "embeddings")]
impl OpenAICompatibleEmbeddings {
    pub fn new(api_key: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .expect("reqwest client build should not fail");

        let api_key = api_key.into();
        let auth = if api_key.trim().is_empty() {
            None
        } else {
            HttpAuth::bearer(&api_key).ok().map(RequestAuth::Http)
        };

        Self {
            http,
            base_url: "https://api.openai.com/v1".to_string(),
            auth,
            model: String::new(),
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
        const DEFAULT_KEYS: &[&str] = &[
            "OPENAI_COMPAT_API_KEY",
            "OPENAI_API_KEY",
            "CODE_PM_OPENAI_API_KEY",
        ];

        let auth = match config.auth.clone() {
            Some(auth) => Some(
                resolve_request_auth_with_default_keys(
                    &auth,
                    env,
                    DEFAULT_KEYS,
                    "authorization",
                    Some("Bearer "),
                )
                .await?,
            ),
            None => DEFAULT_KEYS
                .iter()
                .find_map(|key| env.get(key))
                .and_then(|token| HttpAuth::bearer(&token).ok().map(RequestAuth::Http)),
        };

        let mut out = Self::new("");
        out.auth = auth;
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
        match self.auth.as_ref() {
            Some(auth) => auth.apply(req),
            None => req,
        }
    }

    fn embeddings_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/embeddings") {
            base.to_string()
        } else {
            format!("{base}/embeddings")
        }
    }

    fn resolve_model(&self) -> Result<&str> {
        if !self.model.trim().is_empty() {
            return Ok(self.model.as_str());
        }
        Err(DittoError::InvalidResponse(
            "openai-compatible embedding model is not set (set OpenAICompatibleEmbeddings::with_model)"
                .to_string(),
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
impl EmbeddingModel for OpenAICompatibleEmbeddings {
    fn provider(&self) -> &str {
        "openai-compatible"
    }

    fn model_id(&self) -> &str {
        self.model.as_str()
    }

    async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let model = self.resolve_model()?;
        let url = self.embeddings_url();

        let req = self.http.post(url);
        let response = self
            .apply_auth(req)
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
    use httpmock::{Method::POST, MockServer};
    use std::collections::BTreeMap;

    #[tokio::test]
    async fn from_config_resolves_api_key_and_model() -> Result<()> {
        let config = ProviderConfig {
            base_url: Some("http://localhost:1234/v1".to_string()),
            default_model: Some("test-model".to_string()),
            auth: Some(crate::ProviderAuth::ApiKeyEnv {
                keys: vec!["CODEPM_TEST_OPENAI_COMPAT_KEY".to_string()],
            }),
            ..ProviderConfig::default()
        };
        let env = Env {
            dotenv: std::collections::BTreeMap::from([(
                "CODEPM_TEST_OPENAI_COMPAT_KEY".to_string(),
                "sk-test".to_string(),
            )]),
        };

        let client = OpenAICompatible::from_config(&config, &env).await?;
        assert_eq!(client.provider(), "openai-compatible");
        assert_eq!(client.model_id(), "test-model");
        Ok(())
    }

    #[tokio::test]
    async fn upload_file_uses_custom_auth_header() -> Result<()> {
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v1/files")
                    .header("api-key", "sk-test")
                    .body_includes("name=\"purpose\"")
                    .body_includes("assistants")
                    .body_includes("name=\"file\"")
                    .body_includes("hello");
                then.status(200)
                    .header("content-type", "application/json")
                    .body("{\"id\":\"file_123\"}");
            })
            .await;

        let config = ProviderConfig {
            base_url: Some(server.url("/v1")),
            default_model: Some("test-model".to_string()),
            auth: Some(crate::ProviderAuth::HttpHeaderEnv {
                header: "api-key".to_string(),
                keys: vec!["CODEPM_TEST_OPENAI_COMPAT_KEY".to_string()],
                prefix: None,
            }),
            ..ProviderConfig::default()
        };
        let env = Env {
            dotenv: BTreeMap::from([(
                "CODEPM_TEST_OPENAI_COMPAT_KEY".to_string(),
                "sk-test".to_string(),
            )]),
        };

        let client = OpenAICompatible::from_config(&config, &env).await?;
        let id = client.upload_file("hello.txt", b"hello".to_vec()).await?;

        mock.assert_async().await;
        assert_eq!(id, "file_123");
        Ok(())
    }

    #[tokio::test]
    async fn upload_file_uses_query_param_auth() -> Result<()> {
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v1/files")
                    .query_param("api_key", "sk-test")
                    .body_includes("name=\"purpose\"")
                    .body_includes("assistants")
                    .body_includes("name=\"file\"")
                    .body_includes("hello");
                then.status(200)
                    .header("content-type", "application/json")
                    .body("{\"id\":\"file_123\"}");
            })
            .await;

        let config = ProviderConfig {
            base_url: Some(server.url("/v1")),
            default_model: Some("test-model".to_string()),
            auth: Some(crate::ProviderAuth::QueryParamEnv {
                param: "api_key".to_string(),
                keys: vec!["CODEPM_TEST_OPENAI_COMPAT_KEY".to_string()],
                prefix: None,
            }),
            ..ProviderConfig::default()
        };
        let env = Env {
            dotenv: BTreeMap::from([(
                "CODEPM_TEST_OPENAI_COMPAT_KEY".to_string(),
                "sk-test".to_string(),
            )]),
        };

        let client = OpenAICompatible::from_config(&config, &env).await?;
        let id = client.upload_file("hello.txt", b"hello".to_vec()).await?;

        mock.assert_async().await;
        assert_eq!(id, "file_123");
        Ok(())
    }

    #[cfg(feature = "embeddings")]
    #[tokio::test]
    async fn embeddings_from_config_resolves_model() -> Result<()> {
        let config = ProviderConfig {
            base_url: Some("http://localhost:1234/v1".to_string()),
            default_model: Some("test-embed-model".to_string()),
            auth: Some(crate::ProviderAuth::ApiKeyEnv {
                keys: vec!["CODEPM_TEST_OPENAI_COMPAT_KEY".to_string()],
            }),
            ..ProviderConfig::default()
        };
        let env = Env {
            dotenv: std::collections::BTreeMap::from([(
                "CODEPM_TEST_OPENAI_COMPAT_KEY".to_string(),
                "sk-test".to_string(),
            )]),
        };

        let client = OpenAICompatibleEmbeddings::from_config(&config, &env).await?;
        assert_eq!(client.provider(), "openai-compatible");
        assert_eq!(client.model_id(), "test-embed-model");
        Ok(())
    }

    #[cfg(feature = "embeddings")]
    #[tokio::test]
    async fn embeddings_embed_posts_to_embeddings_endpoint_with_query_param_auth() -> Result<()> {
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v1/embeddings")
                    .query_param("api_key", "sk-test")
                    .body_includes("\"model\":\"test-embed-model\"")
                    .body_includes("\"input\":[\"hello\"]");
                then.status(200)
                    .header("content-type", "application/json")
                    .body("{\"data\":[{\"embedding\":[1.0,2.0]}]}");
            })
            .await;

        let config = ProviderConfig {
            base_url: Some(server.url("/v1")),
            default_model: Some("test-embed-model".to_string()),
            auth: Some(crate::ProviderAuth::QueryParamEnv {
                param: "api_key".to_string(),
                keys: vec!["CODEPM_TEST_OPENAI_COMPAT_KEY".to_string()],
                prefix: None,
            }),
            ..ProviderConfig::default()
        };
        let env = Env {
            dotenv: BTreeMap::from([(
                "CODEPM_TEST_OPENAI_COMPAT_KEY".to_string(),
                "sk-test".to_string(),
            )]),
        };

        let client = OpenAICompatibleEmbeddings::from_config(&config, &env).await?;
        let out = client.embed(vec!["hello".to_string()]).await?;

        mock.assert_async().await;
        assert_eq!(out, vec![vec![1.0, 2.0]]);
        Ok(())
    }

    #[test]
    fn tool_choice_required_maps_to_auto() {
        let mut warnings = Vec::<Warning>::new();
        let mapped = OpenAICompatible::tool_choice_to_openai(&ToolChoice::Required, &mut warnings);
        assert_eq!(mapped, Value::String("auto".to_string()));
        assert!(
            warnings
                .iter()
                .any(|w| matches!(w, Warning::Compatibility { .. }))
        );
    }

    #[test]
    fn converts_tool_result_to_tool_message() {
        let messages = vec![Message {
            role: Role::Tool,
            content: vec![ContentPart::ToolResult {
                tool_call_id: "call_1".to_string(),
                content: "{\"ok\":true}".to_string(),
                is_error: None,
            }],
        }];

        let (mapped, warnings) = OpenAICompatible::messages_to_chat_messages(&messages);
        assert!(warnings.is_empty());
        assert_eq!(
            mapped,
            vec![serde_json::json!({
                "role": "tool",
                "tool_call_id": "call_1",
                "content": "{\"ok\":true}",
            })]
        );
    }

    #[test]
    fn preserves_raw_tool_call_arguments_string() {
        let messages = vec![Message {
            role: Role::Assistant,
            content: vec![ContentPart::ToolCall {
                id: "call_1".to_string(),
                name: "add".to_string(),
                arguments: Value::String("{\"a\":1}".to_string()),
            }],
        }];

        let (mapped, warnings) = OpenAICompatible::messages_to_chat_messages(&messages);
        assert!(warnings.is_empty());
        assert_eq!(mapped.len(), 1);

        let tool_calls = mapped[0]
            .get("tool_calls")
            .and_then(Value::as_array)
            .expect("tool_calls array");
        assert_eq!(tool_calls.len(), 1);
        let arguments = tool_calls[0]
            .get("function")
            .and_then(Value::as_object)
            .and_then(|obj| obj.get("arguments"))
            .and_then(Value::as_str)
            .expect("tool_call arguments string");
        assert_eq!(arguments, "{\"a\":1}");
    }

    #[test]
    fn converts_pdf_file_part_to_chat_file_content() {
        let messages = vec![Message {
            role: Role::User,
            content: vec![ContentPart::File {
                filename: Some("doc.pdf".to_string()),
                media_type: "application/pdf".to_string(),
                source: FileSource::Base64 {
                    data: "AQIDBAU=".to_string(),
                },
            }],
        }];

        let (mapped, warnings) = OpenAICompatible::messages_to_chat_messages(&messages);
        assert!(warnings.is_empty());
        assert_eq!(mapped.len(), 1);
        assert_eq!(mapped[0].get("role").and_then(Value::as_str), Some("user"));
        let content = mapped[0]
            .get("content")
            .and_then(Value::as_array)
            .expect("content array");
        assert_eq!(content.len(), 1);
        assert_eq!(content[0].get("type").and_then(Value::as_str), Some("file"));
        assert_eq!(
            content[0]
                .get("file")
                .and_then(Value::as_object)
                .and_then(|o| o.get("filename"))
                .and_then(Value::as_str),
            Some("doc.pdf")
        );
        assert_eq!(
            content[0]
                .get("file")
                .and_then(Value::as_object)
                .and_then(|o| o.get("file_data"))
                .and_then(Value::as_str),
            Some("data:application/pdf;base64,AQIDBAU=")
        );
    }

    #[cfg(feature = "streaming")]
    #[test]
    fn parses_streaming_tool_call_deltas() -> Result<()> {
        let mut state = StreamState::default();

        let (chunks, done) = parse_stream_data(
            &mut state,
            &serde_json::json!({
                "choices": [{
                    "delta": {
                        "tool_calls": [{
                            "index": 0,
                            "id": "call_1",
                            "function": { "name": "add", "arguments": "{\"a\": 4" }
                        }]
                    }
                }]
            })
            .to_string(),
        )?;
        assert!(!done);
        assert_eq!(
            chunks,
            vec![
                StreamChunk::ToolCallStart {
                    id: "call_1".to_string(),
                    name: "add".to_string(),
                },
                StreamChunk::ToolCallDelta {
                    id: "call_1".to_string(),
                    arguments_delta: "{\"a\": 4".to_string(),
                }
            ]
        );

        let (chunks, done) = parse_stream_data(
            &mut state,
            &serde_json::json!({
                "choices": [{
                    "delta": {
                        "tool_calls": [{
                            "index": 0,
                            "function": { "arguments": ", \"b\": 2}" }
                        }]
                    },
                    "finish_reason": "tool_calls"
                }]
            })
            .to_string(),
        )?;
        assert!(done);
        assert_eq!(
            chunks,
            vec![
                StreamChunk::ToolCallDelta {
                    id: "call_1".to_string(),
                    arguments_delta: ", \"b\": 2}".to_string(),
                },
                StreamChunk::FinishReason(FinishReason::ToolCalls),
            ]
        );

        Ok(())
    }

    #[cfg(feature = "streaming")]
    #[test]
    fn parses_streaming_response_id() -> Result<()> {
        let mut state = StreamState::default();

        let (chunks, done) = parse_stream_data(
            &mut state,
            &serde_json::json!({
                "id": "resp_1",
                "choices": [{
                    "delta": { "content": "hi" }
                }]
            })
            .to_string(),
        )?;

        assert!(!done);
        assert_eq!(
            chunks,
            vec![
                StreamChunk::ResponseId {
                    id: "resp_1".to_string()
                },
                StreamChunk::TextDelta {
                    text: "hi".to_string()
                }
            ]
        );
        Ok(())
    }

    #[cfg(feature = "streaming")]
    #[test]
    fn flushes_tool_call_without_id_on_finish_reason() -> Result<()> {
        let mut state = StreamState::default();

        let (chunks, done) = parse_stream_data(
            &mut state,
            &serde_json::json!({
                "choices": [{
                    "delta": {
                        "tool_calls": [{
                            "index": 0,
                            "function": { "name": "add", "arguments": "{\"a\": 1" }
                        }]
                    }
                }]
            })
            .to_string(),
        )?;
        assert!(!done);
        assert!(chunks.is_empty());

        let (chunks, done) = parse_stream_data(
            &mut state,
            &serde_json::json!({
                "choices": [{
                    "delta": {},
                    "finish_reason": "tool_calls"
                }]
            })
            .to_string(),
        )?;
        assert!(done);
        assert!(
            matches!(chunks.first(), Some(StreamChunk::Warnings { .. })),
            "expected warnings for synthesized tool_call id"
        );
        assert!(
            chunks
                .iter()
                .any(|c| matches!(c, StreamChunk::ToolCallStart { .. })),
            "expected tool call start"
        );
        assert!(
            chunks
                .iter()
                .any(|c| matches!(c, StreamChunk::ToolCallDelta { .. })),
            "expected tool call delta"
        );
        assert!(
            chunks
                .iter()
                .any(|c| matches!(c, StreamChunk::FinishReason(FinishReason::ToolCalls))),
            "expected finish reason tool_calls"
        );
        Ok(())
    }
}
