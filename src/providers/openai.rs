use std::collections::{BTreeMap, VecDeque};

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
    Env, HttpAuth, ProviderAuth, ProviderConfig, RequestAuth, apply_http_query_params,
    resolve_request_auth_with_default_keys,
};
use crate::types::{
    ContentPart, FileSource, FinishReason, GenerateRequest, GenerateResponse, ImageSource, Message,
    Role, StreamChunk, Tool, ToolChoice, Usage, Warning,
};
use crate::{DittoError, Result};

#[derive(Clone)]
pub struct OpenAI {
    http: reqwest::Client,
    base_url: String,
    auth: Option<RequestAuth>,
    default_model: String,
    http_query_params: BTreeMap<String, String>,
}

impl OpenAI {
    pub fn new(api_key: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

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
        const DEFAULT_KEYS: &[&str] = &["OPENAI_API_KEY", "CODE_PM_OPENAI_API_KEY"];
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

    fn responses_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/responses") {
            base.to_string()
        } else {
            format!("{base}/responses")
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
        out.insert(
            "strict".to_string(),
            Value::Bool(tool.strict.unwrap_or(true)),
        );
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

    fn messages_to_input(messages: &[Message]) -> (Option<String>, Vec<Value>, Vec<Warning>) {
        let mut input = Vec::<Value>::new();
        let mut instructions = Vec::<String>::new();
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
                    let text = text.trim();
                    if !text.is_empty() {
                        instructions.push(text.to_string());
                    }
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
                            ContentPart::File {
                                filename,
                                media_type,
                                source,
                            } => {
                                if media_type != "application/pdf" {
                                    warnings.push(Warning::Unsupported {
                                        feature: "file".to_string(),
                                        details: Some(format!(
                                            "unsupported file media type for OpenAI Responses: {media_type}"
                                        )),
                                    });
                                    continue;
                                }

                                let item = match source {
                                    FileSource::Url { url } => {
                                        serde_json::json!({ "type": "input_file", "file_url": url })
                                    }
                                    FileSource::Base64 { data } => serde_json::json!({
                                        "type": "input_file",
                                        "filename": filename.clone().unwrap_or_else(|| "file.pdf".to_string()),
                                        "file_data": format!("data:{media_type};base64,{data}"),
                                    }),
                                    FileSource::FileId { file_id } => {
                                        serde_json::json!({ "type": "input_file", "file_id": file_id })
                                    }
                                };
                                content.push(item);
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
                                    "arguments": Self::tool_call_arguments_to_openai_string(arguments),
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

        let instructions = if instructions.is_empty() {
            None
        } else {
            Some(instructions.join("\n\n"))
        };
        (instructions, input, warnings)
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
    if let Some(parallel_tool_calls) = provider_options.parallel_tool_calls {
        body.insert(
            "parallel_tool_calls".to_string(),
            Value::Bool(parallel_tool_calls),
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

fn parse_openai_output(output: &[Value], warnings: &mut Vec<Warning>) -> Vec<ContentPart> {
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
                let raw = arguments_raw.trim();
                let raw_json = if raw.is_empty() { "{}" } else { raw };
                let arguments = serde_json::from_str::<Value>(raw_json).unwrap_or_else(|err| {
                    warnings.push(Warning::Compatibility {
                        feature: "tool_call.arguments".to_string(),
                        details: format!(
                            "failed to parse tool_call arguments as JSON for id={call_id}: {err}; preserving raw string"
                        ),
                    });
                    Value::String(arguments_raw.to_string())
                });
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
        let selected_provider_options = request.provider_options_value_for(self.provider())?;
        let provider_options = selected_provider_options
            .as_ref()
            .map(crate::types::ProviderOptions::from_value)
            .transpose()?
            .unwrap_or_default();
        let (instructions, input, mut warnings) = Self::messages_to_input(&request.messages);

        if request.stop_sequences.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "stop_sequences".to_string(),
                details: Some("OpenAI Responses API stop sequences are not supported".to_string()),
            });
        }

        let mut body = Map::<String, Value>::new();
        body.insert("model".to_string(), Value::String(model.to_string()));
        if let Some(instructions) = instructions {
            body.insert("instructions".to_string(), Value::String(instructions));
        }
        body.insert("input".to_string(), Value::Array(input));
        body.insert("stream".to_string(), Value::Bool(false));
        body.insert("store".to_string(), Value::Bool(false));

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
            body.insert(
                "max_output_tokens".to_string(),
                Value::Number(max_tokens.into()),
            );
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
        crate::types::merge_provider_options_into_body(
            &mut body,
            selected_provider_options.as_ref(),
            &["reasoning_effort", "response_format", "parallel_tool_calls"],
            "generate.provider_options",
            &mut warnings,
        );

        let url = self.responses_url();
        let req = self.http.post(url);
        let response = self.apply_auth(req).json(&body).send().await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(DittoError::Api { status, body: text });
        }

        let parsed = response.json::<ResponsesApiResponse>().await?;
        let content = parse_openai_output(&parsed.output, &mut warnings);
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
            let selected_provider_options = request.provider_options_value_for(self.provider())?;
            let provider_options = selected_provider_options
                .as_ref()
                .map(crate::types::ProviderOptions::from_value)
                .transpose()?
                .unwrap_or_default();
            let (instructions, input, mut warnings) = Self::messages_to_input(&request.messages);

            let mut body = Map::<String, Value>::new();
            body.insert("model".to_string(), Value::String(model.to_string()));
            if let Some(instructions) = instructions {
                body.insert("instructions".to_string(), Value::String(instructions));
            }
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
                body.insert(
                    "max_output_tokens".to_string(),
                    Value::Number(max_tokens.into()),
                );
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
            crate::types::merge_provider_options_into_body(
                &mut body,
                selected_provider_options.as_ref(),
                &["reasoning_effort", "response_format", "parallel_tool_calls"],
                "stream.provider_options",
                &mut warnings,
            );

            let url = self.responses_url();
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
                (data_stream, buffer, false, false, None::<String>),
                |(mut data_stream, mut buffer, mut done, mut has_tool_calls, mut response_id)| async move {
                    loop {
                        if let Some(item) = buffer.pop_front() {
                            return Some((
                                item,
                                (data_stream, buffer, done, has_tool_calls, response_id),
                            ));
                        }

                        if done {
                            return None;
                        }

                        let next = data_stream.next().await;
                        match next {
                            Some(Ok(data)) => {
                                match serde_json::from_str::<ResponsesStreamEvent>(&data) {
                                    Ok(event) => match event.kind.as_str() {
                                        "response.created" => {
                                            if response_id.is_none() {
                                                if let Some(id) = event
                                                    .response
                                                    .as_ref()
                                                    .and_then(|resp| {
                                                        resp.get("id").and_then(Value::as_str)
                                                    })
                                                    .filter(|id| !id.trim().is_empty())
                                                {
                                                    response_id = Some(id.to_string());
                                                    buffer.push_back(Ok(StreamChunk::ResponseId {
                                                        id: id.to_string(),
                                                    }));
                                                }
                                            }
                                        }
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
                                                if response_id.is_none() {
                                                    if let Some(id) =
                                                        resp.get("id").and_then(Value::as_str)
                                                    {
                                                        if !id.trim().is_empty() {
                                                            response_id = Some(id.to_string());
                                                            buffer.push_back(Ok(
                                                                StreamChunk::ResponseId {
                                                                    id: id.to_string(),
                                                                },
                                                            ));
                                                        }
                                                    }
                                                }
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
    auth: Option<RequestAuth>,
    model: String,
    http_query_params: BTreeMap<String, String>,
}

#[cfg(feature = "embeddings")]
impl OpenAIEmbeddings {
    pub fn new(api_key: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

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
        const DEFAULT_KEYS: &[&str] = &["OPENAI_API_KEY", "CODE_PM_OPENAI_API_KEY"];
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
    use serde_json::json;
    use std::collections::BTreeMap;

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

    #[tokio::test]
    async fn upload_file_posts_to_files_endpoint() -> crate::Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v1/files")
                    .header("authorization", "Bearer sk-test")
                    .body_includes("name=\"purpose\"")
                    .body_includes("assistants")
                    .body_includes("name=\"file\"")
                    .body_includes("hello");
                then.status(200)
                    .header("content-type", "application/json")
                    .body("{\"id\":\"file_123\"}");
            })
            .await;

        let client = OpenAI::new("sk-test").with_base_url(server.url("/v1"));
        let id = client.upload_file("hello.txt", b"hello".to_vec()).await?;

        mock.assert_async().await;
        assert_eq!(id, "file_123");
        Ok(())
    }

    #[tokio::test]
    async fn upload_file_uses_query_param_auth() -> crate::Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
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
                keys: vec!["CODEPM_TEST_OPENAI_KEY".to_string()],
                prefix: None,
            }),
            ..ProviderConfig::default()
        };
        let env = Env {
            dotenv: BTreeMap::from([("CODEPM_TEST_OPENAI_KEY".to_string(), "sk-test".to_string())]),
        };

        let client = OpenAI::from_config(&config, &env).await?;
        let id = client.upload_file("hello.txt", b"hello".to_vec()).await?;

        mock.assert_async().await;
        assert_eq!(id, "file_123");
        Ok(())
    }

    #[tokio::test]
    async fn upload_file_includes_default_query_params() -> crate::Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v1/files")
                    .query_param("api-version", "2024-02-01")
                    .header("authorization", "Bearer sk-test")
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
            http_query_params: BTreeMap::from([(
                "api-version".to_string(),
                "2024-02-01".to_string(),
            )]),
            auth: Some(crate::ProviderAuth::ApiKeyEnv {
                keys: vec!["CODEPM_TEST_OPENAI_KEY".to_string()],
            }),
            ..ProviderConfig::default()
        };
        let env = Env {
            dotenv: BTreeMap::from([("CODEPM_TEST_OPENAI_KEY".to_string(), "sk-test".to_string())]),
        };

        let client = OpenAI::from_config(&config, &env).await?;
        let id = client.upload_file("hello.txt", b"hello".to_vec()).await?;

        mock.assert_async().await;
        assert_eq!(id, "file_123");
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

        let (instructions, input, warnings) = OpenAI::messages_to_input(&messages);
        assert!(warnings.is_empty());
        assert_eq!(instructions.as_deref(), Some("sys"));
        assert_eq!(input.len(), 3);
        assert_eq!(input[0].get("role").and_then(Value::as_str), Some("user"));
        assert_eq!(
            input[1].get("type").and_then(Value::as_str),
            Some("function_call")
        );
        assert_eq!(
            input[2].get("type").and_then(Value::as_str),
            Some("function_call_output")
        );
    }

