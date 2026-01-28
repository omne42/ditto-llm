use std::collections::{BTreeMap, HashMap, VecDeque};

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use futures_util::StreamExt;
use futures_util::stream;
use reqwest::Url;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::auth::sigv4::{SigV4Signer, SigV4Timestamp, resolve_sigv4_signer};
use crate::model::{LanguageModel, StreamResult};
use crate::profile::Env;
use crate::profile::ProviderConfig;
use crate::types::{
    ContentPart, FileSource, FinishReason, GenerateRequest, GenerateResponse, ImageSource, Message,
    Role, StreamChunk, Tool, ToolChoice, Usage, Warning,
};
use crate::{DittoError, Result};

const DEFAULT_VERSION: &str = "bedrock-2023-05-31";

#[derive(Clone)]
pub struct Bedrock {
    http: reqwest::Client,
    base_url: String,
    default_model: String,
    signer: SigV4Signer,
    http_headers: BTreeMap<String, String>,
    http_query_params: BTreeMap<String, String>,
}

impl Bedrock {
    pub fn new(
        signer: SigV4Signer,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
    ) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .map_err(DittoError::Http)?;
        Ok(Self {
            http,
            base_url: base_url.into(),
            default_model: default_model.into(),
            signer,
            http_headers: BTreeMap::new(),
            http_query_params: BTreeMap::new(),
        })
    }

    pub fn with_http_client(mut self, http: reqwest::Client) -> Self {
        self.http = http;
        self
    }

    pub fn with_http_headers(mut self, headers: BTreeMap<String, String>) -> Self {
        self.http_headers = headers;
        self
    }

    pub fn with_http_query_params(mut self, params: BTreeMap<String, String>) -> Self {
        self.http_query_params = params;
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
        let base_url = config.base_url.as_deref().ok_or_else(|| {
            DittoError::InvalidResponse("provider base_url is missing".to_string())
        })?;
        let model = config.default_model.as_deref().ok_or_else(|| {
            DittoError::InvalidResponse("provider default_model is missing".to_string())
        })?;
        let auth = config
            .auth
            .clone()
            .ok_or_else(|| DittoError::InvalidResponse("bedrock auth is missing".to_string()))?;
        let signer = resolve_sigv4_signer(&auth, env)?;

        let mut out = Self::new(signer, base_url, model)?;
        out.http_headers = config.http_headers.clone();
        out.http_query_params = config.http_query_params.clone();
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
            "bedrock model is not set".to_string(),
        ))
    }

    fn invoke_url(&self, model: &str) -> String {
        if self.base_url.contains("{model}") {
            return self.base_url.replace("{model}", model);
        }
        let base = self.base_url.trim_end_matches('/');
        format!("{base}/model/{model}/invoke")
    }

    fn invoke_stream_url(&self, model: &str) -> String {
        if self.base_url.contains("{model}") {
            let replaced = self.base_url.replace("{model}", model);
            if replaced.ends_with("/invoke-with-response-stream") {
                return replaced;
            }
            if replaced.ends_with("/invoke") {
                return replaced.replace("/invoke", "/invoke-with-response-stream");
            }
            return format!("{replaced}/invoke-with-response-stream");
        }
        let base = self.base_url.trim_end_matches('/');
        format!("{base}/model/{model}/invoke-with-response-stream")
    }

    fn build_url_with_query(&self, base: &str) -> Result<String> {
        let mut url = Url::parse(base).map_err(|err| {
            DittoError::InvalidResponse(format!("invalid bedrock base_url {base:?}: {err}"))
        })?;
        if !self.http_query_params.is_empty() {
            {
                let mut pairs = url.query_pairs_mut();
                for (key, value) in &self.http_query_params {
                    if key.trim().is_empty() {
                        continue;
                    }
                    pairs.append_pair(key, value);
                }
            }
        }
        Ok(url.to_string())
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

    fn tool_to_anthropic(tool: &Tool, warnings: &mut Vec<Warning>) -> Value {
        if tool.strict.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "tool.strict".to_string(),
                details: Some(
                    "Bedrock Anthropic tool strictness is not supported; ignored".to_string(),
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
                            media_type,
                            source,
                            filename,
                        } => {
                            if media_type != "application/pdf" {
                                warnings.push(Warning::Unsupported {
                                    feature: "file_part".to_string(),
                                    details: Some(format!(
                                        "bedrock only supports PDF files (got {media_type})"
                                    )),
                                });
                                continue;
                            }
                            if filename.is_some() {
                                warnings.push(Warning::Unsupported {
                                    feature: "file.filename".to_string(),
                                    details: Some(
                                        "bedrock does not accept filename metadata for documents"
                                            .to_string(),
                                    ),
                                });
                            }
                            match source {
                                FileSource::Url { url } => {
                                    let doc = serde_json::json!({
                                        "type": "url",
                                        "url": url,
                                    });
                                    blocks.push(serde_json::json!({ "type": "document", "source": doc }));
                                }
                                FileSource::Base64 { data } => {
                                    let doc = serde_json::json!({
                                        "type": "base64",
                                        "media_type": media_type,
                                        "data": data,
                                    });
                                    blocks.push(serde_json::json!({ "type": "document", "source": doc }));
                                }
                                FileSource::FileId { file_id } => warnings.push(Warning::Unsupported {
                                    feature: "file_id".to_string(),
                                    details: Some(format!(
                                        "bedrock does not support OpenAI file ids (file_id={file_id})"
                                    )),
                                }),
                            }
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
                            if !text.is_empty() {
                                blocks.push(serde_json::json!({ "type": "text", "text": text }));
                            }
                        }
                        ContentPart::Reasoning { text } => {
                            if !text.is_empty() {
                                blocks.push(serde_json::json!({
                                    "type": "thinking",
                                    "thinking": text,
                                }));
                            }
                        }
                        ContentPart::ToolCall {
                            id,
                            name,
                            arguments,
                            ..
                        } => {
                            blocks.push(serde_json::json!({
                                "type": "tool_use",
                                "id": id,
                                "name": name,
                                "input": arguments,
                            }));
                        }
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
                            let Some(_tool_name) = tool_names.get(tool_call_id) else {
                                warnings.push(Warning::Compatibility {
                                    feature: "tool_result".to_string(),
                                    details: format!(
                                        "tool_result references unknown tool_call_id={tool_call_id}; skipped"
                                    ),
                                });
                                continue;
                            };
                            let mut output = serde_json::json!({
                                "type": "tool_result",
                                "tool_use_id": tool_call_id,
                                "content": content,
                            });
                            if let Some(is_error) = is_error {
                                if let Some(obj) = output.as_object_mut() {
                                    obj.insert("is_error".to_string(), Value::Bool(*is_error));
                                }
                            }
                            blocks.push(output);
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
        let mut out = Vec::new();
        for message in messages {
            for part in &message.content {
                if let ContentPart::File { media_type, .. } = part {
                    if media_type == "application/pdf" {
                        out.push("pdfs-2024-09-25");
                    }
                }
            }
        }
        out
    }

    fn stop_reason_to_finish_reason(reason: Option<&str>) -> FinishReason {
        match reason {
            Some("stop") => FinishReason::Stop,
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

    fn parse_anthropic_content(blocks: &[Value]) -> Vec<ContentPart> {
        let mut out = Vec::<ContentPart>::new();
        for block in blocks {
            let Some(kind) = block.get("type").and_then(Value::as_str) else {
                continue;
            };
            match kind {
                "text" => {
                    let Some(text) = block.get("text").and_then(Value::as_str) else {
                        continue;
                    };
                    if !text.is_empty() {
                        out.push(ContentPart::Text {
                            text: text.to_string(),
                        });
                    }
                }
                "tool_use" => {
                    let Some(id) = block.get("id").and_then(Value::as_str) else {
                        continue;
                    };
                    let Some(name) = block.get("name").and_then(Value::as_str) else {
                        continue;
                    };
                    let arguments = block.get("input").cloned().unwrap_or(Value::Null);
                    out.push(ContentPart::ToolCall {
                        id: id.to_string(),
                        name: name.to_string(),
                        arguments,
                    });
                }
                "thinking" => {
                    let Some(thinking) = block.get("thinking").and_then(Value::as_str) else {
                        continue;
                    };
                    if !thinking.is_empty() {
                        out.push(ContentPart::Reasoning {
                            text: thinking.to_string(),
                        });
                    }
                }
                _ => {}
            }
        }
        out
    }

    fn build_bedrock_body(
        request: &GenerateRequest,
        model: &str,
        warnings: &mut Vec<Warning>,
    ) -> Result<Value> {
        let tool_names = Self::build_tool_name_map(&request.messages);
        let mut system = Vec::<String>::new();
        let mut saw_non_system = false;
        let mut messages = Vec::<Value>::new();

        for message in &request.messages {
            if message.role == Role::System && !saw_non_system {
                if let Some(text) = Self::extract_system_text(message, warnings) {
                    system.push(text);
                }
                continue;
            }
            saw_non_system = true;

            if let Some((role, content)) =
                Self::message_to_anthropic_blocks(message, &tool_names, warnings)
            {
                messages.push(serde_json::json!({ "role": role, "content": content }));
            }
        }

        let mut body = Map::<String, Value>::new();
        body.insert(
            "anthropic_version".to_string(),
            Value::String(DEFAULT_VERSION.to_string()),
        );
        body.insert("messages".to_string(), Value::Array(messages));
        body.insert(
            "max_tokens".to_string(),
            Value::Number(request.max_tokens.unwrap_or(1024).into()),
        );

        if !system.is_empty() {
            body.insert("system".to_string(), Value::String(system.join("\n\n")));
        }

        if let Some(temperature) = request.temperature {
            if let Some(value) = crate::utils::params::clamped_number_from_f32(
                "temperature",
                temperature,
                0.0,
                1.0,
                warnings,
            ) {
                body.insert("temperature".to_string(), Value::Number(value));
            }
        }
        if let Some(top_p) = request.top_p {
            if let Some(value) =
                crate::utils::params::clamped_number_from_f32("top_p", top_p, 0.0, 1.0, warnings)
            {
                body.insert("top_p".to_string(), Value::Number(value));
            }
        }
        if let Some(stop_sequences) = request.stop_sequences.clone() {
            let stop_sequences =
                crate::utils::params::sanitize_stop_sequences(&stop_sequences, Some(4), warnings);
            if !stop_sequences.is_empty() {
                body.insert(
                    "stop_sequences".to_string(),
                    Value::Array(stop_sequences.into_iter().map(Value::String).collect()),
                );
            }
        }

        if let Some(tools) = request.tools.clone() {
            if cfg!(feature = "tools") {
                let mapped = tools
                    .iter()
                    .map(|tool| Self::tool_to_anthropic(tool, warnings))
                    .collect::<Vec<_>>();
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
                if *tool_choice == ToolChoice::None {
                    body.remove("tools");
                } else if let Some(mapped) = Self::tool_choice_to_anthropic(tool_choice) {
                    body.insert("tool_choice".to_string(), mapped);
                }
            } else {
                warnings.push(Warning::Unsupported {
                    feature: "tool_choice".to_string(),
                    details: Some("ditto-llm built without tools feature".to_string()),
                });
            }
        }

        crate::types::merge_provider_options_into_body(
            &mut body,
            request.provider_options_value_for("bedrock")?.as_ref(),
            &["reasoning_effort", "response_format", "parallel_tool_calls"],
            "bedrock.provider_options",
            warnings,
        );

        let betas = Self::required_betas(&request.messages);
        if !betas.is_empty() {
            body.insert("anthropic_beta".to_string(), Value::String(betas.join(",")));
        }

        if model.to_lowercase().contains("sonnet") {
            body.entry("anthropic_beta".to_string())
                .or_insert(Value::String("context-1m-2025-08-07".to_string()));
        }

        Ok(Value::Object(body))
    }

    async fn post_json<T: serde::Serialize>(
        &self,
        url: &str,
        body: &T,
        accept: Option<&str>,
    ) -> Result<reqwest::Response> {
        let payload = serde_json::to_vec(body)?;
        let mut headers = self.http_headers.clone();
        headers
            .entry("content-type".to_string())
            .or_insert_with(|| "application/json".to_string());
        if let Some(accept) = accept {
            headers.insert("accept".to_string(), accept.to_string());
        }

        let timestamp = SigV4Timestamp::now()?;
        let signed = self
            .signer
            .sign("POST", url, &headers, &payload, timestamp)?;

        let mut req = self.http.post(url).body(payload);
        for (name, value) in &headers {
            req = req.header(name, value);
        }
        req = signed.headers.apply(req);

        let response = req.send().await?;
        Ok(response)
    }
}

#[derive(Debug, Deserialize)]
struct MessagesApiResponse {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    content: Vec<Value>,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    usage: Option<Value>,
}

#[async_trait]
impl LanguageModel for Bedrock {
    fn provider(&self) -> &str {
        "bedrock"
    }

    fn model_id(&self) -> &str {
        &self.default_model
    }

    async fn generate(&self, request: GenerateRequest) -> Result<GenerateResponse> {
        let model = self.resolve_model(&request)?;
        let selected_provider_options = request.provider_options_value_for(self.provider())?;
        let provider_options = selected_provider_options
            .as_ref()
            .map(crate::types::ProviderOptions::from_value)
            .transpose()?
            .unwrap_or_default();

        let mut warnings = Vec::<Warning>::new();
        if provider_options.reasoning_effort.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "reasoning_effort".to_string(),
                details: Some("Bedrock Anthropic does not support reasoning_effort".to_string()),
            });
        }
        if provider_options.response_format.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "response_format".to_string(),
                details: Some("Bedrock Anthropic does not support response_format".to_string()),
            });
        }
        if provider_options.parallel_tool_calls == Some(true) {
            warnings.push(Warning::Unsupported {
                feature: "parallel_tool_calls".to_string(),
                details: Some("Bedrock Anthropic does not support parallel_tool_calls".to_string()),
            });
        }

        let body = Self::build_bedrock_body(&request, model, &mut warnings)?;
        let url = self.build_url_with_query(&self.invoke_url(model))?;
        let response = self.post_json(&url, &body, None).await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(DittoError::Api { status, body: text });
        }

        let parsed = response.json::<MessagesApiResponse>().await?;
        let content = Self::parse_anthropic_content(&parsed.content);
        let finish_reason = Self::stop_reason_to_finish_reason(parsed.stop_reason.as_deref());
        let usage = parsed
            .usage
            .as_ref()
            .map(Self::parse_usage)
            .unwrap_or_default();

        Ok(GenerateResponse {
            content,
            finish_reason,
            usage,
            warnings,
            provider_metadata: parsed.id.map(|id| serde_json::json!({ "id": id })),
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

            let mut warnings = Vec::<Warning>::new();
            if provider_options.reasoning_effort.is_some() {
                warnings.push(Warning::Unsupported {
                    feature: "reasoning_effort".to_string(),
                    details: Some(
                        "Bedrock Anthropic does not support reasoning_effort".to_string(),
                    ),
                });
            }
            if provider_options.response_format.is_some() {
                warnings.push(Warning::Unsupported {
                    feature: "response_format".to_string(),
                    details: Some("Bedrock Anthropic does not support response_format".to_string()),
                });
            }
            if provider_options.parallel_tool_calls == Some(true) {
                warnings.push(Warning::Unsupported {
                    feature: "parallel_tool_calls".to_string(),
                    details: Some(
                        "Bedrock Anthropic does not support parallel_tool_calls".to_string(),
                    ),
                });
            }

            let body = Self::build_bedrock_body(&request, model, &mut warnings)?;
            let url = self.build_url_with_query(&self.invoke_stream_url(model))?;
            let response = self
                .post_json(&url, &body, Some("application/vnd.amazon.eventstream"))
                .await?;

            let status = response.status();
            if !status.is_success() {
                let text = response.text().await.unwrap_or_default();
                return Err(DittoError::Api { status, body: text });
            }

            let data_stream = Box::pin(bedrock_event_stream_from_response(response));
            let mut buffer = VecDeque::<Result<StreamChunk>>::new();
            if !warnings.is_empty() {
                buffer.push_back(Ok(StreamChunk::Warnings { warnings }));
            }

            #[derive(Debug, Deserialize)]
            struct StreamEvent {
                #[serde(rename = "type")]
                kind: String,
                #[serde(default)]
                index: Option<usize>,
                #[serde(default)]
                delta: Option<Value>,
                #[serde(default)]
                content_block: Option<Value>,
                #[serde(default)]
                message: Option<Value>,
                #[serde(default)]
                usage: Option<Value>,
            }

            let stream = stream::unfold(
                (
                    data_stream,
                    buffer,
                    false,
                    HashMap::<usize, (String, String)>::new(),
                    None::<Usage>,
                    None::<FinishReason>,
                ),
                |(
                    mut data_stream,
                    mut buffer,
                    mut done,
                    mut tool_calls,
                    mut pending_usage,
                    mut pending_finish,
                )| async move {
                    loop {
                        if let Some(item) = buffer.pop_front() {
                            return Some((
                                item,
                                (
                                    data_stream,
                                    buffer,
                                    done,
                                    tool_calls,
                                    pending_usage,
                                    pending_finish,
                                ),
                            ));
                        }

                        if done {
                            return None;
                        }

                        let next = data_stream.as_mut().next().await;
                        match next {
                            Some(Ok(data)) => match serde_json::from_str::<StreamEvent>(&data) {
                                Ok(event) => match event.kind.as_str() {
                                    "content_block_start" => {
                                        let Some(index) = event.index else { continue };
                                        let Some(block) = event.content_block else {
                                            continue;
                                        };
                                        let Some(block_type) =
                                            block.get("type").and_then(Value::as_str)
                                        else {
                                            continue;
                                        };
                                        if block_type == "tool_use" {
                                            let Some(id) = block.get("id").and_then(Value::as_str)
                                            else {
                                                continue;
                                            };
                                            let Some(name) =
                                                block.get("name").and_then(Value::as_str)
                                            else {
                                                continue;
                                            };
                                            tool_calls
                                                .insert(index, (id.to_string(), name.to_string()));
                                            buffer.push_back(Ok(StreamChunk::ToolCallStart {
                                                id: id.to_string(),
                                                name: name.to_string(),
                                            }));
                                            if let Some(input) = block.get("input") {
                                                buffer.push_back(Ok(StreamChunk::ToolCallDelta {
                                                    id: id.to_string(),
                                                    arguments_delta: input.to_string(),
                                                }));
                                            }
                                        }
                                    }
                                    "content_block_delta" => {
                                        let Some(index) = event.index else { continue };
                                        let Some(delta) = event.delta else { continue };
                                        let Some(delta_type) =
                                            delta.get("type").and_then(Value::as_str)
                                        else {
                                            continue;
                                        };
                                        match delta_type {
                                            "text_delta" => {
                                                if let Some(text) =
                                                    delta.get("text").and_then(Value::as_str)
                                                {
                                                    buffer.push_back(Ok(StreamChunk::TextDelta {
                                                        text: text.to_string(),
                                                    }));
                                                }
                                            }
                                            "thinking_delta" => {
                                                if let Some(thinking) =
                                                    delta.get("thinking").and_then(Value::as_str)
                                                {
                                                    buffer.push_back(Ok(
                                                        StreamChunk::ReasoningDelta {
                                                            text: thinking.to_string(),
                                                        },
                                                    ));
                                                }
                                            }
                                            "input_json_delta" => {
                                                let Some((tool_call_id, _name)) =
                                                    tool_calls.get(&index)
                                                else {
                                                    continue;
                                                };
                                                if let Some(partial) = delta
                                                    .get("partial_json")
                                                    .and_then(Value::as_str)
                                                {
                                                    buffer.push_back(Ok(
                                                        StreamChunk::ToolCallDelta {
                                                            id: tool_call_id.clone(),
                                                            arguments_delta: partial.to_string(),
                                                        },
                                                    ));
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                    "message_delta" => {
                                        if let Some(usage) = event.usage.as_ref() {
                                            pending_usage = Some(Self::parse_usage(usage));
                                        }
                                        if let Some(message) =
                                            event.message.as_ref().or(event.delta.as_ref())
                                        {
                                            if let Some(stop_reason) =
                                                message.get("stop_reason").and_then(Value::as_str)
                                            {
                                                pending_finish =
                                                    Some(Self::stop_reason_to_finish_reason(Some(
                                                        stop_reason,
                                                    )));
                                            }
                                        }
                                        if let Some(delta) = event.delta.as_ref() {
                                            if let Some(stop_reason) =
                                                delta.get("stop_reason").and_then(Value::as_str)
                                            {
                                                pending_finish =
                                                    Some(Self::stop_reason_to_finish_reason(Some(
                                                        stop_reason,
                                                    )));
                                            }
                                        }
                                    }
                                    "message_stop" => {
                                        done = true;
                                        if let Some(usage) = pending_usage.take() {
                                            buffer.push_back(Ok(StreamChunk::Usage(usage)));
                                        }
                                        buffer.push_back(Ok(StreamChunk::FinishReason(
                                            pending_finish.take().unwrap_or(FinishReason::Stop),
                                        )));
                                    }
                                    "error" => {
                                        done = true;
                                        buffer.push_back(Err(DittoError::InvalidResponse(data)));
                                    }
                                    _ => {}
                                },
                                Err(err) => {
                                    done = true;
                                    buffer.push_back(Err(err.into()));
                                }
                            },
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

#[derive(Debug)]
struct EventStreamMessage {
    headers: HashMap<String, String>,
    payload: Vec<u8>,
}

#[derive(Debug, Default)]
struct EventStreamDecoder {
    buffer: Vec<u8>,
}

impl EventStreamDecoder {
    fn push(&mut self, chunk: &[u8]) {
        self.buffer.extend_from_slice(chunk);
    }

    fn next_message(&mut self) -> Option<Result<EventStreamMessage>> {
        if self.buffer.len() < 12 {
            return None;
        }
        let total_len = u32::from_be_bytes(self.buffer[0..4].try_into().ok()?) as usize;
        if total_len < 16 {
            return Some(Err(DittoError::InvalidResponse(
                "eventstream total_len too small".to_string(),
            )));
        }
        if self.buffer.len() < total_len {
            return None;
        }
        let message = self.buffer.drain(0..total_len).collect::<Vec<u8>>();
        let headers_len = u32::from_be_bytes(message[4..8].try_into().ok()?) as usize;
        let headers_start = 12usize;
        let headers_end = headers_start.saturating_add(headers_len);
        if headers_end > message.len() {
            return Some(Err(DittoError::InvalidResponse(
                "eventstream invalid headers length".to_string(),
            )));
        }
        let payload_end = total_len.saturating_sub(4);
        if headers_end > payload_end {
            return Some(Err(DittoError::InvalidResponse(
                "eventstream invalid payload length".to_string(),
            )));
        }

        let headers = match parse_eventstream_headers(&message[headers_start..headers_end]) {
            Ok(headers) => headers,
            Err(err) => return Some(Err(err)),
        };
        let payload = message[headers_end..payload_end].to_vec();
        Some(Ok(EventStreamMessage { headers, payload }))
    }
}

fn parse_eventstream_headers(bytes: &[u8]) -> Result<HashMap<String, String>> {
    let mut out = HashMap::<String, String>::new();
    let mut idx = 0usize;
    while idx < bytes.len() {
        let name_len = *bytes.get(idx).ok_or_else(|| {
            DittoError::InvalidResponse("eventstream header missing name length".to_string())
        })? as usize;
        idx += 1;
        if idx + name_len > bytes.len() {
            return Err(DittoError::InvalidResponse(
                "eventstream header name truncated".to_string(),
            ));
        }
        let name = std::str::from_utf8(&bytes[idx..idx + name_len]).map_err(|err| {
            DittoError::InvalidResponse(format!("eventstream bad header name: {err}"))
        })?;
        idx += name_len;
        let value_type = *bytes.get(idx).ok_or_else(|| {
            DittoError::InvalidResponse("eventstream header missing type".to_string())
        })?;
        idx += 1;
        match value_type {
            0 | 1 => {}
            2 => idx += 1,
            3 => idx += 2,
            4 => idx += 4,
            5 => idx += 8,
            6 | 7 => {
                if idx + 2 > bytes.len() {
                    return Err(DittoError::InvalidResponse(
                        "eventstream header length truncated".to_string(),
                    ));
                }
                let len = u16::from_be_bytes([bytes[idx], bytes[idx + 1]]) as usize;
                idx += 2;
                if idx + len > bytes.len() {
                    return Err(DittoError::InvalidResponse(
                        "eventstream header value truncated".to_string(),
                    ));
                }
                if value_type == 7 {
                    let value = std::str::from_utf8(&bytes[idx..idx + len]).map_err(|err| {
                        DittoError::InvalidResponse(format!(
                            "eventstream header value utf8 error: {err}"
                        ))
                    })?;
                    out.insert(name.to_string(), value.to_string());
                }
                idx += len;
            }
            8 => idx += 8,
            9 => idx += 16,
            other => {
                return Err(DittoError::InvalidResponse(format!(
                    "eventstream unsupported header type {other}"
                )));
            }
        }
    }
    Ok(out)
}

fn bedrock_event_stream_from_response(
    response: reqwest::Response,
) -> impl futures_util::Stream<Item = Result<String>> {
    stream::unfold(
        (
            response.bytes_stream(),
            EventStreamDecoder::default(),
            VecDeque::<Result<String>>::new(),
        ),
        |(mut bytes_stream, mut decoder, mut buffer)| async move {
            loop {
                if let Some(item) = buffer.pop_front() {
                    return Some((item, (bytes_stream, decoder, buffer)));
                }
                let next = bytes_stream.next().await;
                match next {
                    Some(Ok(chunk)) => {
                        decoder.push(&chunk);
                        while let Some(message) = decoder.next_message() {
                            match message {
                                Ok(message) => match parse_bedrock_event(&message) {
                                    Ok(Some(data)) => buffer.push_back(Ok(data)),
                                    Ok(None) => {}
                                    Err(err) => buffer.push_back(Err(err)),
                                },
                                Err(err) => buffer.push_back(Err(err)),
                            }
                        }
                    }
                    Some(Err(err)) => {
                        buffer.push_back(Err(DittoError::Http(err)));
                    }
                    None => return None,
                }
            }
        },
    )
}

fn parse_bedrock_event(message: &EventStreamMessage) -> Result<Option<String>> {
    let message_type = message
        .headers
        .get(":message-type")
        .map(String::as_str)
        .unwrap_or("event");
    if message_type != "event" {
        return Err(DittoError::InvalidResponse(format!(
            "bedrock eventstream message-type={message_type}"
        )));
    }

    let outer: Value = serde_json::from_slice(&message.payload)?;
    let bytes = outer
        .get("bytes")
        .and_then(Value::as_str)
        .ok_or_else(|| DittoError::InvalidResponse("bedrock event missing bytes".to_string()))?;
    let decoded = BASE64.decode(bytes).map_err(|err| {
        DittoError::InvalidResponse(format!("bedrock base64 decode failed: {err}"))
    })?;
    let json = String::from_utf8(decoded).map_err(|err| {
        DittoError::InvalidResponse(format!("bedrock event bytes not utf8: {err}"))
    })?;
    Ok(Some(json))
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::{Method::POST, MockServer};
    use serde_json::json;

    fn build_event_stream_message(payload: &Value) -> Vec<u8> {
        let payload_bytes = serde_json::to_vec(payload).expect("payload json");

        let mut headers = Vec::<u8>::new();
        let name = ":message-type".as_bytes();
        headers.push(name.len() as u8);
        headers.extend_from_slice(name);
        headers.push(7u8); // string
        let value = "event".as_bytes();
        headers.extend_from_slice(&(value.len() as u16).to_be_bytes());
        headers.extend_from_slice(value);

        let headers_len = headers.len();
        let total_len = 12 + headers_len + payload_bytes.len() + 4;
        let mut out = Vec::with_capacity(total_len);
        out.extend_from_slice(&(total_len as u32).to_be_bytes());
        out.extend_from_slice(&(headers_len as u32).to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes()); // prelude crc (ignored)
        out.extend_from_slice(&headers);
        out.extend_from_slice(&payload_bytes);
        out.extend_from_slice(&0u32.to_be_bytes()); // message crc (ignored)
        out
    }

    fn bedrock_event(inner: Value) -> Value {
        let bytes = BASE64.encode(serde_json::to_vec(&inner).expect("inner json"));
        json!({
            "bytes": bytes
        })
    }

    #[tokio::test]
    async fn bedrock_generate_maps_anthropic_body() -> Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let signer = SigV4Signer::new("AKID", "SECRET", None, "us-east-1", "bedrock")?;
        let client = Bedrock::new(signer, server.url(""), "claude-test")?;

        let expected_body = json!({
            "anthropic_version": DEFAULT_VERSION,
            "messages": [{
                "role": "user",
                "content": [{"type": "text", "text": "hi"}]
            }],
            "max_tokens": 1024
        });

        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/model/claude-test/invoke")
                    .json_body_includes(expected_body.to_string());
                then.status(200)
                    .header("content-type", "application/json")
                    .body(
                        json!({
                            "content": [{ "type": "text", "text": "ok" }],
                            "stop_reason": "stop",
                            "usage": { "input_tokens": 1, "output_tokens": 2 }
                        })
                        .to_string(),
                    );
            })
            .await;

        let request = GenerateRequest::from(vec![Message::user("hi")]);
        let response = client.generate(request).await?;
        mock.assert_async().await;
        assert_eq!(response.text(), "ok");
        Ok(())
    }

    #[cfg(feature = "streaming")]
    #[tokio::test]
    async fn bedrock_stream_parses_eventstream() -> Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let signer = SigV4Signer::new("AKID", "SECRET", None, "us-east-1", "bedrock")?;
        let client = Bedrock::new(signer, server.url(""), "claude-test")?;

        let events = vec![
            bedrock_event(json!({
                "type": "content_block_start",
                "index": 0,
                "content_block": { "type": "text", "text": "" }
            })),
            bedrock_event(json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": { "type": "text_delta", "text": "Hello" }
            })),
            bedrock_event(json!({
                "type": "message_delta",
                "usage": { "input_tokens": 1, "output_tokens": 2 },
                "delta": { "stop_reason": "stop" }
            })),
            bedrock_event(json!({
                "type": "message_stop"
            })),
        ];
        let mut stream_body = Vec::<u8>::new();
        for event in events {
            stream_body.extend(build_event_stream_message(&event));
        }

        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/model/claude-test/invoke-with-response-stream");
                then.status(200)
                    .header("content-type", "application/vnd.amazon.eventstream")
                    .body(stream_body);
            })
            .await;

        let request = GenerateRequest::from(vec![Message::user("hi")]);
        let mut stream = client.stream(request).await?;
        let mut chunks = Vec::new();
        while let Some(item) = stream.next().await {
            chunks.push(item?);
        }

        mock.assert_async().await;
        assert!(
            chunks
                .iter()
                .any(|c| matches!(c, StreamChunk::TextDelta { text } if text == "Hello"))
        );
        assert!(
            chunks
                .iter()
                .any(|c| matches!(c, StreamChunk::FinishReason(FinishReason::Stop)))
        );
        Ok(())
    }
}
