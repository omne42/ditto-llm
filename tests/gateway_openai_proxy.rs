#![cfg(feature = "gateway")]

use std::collections::{BTreeMap, HashMap};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use ditto_llm::gateway::{
    BackendConfig, Gateway, GatewayConfig, GatewayHttpState, ProxyBackend, RouteRule, RouterConfig,
    VirtualKeyConfig,
};
use httpmock::Method::POST;
use httpmock::MockServer;
use serde_json::json;
use tower::util::ServiceExt;

fn build_proxy_backends(
    config: &GatewayConfig,
) -> Result<HashMap<String, ProxyBackend>, ditto_llm::gateway::GatewayError> {
    let mut out = HashMap::new();
    for backend in &config.backends {
        let mut client = ProxyBackend::new(&backend.base_url)?;
        client = client.with_headers(backend.headers.clone())?;
        out.insert(backend.name.clone(), client);
    }
    Ok(out)
}

fn backend_config(name: &str, base_url: String, auth: &str) -> BackendConfig {
    let mut headers = BTreeMap::new();
    headers.insert("authorization".to_string(), auth.to_string());
    BackendConfig {
        name: name.to_string(),
        base_url,
        headers,
        provider: None,
        provider_config: None,
        model_map: BTreeMap::new(),
    }
}

#[tokio::test]
async fn openai_compat_proxy_forwards_chat_completions_and_injects_upstream_auth() {
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
async fn openai_compat_proxy_enforces_max_in_flight() {
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
async fn openai_compat_proxy_streams_text_event_stream() {
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

#[tokio::test]
async fn openai_compat_proxy_routes_by_model_prefix() {
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
            default_backend: "secondary".to_string(),
            default_backends: Vec::new(),
            rules: vec![RouteRule {
                model_prefix: "gpt-".to_string(),
                backend: "primary".to_string(),
                backends: Vec::new(),
            }],
        },
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
    let primary = MockServer::start();
    let secondary = MockServer::start();

    let primary_mock = primary.mock(|when, then| {
        when.method(POST)
            .path("/v1/responses")
            .header("authorization", "Bearer sk-primary")
            .header("x-request-id", "req-0");
        then.status(500)
            .header("content-type", "application/json")
            .body(r#"{"error":{"message":"overloaded","type":"server_error"}}"#);
    });
    let secondary_mock = secondary.mock(|when, then| {
        when.method(POST)
            .path("/v1/responses")
            .header("authorization", "Bearer sk-secondary")
            .header("x-request-id", "req-0");
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
            default_backend: "primary".to_string(),
            default_backends: vec![
                ditto_llm::gateway::RouteBackend {
                    backend: "primary".to_string(),
                    weight: 1,
                },
                ditto_llm::gateway::RouteBackend {
                    backend: "secondary".to_string(),
                    weight: 1,
                },
            ],
            rules: Vec::new(),
        },
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

    primary_mock.assert_calls(1);
    secondary_mock.assert_calls(1);
}

#[cfg(feature = "gateway-routing-advanced")]
#[tokio::test]
async fn openai_compat_proxy_circuit_breaker_skips_unhealthy_backends() {
    let primary = MockServer::start();
    let secondary = MockServer::start();

    let primary_mock = primary.mock(|when, then| {
        when.method(POST)
            .path("/v1/responses")
            .header("authorization", "Bearer sk-primary")
            .header("x-request-id", "req-0");
        then.status(500)
            .header("content-type", "application/json")
            .body(r#"{"error":{"message":"overloaded","type":"server_error"}}"#);
    });
    let secondary_mock = secondary.mock(|when, then| {
        when.method(POST)
            .path("/v1/responses")
            .header("authorization", "Bearer sk-secondary")
            .header("x-request-id", "req-0");
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
            default_backend: "primary".to_string(),
            default_backends: vec![
                ditto_llm::gateway::RouteBackend {
                    backend: "primary".to_string(),
                    weight: 1,
                },
                ditto_llm::gateway::RouteBackend {
                    backend: "secondary".to_string(),
                    weight: 1,
                },
            ],
            rules: Vec::new(),
        },
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
            .header("x-request-id", "req-0")
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

#[tokio::test]
async fn openai_compat_proxy_rejects_missing_virtual_key() {
    let upstream = MockServer::start();
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
async fn openai_compat_proxy_rejects_not_allowed_model() {
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
            headers: BTreeMap::new(),
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