    #[test]
    fn preserves_raw_tool_call_arguments_string() {
        let messages = vec![Message {
            role: Role::Assistant,
            content: vec![ContentPart::ToolCall {
                id: "c1".to_string(),
                name: "add".to_string(),
                arguments: Value::String("{\"a\":1}".to_string()),
            }],
        }];

        let (instructions, input, warnings) = OpenAI::messages_to_input(&messages);
        assert!(warnings.is_empty());
        assert!(instructions.is_none());
        assert_eq!(input.len(), 1);
        assert_eq!(
            input[0].get("type").and_then(Value::as_str),
            Some("function_call")
        );
        assert_eq!(
            input[0].get("arguments").and_then(Value::as_str),
            Some("{\"a\":1}")
        );
    }

    #[test]
    fn converts_pdf_file_part_to_input_file() {
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

        let (instructions, input, warnings) = OpenAI::messages_to_input(&messages);
        assert!(warnings.is_empty());
        assert!(instructions.is_none());
        assert_eq!(input.len(), 1);
        assert_eq!(input[0].get("role").and_then(Value::as_str), Some("user"));
        let content = input[0]
            .get("content")
            .and_then(Value::as_array)
            .expect("content array");
        assert_eq!(content.len(), 1);
        assert_eq!(
            content[0].get("type").and_then(Value::as_str),
            Some("input_file")
        );
        assert_eq!(
            content[0].get("filename").and_then(Value::as_str),
            Some("doc.pdf")
        );
        assert_eq!(
            content[0].get("file_data").and_then(Value::as_str),
            Some("data:application/pdf;base64,AQIDBAU=")
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

        let mut warnings = Vec::<Warning>::new();
        let parsed = parse_openai_output(&output, &mut warnings);
        assert_eq!(parsed.len(), 1);
        assert!(warnings.is_empty());

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
    fn preserves_invalid_tool_call_arguments_with_warning() {
        let output = vec![serde_json::json!({
            "type": "function_call",
            "call_id": "c1",
            "name": "add",
            "arguments": "{\"a\":1"
        })];

        let mut warnings = Vec::<Warning>::new();
        let parsed = parse_openai_output(&output, &mut warnings);
        assert_eq!(parsed.len(), 1);
        assert!(warnings.iter().any(|w| matches!(
            w,
            Warning::Compatibility { feature, .. } if feature == "tool_call.arguments"
        )));

        match &parsed[0] {
            ContentPart::ToolCall { arguments, .. } => {
                assert_eq!(arguments, &Value::String("{\"a\":1".to_string()));
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
            parallel_tool_calls: Some(false),
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
        assert_eq!(body.get("parallel_tool_calls"), Some(&json!(false)));
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
