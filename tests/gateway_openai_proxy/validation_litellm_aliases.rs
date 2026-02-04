#[tokio::test]
async fn openai_compat_proxy_schema_validation_rejects_invalid_chat_completions_request_without_v1_prefix()
-> ditto_llm::Result<()> {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return Ok(());
    }
    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/chat/completions")
            .header("authorization", "Bearer sk-test");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"id":"ok"}"#);
    });

    let mut key = VirtualKeyConfig::new("key-1", "vk-1");
    key.guardrails.validate_schema = true;

    let config = GatewayConfig {
        backends: vec![backend_config(
            "primary",
            upstream.base_url(),
            "Bearer sk-test",
        )],
        virtual_keys: vec![key],
        router: RouterConfig {
            default_backends: vec![RouteBackend { backend: "primary".to_string(), weight: 1.0 }],
            rules: Vec::new(),
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
    };

    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway).with_proxy_backends(proxy_backends);
    let app = ditto_llm::gateway::http::router(state);

    let body = json!({
        "model": "gpt-4o-mini"
    });
    let request = Request::builder()
        .method("POST")
        .uri("/chat/completions")
        .header("authorization", "Bearer vk-1")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(
        parsed
            .get("error")
            .and_then(|value| value.get("code"))
            .and_then(|value| value.as_str()),
        Some("invalid_request")
    );
    mock.assert_calls(0);
    Ok(())
}
