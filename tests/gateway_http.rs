#![cfg(feature = "gateway")]

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use ditto_llm::gateway::observability::ObservabilitySnapshot;
use ditto_llm::gateway::{
    Backend, BudgetConfig, CacheConfig, Gateway, GatewayConfig, GatewayError, GatewayHttpState,
    GatewayRequest, GatewayResponse, GuardrailsConfig, LimitsConfig, PassthroughConfig,
    RouterConfig, VirtualKeyConfig,
};
use serde_json::json;
use tower::util::ServiceExt;

struct EchoBackend;

#[async_trait]
impl Backend for EchoBackend {
    async fn call(&self, request: &GatewayRequest) -> Result<GatewayResponse, GatewayError> {
        Ok(GatewayResponse {
            content: format!("echo: {}", request.prompt),
            output_tokens: 1,
            backend: String::new(),
            cached: false,
        })
    }
}

fn base_key() -> VirtualKeyConfig {
    VirtualKeyConfig {
        id: "key-1".to_string(),
        token: "vk-1".to_string(),
        enabled: true,
        project_id: None,
        user_id: None,
        limits: LimitsConfig::default(),
        budget: BudgetConfig::default(),
        cache: CacheConfig::default(),
        guardrails: GuardrailsConfig::default(),
        passthrough: PassthroughConfig::default(),
        route: None,
    }
}

fn base_config() -> GatewayConfig {
    GatewayConfig {
        backends: Vec::new(),
        virtual_keys: vec![base_key()],
        router: RouterConfig {
            default_backend: "primary".to_string(),
            default_backends: Vec::new(),
            rules: Vec::new(),
        },
    }
}

#[tokio::test]
async fn gateway_http_routes_and_metrics() -> ditto_llm::Result<()> {
    let mut gateway = Gateway::new(base_config());
    gateway.register_backend("primary", EchoBackend);

    let state = GatewayHttpState::new(gateway);
    let app = ditto_llm::gateway::http::router(state);

    let payload = json!({
        "model": "gpt-4o-mini",
        "prompt": "hi",
        "input_tokens": 1,
        "max_output_tokens": 2
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/gateway")
        .header("authorization", "Bearer vk-1")
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: GatewayResponse = serde_json::from_slice(&body)?;
    assert_eq!(parsed.content, "echo: hi");

    let metrics_request = Request::builder()
        .method("GET")
        .uri("/metrics")
        .body(Body::empty())
        .unwrap();
    let metrics_response = app.clone().oneshot(metrics_request).await.unwrap();
    assert_eq!(metrics_response.status(), StatusCode::OK);
    let metrics_body = to_bytes(metrics_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let metrics: ObservabilitySnapshot = serde_json::from_slice(&metrics_body)?;
    assert!(metrics.requests >= 1);

    Ok(())
}

#[tokio::test]
async fn gateway_http_admin_requires_token_and_supports_crud() -> ditto_llm::Result<()> {
    let mut gateway = Gateway::new(base_config());
    gateway.register_backend("primary", EchoBackend);
    let state = GatewayHttpState::new(gateway).with_admin_token("admin-token");
    let app = ditto_llm::gateway::http::router(state);

    let unauthorized = Request::builder()
        .method("GET")
        .uri("/admin/keys")
        .body(Body::empty())
        .unwrap();
    let unauthorized_response = app.clone().oneshot(unauthorized).await.unwrap();
    assert_eq!(unauthorized_response.status(), StatusCode::UNAUTHORIZED);

    let new_key = VirtualKeyConfig::new("key-2", "vk-2");
    let upsert = Request::builder()
        .method("POST")
        .uri("/admin/keys")
        .header("x-admin-token", "admin-token")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&new_key)?))
        .unwrap();
    let upsert_response = app.clone().oneshot(upsert).await.unwrap();
    assert_eq!(upsert_response.status(), StatusCode::CREATED);

    let list = Request::builder()
        .method("GET")
        .uri("/admin/keys")
        .header("x-admin-token", "admin-token")
        .body(Body::empty())
        .unwrap();
    let list_response = app.clone().oneshot(list).await.unwrap();
    assert_eq!(list_response.status(), StatusCode::OK);
    let list_body = to_bytes(list_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let keys: Vec<VirtualKeyConfig> = serde_json::from_slice(&list_body)?;
    let created = keys.iter().find(|key| key.id == "key-2").expect("key-2");
    assert_eq!(created.token, "redacted");

    let list_with_tokens = Request::builder()
        .method("GET")
        .uri("/admin/keys?include_tokens=true")
        .header("x-admin-token", "admin-token")
        .body(Body::empty())
        .unwrap();
    let list_with_tokens_response = app.clone().oneshot(list_with_tokens).await.unwrap();
    assert_eq!(list_with_tokens_response.status(), StatusCode::OK);
    let list_with_tokens_body = to_bytes(list_with_tokens_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let keys_with_tokens: Vec<VirtualKeyConfig> = serde_json::from_slice(&list_with_tokens_body)?;
    let created_with_tokens = keys_with_tokens
        .iter()
        .find(|key| key.id == "key-2")
        .expect("key-2");
    assert_eq!(created_with_tokens.token, "vk-2");

    let delete = Request::builder()
        .method("DELETE")
        .uri("/admin/keys/key-2")
        .header("x-admin-token", "admin-token")
        .body(Body::empty())
        .unwrap();
    let delete_response = app.oneshot(delete).await.unwrap();
    assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

    Ok(())
}

#[tokio::test]
async fn gateway_http_admin_routes_are_disabled_without_admin_token() -> ditto_llm::Result<()> {
    let mut gateway = Gateway::new(base_config());
    gateway.register_backend("primary", EchoBackend);

    let state = GatewayHttpState::new(gateway);
    let app = ditto_llm::gateway::http::router(state);

    let request = Request::builder()
        .method("GET")
        .uri("/admin/keys")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}
