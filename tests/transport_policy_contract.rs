use ditto_llm::provider_transport::HttpTransportPolicy;
use ditto_llm::session_transport::{
    SessionTransportPolicy, SseLimits, WebsocketBaseUrlRewrite, resolve_websocket_base_url,
};

#[test]
fn provider_transport_exposes_machine_readable_default_policy() {
    let policy = HttpTransportPolicy::default();
    assert_eq!(policy.client.timeout_ms, 300_000);
    assert_eq!(policy.body.max_error_body_bytes, 64 * 1024);
    assert_eq!(policy.body.max_response_body_bytes, 16 * 1024 * 1024);

    let json = serde_json::to_value(policy).expect("policy should serialize");
    assert_eq!(json["client"]["timeout_ms"], 300_000);
}

#[test]
fn session_transport_exposes_rewrite_resolution_and_default_sse_policy() {
    let secure = resolve_websocket_base_url("https://api.openai.com/v1");
    assert_eq!(secure.base_url, "wss://api.openai.com/v1");
    assert_eq!(
        secure.rewrite,
        Some(WebsocketBaseUrlRewrite::HttpsToSecureWebsocket)
    );

    let insecure = resolve_websocket_base_url("http://localhost:8080/v1");
    assert_eq!(insecure.base_url, "ws://localhost:8080/v1");
    assert_eq!(
        insecure.rewrite,
        Some(WebsocketBaseUrlRewrite::HttpToWebsocket)
    );

    let policy = SessionTransportPolicy::default();
    assert_eq!(policy.sse, SseLimits::default());
}
