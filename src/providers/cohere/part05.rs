#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::{Method::POST, MockServer};

    #[tokio::test]
    async fn chat_posts_and_parses_text_and_tool_calls() -> Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v2/chat")
                    .header("authorization", "Bearer sk-test")
                    .body_includes("\"model\":\"command-r\"")
                    .body_includes("\"messages\"")
                    .body_includes("\"tools\"")
                    .body_includes("\"parameter_definitions\"")
                    .body_includes("\"tool_choice\":\"REQUIRED\"")
                    .body_includes("\"tool_call_id\":\"call_1\"")
                    .body_includes("\"tool_calls\"");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(
                        serde_json::json!({
                            "id": "chat_123",
                            "finish_reason": "COMPLETE",
                            "message": {
                                "role": "assistant",
                                "content": [
                                    { "type": "text", "text": "hello" }
                                ],
                                "tool_calls": [
                                    {
                                        "id": "call_2",
                                        "type": "tool_call",
                                        "function": { "name": "get_weather", "arguments": "{\"city\":\"sf\"}" }
                                    }
                                ]
                            },
                            "usage": { "tokens": { "input_tokens": 10, "output_tokens": 5 } }
                        })
                        .to_string(),
                    );
            })
            .await;

        let config = ProviderConfig {
            base_url: Some(server.url("/v2")),
            default_model: Some("command-r".to_string()),
            auth: Some(crate::ProviderAuth::ApiKeyEnv {
                keys: vec!["DITTO_TEST_COHERE_KEY".to_string()],
            }),
            ..ProviderConfig::default()
        };
        let env = Env {
            dotenv: BTreeMap::from([(
                "DITTO_TEST_COHERE_KEY".to_string(),
                "sk-test".to_string(),
            )]),
        };

        let tool = Tool {
            name: "get_weather".to_string(),
            description: Some("get weather".to_string()),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "city": { "type": "string", "description": "city name" }
                },
                "required": ["city"]
            }),
            strict: None,
        };

        let client = Cohere::from_config(&config, &env).await?;
        let response = client
            .generate(GenerateRequest {
                messages: vec![
                    Message::user("hi"),
                    Message {
                        role: Role::Assistant,
                        content: vec![ContentPart::ToolCall {
                            id: "call_1".to_string(),
                            name: "get_weather".to_string(),
                            arguments: serde_json::json!({ "city": "sf" }),
                        }],
                    },
                    Message::tool_result("call_1", "sunny"),
                ],
                model: None,
                temperature: None,
                max_tokens: None,
                top_p: None,
                seed: None,
                presence_penalty: None,
                frequency_penalty: None,
                logprobs: None,
                top_logprobs: None,
                user: None,
                stop_sequences: None,
                tools: Some(vec![tool]),
                tool_choice: Some(ToolChoice::Required),
                provider_options: None,
            })
            .await?;

        mock.assert_async().await;
        assert!(matches!(response.finish_reason, FinishReason::ToolCalls));
        assert!(response.content.iter().any(|part| matches!(
            part,
            ContentPart::Text { text } if text == "hello"
        )));
        assert!(response.content.iter().any(|part| matches!(
            part,
            ContentPart::ToolCall { id, name, arguments } if id == "call_2" && name == "get_weather" && arguments.get("city").and_then(Value::as_str) == Some("sf")
        )));
        Ok(())
    }

    #[cfg(feature = "streaming")]
    #[tokio::test]
    async fn stream_parses_text_and_tool_call_deltas() -> Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;

        let sse = concat!(
            "data: {\"type\":\"message-start\",\"delta\":{\"message\":{\"id\":\"chat_123\"}}}\n\n",
            "data: {\"type\":\"content-delta\",\"delta\":{\"message\":{\"content\":{\"text\":\"Hello\"}}}}\n\n",
            "data: {\"type\":\"tool-call-start\",\"delta\":{\"tool_call\":{\"id\":\"call_1\",\"type\":\"tool_call\",\"function\":{\"name\":\"get_weather\",\"arguments\":\"{\\\"city\\\":\\\"sf\\\"}\"}}}}\n\n",
            "data: {\"type\":\"tool-call-delta\",\"delta\":{\"tool_call\":{\"id\":\"call_1\",\"type\":\"tool_call\",\"function\":{\"arguments\":\"{\\\"city\\\":\\\"sf\\\",\\\"unit\\\":\\\"c\\\"}\"}}}}\n\n",
            "data: {\"type\":\"message-end\",\"delta\":{\"finish_reason\":\"COMPLETE\",\"usage\":{\"tokens\":{\"input_tokens\":10,\"output_tokens\":5}}}}\n\n",
        );

        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v2/chat")
                    .header("authorization", "Bearer sk-test")
                    .body_includes("\"stream\":true");
                then.status(200)
                    .header("content-type", "text/event-stream")
                    .body(sse);
            })
            .await;

        let client = Cohere::new("sk-test")
            .with_base_url(server.url("/v2"))
            .with_model("command-r");

        let stream = client
            .stream(GenerateRequest::from(vec![Message::user("hi")]))
            .await?;
        let collected = crate::collect_stream(stream).await?;

        mock.assert_async().await;

        assert_eq!(collected.response_id.as_deref(), Some("chat_123"));
        assert!(matches!(
            collected.response.finish_reason,
            FinishReason::ToolCalls
        ));
        assert_eq!(collected.response.usage.input_tokens, Some(10));
        assert_eq!(collected.response.usage.output_tokens, Some(5));

        assert!(
            collected
                .response
                .content
                .iter()
                .any(|part| matches!(part, ContentPart::Text { text } if text == "Hello"))
        );
        assert!(collected.response.content.iter().any(|part| matches!(
            part,
            ContentPart::ToolCall { id, name, arguments } if id == "call_1" && name == "get_weather" && arguments.get("city").and_then(Value::as_str) == Some("sf") && arguments.get("unit").and_then(Value::as_str) == Some("c")
        )));

        Ok(())
    }

    #[cfg(feature = "rerank")]
    #[tokio::test]
    async fn rerank_posts_and_parses_results() -> Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v2/rerank")
                    .header("authorization", "Bearer sk-test")
                    .body_includes("\"model\":\"rerank-v3.5\"")
                    .body_includes("\"query\":\"hello\"")
                    .body_includes("\"top_n\":2")
                    .body_includes("\"documents\":[\"a\",\"b\"]");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(
                        serde_json::json!({
                            "id": "rr-123",
                            "results": [
                                { "index": 0, "relevance_score": 0.9 },
                                { "index": 1, "relevance_score": 0.1 }
                            ],
                            "meta": { "billed_units": { "search_units": 1 } }
                        })
                        .to_string(),
                    );
            })
            .await;

        let config = ProviderConfig {
            base_url: Some(server.url("/v2")),
            default_model: Some("rerank-v3.5".to_string()),
            auth: Some(crate::ProviderAuth::ApiKeyEnv {
                keys: vec!["DITTO_TEST_COHERE_KEY".to_string()],
            }),
            ..ProviderConfig::default()
        };
        let env = Env {
            dotenv: BTreeMap::from([(
                "DITTO_TEST_COHERE_KEY".to_string(),
                "sk-test".to_string(),
            )]),
        };

        let client = CohereRerank::from_config(&config, &env).await?;
        let response = client
            .rerank(RerankRequest {
                query: "hello".to_string(),
                documents: vec![
                    RerankDocument::Text("a".to_string()),
                    RerankDocument::Text("b".to_string()),
                ],
                model: None,
                top_n: Some(2),
                provider_options: None,
            })
            .await?;

        mock.assert_async().await;
        assert_eq!(response.ranking.len(), 2);
        assert_eq!(response.ranking[0].index, 0);
        assert_eq!(response.ranking[0].relevance_score, 0.9);
        assert_eq!(response.ranking[1].index, 1);
        assert_eq!(response.ranking[1].relevance_score, 0.1);
        Ok(())
    }

    #[cfg(feature = "rerank")]
    #[tokio::test]
    async fn rerank_warns_on_object_documents() -> Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v2/rerank")
                    .body_includes("\\\"answer\\\":42");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(
                        serde_json::json!({
                            "id": "rr-123",
                            "results": [],
                            "meta": {}
                        })
                        .to_string(),
                    );
            })
            .await;

        let client = CohereRerank::new("sk-test")
            .with_base_url(server.url("/v2"))
            .with_model("rerank-v3.5");

        let response = client
            .rerank(RerankRequest {
                query: "hello".to_string(),
                documents: vec![RerankDocument::Json(serde_json::json!({ "answer": 42 }))],
                model: None,
                top_n: None,
                provider_options: None,
            })
            .await?;

        mock.assert_async().await;
        assert!(
            response
                .warnings
                .iter()
                .any(|warning| matches!(warning, Warning::Compatibility { feature, .. } if feature == "cohere.rerank.object_documents"))
        );
        Ok(())
    }
}
