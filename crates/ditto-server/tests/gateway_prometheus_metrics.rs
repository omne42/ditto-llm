#![cfg(all(feature = "gateway", feature = "gateway-metrics-prometheus"))]

use std::collections::{BTreeMap, HashMap};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use ditto_server::gateway::{
    BackendConfig, Gateway, GatewayConfig, GatewayHttpState, GatewayRedactionConfig, ProxyBackend,
    RouteBackend, RouterConfig, VirtualKeyConfig,
};
use httpmock::Method::POST;
use httpmock::MockServer;
use serde_json::json;
use tower::util::ServiceExt;

fn backend_config(name: &str, base_url: String, auth: &str) -> BackendConfig {
    let mut headers = BTreeMap::new();
    headers.insert("authorization".to_string(), auth.to_string());
    BackendConfig {
        name: name.to_string(),
        base_url,
        max_in_flight: None,
        timeout_seconds: None,
        headers,
        query_params: BTreeMap::new(),
        provider: None,
        provider_config: None,
        model_map: BTreeMap::new(),
    }
}

fn build_proxy_backends(
    config: &GatewayConfig,
) -> Result<HashMap<String, ProxyBackend>, ditto_server::gateway::GatewayError> {
    let mut out = HashMap::new();
    for backend in &config.backends {
        let mut client = ProxyBackend::new(&backend.base_url)?;
        client = client.with_headers(backend.headers.clone())?;
        client = client.with_query_params(backend.query_params.clone());
        client = client.with_request_timeout_seconds(backend.timeout_seconds);
        out.insert(backend.name.clone(), client);
    }
    Ok(out)
}

