#![cfg(feature = "gateway")]

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use ditto_llm::gateway::{
    Gateway, GatewayConfig, GatewayHttpState, RouteBackend, RouterConfig, VirtualKeyConfig,
};
use httpmock::Method::POST;
use httpmock::MockServer;
use serde_json::{Value, json};
use tower::util::ServiceExt;

fn base_config() -> GatewayConfig {
    GatewayConfig {
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
    }
}

#[tokio::test]
async fn gateway_mcp_jsonrpc_tools_list_proxies_and_returns_tools() -> ditto_llm::Result<()> {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return Ok(());
    }

    let upstream = MockServer::start();
    let upstream_mock = upstream.mock(|when, then| {
        when.method(POST).path("/mcp").json_body(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {},
        }));
        then.status(200)
            .header("content-type", "application/json")
            .body(
                json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": {
                        "tools": [{
                            "name": "hello",
                            "description": "hi",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "who": { "type": "string" }
                                }
                            }
                        }]
                    }
                })
                .to_string(),
            );
    });

    let mut mcp_servers = HashMap::new();
    mcp_servers.insert(
        "local".to_string(),
        ditto_llm::gateway::http::McpServerState::new("local".to_string(), upstream.url("/mcp"))
            .expect("mcp state"),
    );

    let gateway = Gateway::new(base_config());
    let state = GatewayHttpState::new(gateway).with_mcp_servers(mcp_servers);
    let app = ditto_llm::gateway::http::router(state);

    let request = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/list",
            })
            .to_string(),
        ))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let payload: Value = serde_json::from_slice(&bytes)?;
    assert_eq!(payload.get("jsonrpc").and_then(|v| v.as_str()), Some("2.0"));
    assert_eq!(
        payload
            .get("result")
            .and_then(|v| v.get("tools"))
            .and_then(|v| v.get(0))
            .and_then(|v| v.get("name"))
            .and_then(|v| v.as_str()),
        Some("hello")
    );

    upstream_mock.assert();
    Ok(())
}

#[tokio::test]
async fn gateway_mcp_tools_list_autopaginates_until_complete() -> ditto_llm::Result<()> {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return Ok(());
    }

    let upstream = MockServer::start();
    let page_1 = upstream.mock(|when, then| {
        when.method(POST).path("/mcp").json_body(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {},
        }));
        then.status(200)
            .header("content-type", "application/json")
            .body(
                json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": { "tools": [{ "name": "t1", "inputSchema": {} }], "nextCursor": "c1" }
                })
                .to_string(),
            );
    });
    let page_2 = upstream.mock(|when, then| {
        when.method(POST).path("/mcp").json_body(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": { "cursor": "c1" },
        }));
        then.status(200)
            .header("content-type", "application/json")
            .body(
                json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": { "tools": [{ "name": "t2", "inputSchema": {} }] }
                })
                .to_string(),
            );
    });

    let mut mcp_servers = HashMap::new();
    mcp_servers.insert(
        "local".to_string(),
        ditto_llm::gateway::http::McpServerState::new("local".to_string(), upstream.url("/mcp"))
            .expect("mcp state"),
    );

    let gateway = Gateway::new(base_config());
    let state = GatewayHttpState::new(gateway).with_mcp_servers(mcp_servers);
    let app = ditto_llm::gateway::http::router(state);

    let request = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/list",
            })
            .to_string(),
        ))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let payload: Value = serde_json::from_slice(&bytes)?;

    let tool_names: Vec<String> = payload
        .get("result")
        .and_then(|v| v.get("tools"))
        .and_then(|v| v.as_array())
        .map(|arr| arr.as_slice())
        .unwrap_or(&[])
        .iter()
        .filter_map(|tool| tool.get("name").and_then(|v| v.as_str()))
        .map(|s| s.to_string())
        .collect();
    assert_eq!(tool_names, vec!["t1".to_string(), "t2".to_string()]);
    assert!(
        payload
            .get("result")
            .and_then(|v| v.get("nextCursor"))
            .is_none()
    );

    page_1.assert();
    page_2.assert();
    Ok(())
}

