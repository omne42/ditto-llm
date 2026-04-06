#[cfg(feature = "gateway-translation")]
use std::sync::Arc;

#[cfg(feature = "gateway-translation")]
use async_trait::async_trait;
#[cfg(feature = "gateway-translation")]
use ditto_core::contracts::{ContentPart, FinishReason, GenerateRequest, GenerateResponse, Usage};
#[cfg(feature = "gateway-translation")]
use ditto_core::llm_core::model::{LanguageModel, StreamResult};
#[cfg(feature = "gateway-translation")]
use ditto_server::gateway::TranslationBackend;
#[cfg(feature = "gateway-translation")]
use futures_util::StreamExt;

#[cfg(feature = "gateway-translation")]
#[derive(Clone)]
struct TranslationModelsStub;

#[cfg(feature = "gateway-translation")]
#[async_trait]
impl LanguageModel for TranslationModelsStub {
    fn provider(&self) -> &str {
        "fake"
    }

    fn model_id(&self) -> &str {
        "fake-model"
    }

    async fn generate(
        &self,
        _request: GenerateRequest,
    ) -> ditto_core::error::Result<GenerateResponse> {
        Ok(GenerateResponse {
            content: vec![ContentPart::Text {
                text: "ok".to_string(),
            }],
            finish_reason: FinishReason::Stop,
            usage: Usage::default(),
            warnings: Vec::new(),
            provider_metadata: None,
        })
    }

    async fn stream(&self, _request: GenerateRequest) -> ditto_core::error::Result<StreamResult> {
        Ok(futures_util::stream::empty().boxed())
    }
}

