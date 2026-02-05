#[tokio::test]
async fn openai_compat_proxy_routes_by_model_prefix() {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return;
    }
    let primary = MockServer::start();
    let secondary = MockServer::start();

    let primary_mock = primary.mock(|when, then| {
        when.method(POST)
            .path("/v1/responses")
            .header("authorization", "Bearer sk-primary");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"backend":"primary"}"#);
    });
    let secondary_mock = secondary.mock(|when, then| {
        when.method(POST)
            .path("/v1/responses")
            .header("authorization", "Bearer sk-secondary");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"backend":"secondary"}"#);
    });

    let config = GatewayConfig {
        backends: vec![
            backend_config("primary", primary.base_url(), "Bearer sk-primary"),
            backend_config("secondary", secondary.base_url(), "Bearer sk-secondary"),
        ],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
        router: RouterConfig {
            default_backends: vec![RouteBackend { backend: "secondary".to_string(), weight: 1.0 }],
            rules: vec![RouteRule {
                model_prefix: "gpt-".to_string(),
                exact: false,
                backend: "primary".to_string(),
                backends: Vec::new(),
                guardrails: None,
            }],
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
        observability: Default::default(),
    };

    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway).with_proxy_backends(proxy_backends);
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

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(bytes, r#"{"backend":"primary"}"#);

    primary_mock.assert();
    secondary_mock.assert_calls(0);
}