#[tokio::test]
async fn prometheus_metrics_endpoint_tracks_proxy_counters() -> ditto_core::error::Result<()> {
    if ditto_core::utils::test_support::should_skip_httpmock() {
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

    let config = GatewayConfig {
        backends: vec![backend_config(
            "primary",
            upstream.base_url(),
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
        i18n: Default::default(),
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(proxy_backends)
        .with_prometheus_metrics(Default::default());
    let app = ditto_server::gateway::http::router(state);

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

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    mock.assert();

    let metrics_request = Request::builder()
        .method("GET")
        .uri("/metrics/prometheus")
        .body(Body::empty())
        .unwrap();
    let metrics_response = app.oneshot(metrics_request).await.unwrap();
    assert_eq!(metrics_response.status(), StatusCode::OK);
    assert_eq!(
        metrics_response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default(),
        "text/plain; version=0.0.4"
    );

    let metrics_body = to_bytes(metrics_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let rendered = String::from_utf8_lossy(&metrics_body);
    assert!(rendered.contains("ditto_gateway_proxy_requests_total 1\n"));
    assert!(
        rendered
            .contains("ditto_gateway_proxy_requests_by_key_total{virtual_key_id=\"key-1\"} 1\n")
    );
    assert!(
        rendered.contains("ditto_gateway_proxy_requests_by_model_total{model=\"gpt-4o-mini\"} 1\n")
    );
    assert!(
        rendered.contains(
            "ditto_gateway_proxy_requests_by_path_total{path=\"/v1/chat/completions\"} 1\n"
        )
    );
    assert!(
        rendered.contains("ditto_gateway_proxy_backend_attempts_total{backend=\"primary\"} 1\n")
    );
    assert!(
        rendered.contains("ditto_gateway_proxy_backend_success_total{backend=\"primary\"} 1\n")
    );
    assert!(rendered.contains("ditto_gateway_proxy_responses_total{status=\"200\"} 1\n"));
    assert!(rendered.contains(
        "ditto_gateway_proxy_responses_by_path_status_total{path=\"/v1/chat/completions\",status=\"200\"} 1\n"
    ));
    assert!(rendered.contains("ditto_gateway_proxy_backend_in_flight{backend=\"primary\"} 0\n"));
    assert!(rendered.contains(
        "ditto_gateway_proxy_backend_request_duration_seconds_bucket{backend=\"primary\",le=\"+Inf\"} 1\n"
    ));
    assert!(rendered.contains(
        "ditto_gateway_proxy_backend_request_duration_seconds_count{backend=\"primary\"} 1\n"
    ));
    assert!(rendered.contains(
        "ditto_gateway_proxy_request_duration_seconds_bucket{path=\"/v1/chat/completions\",le=\"+Inf\"} 1\n"
    ));
    assert!(rendered.contains(
        "ditto_gateway_proxy_request_duration_seconds_count{path=\"/v1/chat/completions\"} 1\n"
    ));

    Ok(())
}

#[cfg(feature = "gateway-proxy-cache")]
#[tokio::test]
async fn prometheus_metrics_endpoint_tracks_proxy_cache_counters() -> ditto_core::error::Result<()>
{
    if ditto_core::utils::test_support::should_skip_httpmock() {
        return Ok(());
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
            default_backends: vec![RouteBackend {
                backend: "primary".to_string(),
                weight: 1.0,
            }],
            rules: Vec::new(),
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
        observability: Default::default(),
        i18n: Default::default(),
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(proxy_backends)
        .with_proxy_cache(Default::default())
        .with_prometheus_metrics(Default::default());
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
    mock.assert_calls(1);

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
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default(),
        "hit"
    );
    assert_eq!(
        response
            .headers()
            .get("x-ditto-cache-source")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default(),
        "memory"
    );
    mock.assert_calls(1);

    let metrics_request = Request::builder()
        .method("GET")
        .uri("/metrics/prometheus")
        .body(Body::empty())
        .unwrap();
    let metrics_response = app.oneshot(metrics_request).await.unwrap();
    assert_eq!(metrics_response.status(), StatusCode::OK);

    let metrics_body = to_bytes(metrics_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let rendered = String::from_utf8_lossy(&metrics_body);
    assert!(rendered.contains("ditto_gateway_proxy_cache_lookups_total 2\n"));
    assert!(
        rendered.contains(
            "ditto_gateway_proxy_cache_lookups_by_path_total{path=\"/v1/responses\"} 2\n"
        )
    );
    assert!(rendered.contains("ditto_gateway_proxy_cache_hits_total 1\n"));
    assert!(
        rendered.contains("ditto_gateway_proxy_cache_hits_by_source_total{source=\"memory\"} 1\n")
    );
    assert!(
        rendered
            .contains("ditto_gateway_proxy_cache_hits_by_path_total{path=\"/v1/responses\"} 1\n")
    );
    assert!(rendered.contains("ditto_gateway_proxy_cache_misses_total 1\n"));
    assert!(
        rendered
            .contains("ditto_gateway_proxy_cache_misses_by_path_total{path=\"/v1/responses\"} 1\n")
    );
    assert!(rendered.contains("ditto_gateway_proxy_cache_stores_total{target=\"memory\"} 1\n"));
    assert!(rendered.contains(
        "ditto_gateway_proxy_responses_by_path_status_total{path=\"/v1/responses\",status=\"200\"} 2\n"
    ));

    Ok(())
}

#[tokio::test]
async fn prometheus_metrics_endpoint_redacts_labels_with_observability_policy()
-> ditto_core::error::Result<()> {
    if ditto_core::utils::test_support::should_skip_httpmock() {
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

    let mut redaction = GatewayRedactionConfig::default();
    redaction
        .redact_key_names
        .push("virtual_key_id".to_string());
    redaction.redact_key_names.push("backend".to_string());
    let config = GatewayConfig {
        backends: vec![backend_config(
            "sk-backend-1234567890",
            upstream.base_url(),
            "Bearer sk-test",
        )],
        virtual_keys: vec![VirtualKeyConfig::new("vk-secret-id", "vk-1")],
        router: RouterConfig {
            default_backends: vec![RouteBackend {
                backend: "sk-backend-1234567890".to_string(),
                weight: 1.0,
            }],
            rules: Vec::new(),
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
        observability: ditto_server::gateway::GatewayObservabilityConfig {
            redaction,
            sampling: Default::default(),
        },
        i18n: Default::default(),
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(proxy_backends)
        .with_prometheus_metrics(Default::default());
    let app = ditto_server::gateway::http::router(state);

    let body = json!({
        "model": "sk-1234567890",
        "messages": [{"role":"user","content":"hi"}]
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("authorization", "Bearer vk-1")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    mock.assert();

    let metrics_request = Request::builder()
        .method("GET")
        .uri("/metrics/prometheus")
        .body(Body::empty())
        .unwrap();
    let metrics_response = app.oneshot(metrics_request).await.unwrap();
    assert_eq!(metrics_response.status(), StatusCode::OK);

    let metrics_body = to_bytes(metrics_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let rendered = String::from_utf8_lossy(&metrics_body);
    assert!(
        rendered.contains(
            "ditto_gateway_proxy_requests_by_key_total{virtual_key_id=\"<redacted>\"} 1\n"
        )
    );
    assert!(
        rendered.contains("ditto_gateway_proxy_requests_by_model_total{model=\"<redacted>\"} 1\n")
    );
    assert!(
        rendered.contains("ditto_gateway_proxy_backend_attempts_total{backend=\"<redacted>\"} 1\n")
    );
    assert!(!rendered.contains("vk-secret-id"));
    assert!(!rendered.contains("sk-backend-1234567890"));
    assert!(!rendered.contains("model=\"sk-1234567890\""));

    Ok(())
}
