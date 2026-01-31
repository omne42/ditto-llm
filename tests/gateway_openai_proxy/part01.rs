#[tokio::test]
async fn openai_compat_proxy_forwards_chat_completions_and_injects_upstream_auth() {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return;
    }
    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/chat/completions")
            .header("authorization", "Bearer sk-test")
            .header("x-request-id", "req-1");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"id":"ok"}"#);
    });

    let config = GatewayConfig {
        backends: vec![backend_config(
            "primary",
            upstream.base_url(),
            "Bearer sk-test",
        )],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
        router: RouterConfig {
            default_backend: "primary".to_string(),
            default_backends: Vec::new(),
            rules: Vec::new(),
        },
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway).with_proxy_backends(proxy_backends);
    let app = ditto_llm::gateway::http::router(state);

    let body = json!({
        "model": "gpt-4o-mini",
        "messages": [{"role":"user","content":"hi"}]
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
    assert_eq!(
        response
            .headers()
            .get("x-ditto-request-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default(),
        "req-1"
    );
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(bytes, r#"{"id":"ok"}"#);
    mock.assert();
}

#[tokio::test]
async fn openai_compat_proxy_accepts_virtual_key_via_x_api_key_header() {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return;
    }
    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/chat/completions")
            .header("authorization", "Bearer sk-test")
            .is_true(|req: &httpmock::prelude::HttpMockRequest| {
                !req.headers()
                    .iter()
                    .any(|(name, _)| name.as_str() == "x-api-key")
            });
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"id":"ok"}"#);
    });

    let config = GatewayConfig {
        backends: vec![backend_config(
            "primary",
            upstream.base_url(),
            "Bearer sk-test",
        )],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
        router: RouterConfig {
            default_backend: "primary".to_string(),
            default_backends: Vec::new(),
            rules: Vec::new(),
        },
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway).with_proxy_backends(proxy_backends);
    let app = ditto_llm::gateway::http::router(state);

    let body = json!({
        "model": "gpt-4o-mini",
        "messages": [{"role":"user","content":"hi"}]
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("x-api-key", "vk-1")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(bytes, r#"{"id":"ok"}"#);
    mock.assert();
}

#[tokio::test]
async fn openai_compat_proxy_appends_backend_query_params() {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return;
    }
    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/chat/completions")
            .query_param("api-version", "2024-01-01")
            .header("authorization", "Bearer sk-test");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"id":"ok"}"#);
    });

    let mut backend = backend_config("primary", upstream.base_url(), "Bearer sk-test");
    backend
        .query_params
        .insert("api-version".to_string(), "2024-01-01".to_string());

    let config = GatewayConfig {
        backends: vec![backend],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
        router: RouterConfig {
            default_backend: "primary".to_string(),
            default_backends: Vec::new(),
            rules: Vec::new(),
        },
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway).with_proxy_backends(proxy_backends);
    let app = ditto_llm::gateway::http::router(state);

    let body = json!({
        "model": "gpt-4o-mini",
        "messages": [{"role":"user","content":"hi"}]
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
    mock.assert();
}

#[tokio::test]
async fn openai_compat_proxy_applies_backend_model_map() {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return;
    }
    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/chat/completions")
            .header("authorization", "Bearer sk-test")
            .json_body(json!({
                "model": "mapped-model",
                "messages": [{"role":"user","content":"hi"}]
            }));
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"id":"ok"}"#);
    });

    let mut backend = backend_config("primary", upstream.base_url(), "Bearer sk-test");
    backend
        .model_map
        .insert("gpt-4o-mini".to_string(), "mapped-model".to_string());

    let config = GatewayConfig {
        backends: vec![backend],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
        router: RouterConfig {
            default_backend: "primary".to_string(),
            default_backends: Vec::new(),
            rules: Vec::new(),
        },
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway).with_proxy_backends(proxy_backends);
    let app = ditto_llm::gateway::http::router(state);

    let body = json!({
        "model": "gpt-4o-mini",
        "messages": [{"role":"user","content":"hi"}]
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

#[tokio::test]
async fn openai_compat_proxy_enforces_max_in_flight() {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return;
    }
    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/chat/completions")
            .header("authorization", "Bearer sk-test");
        then.status(200)
            .delay(std::time::Duration::from_millis(200))
            .header("content-type", "application/json")
            .body(r#"{"id":"ok"}"#);
    });

    let config = GatewayConfig {
        backends: vec![backend_config(
            "primary",
            upstream.base_url(),
            "Bearer sk-test",
        )],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
        router: RouterConfig {
            default_backend: "primary".to_string(),
            default_backends: Vec::new(),
            rules: Vec::new(),
        },
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(proxy_backends)
        .with_proxy_max_in_flight(1);
    let app = ditto_llm::gateway::http::router(state);

    let body = json!({
        "model": "gpt-4o-mini",
        "messages": [{"role":"user","content":"hi"}]
    });
    let request_1 = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("authorization", "Bearer vk-1")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let request_2 = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("authorization", "Bearer vk-1")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let (response_1, response_2) =
        tokio::join!(app.clone().oneshot(request_1), app.oneshot(request_2));
    let response_1 = response_1.unwrap();
    let response_2 = response_2.unwrap();
    let statuses = [response_1.status(), response_2.status()];
    assert!(statuses.contains(&StatusCode::OK));
    assert!(statuses.contains(&StatusCode::TOO_MANY_REQUESTS));

    mock.assert_calls(1);
}

#[tokio::test]
async fn openai_compat_proxy_enforces_backend_max_in_flight() {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return;
    }
    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/chat/completions")
            .header("authorization", "Bearer sk-test");
        then.status(200)
            .delay(std::time::Duration::from_millis(200))
            .header("content-type", "application/json")
            .body(r#"{"id":"ok"}"#);
    });

    let mut backend = backend_config("primary", upstream.base_url(), "Bearer sk-test");
    backend.max_in_flight = Some(1);

    let config = GatewayConfig {
        backends: vec![backend],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
        router: RouterConfig {
            default_backend: "primary".to_string(),
            default_backends: Vec::new(),
            rules: Vec::new(),
        },
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway).with_proxy_backends(proxy_backends);
    let app = ditto_llm::gateway::http::router(state);

    let body = json!({
        "model": "gpt-4o-mini",
        "messages": [{"role":"user","content":"hi"}]
    });
    let request_1 = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("authorization", "Bearer vk-1")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let request_2 = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("authorization", "Bearer vk-1")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let (response_1, response_2) =
        tokio::join!(app.clone().oneshot(request_1), app.oneshot(request_2));
    let response_1 = response_1.unwrap();
    let response_2 = response_2.unwrap();
    let statuses = [response_1.status(), response_2.status()];
    assert!(statuses.contains(&StatusCode::OK));
    assert!(statuses.contains(&StatusCode::TOO_MANY_REQUESTS));

    mock.assert_calls(1);
}

