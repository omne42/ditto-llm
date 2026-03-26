#![cfg(feature = "gateway-store-sqlite")]

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use ditto_server::gateway::{
    Backend, Gateway, GatewayConfig, GatewayError, GatewayHttpState, GatewayRequest,
    GatewayResponse, RouteBackend, RouterConfig, SqliteStore, VirtualKeyConfig,
};
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
    let mut gateway = Gateway::new(config);
    gateway.register_backend("primary", EchoBackend);
    let state = GatewayHttpState::new(gateway)
        .with_admin_token("adm")
        .with_sqlite_store(store.clone());
    let app = ditto_server::gateway::http::router(state);

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
    assert!(loaded[0].token.starts_with("sha256:"));
    let persisted_config = GatewayConfig {
        backends: Vec::new(),
        virtual_keys: loaded.clone(),
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
    assert!(persisted_config.virtual_key("vk-1").is_some());

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

#[tokio::test]
async fn admin_router_mutation_persists_router_to_sqlite_store() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("gateway.sqlite");
    let store = SqliteStore::new(&db_path);
    store.init().await.expect("init");

    let config = GatewayConfig {
        backends: Vec::new(),
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
    let mut gateway = Gateway::new(config);
    gateway.register_backend("primary", EchoBackend);
    let state = GatewayHttpState::new(gateway)
        .with_admin_token("adm")
        .with_sqlite_store(store.clone());
    let app = ditto_server::gateway::http::router(state);

    let request = Request::builder()
        .method("PUT")
        .uri("/admin/config/router")
        .header("authorization", "Bearer adm")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
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
            })
            .to_string(),
        ))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let loaded_router = store.load_router_config().await.expect("load router");
    assert!(loaded_router.is_some());
    let loaded_router = loaded_router.expect("router");
    assert_eq!(loaded_router.rules.len(), 1);
    assert_eq!(loaded_router.rules[0].model_prefix, "gpt-4o*");
}
