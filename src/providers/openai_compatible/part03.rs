#[cfg(feature = "embeddings")]
#[derive(Clone)]
pub struct OpenAICompatibleEmbeddings {
    http: reqwest::Client,
    base_url: String,
    auth: Option<RequestAuth>,
    model: String,
    http_query_params: BTreeMap<String, String>,
}

#[cfg(feature = "embeddings")]
impl OpenAICompatibleEmbeddings {
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
        const DEFAULT_KEYS: &[&str] = &[
            "OPENAI_COMPAT_API_KEY",
            "OPENAI_API_KEY",
            "CODE_PM_OPENAI_API_KEY",
        ];

        let auth = match config.auth.clone() {
            Some(auth) => Some(
                resolve_request_auth_with_default_keys(
                    &auth,
                    env,
                    DEFAULT_KEYS,
                    "authorization",
                    Some("Bearer "),
                )
                .await?,
            ),
            None => DEFAULT_KEYS
                .iter()
                .find_map(|key| env.get(key))
                .and_then(|token| HttpAuth::bearer(&token).ok().map(RequestAuth::Http)),
        };

        let mut out = Self::new("");
        out.auth = auth;
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

    fn embeddings_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/embeddings") {
            base.to_string()
        } else {
            format!("{base}/embeddings")
        }
    }

    fn resolve_model(&self) -> Result<&str> {
        if !self.model.trim().is_empty() {
            return Ok(self.model.as_str());
        }
        Err(DittoError::InvalidResponse(
            "openai-compatible embedding model is not set (set OpenAICompatibleEmbeddings::with_model)"
                .to_string(),
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
impl EmbeddingModel for OpenAICompatibleEmbeddings {
    fn provider(&self) -> &str {
        "openai-compatible"
    }

    fn model_id(&self) -> &str {
        self.model.as_str()
    }

    async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let model = self.resolve_model()?;
        let url = self.embeddings_url();

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

    #[tokio::test]
    async fn upload_file_uses_custom_auth_header() -> Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v1/files")
                    .header("api-key", "sk-test")
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
            auth: Some(crate::ProviderAuth::HttpHeaderEnv {
                header: "api-key".to_string(),
                keys: vec!["CODEPM_TEST_OPENAI_COMPAT_KEY".to_string()],
                prefix: None,
            }),
            ..ProviderConfig::default()
        };
        let env = Env {
            dotenv: BTreeMap::from([(
                "CODEPM_TEST_OPENAI_COMPAT_KEY".to_string(),
                "sk-test".to_string(),
            )]),
        };

        let client = OpenAICompatible::from_config(&config, &env).await?;
        let id = client.upload_file("hello.txt", b"hello".to_vec()).await?;

        mock.assert_async().await;
        assert_eq!(id, "file_123");
        Ok(())
    }

    #[tokio::test]
    async fn upload_file_includes_default_query_params() -> Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v1/files")
                    .query_param("api-version", "2024-02-01")
                    .header("api-key", "sk-test")
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
            auth: Some(crate::ProviderAuth::HttpHeaderEnv {
                header: "api-key".to_string(),
                keys: vec!["CODEPM_TEST_OPENAI_COMPAT_KEY".to_string()],
                prefix: None,
            }),
            ..ProviderConfig::default()
        };
        let env = Env {
            dotenv: BTreeMap::from([(
                "CODEPM_TEST_OPENAI_COMPAT_KEY".to_string(),
                "sk-test".to_string(),
            )]),
        };

        let client = OpenAICompatible::from_config(&config, &env).await?;
        let id = client.upload_file("hello.txt", b"hello".to_vec()).await?;

        mock.assert_async().await;
        assert_eq!(id, "file_123");
        Ok(())
    }

    #[tokio::test]
    async fn upload_file_uses_query_param_auth() -> Result<()> {
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
                keys: vec!["CODEPM_TEST_OPENAI_COMPAT_KEY".to_string()],
                prefix: None,
            }),
            ..ProviderConfig::default()
        };
        let env = Env {
            dotenv: BTreeMap::from([(
                "CODEPM_TEST_OPENAI_COMPAT_KEY".to_string(),
                "sk-test".to_string(),
            )]),
        };

        let client = OpenAICompatible::from_config(&config, &env).await?;
        let id = client.upload_file("hello.txt", b"hello".to_vec()).await?;

        mock.assert_async().await;
        assert_eq!(id, "file_123");
        Ok(())
    }

    #[cfg(feature = "embeddings")]
    #[tokio::test]
    async fn embeddings_from_config_resolves_model() -> Result<()> {
        let config = ProviderConfig {
            base_url: Some("http://localhost:1234/v1".to_string()),
            default_model: Some("test-embed-model".to_string()),
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

        let client = OpenAICompatibleEmbeddings::from_config(&config, &env).await?;
        assert_eq!(client.provider(), "openai-compatible");
        assert_eq!(client.model_id(), "test-embed-model");
        Ok(())
    }

    #[cfg(feature = "embeddings")]
    #[tokio::test]
    async fn embeddings_embed_posts_to_embeddings_endpoint_with_query_param_auth() -> Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v1/embeddings")
                    .query_param("api_key", "sk-test")
                    .body_includes("\"model\":\"test-embed-model\"")
                    .body_includes("\"input\":[\"hello\"]");
                then.status(200)
                    .header("content-type", "application/json")
                    .body("{\"data\":[{\"embedding\":[1.0,2.0]}]}");
            })
            .await;

        let config = ProviderConfig {
            base_url: Some(server.url("/v1")),
            default_model: Some("test-embed-model".to_string()),
            auth: Some(crate::ProviderAuth::QueryParamEnv {
                param: "api_key".to_string(),
                keys: vec!["CODEPM_TEST_OPENAI_COMPAT_KEY".to_string()],
                prefix: None,
            }),
            ..ProviderConfig::default()
        };
        let env = Env {
            dotenv: BTreeMap::from([(
                "CODEPM_TEST_OPENAI_COMPAT_KEY".to_string(),
                "sk-test".to_string(),
            )]),
        };

        let client = OpenAICompatibleEmbeddings::from_config(&config, &env).await?;
        let out = client.embed(vec!["hello".to_string()]).await?;

        mock.assert_async().await;
        assert_eq!(out, vec![vec![1.0, 2.0]]);
        Ok(())
    }

    #[test]
    fn tool_choice_required_maps_to_required() {
        let mapped = OpenAICompatible::tool_choice_to_openai(&ToolChoice::Required);
        assert_eq!(mapped, Value::String("required".to_string()));
    }

    #[tokio::test]
    async fn generate_sends_stream_false_and_ignores_provider_override() -> Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }

        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v1/chat/completions")
                    .body_includes("\"stream\":false")
                    .body_includes("\"model\":\"test-model\"")
                    .body_includes("\"messages\":[");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(
                    json!({
                        "id": "chatcmpl_123",
                        "model": "test-model",
                        "choices": [
                            {
                                "message": { "content": "hi" },
                                "finish_reason": "stop"
                            }
                        ],
                        "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
                    })
                    .to_string(),
                );
            })
            .await;

        let client = OpenAICompatible::new("sk-test")
            .with_base_url(server.url("/v1"))
            .with_model("test-model");

        let mut request = GenerateRequest::from(vec![Message::user("hi")]);
        request.provider_options = Some(json!({
            "openai-compatible": { "stream": true }
        }));

        let response = client.generate(request).await?;

        mock.assert_async().await;
        assert_eq!(response.text(), "hi".to_string());
        assert!(response.warnings.iter().any(|w| matches!(
            w,
            Warning::Compatibility { feature, details }
                if feature == "generate.provider_options" && details.contains("overrides stream")
        )));
        Ok(())
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

    #[test]
    fn preserves_raw_tool_call_arguments_string() {
        let messages = vec![Message {
            role: Role::Assistant,
            content: vec![ContentPart::ToolCall {
                id: "call_1".to_string(),
                name: "add".to_string(),
                arguments: Value::String("{\"a\":1}".to_string()),
            }],
        }];

        let (mapped, warnings) = OpenAICompatible::messages_to_chat_messages(&messages);
        assert!(warnings.is_empty());
        assert_eq!(mapped.len(), 1);

        let tool_calls = mapped[0]
            .get("tool_calls")
            .and_then(Value::as_array)
            .expect("tool_calls array");
        assert_eq!(tool_calls.len(), 1);
        let arguments = tool_calls[0]
            .get("function")
            .and_then(Value::as_object)
            .and_then(|obj| obj.get("arguments"))
            .and_then(Value::as_str)
            .expect("tool_call arguments string");
        assert_eq!(arguments, "{\"a\":1}");
    }

    #[test]
    fn converts_pdf_file_part_to_chat_file_content() {
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

        let (mapped, warnings) = OpenAICompatible::messages_to_chat_messages(&messages);
        assert!(warnings.is_empty());
        assert_eq!(mapped.len(), 1);
        assert_eq!(mapped[0].get("role").and_then(Value::as_str), Some("user"));
        let content = mapped[0]
            .get("content")
            .and_then(Value::as_array)
            .expect("content array");
        assert_eq!(content.len(), 1);
        assert_eq!(content[0].get("type").and_then(Value::as_str), Some("file"));
        assert_eq!(
            content[0]
                .get("file")
                .and_then(Value::as_object)
                .and_then(|o| o.get("filename"))
                .and_then(Value::as_str),
            Some("doc.pdf")
        );
        assert_eq!(
            content[0]
                .get("file")
                .and_then(Value::as_object)
                .and_then(|o| o.get("file_data"))
                .and_then(Value::as_str),
            Some("data:application/pdf;base64,AQIDBAU=")
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

    #[tokio::test]
    async fn generate_supports_legacy_function_call() -> Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST).path("/v1/chat/completions");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(
                        serde_json::json!({
                            "id": "cmpl_1",
                            "model": "test-model",
                            "choices": [{
                                "message": {
                                    "role": "assistant",
                                    "content": null,
                                    "function_call": {
                                        "name": "add",
                                        "arguments": "{\"a\":1,\"b\":2}"
                                    }
                                },
                                "finish_reason": "function_call"
                            }],
                            "usage": {
                                "prompt_tokens": 1,
                                "completion_tokens": 2,
                                "total_tokens": 3
                            }
                        })
                        .to_string(),
                    );
            })
            .await;

        let client = OpenAICompatible::new("")
            .with_base_url(server.url("/v1"))
            .with_model("test-model");

        let response = client.generate(vec![Message::user("hi")].into()).await?;
        mock.assert_async().await;

        assert_eq!(response.finish_reason, FinishReason::ToolCalls);
        assert!(
            response.warnings.iter().any(
                |w| matches!(w, Warning::Compatibility { feature, .. } if feature == "tool_call.id")
            ),
            "expected compatibility warning for synthesized tool_call id"
        );
        assert_eq!(response.content.len(), 1);
        match &response.content[0] {
            ContentPart::ToolCall {
                id,
                name,
                arguments,
            } => {
                assert_eq!(id, "call_0");
                assert_eq!(name, "add");
                assert_eq!(arguments, &serde_json::json!({ "a": 1, "b": 2 }));
            }
            other => panic!("unexpected content part: {other:?}"),
        }
        assert_eq!(response.usage.total_tokens, Some(3));
        Ok(())
    }

    #[cfg(feature = "streaming")]
    #[test]
    fn parses_streaming_legacy_function_call_deltas() -> Result<()> {
        let mut state = StreamState::default();

        let (chunks, done) = parse_stream_data(
            &mut state,
            &serde_json::json!({
                "choices": [{
                    "delta": {
                        "function_call": { "name": "add", "arguments": "{\"a\": 4" }
                    }
                }]
            })
            .to_string(),
        )?;
        assert!(!done);
        assert!(
            matches!(chunks.first(), Some(StreamChunk::Warnings { .. })),
            "expected warnings for synthesized tool_call id"
        );
        assert!(
            chunks
                .iter()
                .any(|c| matches!(c, StreamChunk::ToolCallStart { id, name } if id == "call_0" && name == "add")),
            "expected tool call start"
        );
        assert!(
            chunks
                .iter()
                .any(|c| matches!(c, StreamChunk::ToolCallDelta { id, arguments_delta } if id == "call_0" && arguments_delta == "{\"a\": 4")),
            "expected tool call delta"
        );

        let (chunks, done) = parse_stream_data(
            &mut state,
            &serde_json::json!({
                "choices": [{
                    "delta": {
                        "function_call": { "arguments": ", \"b\": 2}" }
                    },
                    "finish_reason": "function_call"
                }]
            })
            .to_string(),
        )?;
        assert!(done);
        assert_eq!(
            chunks,
            vec![
                StreamChunk::ToolCallDelta {
                    id: "call_0".to_string(),
                    arguments_delta: ", \"b\": 2}".to_string(),
                },
                StreamChunk::FinishReason(FinishReason::ToolCalls),
            ]
        );

        Ok(())
    }

    #[cfg(feature = "streaming")]
    #[test]
    fn parses_streaming_response_id() -> Result<()> {
        let mut state = StreamState::default();

        let (chunks, done) = parse_stream_data(
            &mut state,
            &serde_json::json!({
                "id": "resp_1",
                "choices": [{
                    "delta": { "content": "hi" }
                }]
            })
            .to_string(),
        )?;

        assert!(!done);
        assert_eq!(
            chunks,
            vec![
                StreamChunk::ResponseId {
                    id: "resp_1".to_string()
                },
                StreamChunk::TextDelta {
                    text: "hi".to_string()
                }
            ]
        );
        Ok(())
    }

    #[cfg(feature = "streaming")]
    #[test]
    fn flushes_tool_call_without_id_on_finish_reason() -> Result<()> {
        let mut state = StreamState::default();

        let (chunks, done) = parse_stream_data(
            &mut state,
            &serde_json::json!({
                "choices": [{
                    "delta": {
                        "tool_calls": [{
                            "index": 0,
                            "function": { "name": "add", "arguments": "{\"a\": 1" }
                        }]
                    }
                }]
            })
            .to_string(),
        )?;
        assert!(!done);
        assert!(chunks.is_empty());

        let (chunks, done) = parse_stream_data(
            &mut state,
            &serde_json::json!({
                "choices": [{
                    "delta": {},
                    "finish_reason": "tool_calls"
                }]
            })
            .to_string(),
        )?;
        assert!(done);
        assert!(
            matches!(chunks.first(), Some(StreamChunk::Warnings { .. })),
            "expected warnings for synthesized tool_call id"
        );
        assert!(
            chunks
                .iter()
                .any(|c| matches!(c, StreamChunk::ToolCallStart { .. })),
            "expected tool call start"
        );
        assert!(
            chunks
                .iter()
                .any(|c| matches!(c, StreamChunk::ToolCallDelta { .. })),
            "expected tool call delta"
        );
        assert!(
            chunks
                .iter()
                .any(|c| matches!(c, StreamChunk::FinishReason(FinishReason::ToolCalls))),
            "expected finish reason tool_calls"
        );
        Ok(())
    }
}
