#[cfg(feature = "embeddings")]
#[derive(Clone)]
pub struct OpenAIEmbeddings {
    client: openai_like::OpenAiLikeClient,
}

#[cfg(feature = "embeddings")]
impl OpenAIEmbeddings {
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

    fn resolve_model(&self) -> Result<&str> {
        if !self.client.model.trim().is_empty() {
            return Ok(self.client.model.as_str());
        }
        Err(DittoError::InvalidResponse(
            "openai embedding model is not set (set OpenAIEmbeddings::with_model)".to_string(),
        ))
    }
}

#[cfg(feature = "embeddings")]
#[async_trait]
impl EmbeddingModel for OpenAIEmbeddings {
    fn provider(&self) -> &str {
        "openai"
    }

    fn model_id(&self) -> &str {
        self.client.model.as_str()
    }

    async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let model = self.resolve_model()?;
        super::openai_embeddings_common::embed(&self.client, model, texts).await
    }
}

#[derive(Debug, Clone)]
pub struct OpenAIResponsesRawRequest<'a> {
    pub model: &'a str,
    pub instructions: &'a str,
    pub input: &'a [Value],
    pub tools: Option<&'a [Tool]>,
    pub tool_choice: Option<&'a ToolChoice>,
    pub parallel_tool_calls: bool,
    pub store: bool,
    pub stream: bool,
    pub reasoning_effort: Option<crate::types::ReasoningEffort>,
    pub reasoning_summary: Option<crate::types::ReasoningSummary>,
    pub response_format: Option<&'a crate::types::ResponseFormat>,
    pub include: Vec<String>,
    pub prompt_cache_key: Option<String>,
    pub extra_headers: reqwest::header::HeaderMap,
}

#[derive(Debug, Serialize)]
pub struct OpenAIResponsesCompactionRequest<'a> {
    pub model: &'a str,
    pub input: &'a [Value],
    pub instructions: &'a str,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OpenAIResponsesRawEvent {
    Created {
        response_id: Option<String>,
    },
    OutputTextDelta(String),
    ReasoningTextDelta(String),
    ReasoningSummaryTextDelta(String),
    OutputItemDone(Value),
    Failed {
        response_id: Option<String>,
        error: Value,
    },
    Completed {
        response_id: Option<String>,
        usage: Option<Value>,
    },
}

pub struct OpenAIResponsesRawEventStream {
    rx_event: mpsc::Receiver<Result<OpenAIResponsesRawEvent>>,
    task: tokio::task::JoinHandle<()>,
}

impl OpenAIResponsesRawEventStream {
    pub async fn recv(&mut self) -> Option<Result<OpenAIResponsesRawEvent>> {
        self.rx_event.recv().await
    }
}

impl Drop for OpenAIResponsesRawEventStream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Debug, Deserialize)]
struct RawResponsesStreamEvent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    response: Option<Value>,
    #[serde(default)]
    item: Option<Value>,
    #[serde(default)]
    delta: Option<String>,
}