#[cfg(feature = "gateway-routing-advanced")]
#[tokio::test]
async fn openai_compat_proxy_retries_retryable_statuses_across_backends() {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return;
    }
    let primary = MockServer::start();
    let secondary = MockServer::start();

    let primary_mock = primary.mock(|when, then| {
        when.method(POST)
            .path("/v1/responses")
            .header("authorization", "Bearer sk-primary")
            .header("x-request-id", "req-primary");
        then.status(500)
            .header("content-type", "application/json")
            .body(r#"{"error":{"message":"overloaded","type":"server_error"}}"#);
    });
    let secondary_mock = secondary.mock(|when, then| {
        when.method(POST)
            .path("/v1/responses")
            .header("authorization", "Bearer sk-secondary")
            .header("x-request-id", "req-primary");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"id":"ok"}"#);
    });

    let config = GatewayConfig {
        backends: vec![
            backend_config("primary", primary.base_url(), "Bearer sk-primary"),
            backend_config("secondary", secondary.base_url(), "Bearer sk-secondary"),
        ],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
        router: RouterConfig {
            default_backends: vec![
                ditto_llm::gateway::RouteBackend {
                    backend: "primary".to_string(),
                    weight: 1.0,
                },
                ditto_llm::gateway::RouteBackend {
                    backend: "secondary".to_string(),
                    weight: 1.0,
                },
            ],
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
        .with_proxy_routing(ditto_llm::gateway::ProxyRoutingConfig {
            retry: ditto_llm::gateway::ProxyRetryConfig {
                enabled: true,
                retry_status_codes: vec![500],
                max_attempts: Some(2),
            },
            circuit_breaker: ditto_llm::gateway::ProxyCircuitBreakerConfig::default(),
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
        .header("x-request-id", "req-primary")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-ditto-backend")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default(),
        "secondary"
    );

    primary_mock.assert_calls(1);
    secondary_mock.assert_calls(1);
}

#[cfg(feature = "gateway-routing-advanced")]
#[tokio::test]
async fn openai_compat_proxy_circuit_breaker_skips_unhealthy_backends() {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return;
    }
    let primary = MockServer::start();
    let secondary = MockServer::start();

    let primary_mock = primary.mock(|when, then| {
        when.method(POST)
            .path("/v1/responses")
            .header("authorization", "Bearer sk-primary")
            .header("x-request-id", "req-primary");
        then.status(500)
            .header("content-type", "application/json")
            .body(r#"{"error":{"message":"overloaded","type":"server_error"}}"#);
    });
    let secondary_mock = secondary.mock(|when, then| {
        when.method(POST)
            .path("/v1/responses")
            .header("authorization", "Bearer sk-secondary")
            .header("x-request-id", "req-primary");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"id":"ok"}"#);
    });

    let config = GatewayConfig {
        backends: vec![
            backend_config("primary", primary.base_url(), "Bearer sk-primary"),
            backend_config("secondary", secondary.base_url(), "Bearer sk-secondary"),
        ],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
        router: RouterConfig {
            default_backends: vec![
                ditto_llm::gateway::RouteBackend {
                    backend: "primary".to_string(),
                    weight: 1.0,
                },
                ditto_llm::gateway::RouteBackend {
                    backend: "secondary".to_string(),
                    weight: 1.0,
                },
            ],
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
        .with_proxy_routing(ditto_llm::gateway::ProxyRoutingConfig {
            retry: ditto_llm::gateway::ProxyRetryConfig {
                enabled: true,
                retry_status_codes: vec![500],
                max_attempts: Some(2),
            },
            circuit_breaker: ditto_llm::gateway::ProxyCircuitBreakerConfig {
                enabled: true,
                failure_threshold: 1,
                cooldown_seconds: 300,
            },
            ..Default::default()
        });
    let app = ditto_llm::gateway::http::router(state);

    let body = json!({
        "model": "gpt-4o-mini",
        "input": "hi"
    });

    for _ in 0..2 {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/responses")
            .header("authorization", "Bearer vk-1")
            .header("x-request-id", "req-primary")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();

        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("x-ditto-backend")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default(),
            "secondary"
        );
    }

    primary_mock.assert_calls(1);
    secondary_mock.assert_calls(2);
}

#[cfg(feature = "gateway-routing-advanced")]
#[tokio::test]
async fn openai_compat_proxy_health_check_skips_unhealthy_backends() {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return;
    }
    let primary = MockServer::start();
    let secondary = MockServer::start();

    let primary_health = primary.mock(|when, then| {
        when.method(httpmock::Method::GET)
            .path("/v1/models")
            .header("authorization", "Bearer sk-primary");
        then.status(500)
            .header("content-type", "application/json")
            .body(r#"{"error":{"message":"unhealthy","type":"server_error"}}"#);
    });
    let secondary_health = secondary.mock(|when, then| {
        when.method(httpmock::Method::GET)
            .path("/v1/models")
            .header("authorization", "Bearer sk-secondary");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"data":[]}"#);
    });

    let primary_mock = primary.mock(|when, then| {
        when.method(POST)
            .path("/v1/responses")
            .header("authorization", "Bearer sk-primary")
            .header("x-request-id", "req-0");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"backend":"primary"}"#);
    });
    let secondary_mock = secondary.mock(|when, then| {
        when.method(POST)
            .path("/v1/responses")
            .header("authorization", "Bearer sk-secondary")
            .header("x-request-id", "req-0");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"backend":"secondary"}"#);
    });

    let config = GatewayConfig {
        backends: vec![
            backend_config("primary", primary.base_url(), "Bearer sk-primary"),
            backend_config("secondary", secondary.base_url(), "Bearer sk-secondary"),
        ],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
        router: RouterConfig {
            default_backends: vec![
                ditto_llm::gateway::RouteBackend {
                    backend: "primary".to_string(),
                    weight: 1.0,
                },
                ditto_llm::gateway::RouteBackend {
                    backend: "secondary".to_string(),
                    weight: 1.0,
                },
            ],
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
        .with_proxy_routing(ditto_llm::gateway::ProxyRoutingConfig {
            health_check: ditto_llm::gateway::proxy_routing::ProxyHealthCheckConfig {
                enabled: true,
                path: "/v1/models".to_string(),
                interval_seconds: 60,
                timeout_seconds: 1,
            },
            ..Default::default()
        });
    let app = ditto_llm::gateway::http::router(state);

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let body = json!({
        "model": "gpt-4o-mini",
        "input": "hi"
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("authorization", "Bearer vk-1")
        .header("x-request-id", "req-0")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-ditto-backend")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default(),
        "secondary"
    );

    primary_mock.assert_calls(0);
    secondary_mock.assert_calls(1);
    primary_health.assert_calls(1);
    secondary_health.assert_calls(1);
}

#[tokio::test]
async fn openai_compat_proxy_rejects_missing_virtual_key() {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return;
    }
    let upstream = MockServer::start();
    let config = GatewayConfig {
        backends: vec![backend_config(
            "primary",
            upstream.base_url(),
            "Bearer sk-test",
        )],
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
    let state = GatewayHttpState::new(gateway).with_proxy_backends(proxy_backends);
    let app = ditto_llm::gateway::http::router(state);

    let body = json!({"model":"gpt-4o-mini","input":"hi"});
    let request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    assert!(parsed.get("error").is_some());
}

#[tokio::test]
async fn openai_compat_proxy_rejects_denied_model() {
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

    let mut key = VirtualKeyConfig::new("key-1", "vk-1");
    key.guardrails.deny_models = vec!["gpt-4o-mini".to_string()];

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
        observability: Default::default(),
    };

    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway).with_proxy_backends(proxy_backends);
    let app = ditto_llm::gateway::http::router(state);

    let body = json!({"model":"gpt-4o-mini","input":"hi"});
    let request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("authorization", "Bearer vk-1")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(
        parsed
            .get("error")
            .and_then(|v| v.get("type"))
            .and_then(|v| v.as_str()),
        Some("policy_error")
    );
    assert_eq!(
        parsed
            .get("error")
            .and_then(|v| v.get("code"))
            .and_then(|v| v.as_str()),
        Some("guardrail_rejected")
    );
    mock.assert_calls(0);
}