#[tokio::test]
async fn gateway_mcp_tools_list_with_cursor_returns_next_cursor() -> ditto_llm::Result<()> {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return Ok(());
    }

    let upstream = MockServer::start();
    let page = upstream.mock(|when, then| {
        when.method(POST).path("/mcp").json_body(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": { "cursor": "c1" },
        }));
        then.status(200)
            .header("content-type", "application/json")
            .body(
                json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": { "tools": [{ "name": "t2", "inputSchema": {} }], "nextCursor": "c2" }
                })
                .to_string(),
            );
    });

    let mut mcp_servers = HashMap::new();
    mcp_servers.insert(
        "local".to_string(),
        ditto_llm::gateway::http::McpServerState::new("local".to_string(), upstream.url("/mcp"))
            .expect("mcp state"),
    );

    let gateway = Gateway::new(base_config());
    let state = GatewayHttpState::new(gateway).with_mcp_servers(mcp_servers);
    let app = ditto_llm::gateway::http::router(state);

    let request = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/list",
                "params": { "cursor": "c1" }
            })
            .to_string(),
        ))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let payload: Value = serde_json::from_slice(&bytes)?;

    assert_eq!(
        payload
            .get("result")
            .and_then(|v| v.get("tools"))
            .and_then(|v| v.get(0))
            .and_then(|v| v.get("name"))
            .and_then(|v| v.as_str()),
        Some("t2")
    );
    assert_eq!(
        payload
            .get("result")
            .and_then(|v| v.get("nextCursor"))
            .and_then(|v| v.as_str()),
        Some("c2")
    );

    page.assert();
    Ok(())
}

