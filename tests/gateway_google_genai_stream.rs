#![cfg(all(feature = "gateway", feature = "streaming"))]

use std::collections::{BTreeMap, HashMap};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use ditto_llm::gateway::{
    BackendConfig, Gateway, GatewayConfig, GatewayHttpState, ProxyBackend, RouteBackend,
    RouterConfig, VirtualKeyConfig,
};
use httpmock::Method::POST;
use httpmock::MockServer;
use serde_json::{Value, json};
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

fn parse_sse_json_events(body: &str) -> Vec<Value> {
    body.split("\n\n")
        .filter_map(|event| {
            let line = event.lines().find(|line| line.starts_with("data:"))?;
            let data = line.trim_start_matches("data:").trim();
            if data.is_empty() || data == "[DONE]" {
                return None;
            }
            serde_json::from_str::<Value>(data).ok()
        })
        .collect()
}

#[tokio::test]
async fn gateway_google_genai_stream_chunks_are_incremental() {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return;
    }

    let upstream = MockServer::start();
    upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/chat/completions")
            .header("authorization", "Bearer sk-test");
        then.status(200).header("content-type", "text/event-stream").body(concat!(
            "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-4o-mini\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"he\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-4o-mini\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"llo\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-4o-mini\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":2,\"total_tokens\":3}}\n\n",
            "data: [DONE]\n\n",
        ));
    });

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
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway).with_proxy_backends(proxy_backends);
    let app = ditto_llm::gateway::http::router(state);

    let request_json = json!({
        "contents": [{"role":"user","parts":[{"text":"hi"}]}],
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1beta/models/gemini-pro:streamGenerateContent")
        .header("authorization", "Bearer vk-1")
        .header("content-type", "application/json")
        .body(Body::from(request_json.to_string()))
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
    let body = String::from_utf8_lossy(&bytes);
    let events = parse_sse_json_events(&body);
    assert_eq!(events.len(), 3, "expected 2 deltas + final chunk: {body}");

    let mut text_chunks = Vec::<String>::new();
    for event in &events {
        let Some(text) = event
            .get("candidates")
            .and_then(Value::as_array)
            .and_then(|c| c.first())
            .and_then(|c| c.get("content"))
            .and_then(|c| c.get("parts"))
            .and_then(Value::as_array)
            .and_then(|p| p.first())
            .and_then(|p| p.get("text"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        text_chunks.push(text.to_string());
    }

    assert_eq!(text_chunks, vec!["he", "llo"]);
    assert_eq!(text_chunks.join(""), "hello");

    let final_event = events.last().expect("final event");
    let candidate = final_event
        .get("candidates")
        .and_then(Value::as_array)
        .and_then(|c| c.first())
        .expect("final candidates[0]");
    assert_eq!(
        candidate.get("finishReason").and_then(Value::as_str),
        Some("STOP")
    );
    assert_eq!(
        candidate
            .get("content")
            .and_then(|c| c.get("parts"))
            .and_then(Value::as_array)
            .map(|p| p.len()),
        Some(0),
        "final chunk should include empty content.parts: {final_event}"
    );
    assert_eq!(
        final_event
            .get("usageMetadata")
            .and_then(|u| u.get("totalTokenCount"))
            .and_then(Value::as_u64),
        Some(3)
    );
    assert!(!body.contains("[DONE]"));
}
