use async_trait::async_trait;
use futures_util::StreamExt;
use futures_util::TryStreamExt;
use futures_util::stream;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::sync::mpsc;
use tokio_util::io::StreamReader;

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
pub struct OpenAI {
    client: openai_like::OpenAiLikeClient,
}

impl OpenAI {
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

    pub async fn from_config(config: &ProviderConfig, env: &Env) -> Result<Self> {
        const DEFAULT_KEYS: &[&str] = &["OPENAI_API_KEY"];
        Ok(Self {
            client: openai_like::OpenAiLikeClient::from_config_required(config, env, DEFAULT_KEYS)
                .await?,
        })
    }

    fn apply_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        self.client.apply_auth(req)
    }

    fn responses_url(&self) -> String {
        self.client.endpoint("responses")
    }

    fn responses_compact_url(&self) -> String {
        format!("{}/compact", self.responses_url())
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
            usage.cache_input_tokens = obj
                .get("input_tokens_details")
                .and_then(|details| details.get("cached_tokens"))
                .and_then(Value::as_u64);
            usage.cache_creation_input_tokens = obj
                .get("cache_creation_input_tokens")
                .and_then(Value::as_u64);
            usage.output_tokens = obj.get("output_tokens").and_then(Value::as_u64);
            usage.total_tokens = obj.get("total_tokens").and_then(Value::as_u64);
        }
        usage.merge_total();
        usage
    }

    fn build_responses_body(
        request: &GenerateRequest,
        model: &str,
        provider_options: &crate::types::ProviderOptions,
        selected_provider_options: Option<&Value>,
        stream: bool,
        provider_options_context: &'static str,
    ) -> Result<(Map<String, Value>, Vec<Warning>)> {
        let (instructions, input, mut warnings) = Self::messages_to_input(&request.messages);

        if request.stop_sequences.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "stop_sequences".to_string(),
                details: Some(
                    "OpenAI Responses API stop sequences are not supported".to_string(),
                ),
            });
        }
        crate::types::warn_unsupported_generate_request_options(
            "OpenAI Responses API",
            request,
            crate::types::GenerateRequestSupport::NONE,
            &mut warnings,
        );

        let mut body = Map::<String, Value>::new();
        body.insert("model".to_string(), Value::String(model.to_string()));
        if let Some(instructions) = instructions {
            body.insert("instructions".to_string(), Value::String(instructions));
        }
        body.insert("input".to_string(), Value::Array(input));
        body.insert("stream".to_string(), Value::Bool(stream));
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

        if let Some(tools) = request.tools.as_ref() {
            if cfg!(feature = "tools") {
                let mapped = tools.iter().map(Self::tool_to_openai).collect();
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

        apply_provider_options(&mut body, provider_options)?;
        crate::types::merge_provider_options_into_body(
            &mut body,
            selected_provider_options,
            &["reasoning_effort", "response_format", "parallel_tool_calls"],
            provider_options_context,
            &mut warnings,
        );

        Ok((body, warnings))
    }

    pub async fn compact_responses_history_raw(
        &self,
        request: &OpenAIResponsesCompactionRequest<'_>,
    ) -> Result<Vec<Value>> {
        #[derive(Debug, Deserialize)]
        struct CompactionResponse {
            output: Vec<Value>,
        }

        let url = self.responses_compact_url();
        let req = self.client.http.post(url);
        let parsed = crate::utils::http::send_checked_json::<CompactionResponse>(
            self.apply_auth(req).json(request),
        )
        .await?;
        Ok(parsed.output)
    }

    pub async fn create_response_stream_raw(
        &self,
        request: &OpenAIResponsesRawRequest<'_>,
    ) -> Result<OpenAIResponsesRawEventStream> {
        if !request.stream {
            return Err(DittoError::InvalidResponse(
                "stream=true is required for create_response_stream_raw".to_string(),
            ));
        }

        let mut body = Map::<String, Value>::new();
        body.insert(
            "model".to_string(),
            Value::String(request.model.to_string()),
        );
        body.insert(
            "instructions".to_string(),
            Value::String(request.instructions.to_string()),
        );
        body.insert("input".to_string(), Value::Array(request.input.to_vec()));
        body.insert("store".to_string(), Value::Bool(request.store));
        body.insert("stream".to_string(), Value::Bool(true));
        body.insert(
            "parallel_tool_calls".to_string(),
            Value::Bool(request.parallel_tool_calls),
        );

        if let Some(tools) = request.tools.as_ref() {
            let mapped = tools.iter().map(Self::tool_to_openai).collect();
            body.insert("tools".to_string(), Value::Array(mapped));
        }
        if let Some(tool_choice) = request.tool_choice.as_ref() {
            body.insert(
                "tool_choice".to_string(),
                Self::tool_choice_to_openai(tool_choice),
            );
        }
        if request.reasoning_effort.is_some() || request.reasoning_summary.is_some() {
            let mut reasoning = Map::<String, Value>::new();
            if let Some(effort) = request.reasoning_effort {
                reasoning.insert("effort".to_string(), serde_json::json!(effort));
            }
            if let Some(summary) = request.reasoning_summary {
                reasoning.insert("summary".to_string(), serde_json::json!(summary));
            }
            body.insert("reasoning".to_string(), Value::Object(reasoning));
        }
        if let Some(response_format) = request.response_format.as_ref() {
            body.insert(
                "response_format".to_string(),
                serde_json::to_value(response_format)?,
            );
        }
        if !request.include.is_empty() {
            body.insert(
                "include".to_string(),
                Value::Array(request.include.iter().cloned().map(Value::String).collect()),
            );
        }
        if let Some(key) = request.prompt_cache_key.as_deref() {
            body.insert(
                "prompt_cache_key".to_string(),
                Value::String(key.to_string()),
            );
        }

        let url = self.responses_url();
        let req = self.client.http.post(url);
        let mut req = self.apply_auth(req).json(&body);
        for (name, value) in request.extra_headers.iter() {
            req = req.header(name, value);
        }
        req = req.header("Accept", "text/event-stream");
        let response = crate::utils::http::send_checked(req).await?;

        let byte_stream = response.bytes_stream().map_err(std::io::Error::other);
        let reader = StreamReader::new(byte_stream);
        let reader = tokio::io::BufReader::new(reader);

        let (tx_event, rx_event) = mpsc::channel::<Result<OpenAIResponsesRawEvent>>(512);
        let task = tokio::spawn(process_raw_responses_sse(reader, tx_event));
        Ok(OpenAIResponsesRawEventStream { rx_event, task })
    }
}

fn apply_provider_options(
    body: &mut Map<String, Value>,
    provider_options: &crate::types::ProviderOptions,
) -> Result<()> {
    if provider_options.reasoning_effort.is_some() {
        let mut reasoning = Map::<String, Value>::new();
        if let Some(effort) = provider_options.reasoning_effort {
            reasoning.insert("effort".to_string(), serde_json::json!(effort));
        }
        body.insert("reasoning".to_string(), Value::Object(reasoning));
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