#[tokio::test]
async fn gateway_mcp_tools_list_rejects_cursor_when_multiple_servers_selected()
-> ditto_llm::Result<()> {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return Ok(());
    }

    let upstream_a = MockServer::start();
    let upstream_b = MockServer::start();

    let mut mcp_servers = HashMap::new();
    mcp_servers.insert(
        "a".to_string(),
        ditto_llm::gateway::http::McpServerState::new("a".to_string(), upstream_a.url("/mcp"))
            .expect("mcp state a"),
    );
    mcp_servers.insert(
        "b".to_string(),
        ditto_llm::gateway::http::McpServerState::new("b".to_string(), upstream_b.url("/mcp"))
            .expect("mcp state b"),
    );

    let gateway = Gateway::new(base_config());
    let state = GatewayHttpState::new(gateway).with_mcp_servers(mcp_servers);
    let app = ditto_llm::gateway::http::router(state);

    let request = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("x-mcp-servers", "a,b")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/list",
                "params": { "cursor": "c1" }
            })
            .to_string(),
        ))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let payload: Value = serde_json::from_slice(&bytes)?;

    assert_eq!(
        payload
            .get("error")
            .and_then(|v| v.get("code"))
            .and_then(|v| v.as_i64()),
        Some(-32000)
    );
    let message = payload
        .get("error")
        .and_then(|v| v.get("message"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(message.contains("cursor is only supported"), "{message}");
    Ok(())
}

#[tokio::test]
async fn gateway_mcp_prefixes_tool_names_when_multiple_servers_selected() -> ditto_llm::Result<()> {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return Ok(());
    }

    let upstream_a = MockServer::start();
    let mock_a = upstream_a.mock(|when, then| {
        when.method(POST).path("/mcp").json_body(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {},
        }));
        then.status(200)
            .header("content-type", "application/json")
            .body(
                json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": { "tools": [{ "name": "hello", "inputSchema": {} }] }
                })
                .to_string(),
            );
    });

    let upstream_b = MockServer::start();
    let mock_b = upstream_b.mock(|when, then| {
        when.method(POST).path("/mcp").json_body(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {},
        }));
        then.status(200)
            .header("content-type", "application/json")
            .body(
                json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": { "tools": [{ "name": "hello", "inputSchema": {} }] }
                })
                .to_string(),
            );
    });

    let mut mcp_servers = HashMap::new();
    mcp_servers.insert(
        "a".to_string(),
        ditto_llm::gateway::http::McpServerState::new("a".to_string(), upstream_a.url("/mcp"))
            .expect("mcp state a"),
    );
    mcp_servers.insert(
        "b".to_string(),
        ditto_llm::gateway::http::McpServerState::new("b".to_string(), upstream_b.url("/mcp"))
            .expect("mcp state b"),
    );

    let gateway = Gateway::new(base_config());
    let state = GatewayHttpState::new(gateway).with_mcp_servers(mcp_servers);
    let app = ditto_llm::gateway::http::router(state);

    let request = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("x-mcp-servers", "b,a")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/list",
            })
            .to_string(),
        ))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let payload: Value = serde_json::from_slice(&bytes)?;
    let tool_names: Vec<String> = payload
        .get("result")
        .and_then(|v| v.get("tools"))
        .and_then(|v| v.as_array())
        .map(|arr| arr.as_slice())
        .unwrap_or(&[])
        .iter()
        .filter_map(|tool| {
            tool.get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .collect();
    assert_eq!(
        tool_names,
        vec!["a-hello".to_string(), "b-hello".to_string()]
    );

    mock_a.assert();
    mock_b.assert();
    Ok(())
}

#[tokio::test]
async fn gateway_mcp_bounded_backend_response_prevents_oom() -> ditto_llm::Result<()> {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return Ok(());
    }

    let upstream = MockServer::start();
    let too_large = "x".repeat(6 * 1024 * 1024);
    let upstream_mock = upstream.mock(|when, then| {
        when.method(POST).path("/mcp").json_body(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {},
        }));
        then.status(200)
            .header("content-type", "application/json")
            .body(too_large.clone());
    });

    let mut mcp_servers = HashMap::new();
    mcp_servers.insert(
        "local".to_string(),
        ditto_llm::gateway::http::McpServerState::new("local".to_string(), upstream.url("/mcp"))
            .expect("mcp state"),
    );

    let gateway = Gateway::new(base_config());
    let state = GatewayHttpState::new(gateway).with_mcp_servers(mcp_servers);
    let app = ditto_llm::gateway::http::router(state);

    let request = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/list",
            })
            .to_string(),
        ))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let payload: Value = serde_json::from_slice(&bytes)?;
    assert_eq!(payload.get("jsonrpc").and_then(|v| v.as_str()), Some("2.0"));
    assert_eq!(
        payload
            .get("error")
            .and_then(|v| v.get("code"))
            .and_then(|v| v.as_i64()),
        Some(-32000)
    );
    let message = payload
        .get("error")
        .and_then(|v| v.get("message"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(
        message.contains("max bytes"),
        "unexpected error message: {message}"
    );

    upstream_mock.assert();
    Ok(())
}

#[tokio::test]
async fn gateway_mcp_requires_virtual_key_when_configured() -> ditto_llm::Result<()> {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return Ok(());
    }

    let upstream = MockServer::start();
    let upstream_mock = upstream.mock(|when, then| {
        when.method(POST).path("/mcp").json_body(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {},
        }));
        then.status(200)
            .header("content-type", "application/json")
            .body(
                json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": { "tools": [] }
                })
                .to_string(),
            );
    });

    let mut config = base_config();
    config.virtual_keys = vec![VirtualKeyConfig::new("key-1", "vk-1")];

    let mut mcp_servers = HashMap::new();
    mcp_servers.insert(
        "local".to_string(),
        ditto_llm::gateway::http::McpServerState::new("local".to_string(), upstream.url("/mcp"))
            .expect("mcp state"),
    );

    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway).with_mcp_servers(mcp_servers);
    let app = ditto_llm::gateway::http::router(state);

    let request = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/list",
            })
            .to_string(),
        ))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let request = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("x-litellm-api-key", "Bearer vk-1")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/list",
            })
            .to_string(),
        ))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    upstream_mock.assert();
    Ok(())
}