#[tokio::test]
async fn openai_models_list_merges_across_backends() {
    if ditto_core::utils::test_support::should_skip_httpmock() {
        return;
    }
    let upstream_a = MockServer::start();
    let upstream_b = MockServer::start();

    let mock_a = upstream_a.mock(|when, then| {
        when.method(httpmock::Method::GET)
            .path("/v1/models")
            .header("authorization", "Bearer sk-a");
        then.status(200)
            .header("content-type", "application/json")
            .body(
                json!({
                    "object": "list",
                    "data": [
                        {"id": "gpt-4o", "object": "model"},
                        {"id": "glm-4.7", "object": "model"}
                    ]
                })
                .to_string(),
            );
    });

    let mock_b = upstream_b.mock(|when, then| {
        when.method(httpmock::Method::GET)
            .path("/v1/models")
            .header("authorization", "Bearer sk-b");
        then.status(200)
            .header("content-type", "application/json")
            .body(
                json!({
                    "object": "list",
                    "data": [
                        {"id": "glm-4.7", "object": "model"},
                        {"id": "claude-3.5-sonnet", "object": "model"}
                    ]
                })
                .to_string(),
            );
    });

    let config = GatewayConfig {
        backends: vec![
            backend_config("a", upstream_a.base_url(), "Bearer sk-a"),
            backend_config("b", upstream_b.base_url(), "Bearer sk-b"),
        ],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
        router: RouterConfig {
            default_backends: vec![RouteBackend { backend: "a".to_string(), weight: 1.0 }],
            rules: Vec::new(),
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
        observability: Default::default(),
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway).with_proxy_backends(proxy_backends);
    let app = ditto_server::gateway::http::router(state);

    let request = Request::builder()
        .method("GET")
        .uri("/v1/models")
        .header("authorization", "Bearer vk-1")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    let ids: Vec<String> = json
        .get("data")
        .and_then(|v| v.as_array())
        .into_iter()
        .flat_map(|arr| arr.iter())
        .filter_map(|m| m.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();

    assert_eq!(ids.len(), 3);
    assert!(ids.contains(&"gpt-4o".to_string()));
    assert!(ids.contains(&"glm-4.7".to_string()));
    assert!(ids.contains(&"claude-3.5-sonnet".to_string()));

    mock_a.assert();
    mock_b.assert();
}

#[cfg(feature = "gateway-translation")]
#[tokio::test]
async fn openai_models_list_respects_translation_route() {
    if ditto_core::utils::test_support::should_skip_httpmock() {
        return;
    }
    let upstream = MockServer::start();
    let proxy_mock = upstream.mock(|when, then| {
        when.method(httpmock::Method::GET)
            .path("/v1/models")
            .header("authorization", "Bearer sk-a");
        then.status(200)
            .header("content-type", "application/json")
            .body(
                json!({
                    "object": "list",
                    "data": [{"id": "gpt-4o", "object": "model"}]
                })
                .to_string(),
            );
    });

    let mut key = VirtualKeyConfig::new("key-1", "vk-1");
    key.route = Some("primary".to_string());

    let config = GatewayConfig {
        backends: vec![backend_config("a", upstream.base_url(), "Bearer sk-a")],
        virtual_keys: vec![key],
        router: RouterConfig {
            default_backends: vec![RouteBackend {
                backend: "a".to_string(),
                weight: 1.0,
            }],
            rules: Vec::new(),
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
        observability: Default::default(),
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);

    let mut primary_model_map = BTreeMap::new();
    primary_model_map.insert("gpt-4o-mini".to_string(), "fake-model".to_string());
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(TranslationModelsStub))
            .with_model_map(primary_model_map),
    );
    translation_backends.insert(
        "secondary".to_string(),
        TranslationBackend::new("other", Arc::new(TranslationModelsStub)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(proxy_backends)
        .with_translation_backends(translation_backends);
    let app = ditto_server::gateway::http::router(state);

    let request = Request::builder()
        .method("GET")
        .uri("/v1/models")
        .header("authorization", "Bearer vk-1")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    let ids: Vec<String> = json
        .get("data")
        .and_then(|v| v.as_array())
        .into_iter()
        .flat_map(|arr| arr.iter())
        .filter_map(|m| m.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();

    assert!(ids.contains(&"gpt-4o-mini".to_string()));
    assert!(ids.contains(&"primary/fake-model".to_string()));
    assert!(!ids.contains(&"other/fake-model".to_string()));
    assert!(!ids.contains(&"gpt-4o".to_string()));
    proxy_mock.assert_calls(0);
}

#[cfg(feature = "gateway-translation")]
#[tokio::test]
async fn openai_models_list_excludes_translation_models_when_key_route_targets_proxy_backend() {
    if ditto_core::utils::test_support::should_skip_httpmock() {
        return;
    }
    let upstream = MockServer::start();
    let proxy_mock = upstream.mock(|when, then| {
        when.method(httpmock::Method::GET)
            .path("/v1/models")
            .header("authorization", "Bearer sk-a");
        then.status(200)
            .header("content-type", "application/json")
            .body(
                json!({
                    "object": "list",
                    "data": [{"id": "gpt-4o", "object": "model"}]
                })
                .to_string(),
            );
    });

    let mut key = VirtualKeyConfig::new("key-1", "vk-1");
    key.route = Some("a".to_string());

    let config = GatewayConfig {
        backends: vec![backend_config("a", upstream.base_url(), "Bearer sk-a")],
        virtual_keys: vec![key],
        router: RouterConfig {
            default_backends: vec![RouteBackend {
                backend: "a".to_string(),
                weight: 1.0,
            }],
            rules: Vec::new(),
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
        observability: Default::default(),
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);

    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(TranslationModelsStub)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(proxy_backends)
        .with_translation_backends(translation_backends);
    let app = ditto_server::gateway::http::router(state);

    let request = Request::builder()
        .method("GET")
        .uri("/v1/models")
        .header("authorization", "Bearer vk-1")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    let ids: Vec<String> = json
        .get("data")
        .and_then(|v| v.as_array())
        .into_iter()
        .flat_map(|arr| arr.iter())
        .filter_map(|m| m.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();

    assert_eq!(ids, vec!["gpt-4o".to_string()]);
    proxy_mock.assert_calls(1);
}

#[tokio::test]
async fn openai_models_list_skips_backends_with_oversized_responses() {
    if ditto_core::utils::test_support::should_skip_httpmock() {
        return;
    }
    let upstream_a = MockServer::start();
    let upstream_b = MockServer::start();

    let too_large = "x".repeat(6 * 1024 * 1024);
    let mock_a = upstream_a.mock(|when, then| {
        when.method(httpmock::Method::GET)
            .path("/v1/models")
            .header("authorization", "Bearer sk-a");
        then.status(200)
            .header("content-type", "application/json")
            .body(too_large.clone());
    });

    let mock_b = upstream_b.mock(|when, then| {
        when.method(httpmock::Method::GET)
            .path("/v1/models")
            .header("authorization", "Bearer sk-b");
        then.status(200)
            .header("content-type", "application/json")
            .body(
                json!({
                    "object": "list",
                    "data": [
                        {"id": "gpt-4o-mini", "object": "model"}
                    ]
                })
                .to_string(),
            );
    });

    let config = GatewayConfig {
        backends: vec![
            backend_config("a", upstream_a.base_url(), "Bearer sk-a"),
            backend_config("b", upstream_b.base_url(), "Bearer sk-b"),
        ],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
        router: RouterConfig {
            default_backends: vec![RouteBackend { backend: "a".to_string(), weight: 1.0 }],
            rules: Vec::new(),
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
        observability: Default::default(),
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway).with_proxy_backends(proxy_backends);
    let app = ditto_server::gateway::http::router(state);

    let request = Request::builder()
        .method("GET")
        .uri("/v1/models")
        .header("authorization", "Bearer vk-1")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    let ids: Vec<String> = json
        .get("data")
        .and_then(|v| v.as_array())
        .into_iter()
        .flat_map(|arr| arr.iter())
        .filter_map(|m| m.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();

    assert_eq!(ids, vec!["gpt-4o-mini".to_string()]);

    mock_a.assert();
    mock_b.assert();
}

#[tokio::test]
async fn openai_models_retrieve_uses_model_path_for_routing() {
    if ditto_core::utils::test_support::should_skip_httpmock() {
        return;
    }
    let upstream_a = MockServer::start();
    let upstream_b = MockServer::start();

    let _mock_a = upstream_a.mock(|when, then| {
        when.method(httpmock::Method::GET)
            .path("/v1/models/claude-3.5-sonnet");
        then.status(404);
    });

    let mock_b = upstream_b.mock(|when, then| {
        when.method(httpmock::Method::GET)
            .path("/v1/models/claude-3.5-sonnet")
            .header("authorization", "Bearer sk-b");
        then.status(200)
            .header("content-type", "application/json")
            .body(
                json!({
                    "id": "claude-3.5-sonnet",
                    "object": "model",
                    "owned_by": "anthropic"
                })
                .to_string(),
            );
    });

    let config = GatewayConfig {
        backends: vec![
            backend_config("a", upstream_a.base_url(), "Bearer sk-a"),
            backend_config("b", upstream_b.base_url(), "Bearer sk-b"),
        ],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
        router: RouterConfig {
            default_backends: vec![RouteBackend {
                backend: "a".to_string(),
                weight: 1.0,
            }],
            rules: vec![RouteRule {
                model_prefix: "claude-".to_string(),
                exact: false,
                backend: String::new(),
                backends: vec![RouteBackend {
                    backend: "b".to_string(),
                    weight: 1.0,
                }],
                guardrails: None,
            }],
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
        observability: Default::default(),
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway).with_proxy_backends(proxy_backends);
    let app = ditto_server::gateway::http::router(state);

    let request = Request::builder()
        .method("GET")
        .uri("/v1/models/claude-3.5-sonnet")
        .header("authorization", "Bearer vk-1")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(
        json.get("id").and_then(|value| value.as_str()),
        Some("claude-3.5-sonnet")
    );

    mock_b.assert();
}
