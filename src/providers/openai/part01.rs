use std::collections::{BTreeMap, VecDeque};

use async_trait::async_trait;
use futures_util::StreamExt;
use futures_util::TryStreamExt;
use futures_util::stream;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::io::AsyncBufReadExt;
use tokio::sync::mpsc;
use tokio_util::io::StreamReader;

use super::openai_like;

#[cfg(feature = "embeddings")]
use crate::embedding::EmbeddingModel;
use crate::file::{FileContent, FileDeleteResponse, FileObject};
use crate::model::{LanguageModel, StreamResult};
use crate::profile::{Env, ProviderConfig, RequestAuth};
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
        let api_key = api_key.into();
        let http = openai_like::default_http_client();
        let auth = openai_like::auth_from_api_key(&api_key);

        Self {
            http,
            base_url: openai_like::DEFAULT_BASE_URL.to_string(),
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
        let auth_header = openai_like::resolve_auth_required(config, env, DEFAULT_KEYS).await?;

        let mut out = Self::new("");
        out.auth = Some(auth_header);
        out.http_query_params = config.http_query_params.clone();
        if !config.http_headers.is_empty() {
            out = out.with_http_client(crate::profile::build_http_client(
                openai_like::HTTP_TIMEOUT,
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
        openai_like::apply_auth(req, self.auth.as_ref(), &self.http_query_params)
    }

    fn responses_url(&self) -> String {
        openai_like::join_endpoint(&self.base_url, "responses")
    }

    fn responses_compact_url(&self) -> String {
        format!("{}/compact", self.responses_url())
    }

    fn files_url(&self) -> String {
        openai_like::join_endpoint(&self.base_url, "files")
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
        let url = self.files_url();
        openai_like::upload_file_with_purpose(
            &self.http,
            url,
            self.auth.as_ref(),
            &self.http_query_params,
            crate::file::FileUploadRequest {
                filename: filename.into(),
                bytes,
                purpose: purpose.into(),
                media_type: media_type.map(|s| s.to_string()),
            },
        )
        .await
    }

    pub async fn list_files(&self) -> Result<Vec<FileObject>> {
        let url = self.files_url();
        openai_like::list_files(
            &self.http,
            url,
            self.auth.as_ref(),
            &self.http_query_params,
        )
        .await
    }

    pub async fn retrieve_file(&self, file_id: &str) -> Result<FileObject> {
        let url = format!("{}/{}", self.files_url(), file_id.trim());
        openai_like::retrieve_file(
            &self.http,
            url,
            self.auth.as_ref(),
            &self.http_query_params,
        )
        .await
    }

    pub async fn delete_file(&self, file_id: &str) -> Result<FileDeleteResponse> {
        let url = format!("{}/{}", self.files_url(), file_id.trim());
        openai_like::delete_file(
            &self.http,
            url,
            self.auth.as_ref(),
            &self.http_query_params,
        )
        .await
    }

    pub async fn download_file_content(&self, file_id: &str) -> Result<FileContent> {
        let url = format!("{}/{}/content", self.files_url(), file_id.trim());
        openai_like::download_file_content(
            &self.http,
            url,
            self.auth.as_ref(),
            &self.http_query_params,
        )
        .await
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

    pub async fn compact_responses_history_raw(
        &self,
        request: &OpenAIResponsesCompactionRequest<'_>,
    ) -> Result<Vec<Value>> {
        #[derive(Debug, Deserialize)]
        struct CompactionResponse {
            output: Vec<Value>,
        }

        let url = self.responses_compact_url();
        let req = self.http.post(url);
        let response = self.apply_auth(req).json(request).send().await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(DittoError::Api { status, body: text });
        }

        let parsed = response.json::<CompactionResponse>().await?;
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
        let req = self.http.post(url);
        let mut req = self.apply_auth(req).json(&body);
        for (name, value) in request.extra_headers.iter() {
            req = req.header(name, value);
        }
        req = req.header("Accept", "text/event-stream");
        let response = req.send().await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(DittoError::Api { status, body: text });
        }

        let byte_stream = response.bytes_stream().map_err(std::io::Error::other);
        let reader = StreamReader::new(byte_stream);
        let lines = tokio::io::BufReader::new(reader).lines();

        let (tx_event, rx_event) = mpsc::channel::<Result<OpenAIResponsesRawEvent>>(512);
        let task = tokio::spawn(process_raw_responses_sse(lines, tx_event));
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
