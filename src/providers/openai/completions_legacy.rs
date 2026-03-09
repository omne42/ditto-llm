use std::collections::VecDeque;

use async_trait::async_trait;
#[cfg(feature = "streaming")]
use futures_util::{StreamExt, stream};
use serde::Deserialize;
use serde_json::{Map, Value};

use super::OpenAI;
use crate::model::{LanguageModel, StreamResult};
#[cfg(feature = "streaming")]
use crate::types::StreamChunk;
#[cfg(all(test, feature = "streaming"))]
use crate::types::Usage;
use crate::types::{
    ContentPart, FinishReason, GenerateRequest, GenerateResponse, Message, Role, Warning,
};
use crate::{DittoError, Result};

const OPENAI_LEGACY_COMPLETIONS_RESERVED_PROVIDER_OPTION_KEYS: &[&str] = &[
    "model",
    "prompt",
    "stream",
    "temperature",
    "max_tokens",
    "top_p",
    "seed",
    "presence_penalty",
    "frequency_penalty",
    "logprobs",
    "user",
    "stop",
    "reasoning_effort",
    "response_format",
    "parallel_tool_calls",
    "prompt_cache_key",
];

const OPENAI_LEGACY_COMPLETIONS_UNSUPPORTED_PROVIDER_OPTION_KEYS: &[&str] = &[
    "reasoning_effort",
    "response_format",
    "parallel_tool_calls",
    "prompt_cache_key",
];

#[derive(Clone)]
pub struct OpenAICompletionsLegacy {
    client: OpenAI,
}

#[derive(Debug, Deserialize)]
struct LegacyCompletionResponse {
    id: String,
    #[serde(default)]
    choices: Vec<LegacyCompletionChoice>,
    #[serde(default)]
    usage: Option<Value>,
}

#[derive(Debug, Deserialize, Default)]
struct LegacyCompletionChoice {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[cfg(feature = "streaming")]
#[derive(Debug, Deserialize, Default)]
struct LegacyCompletionChunk {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    choices: Vec<LegacyCompletionChunkChoice>,
    #[serde(default)]
    usage: Option<Value>,
}

#[cfg(feature = "streaming")]
#[derive(Debug, Deserialize, Default)]
struct LegacyCompletionChunkChoice {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[cfg(feature = "streaming")]
#[derive(Debug, Default)]
struct LegacyCompletionStreamState {
    response_id_sent: bool,
    finish_reason_sent: bool,
}

impl OpenAICompletionsLegacy {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: OpenAI::new(api_key),
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

    pub fn with_max_binary_response_bytes(mut self, max_bytes: usize) -> Self {
        self.client = self.client.with_max_binary_response_bytes(max_bytes);
        self
    }

    pub async fn from_config(
        config: &crate::config::ProviderConfig,
        env: &crate::config::Env,
    ) -> Result<Self> {
        Ok(Self {
            client: OpenAI::from_config(config, env).await?,
        })
    }

    fn completions_url(&self) -> String {
        self.client.client.endpoint("completions")
    }

    fn parse_finish_reason(reason: Option<&str>) -> FinishReason {
        match reason {
            Some("stop") => FinishReason::Stop,
            Some("length") => FinishReason::Length,
            Some("content_filter") => FinishReason::ContentFilter,
            Some("error") => FinishReason::Error,
            _ => FinishReason::Unknown,
        }
    }

    fn parse_usage(value: &Value) -> crate::types::Usage {
        let mut usage = crate::types::Usage::default();
        if let Some(obj) = value.as_object() {
            usage.input_tokens = obj
                .get("input_tokens")
                .and_then(Value::as_u64)
                .or_else(|| obj.get("prompt_tokens").and_then(Value::as_u64));
            usage.cache_input_tokens = obj
                .get("input_tokens_details")
                .and_then(|details| details.get("cached_tokens"))
                .and_then(Value::as_u64)
                .or_else(|| {
                    obj.get("prompt_tokens_details")
                        .and_then(|details| details.get("cached_tokens"))
                        .and_then(Value::as_u64)
                })
                .or_else(|| obj.get("cached_tokens").and_then(Value::as_u64));
            usage.cache_creation_input_tokens = obj
                .get("cache_creation_input_tokens")
                .and_then(Value::as_u64);
            usage.output_tokens = obj
                .get("output_tokens")
                .and_then(Value::as_u64)
                .or_else(|| obj.get("completion_tokens").and_then(Value::as_u64));
            usage.total_tokens = obj.get("total_tokens").and_then(Value::as_u64);
        }
        usage.merge_total();
        usage
    }

