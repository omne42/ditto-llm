#![cfg(feature = "gateway")]

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use ditto_llm::gateway::observability::ObservabilitySnapshot;
use ditto_llm::gateway::{
    Backend, BudgetConfig, CacheConfig, Gateway, GatewayConfig, GatewayError, GatewayHttpState,
    GatewayRequest, GatewayResponse, GuardrailsConfig, LimitsConfig, PassthroughConfig,
    RouteBackend, RouterConfig, VirtualKeyConfig,
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
        tenant_id: None,
        project_id: None,
        user_id: None,
        tenant_budget: None,
        project_budget: None,
        user_budget: None,
        tenant_limits: None,
        project_limits: None,
        user_limits: None,
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
            default_backends: vec![RouteBackend {
                backend: "primary".to_string(),
                weight: 1.0,
            }],
            rules: Vec::new(),
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
        observability: Default::default(),
    }
}

#[tokio::test]
async fn gateway_http_routes_and_metrics() -> ditto_llm::foundation::error::Result<()> {
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
async fn gateway_http_admin_requires_token_and_supports_crud()
-> ditto_llm::foundation::error::Result<()> {
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
async fn gateway_http_admin_config_versions_support_rollback()
-> ditto_llm::foundation::error::Result<()> {
    let mut gateway = Gateway::new(base_config());
    gateway.register_backend("primary", EchoBackend);
    let state = GatewayHttpState::new(gateway).with_admin_token("admin-token");
    let app = ditto_llm::gateway::http::router(state);

    let get_current = Request::builder()
        .method("GET")
        .uri("/admin/config/version")
        .header("x-admin-token", "admin-token")
        .body(Body::empty())
        .unwrap();
    let current_response = app.clone().oneshot(get_current).await.unwrap();
    assert_eq!(current_response.status(), StatusCode::OK);
    let current_body = to_bytes(current_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let current_json: serde_json::Value = serde_json::from_slice(&current_body)?;
    let bootstrap_version_id = current_json
        .get("version_id")
        .and_then(|value| value.as_str())
        .expect("bootstrap version id")
        .to_string();
    assert_eq!(
        current_json
            .get("virtual_key_count")
            .and_then(|value| value.as_u64()),
        Some(1)
    );

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

    let list_versions = Request::builder()
        .method("GET")
        .uri("/admin/config/versions")
        .header("x-admin-token", "admin-token")
        .body(Body::empty())
        .unwrap();
    let list_versions_response = app.clone().oneshot(list_versions).await.unwrap();
    assert_eq!(list_versions_response.status(), StatusCode::OK);
    let versions_body = to_bytes(list_versions_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let versions_json: serde_json::Value = serde_json::from_slice(&versions_body)?;
    let versions = versions_json.as_array().expect("versions array");
    assert!(versions.len() >= 2);
    let latest_version_id = versions[0]
        .get("version_id")
        .and_then(|value| value.as_str())
        .expect("latest version id")
        .to_string();
    assert_eq!(
        versions[0]
            .get("virtual_key_count")
            .and_then(|value| value.as_u64()),
        Some(2)
    );

    let export_current_redacted = Request::builder()
        .method("GET")
        .uri("/admin/config/export")
        .header("x-admin-token", "admin-token")
        .body(Body::empty())
        .unwrap();
    let export_current_redacted_response =
        app.clone().oneshot(export_current_redacted).await.unwrap();
    assert_eq!(export_current_redacted_response.status(), StatusCode::OK);
    let export_current_redacted_body =
        to_bytes(export_current_redacted_response.into_body(), usize::MAX)
            .await
            .unwrap();
    let export_current_redacted_json: serde_json::Value =
        serde_json::from_slice(&export_current_redacted_body)?;
    assert_eq!(
        export_current_redacted_json
            .pointer("/version_id")
            .and_then(|value| value.as_str()),
        Some(latest_version_id.as_str())
    );
    assert_eq!(
        export_current_redacted_json
            .pointer("/virtual_keys/0/token")
            .and_then(|value| value.as_str()),
        Some("redacted")
    );

    let export_bootstrap_with_tokens = Request::builder()
        .method("GET")
        .uri(format!(
            "/admin/config/export?version_id={bootstrap_version_id}&include_tokens=true"
        ))
        .header("x-admin-token", "admin-token")
        .body(Body::empty())
        .unwrap();
    let export_bootstrap_with_tokens_response = app
        .clone()
        .oneshot(export_bootstrap_with_tokens)
        .await
        .unwrap();
    assert_eq!(
        export_bootstrap_with_tokens_response.status(),
        StatusCode::OK
    );
    let export_bootstrap_with_tokens_body = to_bytes(
        export_bootstrap_with_tokens_response.into_body(),
        usize::MAX,
    )
    .await
    .unwrap();
    let export_bootstrap_with_tokens_json: serde_json::Value =
        serde_json::from_slice(&export_bootstrap_with_tokens_body)?;
    assert_eq!(
        export_bootstrap_with_tokens_json
            .pointer("/version_id")
            .and_then(|value| value.as_str()),
        Some(bootstrap_version_id.as_str())
    );
    assert_eq!(
        export_bootstrap_with_tokens_json
            .pointer("/virtual_keys/0/token")
            .and_then(|value| value.as_str()),
        Some("vk-1")
    );

    let diff_redacted = Request::builder()
        .method("GET")
        .uri(format!(
            "/admin/config/diff?from_version_id={bootstrap_version_id}&to_version_id={latest_version_id}"
        ))
        .header("x-admin-token", "admin-token")
        .body(Body::empty())
        .unwrap();
    let diff_redacted_response = app.clone().oneshot(diff_redacted).await.unwrap();
    assert_eq!(diff_redacted_response.status(), StatusCode::OK);
    let diff_redacted_body = to_bytes(diff_redacted_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let diff_redacted_json: serde_json::Value = serde_json::from_slice(&diff_redacted_body)?;
    assert_eq!(
        diff_redacted_json
            .pointer("/summary/from_virtual_key_count")
            .and_then(|value| value.as_u64()),
        Some(1)
    );
    assert_eq!(
        diff_redacted_json
            .pointer("/summary/to_virtual_key_count")
            .and_then(|value| value.as_u64()),
        Some(2)
    );
    assert_eq!(
        diff_redacted_json
            .pointer("/summary/added")
            .and_then(|value| value.as_u64()),
        Some(1)
    );
    assert_eq!(
        diff_redacted_json
            .pointer("/summary/removed")
            .and_then(|value| value.as_u64()),
        Some(0)
    );
    assert_eq!(
        diff_redacted_json
            .pointer("/summary/changed")
            .and_then(|value| value.as_u64()),
        Some(0)
    );
    assert_eq!(
        diff_redacted_json
            .pointer("/summary/unchanged")
            .and_then(|value| value.as_u64()),
        Some(1)
    );
    assert_eq!(
        diff_redacted_json
            .pointer("/added/0/id")
            .and_then(|value| value.as_str()),
        Some("key-2")
    );
    assert_eq!(
        diff_redacted_json
            .pointer("/added/0/token")
            .and_then(|value| value.as_str()),
        Some("redacted")
    );

    let diff_with_tokens = Request::builder()
        .method("GET")
        .uri(format!(
            "/admin/config/diff?from_version_id={bootstrap_version_id}&to_version_id={latest_version_id}&include_tokens=true"
        ))
        .header("x-admin-token", "admin-token")
        .body(Body::empty())
        .unwrap();
    let diff_with_tokens_response = app.clone().oneshot(diff_with_tokens).await.unwrap();
    assert_eq!(diff_with_tokens_response.status(), StatusCode::OK);
    let diff_with_tokens_body = to_bytes(diff_with_tokens_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let diff_with_tokens_json: serde_json::Value = serde_json::from_slice(&diff_with_tokens_body)?;
    assert_eq!(
        diff_with_tokens_json
            .pointer("/added/0/token")
            .and_then(|value| value.as_str()),
        Some("vk-2")
    );

    let validate_ok = Request::builder()
        .method("POST")
        .uri("/admin/config/validate")
        .header("x-admin-token", "admin-token")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "virtual_keys": [
                    VirtualKeyConfig::new("valid-1", "vk-valid-1"),
                    VirtualKeyConfig::new("valid-2", "vk-valid-2")
                ]
            })
            .to_string(),
        ))
        .unwrap();
    let validate_ok_response = app.clone().oneshot(validate_ok).await.unwrap();
    assert_eq!(validate_ok_response.status(), StatusCode::OK);
    let validate_ok_body = to_bytes(validate_ok_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let validate_ok_json: serde_json::Value = serde_json::from_slice(&validate_ok_body)?;
    assert_eq!(
        validate_ok_json
            .get("valid")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
    assert_eq!(
        validate_ok_json
            .pointer("/issues")
            .and_then(|value| value.as_array())
            .map(std::vec::Vec::len),
        Some(0)
    );
    assert!(
        validate_ok_json.get("computed_router_sha256").is_none(),
        "router hash should be omitted when router payload is absent"
    );

    let validate_router_ok = Request::builder()
        .method("POST")
        .uri("/admin/config/validate")
        .header("x-admin-token", "admin-token")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "router": {
                    "default_backends": [{"backend": "primary", "weight": 1.0}],
                    "rules": [{
                        "model_prefix": "gpt-4o*",
                        "backends": [{"backend": "primary", "weight": 1.0}]
                    }]
                }
            })
            .to_string(),
        ))
        .unwrap();
    let validate_router_ok_response = app.clone().oneshot(validate_router_ok).await.unwrap();
    assert_eq!(validate_router_ok_response.status(), StatusCode::OK);
    let validate_router_ok_body = to_bytes(validate_router_ok_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let validate_router_ok_json: serde_json::Value =
        serde_json::from_slice(&validate_router_ok_body)?;
    assert_eq!(
        validate_router_ok_json
            .get("valid")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
    assert_eq!(
        validate_router_ok_json
            .pointer("/router_default_backend_count")
            .and_then(|value| value.as_u64()),
        Some(1)
    );
    assert_eq!(
        validate_router_ok_json
            .pointer("/router_rule_count")
            .and_then(|value| value.as_u64()),
        Some(1)
    );
    assert!(
        validate_router_ok_json
            .pointer("/computed_router_sha256")
            .and_then(|value| value.as_str())
            .is_some(),
        "router hash should be returned when router payload is provided"
    );
    assert_eq!(
        validate_router_ok_json
            .pointer("/issues")
            .and_then(|value| value.as_array())
            .map(std::vec::Vec::len),
        Some(0)
    );

    let validate_bad = Request::builder()
        .method("POST")
        .uri("/admin/config/validate")
        .header("x-admin-token", "admin-token")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "virtual_keys": [
                    VirtualKeyConfig::new("", ""),
                    VirtualKeyConfig::new("dup", "dup-token"),
                    VirtualKeyConfig::new("dup", "dup-token")
                ],
                "expected_virtual_keys_sha256": "not-a-real-hash"
            })
            .to_string(),
        ))
        .unwrap();
    let validate_bad_response = app.clone().oneshot(validate_bad).await.unwrap();
    assert_eq!(validate_bad_response.status(), StatusCode::OK);
    let validate_bad_body = to_bytes(validate_bad_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let validate_bad_json: serde_json::Value = serde_json::from_slice(&validate_bad_body)?;
    assert_eq!(
        validate_bad_json
            .get("valid")
            .and_then(|value| value.as_bool()),
        Some(false)
    );
    let issue_codes: std::collections::HashSet<&str> = validate_bad_json
        .pointer("/issues")
        .and_then(|value| value.as_array())
        .expect("issues")
        .iter()
        .filter_map(|item| item.get("code"))
        .filter_map(|value| value.as_str())
        .collect();
    assert!(issue_codes.contains("invalid_id"));
    assert!(issue_codes.contains("invalid_token"));
    assert!(issue_codes.contains("duplicate_id"));
    assert!(issue_codes.contains("duplicate_token"));
    assert!(issue_codes.contains("hash_mismatch"));

    let validate_router_bad = Request::builder()
        .method("POST")
        .uri("/admin/config/validate")
        .header("x-admin-token", "admin-token")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "router": {
                    "default_backends": [{"backend": "unknown-backend", "weight": 1.0}],
                    "rules": []
                },
                "expected_router_sha256": "not-a-real-router-hash"
            })
            .to_string(),
        ))
        .unwrap();
    let validate_router_bad_response = app.clone().oneshot(validate_router_bad).await.unwrap();
    assert_eq!(validate_router_bad_response.status(), StatusCode::OK);
    let validate_router_bad_body = to_bytes(validate_router_bad_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let validate_router_bad_json: serde_json::Value =
        serde_json::from_slice(&validate_router_bad_body)?;
    assert_eq!(
        validate_router_bad_json
            .get("valid")
            .and_then(|value| value.as_bool()),
        Some(false)
    );
    let router_issue_codes: std::collections::HashSet<&str> = validate_router_bad_json
        .pointer("/issues")
        .and_then(|value| value.as_array())
        .expect("issues")
        .iter()
        .filter_map(|item| item.get("code"))
        .filter_map(|value| value.as_str())
        .collect();
    assert!(router_issue_codes.contains("invalid_router"));
    assert!(router_issue_codes.contains("router_hash_mismatch"));

    let get_bootstrap_redacted = Request::builder()
        .method("GET")
        .uri(format!("/admin/config/versions/{bootstrap_version_id}"))
        .header("x-admin-token", "admin-token")
        .body(Body::empty())
        .unwrap();
    let bootstrap_redacted_response = app.clone().oneshot(get_bootstrap_redacted).await.unwrap();
    assert_eq!(bootstrap_redacted_response.status(), StatusCode::OK);
    let bootstrap_redacted_body = to_bytes(bootstrap_redacted_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let bootstrap_redacted_json: serde_json::Value =
        serde_json::from_slice(&bootstrap_redacted_body)?;
    assert_eq!(
        bootstrap_redacted_json
            .pointer("/virtual_keys/0/token")
            .and_then(|value| value.as_str()),
        Some("redacted")
    );

    let get_bootstrap_with_tokens = Request::builder()
        .method("GET")
        .uri(format!(
            "/admin/config/versions/{bootstrap_version_id}?include_tokens=true"
        ))
        .header("x-admin-token", "admin-token")
        .body(Body::empty())
        .unwrap();
    let bootstrap_with_tokens_response = app
        .clone()
        .oneshot(get_bootstrap_with_tokens)
        .await
        .unwrap();
    assert_eq!(bootstrap_with_tokens_response.status(), StatusCode::OK);
    let bootstrap_with_tokens_body =
        to_bytes(bootstrap_with_tokens_response.into_body(), usize::MAX)
            .await
            .unwrap();
    let bootstrap_with_tokens_json: serde_json::Value =
        serde_json::from_slice(&bootstrap_with_tokens_body)?;
    assert_eq!(
        bootstrap_with_tokens_json
            .pointer("/virtual_keys/0/token")
            .and_then(|value| value.as_str()),
        Some("vk-1")
    );

    let rollback_dry_run = Request::builder()
        .method("POST")
        .uri("/admin/config/rollback")
        .header("x-admin-token", "admin-token")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "version_id": bootstrap_version_id,
                "dry_run": true
            })
            .to_string(),
        ))
        .unwrap();
    let rollback_dry_run_response = app.clone().oneshot(rollback_dry_run).await.unwrap();
    assert_eq!(rollback_dry_run_response.status(), StatusCode::OK);
    let rollback_dry_run_body = to_bytes(rollback_dry_run_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let rollback_dry_run_json: serde_json::Value = serde_json::from_slice(&rollback_dry_run_body)?;
    assert_eq!(
        rollback_dry_run_json
            .get("dry_run")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
    assert_eq!(
        rollback_dry_run_json
            .get("noop")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
    assert_eq!(
        rollback_dry_run_json
            .pointer("/current_version/virtual_key_count")
            .and_then(|value| value.as_u64()),
        Some(2)
    );

    let list_keys_after_dry_run = Request::builder()
        .method("GET")
        .uri("/admin/keys?include_tokens=true")
        .header("x-admin-token", "admin-token")
        .body(Body::empty())
        .unwrap();
    let list_keys_after_dry_run_response =
        app.clone().oneshot(list_keys_after_dry_run).await.unwrap();
    assert_eq!(list_keys_after_dry_run_response.status(), StatusCode::OK);
    let keys_after_dry_run_body =
        to_bytes(list_keys_after_dry_run_response.into_body(), usize::MAX)
            .await
            .unwrap();
    let keys_after_dry_run: Vec<VirtualKeyConfig> =
        serde_json::from_slice(&keys_after_dry_run_body)?;
    assert_eq!(keys_after_dry_run.len(), 2);

    let rollback = Request::builder()
        .method("POST")
        .uri("/admin/config/rollback")
        .header("x-admin-token", "admin-token")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({ "version_id": bootstrap_version_id }).to_string(),
        ))
        .unwrap();
    let rollback_response = app.clone().oneshot(rollback).await.unwrap();
    assert_eq!(rollback_response.status(), StatusCode::OK);
    let rollback_body = to_bytes(rollback_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let rollback_json: serde_json::Value = serde_json::from_slice(&rollback_body)?;
    assert!(
        !rollback_json
            .get("dry_run")
            .and_then(|value| value.as_bool())
            .expect("dry_run")
    );
    assert!(
        !rollback_json
            .get("noop")
            .and_then(|value| value.as_bool())
            .expect("noop")
    );
    assert_eq!(
        rollback_json
            .pointer("/current_version/virtual_key_count")
            .and_then(|value| value.as_u64()),
        Some(1)
    );

    let list_keys = Request::builder()
        .method("GET")
        .uri("/admin/keys?include_tokens=true")
        .header("x-admin-token", "admin-token")
        .body(Body::empty())
        .unwrap();
    let list_keys_response = app.oneshot(list_keys).await.unwrap();
    assert_eq!(list_keys_response.status(), StatusCode::OK);
    let keys_body = to_bytes(list_keys_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let keys: Vec<VirtualKeyConfig> = serde_json::from_slice(&keys_body)?;
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].id, "key-1");

    Ok(())
}

