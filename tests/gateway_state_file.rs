#![cfg(feature = "gateway")]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ditto_llm::gateway::{
    Gateway, GatewayConfig, GatewayHttpState, GatewayStateFile, RouteBackend, RouterConfig,
};
use tower::util::ServiceExt;

#[tokio::test]
async fn admin_key_mutations_persist_virtual_keys_to_state_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state_path = dir.path().join("gateway-state.json");

    let config = GatewayConfig {
        backends: Vec::new(),
        virtual_keys: Vec::new(),
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

    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway)
        .with_admin_token("adm")
        .with_state_file(state_path.clone());
    let app = ditto_llm::gateway::http::router(state);

    let key = ditto_llm::gateway::VirtualKeyConfig::new("key-1", "vk-1");
    let request = Request::builder()
        .method("POST")
        .uri("/admin/keys")
        .header("authorization", "Bearer adm")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&key).expect("json")))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let loaded = GatewayStateFile::load(&state_path).expect("state file load");
    assert_eq!(loaded.virtual_keys.len(), 1);
    assert_eq!(loaded.virtual_keys[0].id, "key-1");
    assert_eq!(loaded.virtual_keys[0].token, "vk-1");

    let request = Request::builder()
        .method("DELETE")
        .uri("/admin/keys/key-1")
        .header("authorization", "Bearer adm")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let loaded = GatewayStateFile::load(&state_path).expect("state file load");
    assert!(loaded.virtual_keys.is_empty());
}
