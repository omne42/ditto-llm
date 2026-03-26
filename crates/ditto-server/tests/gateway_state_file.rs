#![cfg(feature = "gateway")]

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use ditto_server::gateway::{
    Backend, Gateway, GatewayConfig, GatewayError, GatewayHttpState, GatewayRequest,
    GatewayResponse, GatewayStateFile, RouteBackend, RouterConfig,
};
use std::fs;
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

    let mut gateway = Gateway::new(config);
    gateway.register_backend("primary", EchoBackend);
    let state = GatewayHttpState::new(gateway)
        .with_admin_token("adm")
        .with_state_file(state_path.clone());
    let app = ditto_server::gateway::http::router(state);

    let key = ditto_server::gateway::VirtualKeyConfig::new("key-1", "vk-1");
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
    assert!(loaded.virtual_keys[0].token.starts_with("sha256:"));
    assert_eq!(
        loaded
            .router
            .as_ref()
            .map(|router| router.default_backends.len()),
        Some(1)
    );
    let persisted_config = GatewayConfig {
        backends: Vec::new(),
        virtual_keys: loaded.virtual_keys.clone(),
        router: loaded.router.clone().expect("router"),
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

    let loaded = GatewayStateFile::load(&state_path).expect("state file load");
    assert!(loaded.virtual_keys.is_empty());
    assert_eq!(
        loaded.router.as_ref().map(|router| router.rules.len()),
        Some(0)
    );
}

#[tokio::test]
async fn admin_router_mutation_persists_router_to_state_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state_path = dir.path().join("gateway-state.json");

    let config = GatewayConfig {
        backends: Vec::new(),
        virtual_keys: vec![ditto_server::gateway::VirtualKeyConfig::new(
            "key-1", "vk-1",
        )],
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
        .with_state_file(state_path.clone());
    let app = ditto_server::gateway::http::router(state);

    let update_router = Request::builder()
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
    let update_router_response = app.oneshot(update_router).await.unwrap();
    assert_eq!(update_router_response.status(), StatusCode::OK);

    let loaded = GatewayStateFile::load(&state_path).expect("state file load");
    assert_eq!(loaded.virtual_keys.len(), 1);
    assert!(loaded.virtual_keys[0].token.starts_with("sha256:"));
    assert_eq!(
        loaded.router.as_ref().map(|router| router.rules.len()),
        Some(1)
    );
    assert_eq!(
        loaded
            .router
            .as_ref()
            .and_then(|router| router.rules.first())
            .map(|rule| rule.model_prefix.as_str()),
        Some("gpt-4o*")
    );
}

#[test]
fn state_file_load_is_backward_compatible_without_router_field() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state_path = dir.path().join("legacy-state.json");
    fs::write(&state_path, r#"{ "virtual_keys": [] }"#).expect("write legacy file");

    let loaded = GatewayStateFile::load(&state_path).expect("state file load");
    assert!(loaded.virtual_keys.is_empty());
    assert!(loaded.router.is_none());
}
