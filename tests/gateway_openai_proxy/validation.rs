#[tokio::test]
async fn openai_compat_proxy_schema_validation_rejects_invalid_completions_request()
-> ditto_llm::Result<()> {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return Ok(());
    }
    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/completions")
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
        "model": "gpt-3.5-turbo-instruct"
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/completions")
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
async fn openai_compat_proxy_schema_validation_rejects_invalid_moderations_request()
-> ditto_llm::Result<()> {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return Ok(());
    }
    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/moderations")
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
        "model": "omni-moderation-latest",
        "input": null
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/moderations")
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
async fn openai_compat_proxy_schema_validation_rejects_invalid_rerank_request()
-> ditto_llm::Result<()> {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return Ok(());
    }
    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/rerank")
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
        "query": "hello"
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/rerank")
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
async fn openai_compat_proxy_schema_validation_rejects_invalid_batches_request()
-> ditto_llm::Result<()> {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return Ok(());
    }
    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/batches")
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
        "endpoint": "/v1/responses",
        "completion_window": "24h"
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/batches")
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
async fn openai_compat_proxy_schema_validation_rejects_invalid_audio_transcriptions_request()
-> ditto_llm::Result<()> {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return Ok(());
    }
    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/audio/transcriptions")
            .header("authorization", "Bearer sk-test");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"text":"ok"}"#);
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
            default_backend: "primary".to_string(),
            default_backends: Vec::new(),
            rules: Vec::new(),
        },
    };

    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway).with_proxy_backends(proxy_backends);
    let app = ditto_llm::gateway::http::router(state);

    let boundary = "BOUNDARY";
    let content_type = format!("multipart/form-data; boundary={boundary}");
    let body = format!(
        "--{boundary}\r\n\
Content-Disposition: form-data; name=\"file\"; filename=\"audio.wav\"\r\n\
Content-Type: application/octet-stream\r\n\
\r\n\
abc\r\n\
--{boundary}--\r\n"
    );
    let request = Request::builder()
        .method("POST")
        .uri("/v1/audio/transcriptions")
        .header("authorization", "Bearer vk-1")
        .header("content-type", content_type)
        .body(Body::from(body))
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
async fn openai_compat_proxy_schema_validation_rejects_invalid_files_upload_request()
-> ditto_llm::Result<()> {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return Ok(());
    }
    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/files")
            .header("authorization", "Bearer sk-test");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"id":"file-ok"}"#);
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
            default_backend: "primary".to_string(),
            default_backends: Vec::new(),
            rules: Vec::new(),
        },
    };

    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway).with_proxy_backends(proxy_backends);
    let app = ditto_llm::gateway::http::router(state);

    let boundary = "BOUNDARY";
    let content_type = format!("multipart/form-data; boundary={boundary}");
    let body = format!(
        "--{boundary}\r\n\
Content-Disposition: form-data; name=\"file\"; filename=\"example.txt\"\r\n\
Content-Type: text/plain\r\n\
\r\n\
hello\r\n\
--{boundary}--\r\n"
    );
    let request = Request::builder()
        .method("POST")
        .uri("/v1/files")
        .header("authorization", "Bearer vk-1")
        .header("content-type", content_type)
        .body(Body::from(body))
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
async fn openai_compat_proxy_applies_route_guardrails_override() {
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
            default_backend: "primary".to_string(),
            default_backends: Vec::new(),
            rules: vec![RouteRule {
                model_prefix: "gpt-".to_string(),
                backend: "primary".to_string(),
                backends: Vec::new(),
                guardrails: Some(GuardrailsConfig::default()),
            }],
        },
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
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    mock.assert_calls(1);

    let body = json!({"model":"o1","input":"hi forbidden"});
    let request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("authorization", "Bearer vk-1")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    mock.assert_calls(1);
}

#[tokio::test]
async fn openai_compat_proxy_rejects_not_allowed_model() {
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
    key.guardrails.allow_models = vec!["gpt-*".to_string()];

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

    let body = json!({"model":"o1","input":"hi"});
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
    mock.assert_calls(0);
}

#[tokio::test]
async fn openai_compat_proxy_allows_requests_without_virtual_keys() {
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
        backends: vec![backend_config(
            "primary",
            upstream.base_url(),
            "Bearer sk-test",
        )],
        virtual_keys: Vec::new(),
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

    let body = json!({"model":"gpt-4o-mini","input":"hi"});
    let request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
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
async fn openai_compat_proxy_forwards_authorization_when_virtual_keys_empty() {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return;
    }
    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/responses")
            .header("authorization", "Bearer sk-client");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"id":"ok"}"#);
    });

    let config = GatewayConfig {
        backends: vec![BackendConfig {
            name: "primary".to_string(),
            base_url: upstream.base_url(),
            max_in_flight: None,
            timeout_seconds: None,
            headers: BTreeMap::new(),
            query_params: BTreeMap::new(),
            provider: None,
            provider_config: None,
            model_map: BTreeMap::new(),
        }],
        virtual_keys: Vec::new(),
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

    let body = json!({"model":"gpt-4o-mini","input":"hi"});
    let request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("authorization", "Bearer sk-client")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(bytes, r#"{"id":"ok"}"#);
    mock.assert();
}

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
        .with_proxy_cache(ditto_llm::gateway::ProxyCacheConfig {
            ttl_seconds: 60,
            max_entries: 16,
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
        .with_admin_token("admin-token")
        .with_proxy_cache(ditto_llm::gateway::ProxyCacheConfig {
            ttl_seconds: 60,
            max_entries: 16,
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
        backends: vec![backend_config(
            "primary",
            upstream.base_url(),
            "Bearer sk-test",
        )],
        virtual_keys: Vec::new(),
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
        .with_proxy_cache(ditto_llm::gateway::ProxyCacheConfig {
            ttl_seconds: 60,
            max_entries: 16,
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
