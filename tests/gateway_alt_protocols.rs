#![cfg(feature = "gateway")]

use std::collections::{BTreeMap, HashMap};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use ditto_llm::gateway::{
    BackendConfig, Gateway, GatewayConfig, GatewayHttpState, ProxyBackend, RouterConfig,
    VirtualKeyConfig,
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

fn backend_config_no_auth(name: &str, base_url: String) -> BackendConfig {
    BackendConfig {
        name: name.to_string(),
        base_url,
        max_in_flight: None,
        timeout_seconds: None,
        headers: BTreeMap::new(),
        query_params: BTreeMap::new(),
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
        client = client.with_query_params(backend.query_params.clone());
        client = client.with_request_timeout_seconds(backend.timeout_seconds);
        out.insert(backend.name.clone(), client);
    }
    Ok(out)
}

#[tokio::test]
async fn anthropic_messages_proxy_translates_to_openai_chat_completions() {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return;
    }
    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/chat/completions")
            .header("authorization", "Bearer sk-test")
            .header("x-request-id", "req-1")
            .json_body(json!({
                "model": "gpt-4o-mini",
                "messages": [{"role":"user","content":[{"type":"text","text":"hi"}]}],
                "stream": false,
                "max_tokens": 64
            }));
        then.status(200)
            .header("content-type", "application/json")
            .body(
                json!({
                    "id": "chatcmpl_123",
                    "model": "gpt-4o-mini",
                    "choices": [{
                        "message": {"role":"assistant","content":"hello"},
                        "finish_reason": "stop"
                    }],
                    "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3}
                })
                .to_string(),
            );
    });

    let config = GatewayConfig {
        backends: vec![backend_config(
            "primary",
            upstream.base_url(),
            "Bearer sk-test",
        )],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
        router: RouterConfig {
            default_backend: "primary".to_string(),
            default_backends: Vec::new(),
            rules: Vec::new(),
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway).with_proxy_backends(proxy_backends);
    let app = ditto_llm::gateway::http::router(state);

    let body = json!({
        "model": "gpt-4o-mini",
        "max_tokens": 64,
        "stream": false,
        "messages": [{"role":"user","content":[{"type":"text","text":"hi"}]}]
    });

    let request = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("x-api-key", "vk-1")
        .header("x-request-id", "req-1")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-ditto-request-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default(),
        "req-1"
    );

    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(value.get("type").and_then(|v| v.as_str()), Some("message"));
    assert_eq!(
        value.get("role").and_then(|v| v.as_str()),
        Some("assistant")
    );
    assert_eq!(
        value.get("id").and_then(|v| v.as_str()),
        Some("chatcmpl_123")
    );
    assert_eq!(
        value
            .get("content")
            .and_then(|v| v.as_array())
            .and_then(|items| items.first())
            .and_then(|item| item.get("text"))
            .and_then(|v| v.as_str()),
        Some("hello")
    );
    assert_eq!(
        value
            .get("usage")
            .and_then(|v| v.get("input_tokens"))
            .and_then(|v| v.as_u64()),
        Some(1)
    );

    mock.assert();
}

#[tokio::test]
async fn anthropic_messages_proxy_forwards_x_api_key_when_virtual_keys_disabled() {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return;
    }
    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/chat/completions")
            .header("authorization", "Bearer sk-upstream")
            .header("x-request-id", "req-1")
            .json_body(json!({
                "model": "gpt-4o-mini",
                "messages": [{"role":"user","content":[{"type":"text","text":"hi"}]}],
                "stream": false,
                "max_tokens": 64
            }));
        then.status(200)
            .header("content-type", "application/json")
            .body(
                json!({
                    "id": "chatcmpl_123",
                    "model": "gpt-4o-mini",
                    "choices": [{
                        "message": {"role":"assistant","content":"hello"},
                        "finish_reason": "stop"
                    }],
                    "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3}
                })
                .to_string(),
            );
    });

    let config = GatewayConfig {
        backends: vec![backend_config_no_auth("primary", upstream.base_url())],
        virtual_keys: Vec::new(),
        router: RouterConfig {
            default_backend: "primary".to_string(),
            default_backends: Vec::new(),
            rules: Vec::new(),
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway).with_proxy_backends(proxy_backends);
    let app = ditto_llm::gateway::http::router(state);

    let body = json!({
        "model": "gpt-4o-mini",
        "max_tokens": 64,
        "stream": false,
        "messages": [{"role":"user","content":[{"type":"text","text":"hi"}]}]
    });

    let request = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("x-api-key", "sk-upstream")
        .header("x-request-id", "req-1")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    mock.assert();
}