fn parse_raw_responses_event(
    event: RawResponsesStreamEvent,
) -> Result<Option<OpenAIResponsesRawEvent>> {
    match event.kind.as_str() {
        "response.created" => {
            let response_id = event
                .response
                .as_ref()
                .and_then(|resp| resp.get("id"))
                .and_then(Value::as_str)
                .map(|s| s.to_string());
            Ok(Some(OpenAIResponsesRawEvent::Created { response_id }))
        }
        "response.output_text.delta" => {
            Ok(event.delta.map(OpenAIResponsesRawEvent::OutputTextDelta))
        }
        "response.reasoning_text.delta" => {
            Ok(event.delta.map(OpenAIResponsesRawEvent::ReasoningTextDelta))
        }
        "response.reasoning_summary_text.delta" => Ok(event
            .delta
            .map(OpenAIResponsesRawEvent::ReasoningSummaryTextDelta)),
        "response.output_item.done" => Ok(event.item.map(OpenAIResponsesRawEvent::OutputItemDone)),
        "response.failed" => {
            let Some(resp) = event.response else {
                return Ok(Some(OpenAIResponsesRawEvent::Failed {
                    response_id: None,
                    error: Value::Null,
                }));
            };

            let response_id = resp
                .get("id")
                .and_then(Value::as_str)
                .map(|v| v.to_string());
            let error = resp.get("error").cloned().unwrap_or(resp);
            Ok(Some(OpenAIResponsesRawEvent::Failed { response_id, error }))
        }
        "response.completed" | "response.done" => {
            let response_id = event
                .response
                .as_ref()
                .and_then(|resp| resp.get("id"))
                .and_then(Value::as_str)
                .map(|s| s.to_string());
            let usage = event
                .response
                .as_ref()
                .and_then(|resp| resp.get("usage").cloned());
            Ok(Some(OpenAIResponsesRawEvent::Completed {
                response_id,
                usage,
            }))
        }
        _ => Ok(None),
    }
}

