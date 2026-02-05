#[tokio::test]
async fn openai_compat_proxy_auto_executes_mcp_tool_calls_for_chat_completions_with_max_steps() {
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
    let mcp_call_world = mcp_upstream.mock(|when, then| {
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
    let mcp_call_mars = mcp_upstream.mock(|when, then| {
        when.method(POST).path("/mcp").json_body(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "hello",
                "arguments": { "who": "mars" }
            }
        }));
        then.status(200)
            .header("content-type", "application/json")
            .body(
                json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": {
                        "content": [{ "type": "text", "text": "hi mars" }]
                    }
                })
                .to_string(),
            );
    });

    let openai_upstream = MockServer::start();
    let openai_step0 = openai_upstream.mock(|when, then| {
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
                "id": "step0",
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

    let openai_step1 = openai_upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/chat/completions")
            .header("authorization", "Bearer sk-test")
            .header("x-request-id", "req-1-mcp1")
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
        then.status(200).header("content-type", "application/json").body(
            json!({
                "id": "step1",
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": "",
                        "tool_calls": [{
                            "id": "call_1",
                            "type": "function",
                            "function": {
                                "name": "hello",
                                "arguments": "{\"who\":\"mars\"}"
                            }
                        }]
                    }
                }]
            })
            .to_string(),
        );
    });

    let openai_final = openai_upstream.mock(|when, then| {
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
                    },
                    {
                        "role": "assistant",
                        "content": "",
                        "tool_calls": [{
                            "id": "call_1",
                            "type": "function",
                            "function": {
                                "name": "hello",
                                "arguments": "{\"who\":\"mars\"}"
                            }
                        }]
                    },
                    {
                        "role": "tool",
                        "tool_call_id": "call_1",
                        "content": "hi mars"
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
            default_backends: vec![RouteBackend {
                backend: "primary".to_string(),
                weight: 1.0,
            }],
            rules: Vec::new(),
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
        observability: Default::default(),
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
            "require_approval": "never",
            "max_steps": 2
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
    mcp_call_world.assert();
    mcp_call_mars.assert();
    openai_step0.assert();
    openai_step1.assert();
    openai_final.assert();
}

#[tokio::test]
async fn openai_compat_proxy_auto_executes_mcp_tool_calls_for_responses_with_max_steps() {
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
    let mcp_call_world = mcp_upstream.mock(|when, then| {
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
    let mcp_call_mars = mcp_upstream.mock(|when, then| {
        when.method(POST).path("/mcp").json_body(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "hello",
                "arguments": { "who": "mars" }
            }
        }));
        then.status(200)
            .header("content-type", "application/json")
            .body(
                json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": {
                        "content": [{ "type": "text", "text": "hi mars" }]
                    }
                })
                .to_string(),
            );
    });

    let openai_upstream = MockServer::start();
    let openai_step0 = openai_upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/responses")
            .header("authorization", "Bearer sk-test")
            .header("x-request-id", "req-1-mcp0")
            .json_body(json!({
                "model": "gpt-4o-mini",
                "input": "hi",
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
                "id": "resp-1",
                "output": [{
                    "type": "function_call",
                    "call_id": "call_0",
                    "name": "hello",
                    "arguments": "{\"who\":\"world\"}"
                }]
            })
            .to_string(),
        );
    });

    let openai_step1 = openai_upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/responses")
            .header("authorization", "Bearer sk-test")
            .header("x-request-id", "req-1-mcp1")
            .json_body(json!({
                "model": "gpt-4o-mini",
                "previous_response_id": "resp-1",
                "input": [{
                    "type": "function_call_output",
                    "call_id": "call_0",
                    "output": "hi world"
                }],
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
                "id": "resp-2",
                "output": [{
                    "type": "function_call",
                    "call_id": "call_1",
                    "name": "hello",
                    "arguments": "{\"who\":\"mars\"}"
                }]
            })
            .to_string(),
        );
    });

    let openai_final = openai_upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/responses")
            .header("authorization", "Bearer sk-test")
            .header("x-request-id", "req-1")
            .json_body(json!({
                "model": "gpt-4o-mini",
                "previous_response_id": "resp-2",
                "input": [{
                    "type": "function_call_output",
                    "call_id": "call_1",
                    "output": "hi mars"
                }],
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
            default_backends: vec![RouteBackend {
                backend: "primary".to_string(),
                weight: 1.0,
            }],
            rules: Vec::new(),
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
        observability: Default::default(),
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
        "input": "hi",
        "tools": [{
            "type": "mcp",
            "server_url": "litellm_proxy/mcp/local",
            "require_approval": "never",
            "max_steps": 2
        }]
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
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
    mcp_call_world.assert();
    mcp_call_mars.assert();
    openai_step0.assert();
    openai_step1.assert();
    openai_final.assert();
}

#[tokio::test]
async fn openai_compat_proxy_auto_executes_mcp_tool_calls_for_responses_via_shim_with_max_steps() {
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
    let mcp_call_world = mcp_upstream.mock(|when, then| {
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
    let mcp_call_mars = mcp_upstream.mock(|when, then| {
        when.method(POST).path("/mcp").json_body(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "hello",
                "arguments": { "who": "mars" }
            }
        }));
        then.status(200)
            .header("content-type", "application/json")
            .body(
                json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": {
                        "content": [{ "type": "text", "text": "hi mars" }]
                    }
                })
                .to_string(),
            );
    });

    let openai_upstream = MockServer::start();
    let openai_responses_404 = openai_upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/responses")
            .header("authorization", "Bearer sk-test")
            .header("x-request-id", "req-1-mcp0");
        then.status(404)
            .header("content-type", "application/json")
            .body(r#"{"error":{"message":"not found"}}"#);
    });

    let openai_chat_step0 = openai_upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/chat/completions")
            .header("authorization", "Bearer sk-test")
            .header("x-request-id", "req-1-mcp0");
        then.status(200).header("content-type", "application/json").body(
            json!({
                "id": "step0",
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

    let openai_chat_step1 = openai_upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/chat/completions")
            .header("authorization", "Bearer sk-test")
            .header("x-request-id", "req-1-mcp1")
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
        then.status(200).header("content-type", "application/json").body(
            json!({
                "id": "step1",
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": "",
                        "tool_calls": [{
                            "id": "call_1",
                            "type": "function",
                            "function": {
                                "name": "hello",
                                "arguments": "{\"who\":\"mars\"}"
                            }
                        }]
                    }
                }]
            })
            .to_string(),
        );
    });

    let openai_chat_final = openai_upstream.mock(|when, then| {
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
                    },
                    {
                        "role": "assistant",
                        "content": "",
                        "tool_calls": [{
                            "id": "call_1",
                            "type": "function",
                            "function": {
                                "name": "hello",
                                "arguments": "{\"who\":\"mars\"}"
                            }
                        }]
                    },
                    {
                        "role": "tool",
                        "tool_call_id": "call_1",
                        "content": "hi mars"
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
        then.status(200).header("content-type", "application/json").body(
            json!({
                "id": "final",
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": "done"
                    }
                }]
            })
            .to_string(),
        );
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
        observability: Default::default(),
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
        "input": "hi",
        "tools": [{
            "type": "mcp",
            "server_url": "litellm_proxy/mcp/local",
            "require_approval": "never",
            "max_steps": 2
        }]
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("authorization", "Bearer vk-1")
        .header("x-request-id", "req-1")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(payload.get("id").and_then(|v| v.as_str()), Some("final"));
    assert_eq!(payload.get("object").and_then(|v| v.as_str()), Some("response"));
    assert_eq!(
        payload.get("output_text").and_then(|v| v.as_str()),
        Some("done")
    );

    mcp_list.assert();
    mcp_call_world.assert();
    mcp_call_mars.assert();
    openai_responses_404.assert();
    openai_chat_step0.assert();
    openai_chat_step1.assert();
    openai_chat_final.assert();
}
