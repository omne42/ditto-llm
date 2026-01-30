#![cfg(all(feature = "gateway", feature = "gateway-costing"))]

use std::collections::{BTreeMap, HashMap};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use ditto_llm::gateway::{
    BackendConfig, Gateway, GatewayConfig, GatewayHttpState, PricingTable, ProxyBackend,
    RouterConfig, VirtualKeyConfig,
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
        headers,
        provider: None,
        provider_config: None,
        model_map: BTreeMap::new(),
    }
}

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

#[tokio::test]
async fn cost_budget_blocks_proxy_request() -> ditto_llm::Result<()> {
    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST).path("/v1/chat/completions");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"id":"ok"}"#);
    });

    let pricing = PricingTable::from_litellm_json_str(
        r#"{
          "gpt-4o-mini": {"input_cost_per_token": 1.0, "output_cost_per_token": 1.0}
        }"#,
    )
    .expect("pricing");

    let mut key = VirtualKeyConfig::new("key-1", "vk-1");
    key.budget.total_usd_micros = Some(500_000);

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
    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(proxy_backends)
        .with_pricing_table(pricing);
    let app = ditto_llm::gateway::http::router(state);

    let body = json!({
        "model": "gpt-4o-mini",
        "max_tokens": 1,
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
    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    assert_eq!(
        value["error"]["code"].as_str().unwrap_or_default(),
        "cost_budget_exceeded"
    );

    mock.assert_calls(0);
    Ok(())
}

#[tokio::test]
async fn cache_read_pricing_allows_second_request() -> ditto_llm::Result<()> {
    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST).path("/v1/chat/completions");
        then.status(200)
            .header("content-type", "application/json")
            .body(
                json!({
                    "id": "ok",
                    "usage": {
                        "prompt_tokens": 1000,
                        "completion_tokens": 0,
                        "total_tokens": 1000,
                        "prompt_tokens_details": { "cached_tokens": 900 }
                    }
                })
                .to_string(),
            );
    });

    let pricing = PricingTable::from_litellm_json_str(
        r#"{
          "claude-3-5-haiku-20241022": {
            "input_cost_per_token": 0.0001,
            "output_cost_per_token": 0.0,
            "cache_read_input_token_cost": 0.000001
          }
        }"#,
    )
    .expect("pricing");

    let mut key = VirtualKeyConfig::new("key-1", "vk-1");
    key.budget.total_usd_micros = Some(50_000);

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
    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(proxy_backends)
        .with_pricing_table(pricing);
    let app = ditto_llm::gateway::http::router(state);

    let body = json!({
        "model": "claude-3-5-haiku-20241022",
        "max_tokens": 1,
        "messages": [{"role":"user","content":"hi"}]
    });

    let request_1 = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("authorization", "Bearer vk-1")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response_1 = app.clone().oneshot(request_1).await.unwrap();
    assert_eq!(response_1.status(), StatusCode::OK);

    let request_2 = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("authorization", "Bearer vk-1")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response_2 = app.oneshot(request_2).await.unwrap();
    assert_eq!(response_2.status(), StatusCode::OK);

    mock.assert_calls(2);
    Ok(())
}