    fn warn_unsupported_provider_options(options: Option<&Value>, warnings: &mut Vec<Warning>) {
        let Some(obj) = options.and_then(Value::as_object) else {
            return;
        };
        for key in OPENAI_LEGACY_COMPLETIONS_UNSUPPORTED_PROVIDER_OPTION_KEYS {
            if obj.contains_key(*key) {
                warnings.push(Warning::Unsupported {
                    feature: format!("legacy_completions.provider_options.{key}"),
                    details: Some(format!(
                        "OpenAI legacy /v1/completions does not support provider option {key}"
                    )),
                });
            }
        }
    }

    fn messages_to_prompt(messages: &[Message], warnings: &mut Vec<Warning>) -> String {
        let transcript_mode =
            messages.len() != 1 || !matches!(messages.first().map(|m| m.role), Some(Role::User));
        let mut sections = Vec::<String>::new();

        for message in messages {
            let mut text = String::new();
            for part in &message.content {
                match part {
                    ContentPart::Text { text: chunk } | ContentPart::Reasoning { text: chunk } => {
                        if !chunk.is_empty() {
                            text.push_str(chunk);
                        }
                    }
                    ContentPart::ToolResult { content, .. } => {
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        text.push_str(content);
                    }
                    other => warnings.push(Warning::Unsupported {
                        feature: "legacy_completions.content_part".to_string(),
                        details: Some(format!(
                            "ignoring unsupported content part when flattening prompt: {other:?}"
                        )),
                    }),
                }
            }

            let text = text.trim();
            if text.is_empty() {
                continue;
            }

            if transcript_mode {
                let role = match message.role {
                    Role::System => "System",
                    Role::User => "User",
                    Role::Assistant => "Assistant",
                    Role::Tool => "Tool",
                };
                sections.push(format!("{role}: {text}"));
            } else {
                sections.push(text.to_string());
            }
        }

        if !transcript_mode {
            return sections.join("\n\n");
        }

        let mut prompt = sections.join("\n\n");
        if !matches!(messages.last().map(|m| m.role), Some(Role::Assistant)) {
            if !prompt.is_empty() {
                prompt.push_str("\n\n");
            }
            prompt.push_str("Assistant:");
            warnings.push(Warning::Compatibility {
                feature: "legacy_completions.messages".to_string(),
                details:
                    "flattened chat-style messages into a legacy completions prompt transcript"
                        .to_string(),
            });
        }
        prompt
    }

    fn build_body(
        &self,
        request: &GenerateRequest,
        model: &str,
        stream: bool,
        feature: &str,
    ) -> Result<(Value, Vec<Warning>)> {
        let mut warnings = Vec::<Warning>::new();
        let mut body = Map::<String, Value>::new();
        body.insert("model".to_string(), Value::String(model.to_string()));
        body.insert("stream".to_string(), Value::Bool(stream));

        let prompt = Self::messages_to_prompt(&request.messages, &mut warnings);
        body.insert("prompt".to_string(), Value::String(prompt));

        if let Some(temperature) = request.temperature.filter(|value| value.is_finite()) {
            body.insert("temperature".to_string(), Value::from(temperature as f64));
        }
        if let Some(max_tokens) = request.max_tokens {
            body.insert("max_tokens".to_string(), Value::from(max_tokens));
        }
        if let Some(top_p) = request.top_p.filter(|value| value.is_finite()) {
            body.insert("top_p".to_string(), Value::from(top_p as f64));
        }
        if request.seed.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "legacy_completions.seed".to_string(),
                details: Some("OpenAI legacy /v1/completions does not support seed".to_string()),
            });
        }
        if let Some(presence_penalty) = request.presence_penalty.filter(|value| value.is_finite()) {
            body.insert(
                "presence_penalty".to_string(),
                Value::from(presence_penalty as f64),
            );
        }
        if let Some(frequency_penalty) = request.frequency_penalty.filter(|value| value.is_finite())
        {
            body.insert(
                "frequency_penalty".to_string(),
                Value::from(frequency_penalty as f64),
            );
        }
        if request.logprobs == Some(true) || request.top_logprobs.is_some() {
            let requested = request.top_logprobs.unwrap_or(1).max(1);
            body.insert("logprobs".to_string(), Value::from(requested.min(5)));
        }
        if let Some(user) = request
            .user
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            body.insert("user".to_string(), Value::String(user.to_string()));
        }
        if let Some(stop_sequences) = request
            .stop_sequences
            .as_ref()
            .filter(|values| !values.is_empty())
        {
            body.insert(
                "stop".to_string(),
                serde_json::to_value(stop_sequences).map_err(|err| {
                    DittoError::InvalidResponse(format!(
                        "failed to serialize stop sequences: {err}"
                    ))
                })?,
            );
        }
        if request
            .tools
            .as_ref()
            .is_some_and(|tools| !tools.is_empty())
        {
            warnings.push(Warning::Unsupported {
                feature: "legacy_completions.tools".to_string(),
                details: Some("OpenAI legacy /v1/completions does not support tools".to_string()),
            });
        }
        if request.tool_choice.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "legacy_completions.tool_choice".to_string(),
                details: Some(
                    "OpenAI legacy /v1/completions does not support tool_choice".to_string(),
                ),
            });
        }

        let selected_provider_options = request.provider_options_value_for("openai")?;
        Self::warn_unsupported_provider_options(selected_provider_options.as_ref(), &mut warnings);
        crate::types::merge_provider_options_into_body(
            &mut body,
            selected_provider_options.as_ref(),
            OPENAI_LEGACY_COMPLETIONS_RESERVED_PROVIDER_OPTION_KEYS,
            feature,
            &mut warnings,
        );

        Ok((Value::Object(body), warnings))
    }
}

