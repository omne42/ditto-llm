#[cfg(feature = "gateway-proxy-cache")]
#[tokio::test]
async fn openai_compat_proxy_caches_non_streaming_json_responses() {
    if ditto_core::utils::test_support::should_skip_httpmock() {
        return;
    }
    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/responses")
            .header("authorization", "Bearer sk-test");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"id":"ok"}"#);
    });

    let config = GatewayConfig {
        backends: vec![backend_config("primary", upstream.base_url(), "Bearer sk-test")],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
        router: RouterConfig {
            default_backends: vec![RouteBackend { backend: "primary".to_string(), weight: 1.0 }],
            rules: Vec::new(),
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
        observability: Default::default(),
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(proxy_backends)
        .with_proxy_cache(ditto_server::gateway::ProxyCacheConfig {
            ttl_seconds: 60,
            max_entries: 16,
            ..Default::default()
        });
    let app = ditto_server::gateway::http::router(state);

    let body = json!({
        "model": "gpt-4o-mini",
        "input": "hi"
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("authorization", "Bearer vk-1")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(bytes, r#"{"id":"ok"}"#);

    let request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("authorization", "Bearer vk-1")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-ditto-cache")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default(),
        "hit"
    );
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(bytes, r#"{"id":"ok"}"#);

    mock.assert_calls(1);
}

#[cfg(feature = "gateway-proxy-cache")]
#[tokio::test]
async fn openai_compat_proxy_caches_streaming_sse_when_enabled() {
    if ditto_core::utils::test_support::should_skip_httpmock() {
        return;
    }
    let upstream = MockServer::start();
    let body_text = "data: first

data: second

data: [DONE]

";
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/responses")
            .header("authorization", "Bearer sk-test");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(body_text);
    });

    let config = GatewayConfig {
        backends: vec![backend_config("primary", upstream.base_url(), "Bearer sk-test")],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
        router: RouterConfig {
            default_backends: vec![RouteBackend { backend: "primary".to_string(), weight: 1.0 }],
            rules: Vec::new(),
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
        observability: Default::default(),
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(proxy_backends)
        .with_proxy_cache(ditto_server::gateway::ProxyCacheConfig {
            ttl_seconds: 60,
            max_entries: 16,
            streaming_enabled: true,
            max_stream_body_bytes: 1024,
            ..Default::default()
        });
    let app = ditto_server::gateway::http::router(state);

    let body = json!({
        "model": "gpt-4o-mini",
        "stream": true,
        "input": "hi"
    })
    .to_string();

    for expected_cache in [None, Some("hit")] {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/responses")
            .header("authorization", "Bearer vk-1")
            .header("content-type", "application/json")
            .body(Body::from(body.clone()))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default(),
            "text/event-stream"
        );
        assert_eq!(
            response
                .headers()
                .get("x-ditto-cache")
                .and_then(|v| v.to_str().ok()),
            expected_cache
        );
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(bytes, body_text);
    }

    mock.assert_calls(1);
}

#[cfg(feature = "gateway-proxy-cache")]
#[tokio::test]
async fn openai_compat_proxy_does_not_cache_streaming_sse_by_default() {
    if ditto_core::utils::test_support::should_skip_httpmock() {
        return;
    }
    let upstream = MockServer::start();
    let body_text = "data: first

data: second

";
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/responses")
            .header("authorization", "Bearer sk-test");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(body_text);
    });

    let config = GatewayConfig {
        backends: vec![backend_config("primary", upstream.base_url(), "Bearer sk-test")],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
        router: RouterConfig {
            default_backends: vec![RouteBackend { backend: "primary".to_string(), weight: 1.0 }],
            rules: Vec::new(),
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
        observability: Default::default(),
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(proxy_backends)
        .with_proxy_cache(ditto_server::gateway::ProxyCacheConfig {
            ttl_seconds: 60,
            max_entries: 16,
            ..Default::default()
        });
    let app = ditto_server::gateway::http::router(state);

    let body = json!({
        "model": "gpt-4o-mini",
        "stream": true,
        "input": "hi"
    })
    .to_string();

    for _ in 0..2 {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/responses")
            .header("authorization", "Bearer vk-1")
            .header("content-type", "application/json")
            .body(Body::from(body.clone()))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get("x-ditto-cache").is_none());
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(bytes, body_text);
    }

    mock.assert_calls(2);
}

#[cfg(feature = "gateway-proxy-cache")]
#[tokio::test]
async fn openai_compat_proxy_admin_can_purge_proxy_cache_key() {
    if ditto_core::utils::test_support::should_skip_httpmock() {
        return;
    }

    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/responses")
            .header("authorization", "Bearer sk-test");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"id":"ok"}"#);
    });

    let config = GatewayConfig {
        backends: vec![backend_config("primary", upstream.base_url(), "Bearer sk-test")],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
        router: RouterConfig {
            default_backends: vec![RouteBackend { backend: "primary".to_string(), weight: 1.0 }],
            rules: Vec::new(),
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
        observability: Default::default(),
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(proxy_backends)
        .with_admin_token("admin-token")
        .with_proxy_cache(ditto_server::gateway::ProxyCacheConfig {
            ttl_seconds: 60,
            max_entries: 16,
            ..Default::default()
        });
    let app = ditto_server::gateway::http::router(state);

    let body = json!({
        "model": "gpt-4o-mini",
        "input": "hi"
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("authorization", "Bearer vk-1")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let cache_key = response
        .headers()
        .get("x-ditto-cache-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(!cache_key.is_empty());
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(bytes, r#"{"id":"ok"}"#);

    let request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("authorization", "Bearer vk-1")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-ditto-cache")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default(),
        "hit"
    );
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(bytes, r#"{"id":"ok"}"#);
    mock.assert_calls(1);

    let purge = Request::builder()
        .method("POST")
        .uri("/admin/proxy_cache/purge")
        .header("x-admin-token", "admin-token")
        .header("content-type", "application/json")
        .body(Body::from(json!({ "cache_key": cache_key }).to_string()))
        .unwrap();
    let purge_response = app.clone().oneshot(purge).await.unwrap();
    assert_eq!(purge_response.status(), StatusCode::OK);
    let purge_bytes = to_bytes(purge_response.into_body(), usize::MAX).await.unwrap();
    let purge_json: serde_json::Value = serde_json::from_slice(&purge_bytes).unwrap();
    assert_eq!(purge_json.get("deleted_memory").and_then(|v| v.as_u64()), Some(1));

    let request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("authorization", "Bearer vk-1")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(bytes, r#"{"id":"ok"}"#);
    mock.assert_calls(2);
}

#[cfg(feature = "gateway-proxy-cache")]
#[tokio::test]
async fn openai_compat_proxy_cache_scopes_by_virtual_key_x_api_key() {
    if ditto_core::utils::test_support::should_skip_httpmock() {
        return;
    }

    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/responses")
            .header("authorization", "Bearer sk-test")
            .is_true(|req: &httpmock::prelude::HttpMockRequest| {
                req.headers()
                    .iter()
                    .any(|(name, _)| name.as_str() == "x-api-key")
            });
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"id":"ok"}"#);
    });

    let config = GatewayConfig {
        backends: vec![backend_config("primary", upstream.base_url(), "Bearer sk-test")],
        virtual_keys: vec![
            VirtualKeyConfig::new("key-a", "sk-client-a"),
            VirtualKeyConfig::new("key-b", "sk-client-b"),
        ],
        router: RouterConfig {
            default_backends: vec![RouteBackend { backend: "primary".to_string(), weight: 1.0 }],
            rules: Vec::new(),
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
        observability: Default::default(),
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(proxy_backends)
        .with_proxy_cache(ditto_server::gateway::ProxyCacheConfig {
            ttl_seconds: 60,
            max_entries: 16,
            ..Default::default()
        });
    let app = ditto_server::gateway::http::router(state);

    let body = json!({
        "model": "gpt-4o-mini",
        "input": "hi"
    });
    let body = body.to_string();

    let request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("x-api-key", "sk-client-a")
        .header("content-type", "application/json")
        .body(Body::from(body.clone()))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().get("x-ditto-cache").is_none());
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(bytes, r#"{"id":"ok"}"#);

    let request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("x-api-key", "sk-client-a")
        .header("content-type", "application/json")
        .body(Body::from(body.clone()))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-ditto-cache")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default(),
        "hit"
    );
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(bytes, r#"{"id":"ok"}"#);

    let request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("x-api-key", "sk-client-b")
        .header("content-type", "application/json")
        .body(Body::from(body.clone()))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().get("x-ditto-cache").is_none());
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(bytes, r#"{"id":"ok"}"#);

    mock.assert_calls(2);
}

#[cfg(feature = "gateway-proxy-cache")]
#[tokio::test]
async fn openai_compat_proxy_cache_varies_by_semantic_request_headers() {
    if ditto_core::utils::test_support::should_skip_httpmock() {
        return;
    }

    let upstream = MockServer::start();
    let beta_a = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/responses")
            .header("authorization", "Bearer sk-test")
            .header("openai-beta", "responses=v1");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"id":"beta-a"}"#);
    });
    let beta_b = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/responses")
            .header("authorization", "Bearer sk-test")
            .header("openai-beta", "responses=v2");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"id":"beta-b"}"#);
    });

    let config = GatewayConfig {
        backends: vec![backend_config("primary", upstream.base_url(), "Bearer sk-test")],
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
    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(proxy_backends)
        .with_proxy_cache(ditto_server::gateway::ProxyCacheConfig {
            ttl_seconds: 60,
            max_entries: 16,
            ..Default::default()
        });
    let app = ditto_server::gateway::http::router(state);

    let body = json!({
        "model": "gpt-4o-mini",
        "input": "hi"
    });
    let body = body.to_string();

    let request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("authorization", "Bearer vk-1")
        .header("content-type", "application/json")
        .header("openai-beta", "responses=v1")
        .body(Body::from(body.clone()))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().get("x-ditto-cache").is_none());
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(bytes, r#"{"id":"beta-a"}"#);

    let request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("authorization", "Bearer vk-1")
        .header("content-type", "application/json")
        .header("openai-beta", "responses=v2")
        .body(Body::from(body.clone()))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().get("x-ditto-cache").is_none());
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(bytes, r#"{"id":"beta-b"}"#);

    let request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("authorization", "Bearer vk-1")
        .header("content-type", "application/json")
        .header("openai-beta", "responses=v1")
        .body(Body::from(body))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-ditto-cache")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default(),
        "hit"
    );
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(bytes, r#"{"id":"beta-a"}"#);

    beta_a.assert_calls(1);
    beta_b.assert_calls(1);
}


