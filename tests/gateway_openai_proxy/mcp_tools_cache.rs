#[tokio::test]
async fn openai_compat_proxy_caches_mcp_tools_list_between_requests() {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return;
    }

    let mcp_upstream = MockServer::start();
    let mcp_list = mcp_upstream.mock(|when, then| {
        when.method(POST).path("/mcp").json_body(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {},
        }));
        then.status(200)
            .header("content-type", "application/json")
            .body(
                json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": {
                        "tools": [{
                            "name": "hello",
                            "description": "hi",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "who": { "type": "string" }
                                }
                            }
                        }]
                    }
                })
                .to_string(),
            );
    });

    let openai_upstream = MockServer::start();
    let openai_req1 = openai_upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/chat/completions")
            .header("authorization", "Bearer sk-test")
            .header("x-request-id", "req-1")
            .json_body(json!({
                "model": "gpt-4o-mini",
                "messages": [{"role":"user","content":"hi"}],
                "stream": false,
                "tools": [{
                    "type": "function",
                    "function": {
                        "name": "hello",
                        "description": "hi",
                        "parameters": {
                            "type": "object",
                            "properties": {
                                "who": { "type": "string" }
                            }
                        }
                    }
                }]
            }));
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"id":"ok1"}"#);
    });
    let openai_req2 = openai_upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/chat/completions")
            .header("authorization", "Bearer sk-test")
            .header("x-request-id", "req-2")
            .json_body(json!({
                "model": "gpt-4o-mini",
                "messages": [{"role":"user","content":"hi"}],
                "stream": false,
                "tools": [{
                    "type": "function",
                    "function": {
                        "name": "hello",
                        "description": "hi",
                        "parameters": {
                            "type": "object",
                            "properties": {
                                "who": { "type": "string" }
                            }
                        }
                    }
                }]
            }));
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"id":"ok2"}"#);
    });

    let config = GatewayConfig {
        backends: vec![backend_config(
            "primary",
            openai_upstream.base_url(),
            "Bearer sk-test",
        )],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
        router: RouterConfig {
            default_backends: vec![RouteBackend {
                backend: "primary".to_string(),
                weight: 1.0,
            }],
            rules: Vec::new(),
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);

    let mut mcp_servers = HashMap::new();
    mcp_servers.insert(
        "local".to_string(),
        ditto_llm::gateway::http::McpServerState::new(
            "local".to_string(),
            mcp_upstream.url("/mcp"),
        )
        .expect("mcp state"),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(proxy_backends)
        .with_mcp_servers(mcp_servers);
    let app = ditto_llm::gateway::http::router(state);

    let body = json!({
        "model": "gpt-4o-mini",
        "messages": [{"role":"user","content":"hi"}],
        "tools": [{
            "type": "mcp",
            "server_url": "litellm_proxy/mcp/local",
        }]
    });

    let request1 = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("authorization", "Bearer vk-1")
        .header("x-request-id", "req-1")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response1 = app.clone().oneshot(request1).await.unwrap();
    assert_eq!(response1.status(), StatusCode::OK);
    let bytes1 = to_bytes(response1.into_body(), usize::MAX).await.unwrap();
    assert_eq!(bytes1, r#"{"id":"ok1"}"#);

    let request2 = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("authorization", "Bearer vk-1")
        .header("x-request-id", "req-2")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response2 = app.oneshot(request2).await.unwrap();
    assert_eq!(response2.status(), StatusCode::OK);
    let bytes2 = to_bytes(response2.into_body(), usize::MAX).await.unwrap();
    assert_eq!(bytes2, r#"{"id":"ok2"}"#);

    mcp_list.assert_calls(1);
    openai_req1.assert();
    openai_req2.assert();
}