#[cfg(feature = "streaming")]
fn parse_stream_data(
    state: &mut LegacyCompletionStreamState,
    data: &str,
) -> Result<Vec<StreamChunk>> {
    let chunk = serde_json::from_str::<LegacyCompletionChunk>(data)?;
    let mut out = Vec::<StreamChunk>::new();

    if !state.response_id_sent {
        if let Some(id) = chunk
            .id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            state.response_id_sent = true;
            out.push(StreamChunk::ResponseId { id: id.to_string() });
        }
    }

    if let Some(usage) = chunk.usage.as_ref() {
        out.push(StreamChunk::Usage(OpenAICompletionsLegacy::parse_usage(
            usage,
        )));
    }

    if let Some(choice) = chunk.choices.first() {
        if let Some(text) = choice.text.as_deref().filter(|value| !value.is_empty()) {
            out.push(StreamChunk::TextDelta {
                text: text.to_string(),
            });
        }
        if !state.finish_reason_sent {
            if let Some(reason) = choice.finish_reason.as_deref() {
                state.finish_reason_sent = true;
                out.push(StreamChunk::FinishReason(
                    OpenAICompletionsLegacy::parse_finish_reason(Some(reason)),
                ));
            }
        }
    }

    Ok(out)
}

#[async_trait]
impl LanguageModel for OpenAICompletionsLegacy {
    fn provider(&self) -> &str {
        "openai"
    }

    fn model_id(&self) -> &str {
        self.client.model_id()
    }

