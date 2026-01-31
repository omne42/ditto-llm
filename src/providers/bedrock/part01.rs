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
        _model: &str,
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

