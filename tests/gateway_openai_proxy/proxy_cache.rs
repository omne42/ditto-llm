#[cfg(feature = "gateway-proxy-cache")]
#[tokio::test]
async fn openai_compat_proxy_caches_non_streaming_json_responses() {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
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
        .with_proxy_cache(ditto_llm::gateway::ProxyCacheConfig {
            ttl_seconds: 60,
            max_entries: 16,
            ..Default::default()
        });
    let app = ditto_llm::gateway::http::router(state);

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
async fn openai_compat_proxy_admin_can_purge_proxy_cache_key() {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
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
        .with_proxy_cache(ditto_llm::gateway::ProxyCacheConfig {
            ttl_seconds: 60,
            max_entries: 16,
            ..Default::default()
        });
    let app = ditto_llm::gateway::http::router(state);

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
async fn openai_compat_proxy_cache_scopes_by_x_api_key_when_no_virtual_keys() {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
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
        virtual_keys: Vec::new(),
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
        .with_proxy_cache(ditto_llm::gateway::ProxyCacheConfig {
            ttl_seconds: 60,
            max_entries: 16,
            ..Default::default()
        });
    let app = ditto_llm::gateway::http::router(state);

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