#[tokio::test]
async fn anthropic_messages_proxy_accepts_messages_alias() {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return;
    }
    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/chat/completions")
            .header("authorization", "Bearer sk-test")
            .header("x-request-id", "req-1")
            .json_body(json!({
                "model": "gpt-4o-mini",
                "messages": [{"role":"user","content":[{"type":"text","text":"hi"}]}],
                "stream": false,
                "max_tokens": 64
            }));
        then.status(200)
            .header("content-type", "application/json")
            .body(
                json!({
                    "id": "chatcmpl_123",
                    "model": "gpt-4o-mini",
                    "choices": [{
                        "message": {"role":"assistant","content":"hello"},
                        "finish_reason": "stop"
                    }],
                    "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3}
                })
                .to_string(),
            );
    });

    let config = GatewayConfig {
        backends: vec![backend_config(
            "primary",
            upstream.base_url(),
            "Bearer sk-test",
        )],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
        router: RouterConfig {
            default_backend: "primary".to_string(),
            default_backends: Vec::new(),
            rules: Vec::new(),
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway).with_proxy_backends(proxy_backends);
    let app = ditto_llm::gateway::http::router(state);

    let body = json!({
        "model": "gpt-4o-mini",
        "max_tokens": 64,
        "stream": false,
        "messages": [{"role":"user","content":[{"type":"text","text":"hi"}]}]
    });

    let request = Request::builder()
        .method("POST")
        .uri("/messages")
        .header("x-litellm-api-key", "Bearer vk-1")
        .header("x-request-id", "req-1")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(value.get("type").and_then(|v| v.as_str()), Some("message"));
    assert_eq!(
        value.get("role").and_then(|v| v.as_str()),
        Some("assistant")
    );
    assert_eq!(
        value.get("id").and_then(|v| v.as_str()),
        Some("chatcmpl_123")
    );

    mock.assert();
}

#[tokio::test]
async fn anthropic_count_tokens_accepts_messages_alias() {
    let config = GatewayConfig {
        backends: Vec::new(),
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
        router: RouterConfig {
            default_backend: "primary".to_string(),
            default_backends: Vec::new(),
            rules: Vec::new(),
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
    };
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway);
    let app = ditto_llm::gateway::http::router(state);

    let body = json!({
        "model": "gpt-4o-mini",
        "messages": [{"role":"user","content":[{"type":"text","text":"hi"}]}]
    });

    let request = Request::builder()
        .method("POST")
        .uri("/messages/count_tokens")
        .header("x-litellm-api-key", "Bearer vk-1")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    assert!(
        value
            .get("input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            > 0
    );
}

#[tokio::test]
async fn anthropic_messages_streaming_translates_openai_sse() {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return;
    }
    let upstream = MockServer::start();
    let sse = concat!(
        "data: {\"id\":\"chatcmpl_123\",\"model\":\"gpt-4o-mini\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"he\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"chatcmpl_123\",\"model\":\"gpt-4o-mini\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"llo\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":2,\"total_tokens\":3}}\n\n",
        "data: [DONE]\n\n",
    );
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/chat/completions")
            .header("authorization", "Bearer sk-test")
            .header("x-request-id", "req-1")
            .body_includes("\"stream\":true");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(sse);
    });

    let config = GatewayConfig {
        backends: vec![backend_config(
            "primary",
            upstream.base_url(),
            "Bearer sk-test",
        )],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
        router: RouterConfig {
            default_backend: "primary".to_string(),
            default_backends: Vec::new(),
            rules: Vec::new(),
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway).with_proxy_backends(proxy_backends);
    let app = ditto_llm::gateway::http::router(state);

    let body = json!({
        "model": "gpt-4o-mini",
        "max_tokens": 64,
        "stream": true,
        "messages": [{"role":"user","content":[{"type":"text","text":"hi"}]}]
    });

    let request = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("x-api-key", "vk-1")
        .header("x-request-id", "req-1")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default(),
        "text/event-stream"
    );

    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("message_start"));
    assert!(text.contains("text_delta"));
    assert!(text.contains("he"));
    assert!(text.contains("llo"));
    assert!(text.contains("message_stop"));

    mock.assert();
}

