#[tokio::test]
async fn openai_compat_proxy_model_map_wildcard_rewrites_model() {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return;
    }
    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/chat/completions")
            .header("authorization", "Bearer sk-test")
            .json_body(json!({
                "model": "glm-4.7",
                "messages": [{"role":"user","content":"hi"}],
                "temperature": 0.1
            }));
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"id":"ok"}"#);
    });

    let mut backend = backend_config("primary", upstream.base_url(), "Bearer sk-test");
    backend
        .model_map
        .insert("*".to_string(), "glm-4.7".to_string());

    let config = GatewayConfig {
        backends: vec![backend],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
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
        "model": "claude-sonnet-4-5-20250929",
        "messages": [{"role":"user","content":"hi"}],
        "temperature": 0.1
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("authorization", "Bearer vk-1")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(bytes, r#"{"id":"ok"}"#);
    mock.assert();
}
