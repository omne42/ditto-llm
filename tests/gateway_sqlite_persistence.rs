#![cfg(feature = "gateway-store-sqlite")]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ditto_llm::gateway::{
    Gateway, GatewayConfig, GatewayHttpState, RouteBackend, RouterConfig, SqliteStore,
    VirtualKeyConfig,
};
use tower::util::ServiceExt;

#[tokio::test]
async fn admin_key_mutations_persist_virtual_keys_to_sqlite_store() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("gateway.sqlite");
    let store = SqliteStore::new(&db_path);
    store.init().await.expect("init");

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
        .with_sqlite_store(store.clone());
    let app = ditto_llm::gateway::http::router(state);

    let key = VirtualKeyConfig::new("key-1", "vk-1");
    let request = Request::builder()
        .method("POST")
        .uri("/admin/keys")
        .header("authorization", "Bearer adm")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&key).expect("json")))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let loaded = store.load_virtual_keys().await.expect("load");
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].id, "key-1");
    assert_eq!(loaded[0].token, "vk-1");

    let request = Request::builder()
        .method("DELETE")
        .uri("/admin/keys/key-1")
        .header("authorization", "Bearer adm")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let loaded = store.load_virtual_keys().await.expect("load");
    assert!(loaded.is_empty());
}