#[tokio::test]
async fn openai_compat_proxy_rejects_banned_regex() {
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

    let mut key = VirtualKeyConfig::new("key-1", "vk-1");
    key.guardrails.banned_regexes = vec!["forbidden".to_string()];

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
        observability: Default::default(),
    };

    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway).with_proxy_backends(proxy_backends);
    let app = ditto_llm::gateway::http::router(state);

    let body = json!({"model":"gpt-4o-mini","input":"hi forbidden"});
    let request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("authorization", "Bearer vk-1")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(
        parsed
            .get("error")
            .and_then(|v| v.get("code"))
            .and_then(|v| v.as_str()),
        Some("guardrail_rejected")
    );
    assert!(
        parsed
            .get("error")
            .and_then(|v| v.get("message"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .contains("banned_regex")
    );

    mock.assert_calls(0);
}

#[tokio::test]
async fn openai_compat_proxy_schema_validation_rejects_invalid_chat_completions_request()
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
        observability: Default::default(),
    };

    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway).with_proxy_backends(proxy_backends);
    let app = ditto_llm::gateway::http::router(state);

    let body = json!({
        "messages": [{
            "role": "user",
            "content": "hi"
        }]
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
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

#[tokio::test]
async fn openai_compat_proxy_schema_validation_rejects_invalid_images_generations_request()
-> ditto_llm::Result<()> {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return Ok(());
    }
    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/images/generations")
            .header("authorization", "Bearer sk-test");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"data":[]}"#);
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
        observability: Default::default(),
    };

    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway).with_proxy_backends(proxy_backends);
    let app = ditto_llm::gateway::http::router(state);

    let body = json!({
        "model": "gpt-image-1"
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/images/generations")
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

#[tokio::test]
async fn openai_compat_proxy_schema_validation_rejects_invalid_audio_speech_request()
-> ditto_llm::Result<()> {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return Ok(());
    }
    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/audio/speech")
            .header("authorization", "Bearer sk-test");
        then.status(200)
            .header("content-type", "audio/mpeg")
            .body("audio");
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
        observability: Default::default(),
    };

    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway).with_proxy_backends(proxy_backends);
    let app = ditto_llm::gateway::http::router(state);

    let body = json!({
        "model": "tts-1",
        "input": "hello"
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/audio/speech")
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
