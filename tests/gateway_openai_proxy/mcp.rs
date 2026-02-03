#[tokio::test]
async fn openai_compat_proxy_rewrites_mcp_tools_into_openai_functions() {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return;
    }

    let mcp_upstream = MockServer::start();
    let mcp_mock = mcp_upstream.mock(|when, then| {
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
    let openai_mock = openai_upstream.mock(|when, then| {
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
            .body(r#"{"id":"ok"}"#);
    });

    let config = GatewayConfig {
        backends: vec![backend_config(
            "primary",
            openai_upstream.base_url(),
            "Bearer sk-test",
        )],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
        router: RouterConfig {
            default_backend: "primary".to_string(),
            default_backends: Vec::new(),
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
    let request = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("authorization", "Bearer vk-1")
        .header("x-request-id", "req-1")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(bytes, r#"{"id":"ok"}"#);

    mcp_mock.assert();
    openai_mock.assert();
}

#[tokio::test]
async fn openai_compat_proxy_auto_executes_mcp_tool_calls_when_require_approval_never() {
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
    let mcp_call = mcp_upstream.mock(|when, then| {
        when.method(POST).path("/mcp").json_body(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "hello",
                "arguments": { "who": "world" }
            }
        }));
        then.status(200)
            .header("content-type", "application/json")
            .body(
                json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": {
                        "content": [{ "type": "text", "text": "hi world" }]
                    }
                })
                .to_string(),
            );
    });

    let openai_upstream = MockServer::start();
    let openai_initial = openai_upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/chat/completions")
            .header("authorization", "Bearer sk-test")
            .header("x-request-id", "req-1-mcp0")
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
        then.status(200).header("content-type", "application/json").body(
            json!({
                "id": "step1",
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": "",
                        "tool_calls": [{
                            "id": "call_0",
                            "type": "function",
                            "function": {
                                "name": "hello",
                                "arguments": "{\"who\":\"world\"}"
                            }
                        }]
                    }
                }]
            })
            .to_string(),
        );
    });

    let openai_follow_up = openai_upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/chat/completions")
            .header("authorization", "Bearer sk-test")
            .header("x-request-id", "req-1")
            .json_body(json!({
                "model": "gpt-4o-mini",
                "messages": [
                    {"role":"user","content":"hi"},
                    {
                        "role": "assistant",
                        "content": "",
                        "tool_calls": [{
                            "id": "call_0",
                            "type": "function",
                            "function": {
                                "name": "hello",
                                "arguments": "{\"who\":\"world\"}"
                            }
                        }]
                    },
                    {
                        "role": "tool",
                        "tool_call_id": "call_0",
                        "content": "hi world"
                    }
                ],
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
            .body(r#"{"id":"final"}"#);
    });

    let config = GatewayConfig {
        backends: vec![backend_config(
            "primary",
            openai_upstream.base_url(),
            "Bearer sk-test",
        )],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
        router: RouterConfig {
            default_backend: "primary".to_string(),
            default_backends: Vec::new(),
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
            "require_approval": "never"
        }]
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("authorization", "Bearer vk-1")
        .header("x-request-id", "req-1")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(bytes, r#"{"id":"final"}"#);

    mcp_list.assert();
    mcp_call.assert();
    openai_initial.assert();
    openai_follow_up.assert();
}

