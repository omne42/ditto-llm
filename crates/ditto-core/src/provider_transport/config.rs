#![allow(dead_code)]

use std::collections::BTreeMap;
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

use crate::error::{DittoError, Result};

use super::policy::HttpClientPolicy;

#[cfg(any(
    feature = "provider-anthropic",
    feature = "provider-cohere",
    feature = "provider-google",
    feature = "provider-bedrock",
    feature = "provider-vertex"
))]
pub(crate) const DEFAULT_HTTP_TIMEOUT: Duration =
    Duration::from_secs(super::policy::DEFAULT_HTTP_TIMEOUT_SECS);

pub(crate) fn header_map_from_pairs(headers: &BTreeMap<String, String>) -> Result<HeaderMap> {
    let mut out = HeaderMap::new();
    for (name, value) in headers {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|err| {
            crate::invalid_response!(
                "error_detail.http.header_name_invalid",
                "name" => name,
                "error" => err.to_string()
            )
        })?;

        let header_value = HeaderValue::from_str(value).map_err(|err| {
            crate::invalid_response!(
                "error_detail.http.header_value_invalid",
                "name" => name,
                "value" => value,
                "error" => err.to_string()
            )
        })?;

        out.insert(header_name, header_value);
    }
    Ok(out)
}

pub(crate) fn build_http_client_with_policy(
    policy: HttpClientPolicy,
    headers: &BTreeMap<String, String>,
) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder().timeout(policy.timeout());
    if !headers.is_empty() {
        builder = builder.default_headers(header_map_from_pairs(headers)?);
    }
    builder.build().map_err(DittoError::Http)
}

#[cfg(any(feature = "provider-bedrock", feature = "provider-vertex"))]
pub(crate) fn build_http_client(
    timeout: Duration,
    headers: &BTreeMap<String, String>,
) -> Result<reqwest::Client> {
    build_http_client_with_policy(HttpClientPolicy::from_timeout(timeout), headers)
}

#[cfg(any(
    feature = "provider-anthropic",
    feature = "provider-google",
    feature = "provider-cohere",
    feature = "provider-openai",
    feature = "provider-openai-compatible",
))]
pub(crate) fn default_http_client(timeout: Duration) -> reqwest::Client {
    build_http_client_with_policy(HttpClientPolicy::from_timeout(timeout), &BTreeMap::new())
        .unwrap_or_else(|_| reqwest::Client::new())
}

#[derive(Clone)]
pub(crate) struct ResolvedHttpProviderConfig {
    pub(crate) http: reqwest::Client,
    pub(crate) base_url: Option<String>,
    #[cfg(any(
        feature = "provider-anthropic",
        feature = "provider-cohere",
        feature = "provider-google",
        feature = "provider-bedrock",
        feature = "provider-vertex",
        feature = "provider-openai",
        feature = "provider-openai-compatible",
    ))]
    pub(crate) default_model: Option<String>,
    pub(crate) http_query_params: BTreeMap<String, String>,
}

impl ResolvedHttpProviderConfig {
    #[cfg(any(feature = "provider-bedrock", feature = "provider-vertex"))]
    pub(crate) fn required_base_url(&self) -> Result<&str> {
        self.base_url
            .as_deref()
            .ok_or_else(|| crate::invalid_response!("error_detail.http.provider_base_url_missing"))
    }

    #[cfg(any(feature = "provider-bedrock", feature = "provider-vertex",))]
    pub(crate) fn required_default_model(&self) -> Result<&str> {
        self.default_model.as_deref().ok_or_else(|| {
            crate::invalid_response!("error_detail.http.provider_default_model_missing")
        })
    }
}

pub(crate) fn resolve_http_provider_config_with_policy(
    policy: HttpClientPolicy,
    config: &crate::config::ProviderConfig,
    default_base_url: Option<&str>,
) -> Result<ResolvedHttpProviderConfig> {
    fn clean(value: Option<&str>) -> Option<String> {
        value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }

    Ok(ResolvedHttpProviderConfig {
        http: build_http_client_with_policy(policy, &config.http_headers)?,
        base_url: clean(config.base_url.as_deref()).or_else(|| clean(default_base_url)),
        #[cfg(any(
            feature = "provider-anthropic",
            feature = "provider-cohere",
            feature = "provider-google",
            feature = "provider-bedrock",
            feature = "provider-vertex",
            feature = "provider-openai",
            feature = "provider-openai-compatible",
        ))]
        default_model: clean(config.default_model.as_deref()),
        http_query_params: config.http_query_params.clone(),
    })
}

pub(crate) fn resolve_http_provider_config(
    timeout: Duration,
    config: &crate::config::ProviderConfig,
    default_base_url: Option<&str>,
) -> Result<ResolvedHttpProviderConfig> {
    resolve_http_provider_config_with_policy(
        HttpClientPolicy::from_timeout(timeout),
        config,
        default_base_url,
    )
}

pub(crate) fn apply_http_query_params(
    mut req: reqwest::RequestBuilder,
    params: &BTreeMap<String, String>,
) -> reqwest::RequestBuilder {
    for (name, value) in params {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        req = req.query(&[(name, value)]);
    }
    req
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn http_headers_accept_valid_pairs() -> Result<()> {
        let headers = BTreeMap::from([
            ("x-test".to_string(), "value".to_string()),
            ("x-other".to_string(), "123".to_string()),
        ]);
        let parsed = header_map_from_pairs(&headers)?;
        assert_eq!(
            parsed
                .get("x-test")
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default(),
            "value"
        );
        Ok(())
    }

    #[test]
    fn http_headers_reject_invalid_name() {
        let headers = BTreeMap::from([("bad header".to_string(), "value".to_string())]);
        let err = header_map_from_pairs(&headers).expect_err("should reject invalid header name");
        match err {
            DittoError::InvalidResponse(_) => {}
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn http_headers_reject_invalid_value() {
        let headers = BTreeMap::from([("x-test".to_string(), "bad\nvalue".to_string())]);
        let err = header_map_from_pairs(&headers).expect_err("should reject invalid header value");
        match err {
            DittoError::InvalidResponse(_) => {}
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
