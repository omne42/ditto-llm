use std::collections::{HashMap, VecDeque};

use async_trait::async_trait;
use futures_util::StreamExt;
use futures_util::stream;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::model::{LanguageModel, StreamResult};
use crate::profile::{Env, ProviderAuth, ProviderConfig, resolve_auth_token_with_default_keys};
use crate::types::{
    ContentPart, FileSource, FinishReason, GenerateRequest, GenerateResponse, ImageSource, Message,
    Role, StreamChunk, Tool, ToolChoice, Usage, Warning,
};
use crate::{DittoError, Result};

#[cfg(feature = "embeddings")]
use crate::embedding::EmbeddingModel;

const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

#[derive(Clone)]
pub struct Google {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    default_model: String,
}

impl Google {
    pub fn new(api_key: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            http,
            base_url: DEFAULT_BASE_URL.to_string(),
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
        const DEFAULT_KEYS: &[&str] =
            &["GOOGLE_API_KEY", "GEMINI_API_KEY", "CODE_PM_GOOGLE_API_KEY"];
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

    fn resolve_model<'a>(&'a self, request: &'a GenerateRequest) -> Result<&'a str> {
        if let Some(model) = request.model.as_deref().filter(|m| !m.trim().is_empty()) {
            return Ok(model);
        }
        if !self.default_model.trim().is_empty() {
            return Ok(self.default_model.as_str());
        }
        Err(DittoError::InvalidResponse(
            "google model is not set (set request.model or Google::with_model)".to_string(),
        ))
    }

    fn model_path(model: &str) -> String {
        let model = model.trim();
        if model.starts_with("models/") {
            model.to_string()
        } else {
            format!("models/{model}")
        }
    }

    fn generate_url(&self, model: &str) -> String {
        let base = self.base_url.trim_end_matches('/');
        let path = Self::model_path(model);
        format!("{base}/{path}:generateContent")
    }

    fn stream_url(&self, model: &str) -> String {
        let base = self.base_url.trim_end_matches('/');
        let path = Self::model_path(model);
        format!("{base}/{path}:streamGenerateContent?alt=sse")
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

    fn convert_messages(
        model: &str,
        messages: &[Message],
        tool_names: &HashMap<String, String>,
        warnings: &mut Vec<Warning>,
    ) -> Result<(Vec<Value>, Option<Value>)> {
        let is_gemma = model.to_lowercase().starts_with("gemma-");
        let mut system_parts = Vec::<String>::new();
        let mut contents = Vec::<Value>::new();
        let mut system_messages_allowed = true;

        for message in messages {
            match message.role {
                Role::System => {
                    if !system_messages_allowed {
                        return Err(DittoError::InvalidResponse(
                            "system messages are only supported at the beginning for google provider".to_string(),
                        ));
                    }
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
                    if !text.trim().is_empty() {
                        system_parts.push(text);
                    }
                }
                Role::User => {
                    system_messages_allowed = false;
                    let mut parts = Vec::<Value>::new();
                    for part in &message.content {
                        match part {
                            ContentPart::Text { text } => {
                                if !text.is_empty() {
                                    parts.push(serde_json::json!({ "text": text }));
                                }
                            }
                            ContentPart::Image { source } => match source {
                                ImageSource::Url { url } => parts.push(serde_json::json!({
                                    "fileData": { "mimeType": "image/jpeg", "fileUri": url }
                                })),
                                ImageSource::Base64 { media_type, data } => {
                                    parts.push(serde_json::json!({
                                        "inlineData": { "mimeType": media_type, "data": data }
                                    }))
                                }
                            },
                            ContentPart::File {
                                media_type,
                                source,
                                ..
                            } => match source {
                                FileSource::Url { url } => parts.push(serde_json::json!({
                                    "fileData": { "mimeType": media_type, "fileUri": url }
                                })),
                                FileSource::Base64 { data } => parts.push(serde_json::json!({
                                    "inlineData": { "mimeType": media_type, "data": data }
                                })),
                                FileSource::FileId { file_id } => warnings.push(Warning::Unsupported {
                                    feature: "file_id".to_string(),
                                    details: Some(format!(
                                        "google provider does not support OpenAI file ids (file_id={file_id})"
                                    )),
                                }),
                            },
                            other => warnings.push(Warning::Unsupported {
                                feature: "user_content_part".to_string(),
                                details: Some(format!("unsupported user content part: {other:?}")),
                            }),
                        }
                    }
                    if !parts.is_empty() {
                        contents.push(serde_json::json!({ "role": "user", "parts": parts }));
                    }
                }
                Role::Assistant => {
                    system_messages_allowed = false;
                    let mut parts = Vec::<Value>::new();
                    for part in &message.content {
                        match part {
                            ContentPart::Text { text } => {
                                if !text.is_empty() {
                                    parts.push(serde_json::json!({ "text": text }));
                                }
                            }
                            ContentPart::Reasoning { text } => {
                                if !text.is_empty() {
                                    parts
                                        .push(serde_json::json!({ "text": text, "thought": true }));
                                }
                            }
                            ContentPart::ToolCall {
                                name, arguments, ..
                            } => {
                                parts.push(serde_json::json!({
                                    "functionCall": { "name": name, "args": arguments }
                                }));
                            }
                            other => warnings.push(Warning::Unsupported {
                                feature: "assistant_content_part".to_string(),
                                details: Some(format!(
                                    "unsupported assistant content part: {other:?}"
                                )),
                            }),
                        }
                    }
                    if !parts.is_empty() {
                        contents.push(serde_json::json!({ "role": "model", "parts": parts }));
                    }
                }
                Role::Tool => {
                    system_messages_allowed = false;
                    let mut parts = Vec::<Value>::new();
                    for part in &message.content {
                        match part {
                            ContentPart::ToolResult {
                                tool_call_id,
                                content,
                                ..
                            } => {
                                let Some(tool_name) = tool_names.get(tool_call_id) else {
                                    warnings.push(Warning::Compatibility {
                                        feature: "tool_result".to_string(),
                                        details: format!(
                                            "tool_result references unknown tool_call_id={tool_call_id}; skipped"
                                        ),
                                    });
                                    continue;
                                };
                                parts.push(serde_json::json!({
                                    "functionResponse": {
                                        "name": tool_name,
                                        "response": { "name": tool_name, "content": content }
                                    }
                                }));
                            }
                            other => warnings.push(Warning::Unsupported {
                                feature: "tool_content_part".to_string(),
                                details: Some(format!("unsupported tool content part: {other:?}")),
                            }),
                        }
                    }
                    if !parts.is_empty() {
                        contents.push(serde_json::json!({ "role": "user", "parts": parts }));
                    }
                }
            }
        }

        if is_gemma && !system_parts.is_empty() {
            if let Some(first) = contents.first_mut() {
                if first.get("role").and_then(Value::as_str) == Some("user") {
                    if let Some(parts) = first.get_mut("parts").and_then(Value::as_array_mut) {
                        let system_text = system_parts.join("\n\n") + "\n\n";
                        parts.insert(0, serde_json::json!({ "text": system_text }));
                        system_parts.clear();
                    }
                }
            }
        }

        let system_instruction = (!system_parts.is_empty() && !is_gemma).then(|| {
            serde_json::json!({
                "parts": system_parts.iter().map(|t| serde_json::json!({ "text": t })).collect::<Vec<_>>()
            })
        });

        Ok((contents, system_instruction))
    }

    fn tool_to_google(tool: Tool) -> Value {
        let mut out = Map::<String, Value>::new();
        out.insert("name".to_string(), Value::String(tool.name));
        out.insert(
            "description".to_string(),
            Value::String(tool.description.unwrap_or_default()),
        );
        if let Some(parameters) =
            crate::utils::json_schema::convert_json_schema_to_openapi_schema(&tool.parameters, true)
        {
            out.insert("parameters".to_string(), parameters);
        }
        Value::Object(out)
    }

    fn tool_config(choice: Option<&ToolChoice>) -> Option<Value> {
        let choice = choice?;
        let config = match choice {
            ToolChoice::Auto => serde_json::json!({ "functionCallingConfig": { "mode": "AUTO" } }),
            ToolChoice::None => serde_json::json!({ "functionCallingConfig": { "mode": "NONE" } }),
            ToolChoice::Required => {
                serde_json::json!({ "functionCallingConfig": { "mode": "ANY" } })
            }
            ToolChoice::Tool { name } => serde_json::json!({
                "functionCallingConfig": { "mode": "ANY", "allowedFunctionNames": [name] }
            }),
        };
        Some(config)
    }

    fn map_finish_reason(finish_reason: Option<&str>, has_tool_calls: bool) -> FinishReason {
        match finish_reason {
            Some("STOP") => {
                if has_tool_calls {
                    FinishReason::ToolCalls
                } else {
                    FinishReason::Stop
                }
            }
            Some("MAX_TOKENS") => FinishReason::Length,
            Some(
                "IMAGE_SAFETY" | "RECITATION" | "SAFETY" | "BLOCKLIST" | "PROHIBITED_CONTENT"
                | "SPII",
            ) => FinishReason::ContentFilter,
            Some("MALFORMED_FUNCTION_CALL") => FinishReason::Error,
            _ => FinishReason::Unknown,
        }
    }

    fn parse_usage_metadata(value: &Value) -> Usage {
        let mut usage = Usage::default();
        if let Some(obj) = value.as_object() {
            usage.input_tokens = obj.get("promptTokenCount").and_then(Value::as_u64);
            usage.output_tokens = obj.get("candidatesTokenCount").and_then(Value::as_u64);
            usage.total_tokens = obj.get("totalTokenCount").and_then(Value::as_u64);
        }
        usage.merge_total();
        usage
    }
}

#[derive(Debug, Deserialize)]
struct GoogleGenerateResponse {
    #[serde(default)]
    candidates: Vec<Value>,
    #[serde(default)]
    usage_metadata: Option<Value>,
}

fn parse_google_candidate(
    candidate: &Value,
    tool_call_seq: &mut u64,
    has_tool_calls: &mut bool,
) -> Vec<ContentPart> {
    let mut out = Vec::<ContentPart>::new();
    let Some(parts) = candidate
        .get("content")
        .and_then(|c| c.get("parts"))
        .and_then(Value::as_array)
    else {
        return out;
    };

    for part in parts {
        if let Some(text) = part.get("text").and_then(Value::as_str) {
            if !text.is_empty() {
                out.push(ContentPart::Text {
                    text: text.to_string(),
                });
            }
            continue;
        }
        if let Some(call) = part.get("functionCall") {
            let Some(name) = call.get("name").and_then(Value::as_str) else {
                continue;
            };
            let args = call.get("args").cloned().unwrap_or(Value::Null);
            let id = format!("call_{}", *tool_call_seq);
            *tool_call_seq = tool_call_seq.saturating_add(1);
            *has_tool_calls = true;
            out.push(ContentPart::ToolCall {
                id,
                name: name.to_string(),
                arguments: args,
            });
        }
    }

    out
}

#[async_trait]
impl LanguageModel for Google {
    fn provider(&self) -> &str {
        "google"
    }

    fn model_id(&self) -> &str {
        self.default_model.as_str()
    }

    async fn generate(&self, request: GenerateRequest) -> Result<GenerateResponse> {
        let model = self.resolve_model(&request)?.to_string();
        let provider_options = request.parsed_provider_options()?.unwrap_or_default();

        let mut warnings = Vec::<Warning>::new();
        if provider_options.reasoning_effort.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "reasoning_effort".to_string(),
                details: Some("Google GenAI does not support reasoning_effort".to_string()),
            });
        }
        if provider_options.response_format.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "response_format".to_string(),
                details: Some("Google GenAI does not support response_format".to_string()),
            });
        }
        if provider_options.parallel_tool_calls == Some(true) {
            warnings.push(Warning::Unsupported {
                feature: "parallel_tool_calls".to_string(),
                details: Some("Google GenAI does not support parallel_tool_calls".to_string()),
            });
        }
        let tool_names = Self::build_tool_name_map(&request.messages);
        let (contents, system_instruction) =
            Self::convert_messages(&model, &request.messages, &tool_names, &mut warnings)?;

        let mut body = Map::<String, Value>::new();
        body.insert("contents".to_string(), Value::Array(contents));

        if let Some(system_instruction) = system_instruction {
            body.insert("systemInstruction".to_string(), system_instruction);
        }

        let mut generation_config = Map::<String, Value>::new();
        if let Some(max_tokens) = request.max_tokens {
            generation_config.insert(
                "maxOutputTokens".to_string(),
                Value::Number(max_tokens.into()),
            );
        }
        if let Some(temperature) = request.temperature {
            generation_config.insert(
                "temperature".to_string(),
                Value::Number(
                    serde_json::Number::from_f64(temperature as f64).unwrap_or_else(|| 0.into()),
                ),
            );
        }
        if let Some(top_p) = request.top_p {
            generation_config.insert(
                "topP".to_string(),
                Value::Number(
                    serde_json::Number::from_f64(top_p as f64).unwrap_or_else(|| 0.into()),
                ),
            );
        }
        if let Some(stop_sequences) = request.stop_sequences {
            generation_config.insert(
                "stopSequences".to_string(),
                Value::Array(stop_sequences.into_iter().map(Value::String).collect()),
            );
        }
        if !generation_config.is_empty() {
            body.insert(
                "generationConfig".to_string(),
                Value::Object(generation_config),
            );
        }

        if let Some(tools) = request.tools {
            if cfg!(feature = "tools") {
                let decls = tools
                    .into_iter()
                    .map(Self::tool_to_google)
                    .collect::<Vec<_>>();
                body.insert(
                    "tools".to_string(),
                    Value::Array(vec![serde_json::json!({ "functionDeclarations": decls })]),
                );
            } else {
                warnings.push(Warning::Unsupported {
                    feature: "tools".to_string(),
                    details: Some("ditto-llm built without tools feature".to_string()),
                });
            }
        }

        if let Some(tool_choice) = request.tool_choice.as_ref() {
            if cfg!(feature = "tools") {
                if let Some(tool_config) = Self::tool_config(Some(tool_choice)) {
                    body.insert("toolConfig".to_string(), tool_config);
                }
            }
        }

        let url = self.generate_url(&model);
        let response = self
            .http
            .post(url)
            .header("x-goog-api-key", &self.api_key)
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(DittoError::Api { status, body: text });
        }

        let parsed = response.json::<GoogleGenerateResponse>().await?;
        let mut tool_call_seq = 0u64;
        let mut has_tool_calls = false;
        let mut content = Vec::<ContentPart>::new();

        let finish_reason_str = parsed
            .candidates
            .first()
            .and_then(|c| c.get("finishReason"))
            .and_then(Value::as_str);

        if let Some(candidate) = parsed.candidates.first() {
            content.extend(parse_google_candidate(
                candidate,
                &mut tool_call_seq,
                &mut has_tool_calls,
            ));
        }

        let usage = parsed
            .usage_metadata
            .as_ref()
            .map(Self::parse_usage_metadata)
            .unwrap_or_default();

        let finish_reason = Self::map_finish_reason(finish_reason_str, has_tool_calls);

        Ok(GenerateResponse {
            content,
            finish_reason,
            usage,
            warnings,
            provider_metadata: None,
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
            let model = self.resolve_model(&request)?.to_string();
            let provider_options = request.parsed_provider_options()?.unwrap_or_default();

            let mut warnings = Vec::<Warning>::new();
            if provider_options.reasoning_effort.is_some() {
                warnings.push(Warning::Unsupported {
                    feature: "reasoning_effort".to_string(),
                    details: Some("Google GenAI does not support reasoning_effort".to_string()),
                });
            }
            if provider_options.response_format.is_some() {
                warnings.push(Warning::Unsupported {
                    feature: "response_format".to_string(),
                    details: Some("Google GenAI does not support response_format".to_string()),
                });
            }
            if provider_options.parallel_tool_calls == Some(true) {
                warnings.push(Warning::Unsupported {
                    feature: "parallel_tool_calls".to_string(),
                    details: Some("Google GenAI does not support parallel_tool_calls".to_string()),
                });
            }
            let tool_names = Self::build_tool_name_map(&request.messages);
            let (contents, system_instruction) =
                Self::convert_messages(&model, &request.messages, &tool_names, &mut warnings)?;

            let mut body = Map::<String, Value>::new();
            body.insert("contents".to_string(), Value::Array(contents));

            if let Some(system_instruction) = system_instruction {
                body.insert("systemInstruction".to_string(), system_instruction);
            }

            let mut generation_config = Map::<String, Value>::new();
            if let Some(max_tokens) = request.max_tokens {
                generation_config.insert(
                    "maxOutputTokens".to_string(),
                    Value::Number(max_tokens.into()),
                );
            }
            if let Some(temperature) = request.temperature {
                generation_config.insert(
                    "temperature".to_string(),
                    Value::Number(
                        serde_json::Number::from_f64(temperature as f64)
                            .unwrap_or_else(|| 0.into()),
                    ),
                );
            }
            if let Some(top_p) = request.top_p {
                generation_config.insert(
                    "topP".to_string(),
                    Value::Number(
                        serde_json::Number::from_f64(top_p as f64).unwrap_or_else(|| 0.into()),
                    ),
                );
            }
            if let Some(stop_sequences) = request.stop_sequences {
                generation_config.insert(
                    "stopSequences".to_string(),
                    Value::Array(stop_sequences.into_iter().map(Value::String).collect()),
                );
            }
            if !generation_config.is_empty() {
                body.insert(
                    "generationConfig".to_string(),
                    Value::Object(generation_config),
                );
            }

            if let Some(tools) = request.tools {
                if cfg!(feature = "tools") {
                    let decls = tools
                        .into_iter()
                        .map(Self::tool_to_google)
                        .collect::<Vec<_>>();
                    body.insert(
                        "tools".to_string(),
                        Value::Array(vec![serde_json::json!({ "functionDeclarations": decls })]),
                    );
                }
            }

            if let Some(tool_choice) = request.tool_choice.as_ref() {
                if cfg!(feature = "tools") {
                    if let Some(tool_config) = Self::tool_config(Some(tool_choice)) {
                        body.insert("toolConfig".to_string(), tool_config);
                    }
                }
            }

            let url = self.stream_url(&model);
            let response = self
                .http
                .post(url)
                .header("x-goog-api-key", &self.api_key)
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
                    String::new(),
                    false,
                    None::<String>,
                    None::<Usage>,
                    0u64,
                ),
                |(
                    mut data_stream,
                    mut buffer,
                    mut done,
                    mut last_text,
                    mut has_tool_calls,
                    mut pending_finish_reason,
                    mut pending_usage,
                    mut tool_call_seq,
                )| async move {
                    loop {
                        if let Some(item) = buffer.pop_front() {
                            return Some((
                                item,
                                (
                                    data_stream,
                                    buffer,
                                    done,
                                    last_text,
                                    has_tool_calls,
                                    pending_finish_reason,
                                    pending_usage,
                                    tool_call_seq,
                                ),
                            ));
                        }

                        if done {
                            return None;
                        }

                        let next = data_stream.next().await;
                        match next {
                            Some(Ok(data)) => match serde_json::from_str::<Value>(&data) {
                                Ok(chunk) => {
                                    if let Some(usage) = chunk.get("usageMetadata") {
                                        pending_usage = Some(Self::parse_usage_metadata(usage));
                                    }
                                    if let Some(finish) = chunk
                                        .get("candidates")
                                        .and_then(Value::as_array)
                                        .and_then(|c| c.first())
                                        .and_then(|c| c.get("finishReason"))
                                        .and_then(Value::as_str)
                                    {
                                        pending_finish_reason = Some(finish.to_string());
                                    }

                                    if let Some(candidate) = chunk
                                        .get("candidates")
                                        .and_then(Value::as_array)
                                        .and_then(|c| c.first())
                                    {
                                        let parts = candidate
                                            .get("content")
                                            .and_then(|c| c.get("parts"))
                                            .and_then(Value::as_array)
                                            .cloned()
                                            .unwrap_or_default();

                                        for part in parts {
                                            if let Some(text) =
                                                part.get("text").and_then(Value::as_str)
                                            {
                                                let delta = if text.starts_with(&last_text) {
                                                    text[last_text.len()..].to_string()
                                                } else {
                                                    text.to_string()
                                                };
                                                last_text = text.to_string();
                                                if !delta.is_empty() {
                                                    buffer.push_back(Ok(StreamChunk::TextDelta {
                                                        text: delta,
                                                    }));
                                                }
                                                continue;
                                            }
                                            if let Some(call) = part.get("functionCall") {
                                                let Some(name) =
                                                    call.get("name").and_then(Value::as_str)
                                                else {
                                                    continue;
                                                };
                                                let args = call
                                                    .get("args")
                                                    .cloned()
                                                    .unwrap_or(Value::Null);
                                                let id = format!("call_{}", tool_call_seq);
                                                tool_call_seq = tool_call_seq.saturating_add(1);
                                                has_tool_calls = true;
                                                buffer.push_back(Ok(StreamChunk::ToolCallStart {
                                                    id: id.clone(),
                                                    name: name.to_string(),
                                                }));
                                                buffer.push_back(Ok(StreamChunk::ToolCallDelta {
                                                    id,
                                                    arguments_delta: args.to_string(),
                                                }));
                                            }
                                        }
                                    }
                                }
                                Err(err) => {
                                    done = true;
                                    buffer.push_back(Err(err.into()));
                                }
                            },
                            Some(Err(err)) => {
                                done = true;
                                buffer.push_back(Err(err));
                            }
                            None => {
                                done = true;
                                if let Some(usage) = pending_usage.take() {
                                    buffer.push_back(Ok(StreamChunk::Usage(usage)));
                                }
                                buffer.push_back(Ok(StreamChunk::FinishReason(
                                    Self::map_finish_reason(
                                        pending_finish_reason.as_deref(),
                                        has_tool_calls,
                                    ),
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
pub struct GoogleEmbeddings {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
}

#[cfg(feature = "embeddings")]
impl GoogleEmbeddings {
    pub fn new(api_key: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            http,
            base_url: DEFAULT_BASE_URL.to_string(),
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
        const DEFAULT_KEYS: &[&str] =
            &["GOOGLE_API_KEY", "GEMINI_API_KEY", "CODE_PM_GOOGLE_API_KEY"];
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
            "google embedding model is not set (set GoogleEmbeddings::with_model)".to_string(),
        ))
    }

    fn embed_url(&self, suffix: &str) -> String {
        let base = self.base_url.trim_end_matches('/');
        let model = Google::model_path(self.model.as_str());
        format!("{base}/{model}:{suffix}")
    }
}

#[cfg(feature = "embeddings")]
#[derive(Debug, Deserialize)]
struct BatchEmbedResponse {
    #[serde(default)]
    embeddings: Vec<EmbeddingItem>,
}

#[cfg(feature = "embeddings")]
#[derive(Debug, Deserialize)]
struct SingleEmbedResponse {
    embedding: EmbeddingItem,
}

#[cfg(feature = "embeddings")]
#[derive(Debug, Deserialize)]
struct EmbeddingItem {
    values: Vec<f32>,
}

#[cfg(feature = "embeddings")]
#[async_trait]
impl EmbeddingModel for GoogleEmbeddings {
    fn provider(&self) -> &str {
        "google"
    }

    fn model_id(&self) -> &str {
        self.model.as_str()
    }

    async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let model = self.resolve_model()?;
        let _ = model;

        if texts.len() == 1 {
            let url = self.embed_url("embedContent");
            let response = self
                .http
                .post(url)
                .header("x-goog-api-key", &self.api_key)
                .json(&serde_json::json!({
                    "model": Google::model_path(self.model.as_str()),
                    "content": { "parts": [{ "text": texts[0] }] }
                }))
                .send()
                .await?;

            let status = response.status();
            if !status.is_success() {
                let text = response.text().await.unwrap_or_default();
                return Err(DittoError::Api { status, body: text });
            }

            let parsed = response.json::<SingleEmbedResponse>().await?;
            return Ok(vec![parsed.embedding.values]);
        }

        let url = self.embed_url("batchEmbedContents");
        let requests = texts
            .into_iter()
            .map(|text| {
                serde_json::json!({
                    "model": Google::model_path(self.model.as_str()),
                    "content": { "role": "user", "parts": [{ "text": text }] }
                })
            })
            .collect::<Vec<_>>();

        let response = self
            .http
            .post(url)
            .header("x-goog-api-key", &self.api_key)
            .json(&serde_json::json!({ "requests": requests }))
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(DittoError::Api { status, body: text });
        }

        let parsed = response.json::<BatchEmbedResponse>().await?;
        Ok(parsed.embeddings.into_iter().map(|e| e.values).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn converts_system_to_system_instruction() -> crate::Result<()> {
        let mut warnings = Vec::new();
        let tool_names = HashMap::new();
        let (contents, system) = Google::convert_messages(
            "gemini-pro",
            &[Message::system("sys"), Message::user("hi")],
            &tool_names,
            &mut warnings,
        )?;
        assert_eq!(warnings.len(), 0);
        assert_eq!(contents.len(), 1);
        assert!(system.is_some());
        Ok(())
    }

    #[test]
    fn tool_choice_maps_to_tool_config() {
        let config = Google::tool_config(Some(&ToolChoice::Tool {
            name: "add".to_string(),
        }))
        .expect("tool config");
        assert_eq!(
            config
                .get("functionCallingConfig")
                .and_then(|v| v.get("mode"))
                .and_then(Value::as_str),
            Some("ANY")
        );
    }

    #[test]
    fn tool_declaration_converts_schema() {
        let tool = Tool {
            name: "add".to_string(),
            description: Some("add".to_string()),
            parameters: json!({
                "type": "object",
                "properties": { "a": { "type": "integer" } }
            }),
            strict: None,
        };
        let decl = Google::tool_to_google(tool);
        assert_eq!(decl.get("name").and_then(Value::as_str), Some("add"));
        assert!(decl.get("parameters").is_some());
    }

    #[test]
    fn converts_pdf_file_part_to_inline_data() -> crate::Result<()> {
        let mut warnings = Vec::new();
        let tool_names = HashMap::new();
        let (contents, _system) = Google::convert_messages(
            "gemini-pro",
            &[Message {
                role: Role::User,
                content: vec![ContentPart::File {
                    filename: Some("doc.pdf".to_string()),
                    media_type: "application/pdf".to_string(),
                    source: FileSource::Base64 {
                        data: "AQIDBAU=".to_string(),
                    },
                }],
            }],
            &tool_names,
            &mut warnings,
        )?;
        assert!(warnings.is_empty());
        assert_eq!(contents.len(), 1);
        let parts = contents[0]
            .get("parts")
            .and_then(Value::as_array)
            .expect("parts array");
        assert_eq!(parts.len(), 1);
        assert_eq!(
            parts[0]
                .get("inlineData")
                .and_then(Value::as_object)
                .and_then(|o| o.get("mimeType"))
                .and_then(Value::as_str),
            Some("application/pdf")
        );
        Ok(())
    }
}