#[tokio::test]
async fn gateway_http_admin_config_router_upsert_and_rollback()
-> ditto_llm::foundation::error::Result<()> {
    let mut gateway = Gateway::new(base_config());
    gateway.register_backend("primary", EchoBackend);
    let state = GatewayHttpState::new(gateway).with_admin_token("admin-token");
    let app = ditto_llm::gateway::http::router(state);

    let get_bootstrap = Request::builder()
        .method("GET")
        .uri("/admin/config/version")
        .header("x-admin-token", "admin-token")
        .body(Body::empty())
        .unwrap();
    let bootstrap_response = app.clone().oneshot(get_bootstrap).await.unwrap();
    assert_eq!(bootstrap_response.status(), StatusCode::OK);
    let bootstrap_body = to_bytes(bootstrap_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let bootstrap_json: serde_json::Value = serde_json::from_slice(&bootstrap_body)?;
    let bootstrap_version_id = bootstrap_json
        .get("version_id")
        .and_then(|value| value.as_str())
        .expect("bootstrap version_id")
        .to_string();
    assert_eq!(
        bootstrap_json
            .get("router_rule_count")
            .and_then(|value| value.as_u64()),
        Some(0)
    );

    let new_router = serde_json::json!({
        "router": {
            "default_backends": [
                {
                    "backend": "primary",
                    "weight": 1.0
                }
            ],
            "rules": [
                {
                    "model_prefix": "gpt-4o*",
                    "backends": [
                        {
                            "backend": "primary",
                            "weight": 1.0
                        }
                    ]
                }
            ]
        },
        "dry_run": true
    });

    let upsert_router_dry_run = Request::builder()
        .method("PUT")
        .uri("/admin/config/router")
        .header("x-admin-token", "admin-token")
        .header("content-type", "application/json")
        .body(Body::from(new_router.to_string()))
        .unwrap();
    let upsert_router_dry_run_response = app.clone().oneshot(upsert_router_dry_run).await.unwrap();
    assert_eq!(upsert_router_dry_run_response.status(), StatusCode::OK);
    let upsert_router_dry_run_body =
        to_bytes(upsert_router_dry_run_response.into_body(), usize::MAX)
            .await
            .unwrap();
    let upsert_router_dry_run_json: serde_json::Value =
        serde_json::from_slice(&upsert_router_dry_run_body)?;
    assert_eq!(
        upsert_router_dry_run_json
            .get("dry_run")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
    assert_eq!(
        upsert_router_dry_run_json
            .get("noop")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
    assert_eq!(
        upsert_router_dry_run_json
            .get("router_changed")
            .and_then(|value| value.as_bool()),
        Some(true)
    );

    let new_router_apply = serde_json::json!({
        "router": {
            "default_backends": [
                {
                    "backend": "primary",
                    "weight": 1.0
                }
            ],
            "rules": [
                {
                    "model_prefix": "gpt-4o*",
                    "backends": [
                        {
                            "backend": "primary",
                            "weight": 1.0
                        }
                    ]
                }
            ]
        }
    });
    let upsert_router = Request::builder()
        .method("PUT")
        .uri("/admin/config/router")
        .header("x-admin-token", "admin-token")
        .header("content-type", "application/json")
        .body(Body::from(new_router_apply.to_string()))
        .unwrap();
    let upsert_router_response = app.clone().oneshot(upsert_router).await.unwrap();
    assert_eq!(upsert_router_response.status(), StatusCode::OK);
    let upsert_router_body = to_bytes(upsert_router_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let upsert_router_json: serde_json::Value = serde_json::from_slice(&upsert_router_body)?;
    assert_eq!(
        upsert_router_json
            .get("noop")
            .and_then(|value| value.as_bool()),
        Some(false)
    );
    let router_version_id = upsert_router_json
        .pointer("/current_version/version_id")
        .and_then(|value| value.as_str())
        .expect("router version id")
        .to_string();
    assert_ne!(router_version_id, bootstrap_version_id);

    let get_router_version = Request::builder()
        .method("GET")
        .uri(format!(
            "/admin/config/versions/{router_version_id}?include_tokens=true"
        ))
        .header("x-admin-token", "admin-token")
        .body(Body::empty())
        .unwrap();
    let get_router_version_response = app.clone().oneshot(get_router_version).await.unwrap();
    assert_eq!(get_router_version_response.status(), StatusCode::OK);
    let get_router_version_body = to_bytes(get_router_version_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let get_router_version_json: serde_json::Value =
        serde_json::from_slice(&get_router_version_body)?;
    assert_eq!(
        get_router_version_json
            .pointer("/router/rules/0/model_prefix")
            .and_then(|value| value.as_str()),
        Some("gpt-4o*")
    );
    assert_eq!(
        get_router_version_json
            .pointer("/router_rule_count")
            .and_then(|value| value.as_u64()),
        Some(1)
    );

    let diff_router = Request::builder()
        .method("GET")
        .uri(format!(
            "/admin/config/diff?from_version_id={bootstrap_version_id}&to_version_id={router_version_id}"
        ))
        .header("x-admin-token", "admin-token")
        .body(Body::empty())
        .unwrap();
    let diff_router_response = app.clone().oneshot(diff_router).await.unwrap();
    assert_eq!(diff_router_response.status(), StatusCode::OK);
    let diff_router_body = to_bytes(diff_router_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let diff_router_json: serde_json::Value = serde_json::from_slice(&diff_router_body)?;
    assert_eq!(
        diff_router_json
            .pointer("/summary/router_changed")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
    assert_eq!(
        diff_router_json
            .pointer("/router_before/rules")
            .and_then(|value| value.as_array())
            .map(std::vec::Vec::len),
        Some(0)
    );
    assert_eq!(
        diff_router_json
            .pointer("/router_after/rules")
            .and_then(|value| value.as_array())
            .map(std::vec::Vec::len),
        Some(1)
    );

    let rollback_router = Request::builder()
        .method("POST")
        .uri("/admin/config/rollback")
        .header("x-admin-token", "admin-token")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({ "version_id": bootstrap_version_id }).to_string(),
        ))
        .unwrap();
    let rollback_router_response = app.clone().oneshot(rollback_router).await.unwrap();
    assert_eq!(rollback_router_response.status(), StatusCode::OK);

    let export_after_router_rollback = Request::builder()
        .method("GET")
        .uri("/admin/config/export?include_tokens=true")
        .header("x-admin-token", "admin-token")
        .body(Body::empty())
        .unwrap();
    let export_after_router_rollback_response = app
        .clone()
        .oneshot(export_after_router_rollback)
        .await
        .unwrap();
    assert_eq!(
        export_after_router_rollback_response.status(),
        StatusCode::OK
    );
    let export_after_router_rollback_body = to_bytes(
        export_after_router_rollback_response.into_body(),
        usize::MAX,
    )
    .await
    .unwrap();
    let export_after_router_rollback_json: serde_json::Value =
        serde_json::from_slice(&export_after_router_rollback_body)?;
    assert_eq!(
        export_after_router_rollback_json
            .pointer("/router/rules")
            .and_then(|value| value.as_array())
            .map(std::vec::Vec::len),
        Some(0)
    );

    Ok(())
}

#[tokio::test]
async fn gateway_http_admin_routes_are_disabled_without_admin_token()
-> ditto_llm::foundation::error::Result<()> {
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

#[tokio::test]
async fn gateway_http_litellm_key_routes_are_disabled_without_admin_token()
-> ditto_llm::foundation::error::Result<()> {
    let mut gateway = Gateway::new(base_config());
    gateway.register_backend("primary", EchoBackend);

    let state = GatewayHttpState::new(gateway);
    let app = ditto_llm::gateway::http::router(state);

    let request = Request::builder()
        .method("GET")
        .uri("/key/list")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn gateway_http_litellm_key_generate_info_delete_round_trip()
-> ditto_llm::foundation::error::Result<()> {
    let mut gateway = Gateway::new(base_config());
    gateway.register_backend("primary", EchoBackend);

    let state = GatewayHttpState::new(gateway).with_admin_token("admin-token");
    let app = ditto_llm::gateway::http::router(state);

    let generate_payload = json!({
        "models": ["gpt-4o-mini"],
        "team_id": "t1",
        "user_id": "u1",
        "rpm_limit": 10,
        "tpm_limit": 100,
        "max_budget": 0.01
    });
    let generate = Request::builder()
        .method("POST")
        .uri("/key/generate")
        .header("x-admin-token", "admin-token")
        .header("content-type", "application/json")
        .body(Body::from(generate_payload.to_string()))
        .unwrap();
    let generate_response = app.clone().oneshot(generate).await.unwrap();
    assert_eq!(generate_response.status(), StatusCode::OK);
    let generate_body = to_bytes(generate_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let generate_json: serde_json::Value = serde_json::from_slice(&generate_body)?;
    let key = generate_json
        .get("key")
        .and_then(|value| value.as_str())
        .expect("key")
        .to_string();
    let key_alias = generate_json
        .get("key_alias")
        .and_then(|value| value.as_str())
        .expect("key_alias")
        .to_string();

    let list = Request::builder()
        .method("GET")
        .uri("/key/list")
        .header("x-admin-token", "admin-token")
        .body(Body::empty())
        .unwrap();
    let list_response = app.clone().oneshot(list).await.unwrap();
    assert_eq!(list_response.status(), StatusCode::OK);
    let list_body = to_bytes(list_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let list_json: serde_json::Value = serde_json::from_slice(&list_body)?;
    let keys = list_json
        .get("keys")
        .and_then(|value| value.as_array())
        .expect("keys");
    assert!(keys.iter().all(|value| value.is_string()));
    assert!(
        keys.iter()
            .filter_map(|value| value.as_str())
            .any(|value| value == key)
    );

    let list_full = Request::builder()
        .method("GET")
        .uri("/key/list?return_full_object=true")
        .header("x-admin-token", "admin-token")
        .body(Body::empty())
        .unwrap();
    let list_full_response = app.clone().oneshot(list_full).await.unwrap();
    assert_eq!(list_full_response.status(), StatusCode::OK);
    let list_full_body = to_bytes(list_full_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let list_full_json: serde_json::Value = serde_json::from_slice(&list_full_body)?;
    let keys_full = list_full_json
        .get("keys")
        .and_then(|value| value.as_array())
        .expect("keys");
    assert!(keys_full.iter().any(|value| {
        value
            .get("token")
            .and_then(|token| token.as_str())
            .is_some_and(|token| token == key)
            && value
                .get("key_alias")
                .and_then(|alias| alias.as_str())
                .is_some_and(|alias| alias == key_alias)
    }));

    let gateway_payload = json!({
        "model": "gpt-4o-mini",
        "prompt": "hi",
        "input_tokens": 1,
        "max_output_tokens": 2
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/gateway")
        .header("authorization", format!("Bearer {key}"))
        .header("content-type", "application/json")
        .body(Body::from(gateway_payload.to_string()))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let info = Request::builder()
        .method("GET")
        .uri(format!("/key/info?key={key}"))
        .header("x-admin-token", "admin-token")
        .body(Body::empty())
        .unwrap();
    let info_response = app.clone().oneshot(info).await.unwrap();
    assert_eq!(info_response.status(), StatusCode::OK);
    let info_body = to_bytes(info_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let info_json: serde_json::Value = serde_json::from_slice(&info_body)?;
    assert_eq!(
        info_json.get("key").and_then(|value| value.as_str()),
        Some(key.as_str())
    );
    let info_value = info_json
        .get("info")
        .and_then(|value| value.as_object())
        .expect("info");
    assert!(!info_value.contains_key("token"));

    let self_info = Request::builder()
        .method("GET")
        .uri("/key/info")
        .header("authorization", format!("Bearer {key}"))
        .body(Body::empty())
        .unwrap();
    let self_info_response = app.clone().oneshot(self_info).await.unwrap();
    assert_eq!(self_info_response.status(), StatusCode::OK);

    let update_payload = json!({
        "key": key.clone(),
        "key_alias": "alias-updated",
        "models": ["gpt-4o-mini", "gpt-4o"],
        "rpm_limit": 50
    });
    let update = Request::builder()
        .method("POST")
        .uri("/key/update")
        .header("x-admin-token", "admin-token")
        .header("content-type", "application/json")
        .body(Body::from(update_payload.to_string()))
        .unwrap();
    let update_response = app.clone().oneshot(update).await.unwrap();
    assert_eq!(update_response.status(), StatusCode::OK);
    let update_body = to_bytes(update_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let update_json: serde_json::Value = serde_json::from_slice(&update_body)?;
    assert_eq!(
        update_json.get("key").and_then(|value| value.as_str()),
        Some(key.as_str())
    );
    assert_eq!(
        update_json
            .get("key_alias")
            .and_then(|value| value.as_str()),
        Some("alias-updated")
    );

    let regenerate_payload = json!({
        "key": key.clone()
    });
    let regenerate = Request::builder()
        .method("POST")
        .uri("/key/regenerate")
        .header("x-admin-token", "admin-token")
        .header("content-type", "application/json")
        .body(Body::from(regenerate_payload.to_string()))
        .unwrap();
    let regenerate_response = app.clone().oneshot(regenerate).await.unwrap();
    assert_eq!(regenerate_response.status(), StatusCode::OK);
    let regenerate_body = to_bytes(regenerate_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let regenerate_json: serde_json::Value = serde_json::from_slice(&regenerate_body)?;
    let regenerated_key = regenerate_json
        .get("key")
        .and_then(|value| value.as_str())
        .expect("regenerated key")
        .to_string();
    assert_ne!(regenerated_key, key);

    let request = Request::builder()
        .method("POST")
        .uri("/v1/gateway")
        .header("authorization", format!("Bearer {key}"))
        .header("content-type", "application/json")
        .body(Body::from(gateway_payload.to_string()))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let request = Request::builder()
        .method("POST")
        .uri("/v1/gateway")
        .header("authorization", format!("Bearer {regenerated_key}"))
        .header("content-type", "application/json")
        .body(Body::from(gateway_payload.to_string()))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let self_info = Request::builder()
        .method("GET")
        .uri("/key/info")
        .header("authorization", format!("Bearer {regenerated_key}"))
        .body(Body::empty())
        .unwrap();
    let self_info_response = app.clone().oneshot(self_info).await.unwrap();
    assert_eq!(self_info_response.status(), StatusCode::OK);

    let delete_payload = json!({
        "keys": [regenerated_key.clone()],
    });
    let delete = Request::builder()
        .method("POST")
        .uri("/key/delete")
        .header("x-admin-token", "admin-token")
        .header("content-type", "application/json")
        .body(Body::from(delete_payload.to_string()))
        .unwrap();
    let delete_response = app.clone().oneshot(delete).await.unwrap();
    assert_eq!(delete_response.status(), StatusCode::OK);

    let deleted_body = to_bytes(delete_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let deleted_json: serde_json::Value = serde_json::from_slice(&deleted_body)?;
    let deleted_keys = deleted_json
        .get("deleted_keys")
        .and_then(|value| value.as_array())
        .expect("deleted_keys");
    assert_eq!(deleted_keys.len(), 1);
    assert_eq!(deleted_keys[0].as_str(), Some(regenerated_key.as_str()));

    let request = Request::builder()
        .method("POST")
        .uri("/v1/gateway")
        .header("authorization", format!("Bearer {regenerated_key}"))
        .header("content-type", "application/json")
        .body(Body::from(gateway_payload.to_string()))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    Ok(())
}

#[tokio::test]
async fn gateway_http_tenant_scoped_admin_tokens_are_isolated()
-> ditto_llm::foundation::error::Result<()> {
    let mut config = base_config();
    config.virtual_keys = vec![
        {
            let mut key = base_key();
            key.tenant_id = Some("t1".to_string());
            key
        },
        {
            let mut key = VirtualKeyConfig::new("key-2", "vk-2");
            key.tenant_id = Some("t2".to_string());
            key
        },
    ];

    let mut gateway = Gateway::new(config);
    gateway.register_backend("primary", EchoBackend);

    let state = GatewayHttpState::new(gateway).with_admin_tenant_token("t1", "tenant-admin");
    let app = ditto_llm::gateway::http::router(state);

    let list = Request::builder()
        .method("GET")
        .uri("/admin/keys")
        .header("x-admin-token", "tenant-admin")
        .body(Body::empty())
        .unwrap();
    let list_response = app.clone().oneshot(list).await.unwrap();
    assert_eq!(list_response.status(), StatusCode::OK);
    let list_body = to_bytes(list_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let keys: Vec<VirtualKeyConfig> = serde_json::from_slice(&list_body)?;
    assert!(
        keys.iter()
            .all(|key| key.tenant_id.as_deref() == Some("t1"))
    );

    let cross_tenant = Request::builder()
        .method("GET")
        .uri("/admin/keys?tenant_id=t2")
        .header("x-admin-token", "tenant-admin")
        .body(Body::empty())
        .unwrap();
    let cross_tenant_response = app.clone().oneshot(cross_tenant).await.unwrap();
    assert_eq!(cross_tenant_response.status(), StatusCode::FORBIDDEN);

    let new_key = VirtualKeyConfig::new("key-3", "vk-3");
    let upsert = Request::builder()
        .method("POST")
        .uri("/admin/keys")
        .header("x-admin-token", "tenant-admin")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&new_key)?))
        .unwrap();
    let upsert_response = app.clone().oneshot(upsert).await.unwrap();
    assert_eq!(upsert_response.status(), StatusCode::CREATED);
    let upsert_body = to_bytes(upsert_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: VirtualKeyConfig = serde_json::from_slice(&upsert_body)?;
    assert_eq!(created.tenant_id.as_deref(), Some("t1"));

    let mut wrong_tenant_key = VirtualKeyConfig::new("key-4", "vk-4");
    wrong_tenant_key.tenant_id = Some("t2".to_string());
    let wrong_tenant_upsert = Request::builder()
        .method("POST")
        .uri("/admin/keys")
        .header("x-admin-token", "tenant-admin")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&wrong_tenant_key)?))
        .unwrap();
    let wrong_tenant_upsert_response = app.clone().oneshot(wrong_tenant_upsert).await.unwrap();
    assert_eq!(wrong_tenant_upsert_response.status(), StatusCode::FORBIDDEN);

    let delete_other_tenant = Request::builder()
        .method("DELETE")
        .uri("/admin/keys/key-2")
        .header("x-admin-token", "tenant-admin")
        .body(Body::empty())
        .unwrap();
    let delete_other_tenant_response = app.clone().oneshot(delete_other_tenant).await.unwrap();
    assert_eq!(delete_other_tenant_response.status(), StatusCode::FORBIDDEN);

    Ok(())
}

#[tokio::test]
async fn gateway_http_tenant_read_token_is_read_only() -> ditto_llm::foundation::error::Result<()> {
    let mut gateway = Gateway::new(base_config());
    gateway.register_backend("primary", EchoBackend);

    let state = GatewayHttpState::new(gateway).with_admin_tenant_read_token("t1", "tenant-read");
    let app = ditto_llm::gateway::http::router(state);

    let list = Request::builder()
        .method("GET")
        .uri("/admin/keys")
        .header("x-admin-token", "tenant-read")
        .body(Body::empty())
        .unwrap();
    let list_response = app.clone().oneshot(list).await.unwrap();
    assert_eq!(list_response.status(), StatusCode::OK);

    let upsert = Request::builder()
        .method("POST")
        .uri("/admin/keys")
        .header("x-admin-token", "tenant-read")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&VirtualKeyConfig::new(
            "k", "v",
        ))?))
        .unwrap();
    let upsert_response = app.clone().oneshot(upsert).await.unwrap();
    assert_eq!(upsert_response.status(), StatusCode::METHOD_NOT_ALLOWED);

    Ok(())
}

#[cfg(feature = "gateway-store-sqlite")]
#[tokio::test]
async fn gateway_http_audit_export_jsonl_has_hash_chain() -> ditto_llm::foundation::error::Result<()>
{
    use ditto_llm::gateway::SqliteStore;

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("gateway.sqlite");
    let store = SqliteStore::new(&path);
    store.init().await.expect("init");

    store
        .append_audit_log("k1", json!({"tenant_id": "t1", "n": 1}))
        .await
        .expect("append");
    store
        .append_audit_log("k2", json!({"tenant_id": "t2", "n": 2}))
        .await
        .expect("append");
    store
        .append_audit_log("k3", json!({"tenant_id": "t1", "n": 3}))
        .await
        .expect("append");

    let mut gateway = Gateway::new(base_config());
    gateway.register_backend("primary", EchoBackend);

    let state = GatewayHttpState::new(gateway)
        .with_admin_token("admin-token")
        .with_sqlite_store(store.clone());
    let app = ditto_llm::gateway::http::router(state);

    let export = Request::builder()
        .method("GET")
        .uri("/admin/audit/export?format=jsonl&limit=10")
        .header("x-admin-token", "admin-token")
        .body(Body::empty())
        .unwrap();
    let export_response = app.clone().oneshot(export).await.unwrap();
    assert_eq!(export_response.status(), StatusCode::OK);
    let export_body = to_bytes(export_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(export_body.to_vec()).expect("utf8");
    let mut prev_hash: Option<String> = None;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let value: serde_json::Value = serde_json::from_str(line)?;
        let record_prev_hash = value
            .get("prev_hash")
            .and_then(serde_json::Value::as_str)
            .map(|s| s.to_string());
        assert_eq!(record_prev_hash, prev_hash);
        let hash = value
            .get("hash")
            .and_then(serde_json::Value::as_str)
            .expect("hash")
            .to_string();
        prev_hash = Some(hash);
    }

    let tenant_export = Request::builder()
        .method("GET")
        .uri("/admin/audit/export?format=jsonl&limit=10")
        .header("x-admin-token", "tenant-admin")
        .body(Body::empty())
        .unwrap();
    let tenant_state = GatewayHttpState::new(Gateway::new(base_config()))
        .with_admin_tenant_read_token("t1", "tenant-admin")
        .with_sqlite_store(store);
    let tenant_app = ditto_llm::gateway::http::router(tenant_state);

    let tenant_export_response = tenant_app.clone().oneshot(tenant_export).await.unwrap();
    assert_eq!(tenant_export_response.status(), StatusCode::OK);
    let tenant_export_body = to_bytes(tenant_export_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let tenant_text = String::from_utf8(tenant_export_body.to_vec()).expect("utf8");
    for line in tenant_text.lines().filter(|line| !line.trim().is_empty()) {
        let value: serde_json::Value = serde_json::from_str(line)?;
        assert_eq!(
            value
                .get("payload")
                .and_then(|p| p.get("tenant_id"))
                .and_then(serde_json::Value::as_str),
            Some("t1")
        );
    }

    Ok(())
}