async fn process_raw_responses_sse<R>(
    reader: R,
    tx_event: mpsc::Sender<Result<OpenAIResponsesRawEvent>>,
) where
    R: tokio::io::AsyncBufRead + Unpin + Send + 'static,
{
    fn truncate_for_error(value: &str, max_bytes: usize) -> &str {
        if value.len() <= max_bytes {
            return value;
        }
        let mut end = max_bytes;
        while end > 0 && !value.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        &value[..end]
    }

    let mut data_stream = crate::utils::sse::sse_data_stream_from_reader(reader);
    while let Some(next) = data_stream.next().await {
        match next {
            Ok(data) => {
                let event =
                    serde_json::from_str::<RawResponsesStreamEvent>(&data).map_err(|err| {
                        DittoError::InvalidResponse(format!(
                            "failed to parse responses SSE event: {err}; data_prefix={}",
                            truncate_for_error(&data, 1024)
                        ))
                    });
                match event.and_then(parse_raw_responses_event) {
                    Ok(Some(parsed)) => {
                        let _ = tx_event.send(Ok(parsed)).await;
                    }
                    Ok(None) => {}
                    Err(err) => {
                        let _ = tx_event.send(Err(err)).await;
                    }
                }
            }
            Err(err) => {
                let _ = tx_event.send(Err(err)).await;
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::{Method::GET, Method::POST, MockServer};
    use serde_json::json;
    use std::collections::BTreeMap;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn from_config_resolves_api_key_and_model() -> crate::Result<()> {
        let config = ProviderConfig {
            base_url: Some("http://localhost:1234/v1".to_string()),
            default_model: Some("test-model".to_string()),
            auth: Some(crate::ProviderAuth::ApiKeyEnv {
                keys: vec!["DITTO_TEST_OPENAI_KEY".to_string()],
            }),
            ..ProviderConfig::default()
        };
        let env = Env {
            dotenv: std::collections::BTreeMap::from([(
                "DITTO_TEST_OPENAI_KEY".to_string(),
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
    async fn download_file_content_is_bounded() -> crate::Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/v1/files/file_123/content")
                    .header("authorization", "Bearer sk-test");
                then.status(200)
                    .header("content-type", "text/plain")
                    .body("hello world");
            })
            .await;

        let client = OpenAI::new("sk-test")
            .with_base_url(server.url("/v1"))
            .with_max_binary_response_bytes(4);
        let err = client.download_file_content("file_123").await.unwrap_err();

        mock.assert_async().await;
        match err {
            DittoError::InvalidResponse(message) => {
                assert!(message.contains("exceeds max bytes"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
        Ok(())
    }

    #[tokio::test]
    async fn compact_responses_history_raw_posts_to_compact_endpoint() -> crate::Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v1/responses/compact")
                    .header("authorization", "Bearer sk-test")
                    .json_body(json!({
                        "model": "test-model",
                        "instructions": "inst",
                        "input": [
                            {"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}
                        ],
                    }));
                then.status(200)
                    .header("content-type", "application/json")
                    .json_body(json!({
                        "output": [
                            {"type":"message","role":"user","content":[{"type":"input_text","text":"compacted"}]}
                        ]
                    }));
            })
            .await;

        let client = OpenAI::new("sk-test").with_base_url(server.url("/v1"));
        let input = vec![json!({
            "type": "message",
            "role": "user",
            "content": [{ "type": "input_text", "text": "hello" }],
        })];
        let request = OpenAIResponsesCompactionRequest {
            model: "test-model",
            instructions: "inst",
            input: &input,
        };

        let output = client.compact_responses_history_raw(&request).await?;
        mock.assert_async().await;
        assert_eq!(
            output,
            vec![json!({
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": "compacted" }],
            })]
        );
        Ok(())
    }

    #[tokio::test]
    async fn raw_responses_sse_parses_expected_events() -> crate::Result<()> {
        let sse = concat!(
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n",
            "data: {\"type\":\"ignored.event\",\"foo\":\"bar\"}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello\"}\n\n",
            "data: {\"type\":\"response.reasoning_summary_text.delta\",\"delta\":\"**Plan**\"}\n\n",
            "data: {\"type\":\"response.reasoning_text.delta\",\"delta\":\"Long reasoning\"}\n\n",
            "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"function_call\",\"call_id\":\"call_1\",\"name\":\"tool\",\"arguments\":\"{}\"}}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"usage\":{\"input_tokens\":1,\"output_tokens\":2,\"total_tokens\":3}}}\n\n",
            "data: [DONE]\n\n",
        );

        let reader = tokio::io::BufReader::new(sse.as_bytes());
        let (tx_event, mut rx_event) = mpsc::channel::<Result<OpenAIResponsesRawEvent>>(16);
        tokio::spawn(process_raw_responses_sse(reader, tx_event));

        let mut events = Vec::new();
        while let Some(evt) = rx_event.recv().await {
            events.push(evt?);
        }

        assert_eq!(events.len(), 6);
        assert!(matches!(
            events[0],
            OpenAIResponsesRawEvent::Created {
                response_id: Some(_)
            }
        ));
        assert_eq!(
            events[1],
            OpenAIResponsesRawEvent::OutputTextDelta("Hello".to_string())
        );
        assert_eq!(
            events[2],
            OpenAIResponsesRawEvent::ReasoningSummaryTextDelta("**Plan**".to_string())
        );
        assert_eq!(
            events[3],
            OpenAIResponsesRawEvent::ReasoningTextDelta("Long reasoning".to_string())
        );
        assert_eq!(
            events[4],
            OpenAIResponsesRawEvent::OutputItemDone(json!({
                "type": "function_call",
                "call_id": "call_1",
                "name": "tool",
                "arguments": "{}",
            }))
        );
        assert_eq!(
            events[5],
            OpenAIResponsesRawEvent::Completed {
                response_id: Some("resp_1".to_string()),
                usage: Some(json!({
                    "input_tokens": 1,
                    "output_tokens": 2,
                    "total_tokens": 3,
                })),
            }
        );
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
                keys: vec!["DITTO_TEST_OPENAI_KEY".to_string()],
                prefix: None,
            }),
            ..ProviderConfig::default()
        };
        let env = Env {
            dotenv: BTreeMap::from([(
                "DITTO_TEST_OPENAI_KEY".to_string(),
                "sk-test".to_string(),
            )]),
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
                keys: vec!["DITTO_TEST_OPENAI_KEY".to_string()],
            }),
            ..ProviderConfig::default()
        };
        let env = Env {
            dotenv: BTreeMap::from([(
                "DITTO_TEST_OPENAI_KEY".to_string(),
                "sk-test".to_string(),
            )]),
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
