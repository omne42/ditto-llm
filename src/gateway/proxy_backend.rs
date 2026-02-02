use std::collections::BTreeMap;
use std::time::Duration;

use axum::http::HeaderMap;
use bytes::Bytes;
use reqwest::Body as ReqwestBody;

use super::GatewayError;

#[derive(Clone)]
pub struct ProxyBackend {
    base_url: String,
    client: reqwest::Client,
    headers: HeaderMap,
    query_params: BTreeMap<String, String>,
    request_timeout: Option<Duration>,
}

impl ProxyBackend {
    pub fn new(base_url: impl Into<String>) -> Result<Self, GatewayError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .map_err(|err| GatewayError::Backend {
                message: format!("backend http client error: {err}"),
            })?;
        Ok(Self {
            base_url: base_url.into(),
            client,
            headers: HeaderMap::new(),
            query_params: BTreeMap::new(),
            request_timeout: None,
        })
    }

    pub fn with_request_timeout_seconds(mut self, timeout_seconds: Option<u64>) -> Self {
        self.request_timeout = timeout_seconds
            .filter(|seconds| *seconds > 0)
            .map(Duration::from_secs);
        self
    }

    pub fn with_headers(mut self, headers: BTreeMap<String, String>) -> Result<Self, GatewayError> {
        self.headers = parse_headers(&headers)?;
        Ok(self)
    }

    pub fn with_query_params(mut self, params: BTreeMap<String, String>) -> Self {
        self.query_params = normalize_query_params(&params);
        self
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        headers: HeaderMap,
        body: Option<Bytes>,
    ) -> Result<reqwest::Response, GatewayError> {
        self.request_with_timeout(method, path, headers, body, None)
            .await
    }

    pub async fn request_with_timeout(
        &self,
        method: reqwest::Method,
        path: &str,
        headers: HeaderMap,
        body: Option<Bytes>,
        timeout: Option<Duration>,
    ) -> Result<reqwest::Response, GatewayError> {
        self.request_with_timeout_body(method, path, headers, body.map(ReqwestBody::from), timeout)
            .await
    }

    pub async fn request_stream(
        &self,
        method: reqwest::Method,
        path: &str,
        headers: HeaderMap,
        body: Option<ReqwestBody>,
    ) -> Result<reqwest::Response, GatewayError> {
        self.request_with_timeout_body(method, path, headers, body, None)
            .await
    }

    pub async fn request_with_timeout_body(
        &self,
        method: reqwest::Method,
        path: &str,
        headers: HeaderMap,
        body: Option<ReqwestBody>,
        timeout: Option<Duration>,
    ) -> Result<reqwest::Response, GatewayError> {
        let url = join_base_url(&self.base_url, path);
        let mut req = self.client.request(method, url).headers(headers);
        if !self.query_params.is_empty() {
            req = req.query(&self.query_params);
        }
        let timeout = timeout.or(self.request_timeout);
        if let Some(timeout) = timeout {
            req = req.timeout(timeout);
        }
        if let Some(body) = body {
            req = req.body(body);
        }
        req.send().await.map_err(|err| GatewayError::Backend {
            message: format!("backend request failed: {err}"),
        })
    }
}

fn join_base_url(base_url: &str, path: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let mut suffix = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };

    // Common ergonomics: allow base_url to include /v1 and still accept /v1/* paths.
    if base.ends_with("/v1") && suffix.starts_with("/v1/") {
        suffix = suffix.trim_start_matches("/v1").to_string();
    }

    format!("{base}{suffix}")
}

fn parse_headers(headers: &BTreeMap<String, String>) -> Result<HeaderMap, GatewayError> {
    let mut out = HeaderMap::new();
    for (name, value) in headers {
        let header_name =
            name.parse::<axum::http::HeaderName>()
                .map_err(|_| GatewayError::InvalidRequest {
                    reason: format!("invalid header name: {name}"),
                })?;
        let header_value =
            value
                .parse::<axum::http::HeaderValue>()
                .map_err(|_| GatewayError::InvalidRequest {
                    reason: format!("invalid header value for {name}"),
                })?;
        out.insert(header_name, header_value);
    }
    Ok(out)
}

fn normalize_query_params(params: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    params
        .iter()
        .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
        .filter(|(name, _)| !name.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_base_url_handles_v1_suffix() {
        assert_eq!(
            join_base_url("http://localhost:8080/v1", "/v1/chat/completions"),
            "http://localhost:8080/v1/chat/completions"
        );
        assert_eq!(
            join_base_url("http://localhost:8080", "/v1/chat/completions"),
            "http://localhost:8080/v1/chat/completions"
        );
        assert_eq!(
            join_base_url("http://localhost:8080/v1/", "v1/chat/completions"),
            "http://localhost:8080/v1/chat/completions"
        );
    }
}