#[tokio::test]
async fn google_generate_content_forwards_x_goog_api_key_when_virtual_keys_disabled() {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return;
    }
    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/chat/completions")
            .header("authorization", "Bearer sk-upstream")
            .json_body(json!({
                "model": "gemini-pro",
                "messages": [{"role":"user","content":"hi"}],
                "stream": false,
                "max_tokens": 64
            }));
        then.status(200)
            .header("content-type", "application/json")
            .body(
                json!({
                    "id": "chatcmpl_123",
                    "model": "gemini-pro",
                    "choices": [{
                        "message": {"role":"assistant","content":"hello"},
                        "finish_reason": "stop"
                    }],
                    "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3}
                })
                .to_string(),
            );
    });

    let config = GatewayConfig {
        backends: vec![backend_config_no_auth("primary", upstream.base_url())],
        virtual_keys: Vec::new(),
        router: RouterConfig {
            default_backend: "primary".to_string(),
            default_backends: Vec::new(),
            rules: Vec::new(),
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway).with_proxy_backends(proxy_backends);
    let app = ditto_llm::gateway::http::router(state);

    let body = json!({
        "contents": [{"role":"user","parts":[{"text":"hi"}]}],
        "generationConfig": { "maxOutputTokens": 64 }
    });

    let request = Request::builder()
        .method("POST")
        .uri("/v1beta/models/gemini-pro:generateContent")
        .header("x-goog-api-key", "sk-upstream")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    mock.assert();
}

#[tokio::test]
async fn google_generate_content_forwards_query_key_when_virtual_keys_disabled() {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return;
    }
    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/chat/completions")
            .header("authorization", "Bearer sk-upstream")
            .json_body(json!({
                "model": "gemini-pro",
                "messages": [{"role":"user","content":"hi"}],
                "stream": false,
                "max_tokens": 64
            }));
        then.status(200)
            .header("content-type", "application/json")
            .body(
                json!({
                    "id": "chatcmpl_123",
                    "model": "gemini-pro",
                    "choices": [{
                        "message": {"role":"assistant","content":"hello"},
                        "finish_reason": "stop"
                    }],
                    "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3}
                })
                .to_string(),
            );
    });

    let config = GatewayConfig {
        backends: vec![backend_config_no_auth("primary", upstream.base_url())],
        virtual_keys: Vec::new(),
        router: RouterConfig {
            default_backend: "primary".to_string(),
            default_backends: Vec::new(),
            rules: Vec::new(),
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway).with_proxy_backends(proxy_backends);
    let app = ditto_llm::gateway::http::router(state);

    let body = json!({
        "contents": [{"role":"user","parts":[{"text":"hi"}]}],
        "generationConfig": { "maxOutputTokens": 64 }
    });

    let request = Request::builder()
        .method("POST")
        .uri("/v1beta/models/gemini-pro:generateContent?key=sk-upstream")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    mock.assert();
}

#[tokio::test]
async fn google_generate_content_translates_to_openai_chat_completions() {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return;
    }
    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/chat/completions")
            .header("authorization", "Bearer sk-test")
            .json_body(json!({
                "model": "gemini-pro",
                "messages": [{"role":"user","content":"hi"}],
                "stream": false,
                "max_tokens": 64
            }));
        then.status(200)
            .header("content-type", "application/json")
            .body(
                json!({
                    "id": "chatcmpl_123",
                    "model": "gemini-pro",
                    "choices": [{
                        "message": {"role":"assistant","content":"hello"},
                        "finish_reason": "stop"
                    }],
                    "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3}
                })
                .to_string(),
            );
    });

    let config = GatewayConfig {
        backends: vec![backend_config(
            "primary",
            upstream.base_url(),
            "Bearer sk-test",
        )],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
        router: RouterConfig {
            default_backend: "primary".to_string(),
            default_backends: Vec::new(),
            rules: Vec::new(),
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway).with_proxy_backends(proxy_backends);
    let app = ditto_llm::gateway::http::router(state);

    let body = json!({
        "contents": [{"role":"user","parts":[{"text":"hi"}]}],
        "generationConfig": { "maxOutputTokens": 64 }
    });

    let request = Request::builder()
        .method("POST")
        .uri("/v1beta/models/gemini-pro:generateContent")
        .header("x-goog-api-key", "vk-1")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(
        value
            .get("candidates")
            .and_then(|v| v.as_array())
            .and_then(|items| items.first())
            .and_then(|c| c.get("content"))
            .and_then(|c| c.get("parts"))
            .and_then(|p| p.as_array())
            .and_then(|p| p.first())
            .and_then(|p| p.get("text"))
            .and_then(|v| v.as_str()),
        Some("hello")
    );
    assert_eq!(
        value
            .get("usageMetadata")
            .and_then(|v| v.get("promptTokenCount"))
            .and_then(|v| v.as_u64()),
        Some(1)
    );

    mock.assert();
}

