#![cfg(all(
    feature = "gateway",
    feature = "gateway-store-sqlite",
    feature = "gateway-costing"
))]

use std::collections::{BTreeMap, HashMap};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use ditto_gateway_contract_types::{
    AuditLogRecord as ContractAuditLogRecord, BudgetLedgerRecord as ContractBudgetLedgerRecord,
    CostLedgerRecord as ContractCostLedgerRecord, HealthResponse, ProxyJsonEnvelope,
    ReapReservationsRequest, ReapReservationsResponse,
};
use ditto_server::gateway::{
    BackendConfig, Gateway, GatewayConfig, GatewayError, GatewayHttpState, ProxyBackend,
    RouteBackend, RouterConfig, SqliteStore, VirtualKeyConfig,
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
) -> Result<HashMap<String, ProxyBackend>, GatewayError> {
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
async fn gateway_contract_v0_1_endpoints_match_types() -> Result<(), Box<dyn std::error::Error>> {
    if ditto_core::utils::test_support::should_skip_httpmock() {
        return Ok(());
    }

    let upstream = MockServer::start();
    let proxy_mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/chat/completions")
            .header("authorization", "Bearer sk-test");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"id":"ok","object":"chat.completion"}"#);
    });

    let tmp = tempfile::tempdir()?;
    let sqlite_path = tmp.path().join("contract-v0_1.sqlite");
    let store = SqliteStore::new(&sqlite_path);
    store.init().await?;
    store.verify_schema().await?;

    store
        .append_audit_log("contract.seed", json!({"from": "test"}))
        .await?;
    store
        .reserve_budget_tokens("contract-budget", "key-1", 10_000, 12)
        .await?;
    store
        .commit_budget_reservation_with_tokens("contract-budget", 5)
        .await?;
    store
        .reserve_cost_usd_micros("contract-cost", "key-1", 10_000, 9)
        .await?;
    store
        .commit_cost_reservation_with_usd_micros("contract-cost", 4)
        .await?;
    store
        .reserve_budget_tokens("contract-reap-budget", "key-1", 10_000, 3)
        .await?;
    store
        .reserve_cost_usd_micros("contract-reap-cost", "key-1", 10_000, 2)
        .await?;

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
    };

    let proxy_backends = build_proxy_backends(&config)?;
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway)
        .with_admin_token("admin-token")
        .with_sqlite_store(store)
        .with_proxy_backends(proxy_backends);
    let app = ditto_server::gateway::http::router(state);

    let health_req = Request::builder()
        .method("GET")
        .uri("/health")
        .body(Body::empty())?;
    let health_res = app.clone().oneshot(health_req).await.unwrap();
    assert_eq!(health_res.status(), StatusCode::OK);
    let health_body = to_bytes(health_res.into_body(), usize::MAX).await?;
    let health: HealthResponse = serde_json::from_slice(&health_body)?;
    assert_eq!(health.status, "ok");

    let proxy_req = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("authorization", "Bearer vk-1")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "model": "gpt-4o-mini",
                "messages": [{"role": "user", "content": "hi"}]
            })
            .to_string(),
        ))?;
    let proxy_res = app.clone().oneshot(proxy_req).await.unwrap();
    assert_eq!(proxy_res.status(), StatusCode::OK);
    let proxy_body = to_bytes(proxy_res.into_body(), usize::MAX).await?;
    let proxy_json: ProxyJsonEnvelope = serde_json::from_slice(&proxy_body)?;
    assert_eq!(proxy_json.get("id").and_then(|v| v.as_str()), Some("ok"));
    proxy_mock.assert();

    let audit_req = Request::builder()
        .method("GET")
        .uri("/admin/audit?limit=10")
        .header("x-admin-token", "admin-token")
        .body(Body::empty())?;
    let audit_res = app.clone().oneshot(audit_req).await.unwrap();
    assert_eq!(audit_res.status(), StatusCode::OK);
    let audit_body = to_bytes(audit_res.into_body(), usize::MAX).await?;
    let audit_logs: Vec<ContractAuditLogRecord> = serde_json::from_slice(&audit_body)?;
    assert!(
        audit_logs
            .iter()
            .any(|record| record.kind == "contract.seed")
    );

    let budgets_req = Request::builder()
        .method("GET")
        .uri("/admin/budgets?limit=10&offset=0")
        .header("x-admin-token", "admin-token")
        .body(Body::empty())?;
    let budgets_res = app.clone().oneshot(budgets_req).await.unwrap();
    assert_eq!(budgets_res.status(), StatusCode::OK);
    let budgets_body = to_bytes(budgets_res.into_body(), usize::MAX).await?;
    let budgets: Vec<ContractBudgetLedgerRecord> = serde_json::from_slice(&budgets_body)?;
    assert!(budgets.iter().any(|ledger| ledger.key_id == "key-1"));

    let costs_req = Request::builder()
        .method("GET")
        .uri("/admin/costs?limit=10&offset=0")
        .header("x-admin-token", "admin-token")
        .body(Body::empty())?;
    let costs_res = app.clone().oneshot(costs_req).await.unwrap();
    assert_eq!(costs_res.status(), StatusCode::OK);
    let costs_body = to_bytes(costs_res.into_body(), usize::MAX).await?;
    let costs: Vec<ContractCostLedgerRecord> = serde_json::from_slice(&costs_body)?;
    assert!(costs.iter().any(|ledger| ledger.key_id == "key-1"));

    let reap_req = Request::builder()
        .method("POST")
        .uri("/admin/reservations/reap")
        .header("x-admin-token", "admin-token")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&ReapReservationsRequest {
            older_than_secs: 24 * 60 * 60,
            limit: 1000,
            dry_run: true,
        })?))?;
    let reap_res = app.oneshot(reap_req).await.unwrap();
    assert_eq!(reap_res.status(), StatusCode::OK);
    let reap_body = to_bytes(reap_res.into_body(), usize::MAX).await?;
    let reap: ReapReservationsResponse = serde_json::from_slice(&reap_body)?;
    assert_eq!(reap.store, "sqlite");
    assert!(reap.dry_run);

    Ok(())
}
