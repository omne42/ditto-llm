#[cfg(all(feature = "cap-llm-streaming", feature = "provider-openai"))]
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::mpsc;

#[cfg(all(feature = "cap-llm-streaming", feature = "provider-openai"))]
use futures_util::StreamExt;

#[cfg(all(test, feature = "provider-openai"))]
use super::OpenAI;
use crate::contracts::{Tool, ToolChoice};
use crate::error::Result;
use crate::provider_options::ResponseFormat;

pub struct OpenAIResponsesRawRequest<'a> {
    pub model: &'a str,
    pub instructions: &'a str,
    pub input: &'a [Value],
    pub tools: Option<&'a [Tool]>,
    pub tool_choice: Option<&'a ToolChoice>,
    pub parallel_tool_calls: bool,
    pub store: bool,
    pub stream: bool,
    pub reasoning_effort: Option<crate::provider_options::ReasoningEffort>,
    pub reasoning_summary: Option<crate::provider_options::ReasoningSummary>,
    pub response_format: Option<&'a ResponseFormat>,
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
    pub(super) rx_event: mpsc::Receiver<Result<OpenAIResponsesRawEvent>>,
    pub(super) task: tokio::task::JoinHandle<()>,
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
#[cfg(all(feature = "cap-llm-streaming", feature = "provider-openai"))]
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

#[cfg(all(feature = "cap-llm-streaming", feature = "provider-openai"))]
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

