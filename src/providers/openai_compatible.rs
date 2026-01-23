use std::collections::VecDeque;

use async_trait::async_trait;
use futures_util::StreamExt;
use futures_util::stream;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::model::{LanguageModel, StreamResult};
use crate::profile::{Env, ProviderConfig, resolve_auth_token_with_default_keys};
use crate::types::{
    ContentPart, FinishReason, GenerateRequest, GenerateResponse, ImageSource, Message, Role,
    StreamChunk, Tool, ToolChoice, Usage, Warning,
};
use crate::{DittoError, Result};

#[derive(Clone)]
pub struct OpenAICompatible {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    default_model: String,
}

impl OpenAICompatible {
    pub fn new(api_key: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .expect("reqwest client build should not fail");

        Self {
            http,
            base_url: "https://api.openai.com/v1".to_string(),
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
        const DEFAULT_KEYS: &[&str] = &["OPENAI_API_KEY", "CODE_PM_OPENAI_API_KEY"];

        let api_key = match config.auth.clone() {
            Some(auth) => resolve_auth_token_with_default_keys(&auth, env, DEFAULT_KEYS).await?,
            None => DEFAULT_KEYS
                .iter()
                .find_map(|key| env.get(key))
                .unwrap_or_default(),
        };

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

    fn chat_completions_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/chat/completions") {
            base.to_string()
        } else {
            format!("{base}/chat/completions")
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
                                        "arguments": arguments.to_string(),
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
    tool_calls: Vec<StreamToolCallState>,
}

#[cfg(feature = "streaming")]
fn parse_stream_data(state: &mut StreamState, data: &str) -> Result<(Vec<StreamChunk>, bool)> {
    let chunk = serde_json::from_str::<ChatCompletionsChunk>(data)?;
    let mut out = Vec::<StreamChunk>::new();
    let mut done = false;

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
        let (messages, mut warnings) = Self::messages_to_chat_messages(&request.messages);

        let mut body = Map::<String, Value>::new();
        body.insert("model".to_string(), Value::String(model.to_string()));
        body.insert("messages".to_string(), Value::Array(messages));

        if let Some(temperature) = request.temperature {
            body.insert(
                "temperature".to_string(),
                Value::Number(
                    serde_json::Number::from_f64(temperature as f64).unwrap_or_else(|| 0.into()),
                ),
            );
        }
        if let Some(max_tokens) = request.max_tokens {
            body.insert("max_tokens".to_string(), Value::Number(max_tokens.into()));
        }
        if let Some(top_p) = request.top_p {
            body.insert(
                "top_p".to_string(),
                Value::Number(
                    serde_json::Number::from_f64(top_p as f64).unwrap_or_else(|| 0.into()),
                ),
            );
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

        if request.provider_options.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "provider_options".to_string(),
                details: Some("provider_options is not supported yet".to_string()),
            });
        }

        let url = self.chat_completions_url();
        let mut req = self.http.post(url);
        if !self.api_key.trim().is_empty() {
            req = req.bearer_auth(&self.api_key);
        }
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
                let arguments = serde_json::from_str::<Value>(arguments_raw)
                    .unwrap_or_else(|_| Value::String(arguments_raw.to_string()));
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
            let (messages, _warnings) = Self::messages_to_chat_messages(&request.messages);

            let mut body = Map::<String, Value>::new();
            body.insert("model".to_string(), Value::String(model.to_string()));
            body.insert("messages".to_string(), Value::Array(messages));
            body.insert("stream".to_string(), Value::Bool(true));

            if let Some(temperature) = request.temperature {
                body.insert(
                    "temperature".to_string(),
                    Value::Number(
                        serde_json::Number::from_f64(temperature as f64)
                            .unwrap_or_else(|| 0.into()),
                    ),
                );
            }
            if let Some(max_tokens) = request.max_tokens {
                body.insert("max_tokens".to_string(), Value::Number(max_tokens.into()));
            }
            if let Some(top_p) = request.top_p {
                body.insert(
                    "top_p".to_string(),
                    Value::Number(
                        serde_json::Number::from_f64(top_p as f64).unwrap_or_else(|| 0.into()),
                    ),
                );
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
                    let mut warnings = Vec::<Warning>::new();
                    let mapped = tools
                        .iter()
                        .map(|t| Self::tool_to_openai(t, &mut warnings))
                        .collect();
                    body.insert("tools".to_string(), Value::Array(mapped));
                }
            }
            if let Some(tool_choice) = request.tool_choice {
                if cfg!(feature = "tools") {
                    let mut warnings = Vec::<Warning>::new();
                    body.insert(
                        "tool_choice".to_string(),
                        Self::tool_choice_to_openai(&tool_choice, &mut warnings),
                    );
                }
            }

            let url = self.chat_completions_url();
            let mut req = self
                .http
                .post(url)
                .header("Accept", "text/event-stream")
                .json(&body);
            if !self.api_key.trim().is_empty() {
                req = req.bearer_auth(&self.api_key);
            }
            let response = req.send().await?;

            let status = response.status();
            if !status.is_success() {
                let text = response.text().await.unwrap_or_default();
                return Err(DittoError::Api { status, body: text });
            }

            let data_stream = crate::utils::sse::sse_data_stream_from_response(response);
            let stream = stream::unfold(
                (
                    data_stream,
                    VecDeque::<Result<StreamChunk>>::new(),
                    StreamState::default(),
                    false,
                ),
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
                            None => return None,
                        }
                    }
                },
            );

            Ok(Box::pin(stream))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
