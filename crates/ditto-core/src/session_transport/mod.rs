//! L0 boundary: session_transport.
//!
//! This module owns stream/session-level transport semantics:
//! SSE framing, websocket base URL negotiation, and stream bootstrap
//! helpers shared by provider adapters and gateway bridges.

mod policy;
mod sse;
mod streaming;

pub use policy::{
    SessionTransportPolicy, WebsocketBaseUrlResolution, WebsocketBaseUrlRewrite,
    resolve_websocket_base_url,
};
pub use sse::SseLimits;

#[allow(unused_imports)]
pub use sse::{
    sse_data_stream_from_reader, sse_data_stream_from_reader_with_limits,
    sse_data_stream_from_response,
};
#[allow(unused_imports)]
pub(crate) use streaming::init_sse_stream;

#[cfg(feature = "cap-realtime")]
pub(crate) fn to_websocket_base_url(base_url: &str) -> String {
    resolve_websocket_base_url(base_url).base_url
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "cap-realtime")]
    #[test]
    fn to_websocket_base_url_rewrites_http_and_https() {
        assert_eq!(
            super::to_websocket_base_url("https://api.openai.com/v1"),
            "wss://api.openai.com/v1"
        );
        assert_eq!(
            super::to_websocket_base_url("http://localhost:8080/v1"),
            "ws://localhost:8080/v1"
        );
        assert_eq!(
            super::to_websocket_base_url("wss://proxy.example/v1"),
            "wss://proxy.example/v1"
        );
    }
}