#[cfg(all(feature = "cap-llm-streaming", feature = "provider-openai"))]
pub(super) async fn process_raw_responses_sse<R>(
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

    let mut data_stream = crate::session_transport::sse_data_stream_from_reader(reader);
    loop {
        if tx_event.is_closed() {
            break;
        }
        let Some(next) = data_stream.next().await else {
            break;
        };
        match next {
            Ok(data) => {
                let event = serde_json::from_str::<RawResponsesStreamEvent>(&data).map_err(|err| {
                    crate::invalid_response!(
                        "error_detail.openai.responses_sse_event_parse_failed",
                        "error" => err.to_string(),
                        "data_prefix" => truncate_for_error(&data, 1024)
                    )
                });
                match event.and_then(parse_raw_responses_event) {
                    Ok(Some(parsed)) => {
                        if tx_event.send(Ok(parsed)).await.is_err() {
                            break;
                        }
                    }
                    Ok(None) => {}
                    Err(err) => {
                        if tx_event.send(Err(err)).await.is_err() {
                            break;
                        }
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

#[cfg(all(test, feature = "provider-openai"))]
mod tests {
    use super::super::client::{
        OPENAI_RESPONSES_DUMMY_THOUGHT_SIGNATURE, apply_provider_options,
        sanitize_openai_responses_provider_options, split_tool_call_id_and_thought_signature,
    };
    #[cfg(feature = "cap-llm-streaming")]
    use super::super::responses::finish_reason_for_final_event;
    use super::super::responses::{map_responses_finish_reason, parse_openai_output};
    use super::*;
    use crate::config::{Env, ProviderConfig};
    use crate::contracts::{
        ContentPart, FileSource, FinishReason, GenerateRequest, Message, Role, Warning,
    };
    use crate::error::DittoError;
    use crate::llm_core::model::LanguageModel;
    use httpmock::{Method::GET, Method::POST, MockServer};
    use serde_json::Map;
    use serde_json::json;
    use std::collections::BTreeMap;
    #[cfg(feature = "cap-llm-streaming")]
    use tokio::io::AsyncWriteExt;
    #[cfg(feature = "cap-llm-streaming")]
    use tokio::sync::mpsc;
    #[cfg(feature = "cap-llm-streaming")]
    use tokio::time::{Duration, timeout};

    #[tokio::test]
    async fn from_config_resolves_api_key_and_model() -> crate::error::Result<()> {
        let config = ProviderConfig {
            base_url: Some("http://localhost:1234/v1".to_string()),
            default_model: Some("test-model".to_string()),
            auth: Some(crate::config::ProviderAuth::ApiKeyEnv {
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
    async fn upload_file_posts_to_files_endpoint() -> crate::error::Result<()> {
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
    async fn download_file_content_is_bounded() -> crate::error::Result<()> {
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
                assert!(matches!(
                    message.as_catalog().map(|message| message.code()),
                    Some("error_detail.openai_like.files_download_response_too_large")
                ));
            }
            other => panic!("unexpected error: {other:?}"),
        }
        Ok(())
    }

    #[tokio::test]
    async fn compact_responses_history_raw_posts_to_compact_endpoint() -> crate::error::Result<()> {
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
    #[cfg(feature = "cap-llm-streaming")]
    async fn raw_responses_sse_parses_expected_events() -> crate::error::Result<()> {
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
    #[cfg(feature = "cap-llm-streaming")]
    async fn raw_responses_sse_stops_when_receiver_is_dropped() -> crate::error::Result<()> {
        let (reader, mut writer) = tokio::io::duplex(1024);
        let reader = tokio::io::BufReader::new(reader);
        let (tx_event, rx_event) = mpsc::channel::<Result<OpenAIResponsesRawEvent>>(1);
        drop(rx_event);

        let task = tokio::spawn(process_raw_responses_sse(reader, tx_event));
        writer
            .write_all(b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"ignored\"}\n\n")
            .await?;
        writer.flush().await?;

        timeout(Duration::from_millis(500), task)
            .await
            .map_err(|_| {
                crate::invalid_response!("error_detail.openai.responses_sse_receiver_drop_timeout")
            })?
            .map_err(|err| {
                crate::invalid_response!(
                    "error_detail.openai.responses_sse_task_failed",
                    "error" => err.to_string()
                )
            })?;

        Ok(())
    }

    #[tokio::test]
    async fn upload_file_uses_query_param_auth() -> crate::error::Result<()> {
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
            auth: Some(crate::config::ProviderAuth::QueryParamEnv {
                param: "api_key".to_string(),
                keys: vec!["DITTO_TEST_OPENAI_KEY".to_string()],
                prefix: None,
            }),
            ..ProviderConfig::default()
        };
        let env = Env {
            dotenv: BTreeMap::from([("DITTO_TEST_OPENAI_KEY".to_string(), "sk-test".to_string())]),
        };

        let client = OpenAI::from_config(&config, &env).await?;
        let id = client.upload_file("hello.txt", b"hello".to_vec()).await?;

        mock.assert_async().await;
        assert_eq!(id, "file_123");
        Ok(())
    }

    #[tokio::test]
    async fn upload_file_includes_default_query_params() -> crate::error::Result<()> {
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
            auth: Some(crate::config::ProviderAuth::ApiKeyEnv {
                keys: vec!["DITTO_TEST_OPENAI_KEY".to_string()],
            }),
            ..ProviderConfig::default()
        };
        let env = Env {
            dotenv: BTreeMap::from([("DITTO_TEST_OPENAI_KEY".to_string(), "sk-test".to_string())]),
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

        let (instructions, input, warnings) =
            OpenAI::messages_to_input_with_quirks(&messages, false);
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

        let (instructions, input, warnings) =
            OpenAI::messages_to_input_with_quirks(&messages, false);
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
    fn messages_to_input_with_quirks_adds_dummy_thought_signature() {
        let messages = vec![Message {
            role: Role::Assistant,
            content: vec![ContentPart::ToolCall {
                id: "c1".to_string(),
                name: "add".to_string(),
                arguments: json!({ "a": 1 }),
            }],
        }];

        let (_instructions, input, warnings) =
            OpenAI::messages_to_input_with_quirks(&messages, true);
        assert!(warnings.is_empty());
        assert_eq!(input.len(), 1);
        assert_eq!(
            input[0].get("thought_signature").and_then(Value::as_str),
            Some(OPENAI_RESPONSES_DUMMY_THOUGHT_SIGNATURE)
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

        let (instructions, input, warnings) =
            OpenAI::messages_to_input_with_quirks(&messages, false);
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
    fn parses_function_call_with_thought_signature_into_encoded_call_id() {
        let output = vec![serde_json::json!({
            "type": "function_call",
            "call_id": "c1",
            "name": "add",
            "arguments": "{\"a\":1}",
            "thought_signature": "hi"
        })];

        let mut warnings = Vec::<Warning>::new();
        let parsed = parse_openai_output(&output, &mut warnings);
        assert_eq!(parsed.len(), 1);
        assert!(warnings.is_empty());

        match &parsed[0] {
            ContentPart::ToolCall { id, .. } => {
                let (base_id, thought_signature) = split_tool_call_id_and_thought_signature(id);
                assert_eq!(base_id, "c1");
                assert_eq!(thought_signature.as_deref(), Some("hi"));
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
    fn apply_provider_options_maps_reasoning_and_response_format() -> crate::error::Result<()> {
        let mut body = Map::<String, Value>::new();
        let options = crate::provider_options::ProviderOptions {
            reasoning_effort: Some(crate::provider_options::ReasoningEffort::High),
            response_format: Some(crate::provider_options::ResponseFormat::JsonSchema {
                json_schema: crate::provider_options::JsonSchemaFormat {
                    name: "unit_test".to_string(),
                    schema: json!({ "type": "object" }),
                    strict: Some(true),
                },
            }),
            parallel_tool_calls: Some(false),
            prompt_cache_key: None,
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
    fn responses_provider_options_schema_drops_unknown_and_reserved_keys() {
        let selected_provider_options = Some(json!({
            "temperature": 0.4,
            "service_tier": "default",
            "parallel_tool_calls": false,
            "unknown_vendor_knob": "x"
        }));
        let (sanitized, warnings) = sanitize_openai_responses_provider_options(
            selected_provider_options,
            "generate.provider_options",
        );

        let sanitized = sanitized
            .as_ref()
            .and_then(Value::as_object)
            .expect("sanitized provider options should be an object");
        assert_eq!(sanitized.get("temperature"), Some(&json!(0.4)));
        assert_eq!(sanitized.get("service_tier"), Some(&json!("default")));
        assert!(!sanitized.contains_key("parallel_tool_calls"));
        assert!(!sanitized.contains_key("unknown_vendor_knob"));
        assert!(warnings.iter().any(|w| matches!(
            w,
            Warning::Unsupported { feature, details }
            if feature == "generate.provider_options"
                && details.as_deref().is_some_and(|d| d.contains("unknown_vendor_knob"))
        )));
    }

    #[test]
    fn build_responses_body_merges_only_sanitized_provider_options() -> crate::error::Result<()> {
        let request = GenerateRequest::from(vec![Message::user("hello")]);
        let provider_options = crate::provider_options::ProviderOptions::default();
        let (sanitized_provider_options, mut schema_warnings) =
            sanitize_openai_responses_provider_options(
                Some(json!({
                    "service_tier": "default",
                    "parallel_tool_calls": true,
                    "unknown_private": 1
                })),
                "generate.provider_options",
            );
        let (body, mut warnings) = OpenAI::build_responses_body(
            &request,
            "gpt-test",
            &provider_options,
            sanitized_provider_options.as_ref(),
            false,
            "generate.provider_options",
            false,
        )?;
        warnings.append(&mut schema_warnings);

        assert_eq!(body.get("service_tier"), Some(&json!("default")));
        assert!(body.get("parallel_tool_calls").is_none());
        assert!(body.get("unknown_private").is_none());
        assert!(warnings.iter().any(|w| matches!(
            w,
            Warning::Unsupported { feature, details }
            if feature == "generate.provider_options"
                && details.as_deref().is_some_and(|d| d.contains("unknown_private"))
        )));
        Ok(())
    }

    #[test]
    fn parse_usage_reads_cache_read_input_tokens_alias() {
        let usage = OpenAI::parse_usage(&json!({
            "input_tokens": 12,
            "output_tokens": 3,
            "cache_read_input_tokens": 7
        }));
        assert_eq!(usage.input_tokens, Some(12));
        assert_eq!(usage.output_tokens, Some(3));
        assert_eq!(usage.cache_input_tokens, Some(7));
    }

    #[test]
    fn parse_usage_reads_cache_write_input_tokens_alias() {
        let usage = OpenAI::parse_usage(&json!({
            "input_tokens": 12,
            "output_tokens": 3,
            "cache_write_input_tokens": 5
        }));
        assert_eq!(usage.cache_creation_input_tokens, Some(5));
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
    #[cfg(feature = "cap-llm-streaming")]
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
    #[cfg(feature = "cap-llm-streaming")]
    fn finish_reason_for_final_event_marks_tool_calls() {
        let response = json!({ "status": "completed" });
        assert_eq!(
            finish_reason_for_final_event("response.completed", Some(&response), true),
            FinishReason::ToolCalls
        );
    }
}
