use std::collections::{HashMap, VecDeque};

use async_trait::async_trait;
use futures_util::StreamExt;
use futures_util::stream;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::model::{LanguageModel, StreamResult};
use crate::types::{
    ContentPart, FinishReason, GenerateRequest, GenerateResponse, ImageSource, Message, Role,
    StreamChunk, Tool, ToolChoice, Usage, Warning,
};
use crate::{DittoError, Result};

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com/v1";
const DEFAULT_VERSION: &str = "2023-06-01";

#[derive(Clone)]
pub struct Anthropic {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    default_model: String,
    version: String,
}

impl Anthropic {
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
            version: DEFAULT_VERSION.to_string(),
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

    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
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

    fn clamp_temperature(temperature: f32, warnings: &mut Vec<Warning>) -> f32 {
        if temperature > 1.0 {
            warnings.push(Warning::Clamped {
                parameter: "temperature".to_string(),
                original: temperature,
                clamped_to: 1.0,
            });
            return 1.0;
        }
        if temperature < 0.0 {
            warnings.push(Warning::Clamped {
                parameter: "temperature".to_string(),
                original: temperature,
                clamped_to: 0.0,
            });
            return 0.0;
        }
        temperature
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

#[async_trait]
impl LanguageModel for Anthropic {
    fn provider(&self) -> &str {
        "anthropic"
    }

    fn model_id(&self) -> &str {
        self.default_model.as_str()
    }

    async fn generate(&self, request: GenerateRequest) -> Result<GenerateResponse> {
        let model = self.resolve_model(&request)?;

        let mut warnings = Vec::<Warning>::new();
        let tool_names = Self::build_tool_name_map(&request.messages);

        let mut system = Vec::<String>::new();
        let mut saw_non_system = false;
        let mut messages = Vec::<Value>::new();

        for message in &request.messages {
            if message.role == Role::System && !saw_non_system {
                if let Some(text) = Self::extract_system_text(message, &mut warnings) {
                    system.push(text);
                }
                continue;
            }
            saw_non_system = true;

            if let Some((role, content)) =
                Self::message_to_anthropic_blocks(message, &tool_names, &mut warnings)
            {
                messages.push(serde_json::json!({ "role": role, "content": content }));
            }
        }

        let mut body = Map::<String, Value>::new();
        body.insert("model".to_string(), Value::String(model.to_string()));
        body.insert("messages".to_string(), Value::Array(messages));
        body.insert(
            "max_tokens".to_string(),
            Value::Number(request.max_tokens.unwrap_or(1024).into()),
        );

        if !system.is_empty() {
            body.insert("system".to_string(), Value::String(system.join("\n\n")));
        }

        if let Some(temperature) = request.temperature {
            body.insert(
                "temperature".to_string(),
                Value::Number(
                    serde_json::Number::from_f64(
                        Self::clamp_temperature(temperature, &mut warnings) as f64,
                    )
                    .unwrap_or_else(|| 0.into()),
                ),
            );
        }
        if let Some(top_p) = request.top_p {
            body.insert(
                "top_p".to_string(),
                Value::Number(
                    serde_json::Number::from_f64(top_p as f64).unwrap_or_else(|| 0.into()),
                ),
            );
        }
        if let Some(stop_sequences) = request.stop_sequences {
            body.insert(
                "stop_sequences".to_string(),
                Value::Array(stop_sequences.into_iter().map(Value::String).collect()),
            );
        }

        if let Some(tools) = request.tools {
            if cfg!(feature = "tools") {
                let mapped = tools
                    .into_iter()
                    .map(|tool| Self::tool_to_anthropic(&tool, &mut warnings))
                    .collect::<Vec<_>>();
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
                if tool_choice == ToolChoice::None {
                    body.remove("tools");
                } else if let Some(mapped) = Self::tool_choice_to_anthropic(&tool_choice) {
                    body.insert("tool_choice".to_string(), mapped);
                }
            } else {
                warnings.push(Warning::Unsupported {
                    feature: "tool_choice".to_string(),
                    details: Some("ditto-llm built without tools feature".to_string()),
                });
            }
        }

        let url = self.messages_url();
        let response = self
            .http
            .post(url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", &self.version)
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(DittoError::Api { status, body: text });
        }

        let parsed = response.json::<MessagesApiResponse>().await?;
        let content = parse_anthropic_content(&parsed.content);
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

            let mut warnings = Vec::<Warning>::new();
            let tool_names = Self::build_tool_name_map(&request.messages);

            let mut system = Vec::<String>::new();
            let mut saw_non_system = false;
            let mut messages = Vec::<Value>::new();

            for message in &request.messages {
                if message.role == Role::System && !saw_non_system {
                    if let Some(text) = Self::extract_system_text(message, &mut warnings) {
                        system.push(text);
                    }
                    continue;
                }
                saw_non_system = true;

                if let Some((role, content)) =
                    Self::message_to_anthropic_blocks(message, &tool_names, &mut warnings)
                {
                    messages.push(serde_json::json!({ "role": role, "content": content }));
                }
            }

            let mut body = Map::<String, Value>::new();
            body.insert("model".to_string(), Value::String(model.to_string()));
            body.insert("messages".to_string(), Value::Array(messages));
            body.insert(
                "max_tokens".to_string(),
                Value::Number(request.max_tokens.unwrap_or(1024).into()),
            );
            body.insert("stream".to_string(), Value::Bool(true));

            if !system.is_empty() {
                body.insert("system".to_string(), Value::String(system.join("\n\n")));
            }

            if let Some(temperature) = request.temperature {
                body.insert(
                    "temperature".to_string(),
                    Value::Number(
                        serde_json::Number::from_f64(Self::clamp_temperature(
                            temperature,
                            &mut warnings,
                        ) as f64)
                        .unwrap_or_else(|| 0.into()),
                    ),
                );
            }
            if let Some(top_p) = request.top_p {
                body.insert(
                    "top_p".to_string(),
                    Value::Number(
                        serde_json::Number::from_f64(top_p as f64).unwrap_or_else(|| 0.into()),
                    ),
                );
            }
            if let Some(stop_sequences) = request.stop_sequences {
                body.insert(
                    "stop_sequences".to_string(),
                    Value::Array(stop_sequences.into_iter().map(Value::String).collect()),
                );
            }

            if let Some(tools) = request.tools {
                if cfg!(feature = "tools") {
                    let mapped = tools
                        .into_iter()
                        .map(|tool| Self::tool_to_anthropic(&tool, &mut warnings))
                        .collect::<Vec<_>>();
                    body.insert("tools".to_string(), Value::Array(mapped));
                }
            }

            if let Some(tool_choice) = request.tool_choice {
                if cfg!(feature = "tools") {
                    if tool_choice == ToolChoice::None {
                        body.remove("tools");
                    } else if let Some(mapped) = Self::tool_choice_to_anthropic(&tool_choice) {
                        body.insert("tool_choice".to_string(), mapped);
                    }
                }
            }

            let url = self.messages_url();
            let response = self
                .http
                .post(url)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", &self.version)
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
                    VecDeque::<Result<StreamChunk>>::new(),
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

                        let next = data_stream.next().await;
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

            let _ = warnings;
            Ok(Box::pin(stream))
        }
    }
}