#[cfg(feature = "gateway-proxy-cache")]
#[tokio::test]
async fn openai_compat_proxy_admin_can_purge_proxy_cache_by_path_and_model() {
    if ditto_core::utils::test_support::should_skip_httpmock() {
        return;
    }

    let upstream = MockServer::start();
    let mini = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/responses")
            .header("authorization", "Bearer sk-test")
            .json_body_includes(serde_json::json!({ "model": "gpt-4o-mini" }).to_string());
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"id":"mini"}"#);
    });
    let turbo = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/responses")
            .header("authorization", "Bearer sk-test")
            .json_body_includes(serde_json::json!({ "model": "gpt-4o" }).to_string());
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"id":"turbo"}"#);
    });

    let config = GatewayConfig {
        backends: vec![backend_config("primary", upstream.base_url(), "Bearer sk-test")],
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
    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(proxy_backends)
        .with_admin_token("admin-token")
        .with_proxy_cache(ditto_server::gateway::ProxyCacheConfig {
            ttl_seconds: 60,
            max_entries: 16,
            ..Default::default()
        });
    let app = ditto_server::gateway::http::router(state);

    let mini_body = serde_json::json!({
        "model": "gpt-4o-mini",
        "input": "hi"
    })
    .to_string();
    let turbo_body = serde_json::json!({
        "model": "gpt-4o",
        "input": "hi"
    })
    .to_string();

    for body in [&mini_body, &turbo_body] {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/responses")
            .header("authorization", "Bearer vk-1")
            .header("content-type", "application/json")
            .body(Body::from(body.clone()))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    for body in [&mini_body, &turbo_body] {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/responses")
            .header("authorization", "Bearer vk-1")
            .header("content-type", "application/json")
            .body(Body::from(body.clone()))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("x-ditto-cache")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default(),
            "hit"
        );
    }
    mini.assert_calls(1);
    turbo.assert_calls(1);

    let purge = Request::builder()
        .method("POST")
        .uri("/admin/proxy_cache/purge")
        .header("x-admin-token", "admin-token")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "path": "/v1/responses",
                "model": "gpt-4o-mini"
            })
            .to_string(),
        ))
        .unwrap();
    let purge_response = app.clone().oneshot(purge).await.unwrap();
    assert_eq!(purge_response.status(), StatusCode::OK);
    let purge_bytes = to_bytes(purge_response.into_body(), usize::MAX).await.unwrap();
    let purge_json: serde_json::Value = serde_json::from_slice(&purge_bytes).unwrap();
    assert_eq!(purge_json.get("deleted_memory").and_then(|v| v.as_u64()), Some(1));

    let mini_request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("authorization", "Bearer vk-1")
        .header("content-type", "application/json")
        .body(Body::from(mini_body))
        .unwrap();
    let mini_response = app.clone().oneshot(mini_request).await.unwrap();
    assert_eq!(mini_response.status(), StatusCode::OK);
    assert!(mini_response.headers().get("x-ditto-cache").is_none());

    let turbo_request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("authorization", "Bearer vk-1")
        .header("content-type", "application/json")
        .body(Body::from(turbo_body))
        .unwrap();
    let turbo_response = app.oneshot(turbo_request).await.unwrap();
    assert_eq!(turbo_response.status(), StatusCode::OK);
    assert_eq!(
        turbo_response
            .headers()
            .get("x-ditto-cache")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default(),
        "hit"
    );

    mini.assert_calls(2);
    turbo.assert_calls(1);
}
