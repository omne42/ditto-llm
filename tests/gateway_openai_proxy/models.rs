#[tokio::test]
async fn openai_models_list_merges_across_backends() {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return;
    }
    let upstream_a = MockServer::start();
    let upstream_b = MockServer::start();

    let mock_a = upstream_a.mock(|when, then| {
        when.method(httpmock::Method::GET)
            .path("/v1/models")
            .header("authorization", "Bearer sk-a");
        then.status(200)
            .header("content-type", "application/json")
            .body(
                json!({
                    "object": "list",
                    "data": [
                        {"id": "gpt-4o", "object": "model"},
                        {"id": "glm-4.7", "object": "model"}
                    ]
                })
                .to_string(),
            );
    });

    let mock_b = upstream_b.mock(|when, then| {
        when.method(httpmock::Method::GET)
            .path("/v1/models")
            .header("authorization", "Bearer sk-b");
        then.status(200)
            .header("content-type", "application/json")
            .body(
                json!({
                    "object": "list",
                    "data": [
                        {"id": "glm-4.7", "object": "model"},
                        {"id": "claude-3.5-sonnet", "object": "model"}
                    ]
                })
                .to_string(),
            );
    });

    let config = GatewayConfig {
        backends: vec![
            backend_config("a", upstream_a.base_url(), "Bearer sk-a"),
            backend_config("b", upstream_b.base_url(), "Bearer sk-b"),
        ],
        virtual_keys: Vec::new(),
        router: RouterConfig {
            default_backend: "a".to_string(),
            default_backends: Vec::new(),
            rules: Vec::new(),
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway).with_proxy_backends(proxy_backends);
    let app = ditto_llm::gateway::http::router(state);

    let request = Request::builder()
        .method("GET")
        .uri("/v1/models")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    let ids: Vec<String> = json
        .get("data")
        .and_then(|v| v.as_array())
        .into_iter()
        .flat_map(|arr| arr.iter())
        .filter_map(|m| m.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();

    assert_eq!(ids.len(), 3);
    assert!(ids.contains(&"gpt-4o".to_string()));
    assert!(ids.contains(&"glm-4.7".to_string()));
    assert!(ids.contains(&"claude-3.5-sonnet".to_string()));

    mock_a.assert();
    mock_b.assert();
}
