use serde::{Deserialize, Serialize};

use super::sse::SseLimits;

// SESSION-TRANSPORT-POLICY-OWNER: session/frame limits and websocket rewrite
// outcomes live here as explicit session transport semantics.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebsocketBaseUrlRewrite {
    HttpToWebsocket,
    HttpsToSecureWebsocket,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebsocketBaseUrlResolution {
    pub base_url: String,
    pub rewrite: Option<WebsocketBaseUrlRewrite>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionTransportPolicy {
    pub sse: SseLimits,
}

pub fn resolve_websocket_base_url(base_url: &str) -> WebsocketBaseUrlResolution {
    let base_url = base_url.trim();
    if let Some(rest) = base_url.strip_prefix("https://") {
        return WebsocketBaseUrlResolution {
            base_url: format!("wss://{rest}"),
            rewrite: Some(WebsocketBaseUrlRewrite::HttpsToSecureWebsocket),
        };
    }
    if let Some(rest) = base_url.strip_prefix("http://") {
        return WebsocketBaseUrlResolution {
            base_url: format!("ws://{rest}"),
            rewrite: Some(WebsocketBaseUrlRewrite::HttpToWebsocket),
        };
    }
    WebsocketBaseUrlResolution {
        base_url: base_url.to_string(),
        rewrite: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_websocket_base_url_reports_rewrite_kind() {
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

        let passthrough = resolve_websocket_base_url("wss://proxy.example/v1");
        assert_eq!(passthrough.base_url, "wss://proxy.example/v1");
        assert_eq!(passthrough.rewrite, None);
    }
}