#[tokio::test]
async fn google_generate_content_accepts_virtual_key_via_query_param() {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return;
    }
    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/chat/completions")
            .header("authorization", "Bearer sk-test")
            .json_body(json!({
                "model": "gemini-pro",
                "messages": [{"role":"user","content":"hi"}],
                "stream": false,
                "max_tokens": 64
            }));
        then.status(200)
            .header("content-type", "application/json")
            .body(
                json!({
                    "id": "chatcmpl_123",
                    "model": "gemini-pro",
                    "choices": [{
                        "message": {"role":"assistant","content":"hello"},
                        "finish_reason": "stop"
                    }],
                    "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3}
                })
                .to_string(),
            );
    });

    let config = GatewayConfig {
        backends: vec![backend_config(
            "primary",
            upstream.base_url(),
            "Bearer sk-test",
        )],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
        router: RouterConfig {
            default_backend: "primary".to_string(),
            default_backends: Vec::new(),
            rules: Vec::new(),
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway).with_proxy_backends(proxy_backends);
    let app = ditto_llm::gateway::http::router(state);

    let body = json!({
        "contents": [{"role":"user","parts":[{"text":"hi"}]}],
        "generationConfig": { "maxOutputTokens": 64 }
    });

    let request = Request::builder()
        .method("POST")
        .uri("/v1beta/models/gemini-pro:generateContent?key=vk-1")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(
        value
            .get("candidates")
            .and_then(|v| v.as_array())
            .and_then(|items| items.first())
            .and_then(|c| c.get("content"))
            .and_then(|c| c.get("parts"))
            .and_then(|p| p.as_array())
            .and_then(|p| p.first())
            .and_then(|p| p.get("text"))
            .and_then(|v| v.as_str()),
        Some("hello")
    );

    mock.assert();
}

#[tokio::test]
async fn cloudcode_generate_content_wraps_google_format() {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return;
    }
    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/chat/completions")
            .header("authorization", "Bearer sk-test")
            .json_body(json!({
                "model": "gemini-pro",
                "messages": [{"role":"user","content":"hi"}],
                "stream": false,
                "max_tokens": 64
            }));
        then.status(200)
            .header("content-type", "application/json")
            .body(
                json!({
                    "id": "chatcmpl_123",
                    "model": "gemini-pro",
                    "choices": [{
                        "message": {"role":"assistant","content":"hello"},
                        "finish_reason": "stop"
                    }],
                    "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3}
                })
                .to_string(),
            );
    });

    let config = GatewayConfig {
        backends: vec![backend_config(
            "primary",
            upstream.base_url(),
            "Bearer sk-test",
        )],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
        router: RouterConfig {
            default_backend: "primary".to_string(),
            default_backends: Vec::new(),
            rules: Vec::new(),
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway).with_proxy_backends(proxy_backends);
    let app = ditto_llm::gateway::http::router(state);

    let body = json!({
        "model": "gemini-pro",
        "request": {
            "contents": [{"role":"user","parts":[{"text":"hi"}]}],
            "generationConfig": { "maxOutputTokens": 64 }
        }
    });

    let request = Request::builder()
        .method("POST")
        .uri("/v1internal:generateContent")
        .header("x-goog-api-key", "vk-1")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(
        value
            .get("response")
            .and_then(|v| v.get("responseId"))
            .and_then(|v| v.as_str()),
        Some("chatcmpl_123")
    );
    assert_eq!(
        value
            .get("response")
            .and_then(|v| v.get("candidates"))
            .and_then(|v| v.as_array())
            .and_then(|items| items.first())
            .and_then(|c| c.get("content"))
            .and_then(|c| c.get("parts"))
            .and_then(|p| p.as_array())
            .and_then(|p| p.first())
            .and_then(|p| p.get("text"))
            .and_then(|v| v.as_str()),
        Some("hello")
    );

    mock.assert();
}