#[tokio::test]
async fn openai_compat_proxy_respects_backend_timeout_seconds() {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return;
    }
    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/chat/completions")
            .header("authorization", "Bearer sk-test");
        then.status(200)
            .delay(std::time::Duration::from_millis(2_000))
            .header("content-type", "application/json")
            .body(r#"{"id":"ok"}"#);
    });

    let mut backend = backend_config("primary", upstream.base_url(), "Bearer sk-test");
    backend.timeout_seconds = Some(1);

    let config = GatewayConfig {
        backends: vec![backend],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
        router: RouterConfig {
            default_backend: "primary".to_string(),
            default_backends: Vec::new(),
            rules: Vec::new(),
        },
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway).with_proxy_backends(proxy_backends);
    let app = ditto_llm::gateway::http::router(state);

    let body = json!({
        "model": "gpt-4o-mini",
        "messages": [{"role":"user","content":"hi"}]
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("authorization", "Bearer vk-1")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
    assert_eq!(
        value
            .get("error")
            .and_then(|err| err.get("code"))
            .and_then(|code| code.as_str()),
        Some("backend_error")
    );

    mock.assert_calls(1);
}

#[tokio::test]
async fn openai_compat_proxy_spends_usage_tokens_when_available() {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return;
    }
    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/chat/completions")
            .header("authorization", "Bearer sk-test");
        then.status(200)
            .header("content-type", "application/json")
            .body(
                r#"{"id":"ok","choices":[{"message":{"role":"assistant","content":"hi"}}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#,
            );
    });

    let max_tokens = 1u32;
    let body = json!({
        "model": "gpt-4o-mini",
        "max_tokens": max_tokens,
        "messages": [{"role":"user","content":"hi"}]
    });
    let body_string = body.to_string();
    let input_tokens_estimate: u64 = {
        #[cfg(feature = "gateway-tokenizer")]
        {
            u64::from(
                ditto_llm::gateway::token_count::estimate_input_tokens(
                    "/v1/chat/completions",
                    "gpt-4o-mini",
                    &body,
                )
                .expect("token_count"),
            )
        }
        #[cfg(not(feature = "gateway-tokenizer"))]
        {
            body_string.len().div_ceil(4) as u64
        }
    };
    let charge_tokens = input_tokens_estimate.saturating_add(u64::from(max_tokens));
    let budget_total = charge_tokens.saturating_mul(2).saturating_sub(1);
    assert!(budget_total > charge_tokens);

    let mut key = VirtualKeyConfig::new("key-1", "vk-1");
    key.budget.total_tokens = Some(budget_total);

    let config = GatewayConfig {
        backends: vec![backend_config(
            "primary",
            upstream.base_url(),
            "Bearer sk-test",
        )],
        virtual_keys: vec![key],
        router: RouterConfig {
            default_backend: "primary".to_string(),
            default_backends: Vec::new(),
            rules: Vec::new(),
        },
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway).with_proxy_backends(proxy_backends);
    let app = ditto_llm::gateway::http::router(state);

    for idx in 0..2 {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header("authorization", "Bearer vk-1")
            .header("content-type", "application/json")
            .header("x-request-id", format!("req-{idx}"))
            .body(Body::from(body_string.clone()))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    mock.assert_calls(2);
}