    async fn generate(&self, request: GenerateRequest) -> Result<GenerateResponse> {
        let model = self.client.resolve_model(&request)?;
        let (body, warnings) =
            self.build_body(&request, model, false, "generate.provider_options")?;

        let url = self.completions_url();
        let req = self.client.client.http.post(url);
        let parsed = crate::utils::http::send_checked_json::<LegacyCompletionResponse>(
            self.client.apply_auth(req).json(&body),
        )
        .await?;

        let choice = parsed.choices.into_iter().next().ok_or_else(|| {
            DittoError::InvalidResponse("legacy completions response has no choices".to_string())
        })?;

        let content = choice
            .text
            .filter(|text| !text.is_empty())
            .map(|text| vec![ContentPart::Text { text }])
            .unwrap_or_default();
        let usage = parsed
            .usage
            .as_ref()
            .map(Self::parse_usage)
            .unwrap_or_default();

        Ok(GenerateResponse {
            content,
            finish_reason: Self::parse_finish_reason(choice.finish_reason.as_deref()),
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
            let model = self.client.resolve_model(&request)?;
            let (body, warnings) =
                self.build_body(&request, model, true, "stream.provider_options")?;

            let url = self.completions_url();
            let req = self.client.client.http.post(url);
            let response = crate::utils::http::send_checked(
                self.client
                    .apply_auth(req)
                    .header("Accept", "text/event-stream")
                    .json(&body),
            )
            .await?;

            let warning_chunk =
                (!warnings.is_empty()).then_some(StreamChunk::Warnings { warnings });
            let data_stream = crate::utils::sse::sse_data_stream_from_response(response);
            let state = LegacyCompletionStreamState::default();
            let stream = stream::try_unfold(
                (
                    warning_chunk,
                    data_stream,
                    VecDeque::<StreamChunk>::new(),
                    state,
                ),
                |(mut warning_chunk, mut data_stream, mut queue, mut state)| async move {
                    loop {
                        if let Some(chunk) = warning_chunk.take() {
                            return Ok(Some((chunk, (warning_chunk, data_stream, queue, state))));
                        }

                        if let Some(chunk) = queue.pop_front() {
                            return Ok(Some((chunk, (warning_chunk, data_stream, queue, state))));
                        }

                        let Some(next) = data_stream.next().await else {
                            return Ok(None);
                        };
                        let data = next?;
                        queue.extend(parse_stream_data(&mut state, &data)?);
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
    use httpmock::{Method::POST, MockServer};
    use serde_json::json;

    #[test]
    fn flattens_chat_messages_into_prompt_transcript() {
        let mut warnings = Vec::new();
        let prompt = OpenAICompletionsLegacy::messages_to_prompt(
            &[
                Message::system("You are terse."),
                Message::user("hello"),
                Message::assistant("hi"),
                Message::user("continue"),
            ],
            &mut warnings,
        );

        assert!(prompt.contains("System: You are terse."));
        assert!(prompt.contains("User: hello"));
        assert!(prompt.ends_with("Assistant:"));
        assert!(warnings.iter().any(|warning| matches!(
            warning,
            Warning::Compatibility { feature, .. } if feature == "legacy_completions.messages"
        )));
    }

    #[tokio::test]
    async fn generate_posts_to_legacy_completions_surface() -> crate::Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }

        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v1/completions")
                    .header("authorization", "Bearer sk-test")
                    .json_body_includes(json!({"model":"davinci-002"}).to_string())
                    .json_body_includes(json!({"prompt":"hello"}).to_string())
                    .json_body_includes(json!({"stream":false}).to_string())
                    .json_body_includes(json!({"best_of":2}).to_string());
                then.status(200)
                    .header("content-type", "application/json")
                    .body(
                        json!({
                            "id": "cmpl_123",
                            "choices": [{
                                "text": "world",
                                "finish_reason": "stop"
                            }],
                            "usage": {
                                "prompt_tokens": 1,
                                "completion_tokens": 1,
                                "total_tokens": 2
                            }
                        })
                        .to_string(),
                    );
            })
            .await;

        let client = OpenAICompletionsLegacy::new("sk-test")
            .with_base_url(server.url("/v1"))
            .with_model("davinci-002");
        let mut request = GenerateRequest::from(vec![Message::user("hello")]);
        request.provider_options = Some(
            serde_json::json!({
                "*": { "temperature": 0.4 },
                "openai": { "best_of": 2 }
            })
            .into(),
        );

        let response = client.generate(request).await?;

        mock.assert_async().await;
        assert_eq!(response.text(), "world");
        assert_eq!(response.finish_reason, FinishReason::Stop);
        Ok(())
    }

    #[cfg(feature = "streaming")]
    #[test]
    fn parses_legacy_completion_stream_chunks() -> crate::Result<()> {
        let mut state = LegacyCompletionStreamState::default();
        let chunks = parse_stream_data(
            &mut state,
            r#"{"id":"cmpl_123","choices":[{"text":"hel","finish_reason":null}]}"#,
        )?;
        assert_eq!(
            chunks,
            vec![
                StreamChunk::ResponseId {
                    id: "cmpl_123".to_string()
                },
                StreamChunk::TextDelta {
                    text: "hel".to_string()
                }
            ]
        );

        let chunks = parse_stream_data(
            &mut state,
            r#"{"choices":[{"text":"lo","finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#,
        )?;
        assert_eq!(
            chunks[0],
            StreamChunk::Usage(Usage {
                input_tokens: Some(1),
                cache_input_tokens: None,
                cache_creation_input_tokens: None,
                output_tokens: Some(1),
                total_tokens: Some(2),
            })
        );
        assert_eq!(
            chunks[1],
            StreamChunk::TextDelta {
                text: "lo".to_string()
            }
        );
        assert_eq!(chunks[2], StreamChunk::FinishReason(FinishReason::Stop));
        Ok(())
    }
}
