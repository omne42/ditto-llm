#[cfg(test)]
mod sanitize_proxy_headers_tests {
    use super::{HeaderMap, sanitize_proxy_headers};

    #[test]
    fn removes_hop_by_hop_and_proxy_auth_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "proxy-authorization",
            axum::http::HeaderValue::from_static("proxy-secret"),
        );
        headers.insert(
            "x-forwarded-authorization",
            axum::http::HeaderValue::from_static("forwarded-secret"),
        );
        headers.insert("connection", axum::http::HeaderValue::from_static("keep-alive"));
        headers.insert("keep-alive", axum::http::HeaderValue::from_static("timeout=5"));
        headers.insert(
            "proxy-authenticate",
            axum::http::HeaderValue::from_static("Basic realm=\"test\""),
        );
        headers.insert(
            "proxy-connection",
            axum::http::HeaderValue::from_static("keep-alive"),
        );
        headers.insert("te", axum::http::HeaderValue::from_static("trailers"));
        headers.insert("trailer", axum::http::HeaderValue::from_static("some-trailer"));
        headers.insert(
            "transfer-encoding",
            axum::http::HeaderValue::from_static("chunked"),
        );
        headers.insert("upgrade", axum::http::HeaderValue::from_static("websocket"));
        headers.insert("content-length", axum::http::HeaderValue::from_static("123"));
        headers.insert(
            "authorization",
            axum::http::HeaderValue::from_static("Bearer abc"),
        );
        headers.insert("x-api-key", axum::http::HeaderValue::from_static("abc"));
        headers.insert(
            "x-litellm-api-key",
            axum::http::HeaderValue::from_static("Bearer abc"),
        );
        headers.insert("x-test", axum::http::HeaderValue::from_static("ok"));

        sanitize_proxy_headers(&mut headers, false);

        for name in [
            "proxy-authorization",
            "x-forwarded-authorization",
            "connection",
            "keep-alive",
            "proxy-authenticate",
            "proxy-connection",
            "te",
            "trailer",
            "transfer-encoding",
            "upgrade",
            "content-length",
        ] {
            assert!(headers.get(name).is_none(), "{name} should be removed");
        }

        assert!(headers.get("authorization").is_some());
        assert!(headers.get("x-api-key").is_some());
        assert!(headers.get("x-litellm-api-key").is_some());
        assert_eq!(headers.get("x-test").unwrap().to_str().unwrap(), "ok");
    }

    #[test]
    fn strips_authorization_and_api_key_when_requested() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            axum::http::HeaderValue::from_static("Bearer abc"),
        );
        headers.insert("x-api-key", axum::http::HeaderValue::from_static("abc"));
        headers.insert(
            "x-litellm-api-key",
            axum::http::HeaderValue::from_static("Bearer abc"),
        );
        headers.insert("x-test", axum::http::HeaderValue::from_static("ok"));

        sanitize_proxy_headers(&mut headers, true);

        assert!(headers.get("authorization").is_none());
        assert!(headers.get("x-api-key").is_none());
        assert!(headers.get("x-litellm-api-key").is_none());
        assert_eq!(headers.get("x-test").unwrap().to_str().unwrap(), "ok");
    }
}