#[tokio::test]
async fn openai_compat_proxy_project_budget_is_shared_across_virtual_keys() -> ditto_llm::Result<()>
{
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
            .body(
                r#"{"id":"ok","choices":[{"message":{"role":"assistant","content":"hi"}}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#,
            );
    });

    let max_tokens = 1u32;
    let body = json!({
        "model": "gpt-4o-mini",
        "max_tokens": max_tokens,
        "messages": [{"role":"user","content":"hi"}]
    });
    let body_string = body.to_string();
    let input_tokens_estimate: u64 = {
        #[cfg(feature = "gateway-tokenizer")]
        {
            u64::from(
                ditto_llm::gateway::token_count::estimate_input_tokens(
                    "/v1/chat/completions",
                    "gpt-4o-mini",
                    &body,
                )
                .expect("token_count"),
            )
        }
        #[cfg(not(feature = "gateway-tokenizer"))]
        {
            body_string.len().div_ceil(4) as u64
        }
    };
    let charge_tokens = input_tokens_estimate.saturating_add(u64::from(max_tokens));
    let budget_total = charge_tokens.saturating_add(1);

    let mut key_1 = VirtualKeyConfig::new("key-1", "vk-1");
    key_1.project_id = Some("project-1".to_string());
    key_1.project_budget = Some(BudgetConfig {
        total_tokens: Some(budget_total),
        total_usd_micros: None,
    });

    let mut key_2 = VirtualKeyConfig::new("key-2", "vk-2");
    key_2.project_id = Some("project-1".to_string());
    key_2.project_budget = Some(BudgetConfig {
        total_tokens: Some(budget_total),
        total_usd_micros: None,
    });

    let config = GatewayConfig {
        backends: vec![backend_config(
            "primary",
            upstream.base_url(),
            "Bearer sk-test",
        )],
        virtual_keys: vec![key_1, key_2],
        router: RouterConfig {
            default_backend: "primary".to_string(),
            default_backends: Vec::new(),
            rules: Vec::new(),
        },
    };

    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway).with_proxy_backends(proxy_backends);
    let app = ditto_llm::gateway::http::router(state);

    let request_1 = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("authorization", "Bearer vk-1")
        .header("content-type", "application/json")
        .header("x-request-id", "req-1")
        .body(Body::from(body_string.clone()))
        .unwrap();
    let response_1 = app.clone().oneshot(request_1).await.unwrap();
    assert_eq!(response_1.status(), StatusCode::OK);

    let request_2 = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("authorization", "Bearer vk-2")
        .header("content-type", "application/json")
        .header("x-request-id", "req-2")
        .body(Body::from(body_string))
        .unwrap();
    let response_2 = app.oneshot(request_2).await.unwrap();
    assert_eq!(response_2.status(), StatusCode::PAYMENT_REQUIRED);
    let bytes = to_bytes(response_2.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    assert_eq!(
        value["error"]["code"].as_str().unwrap_or_default(),
        "budget_exceeded"
    );

    mock.assert_calls(1);
    Ok(())
}

#[tokio::test]
async fn openai_compat_proxy_streams_text_event_stream() {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return;
    }
    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/responses")
            .header("authorization", "Bearer sk-test");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body("data: first\n\ndata: second\n\n");
    });

    let config = GatewayConfig {
        backends: vec![backend_config(
            "primary",
            upstream.base_url(),
            "Bearer sk-test",
        )],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
        router: RouterConfig {
            default_backend: "primary".to_string(),
            default_backends: Vec::new(),
            rules: Vec::new(),
        },
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway).with_proxy_backends(proxy_backends);
    let app = ditto_llm::gateway::http::router(state);

    let body = json!({
        "model": "gpt-4o-mini",
        "stream": true,
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
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default(),
        "text/event-stream"
    );
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(bytes, "data: first\n\ndata: second\n\n");
    mock.assert();
}

