use std::collections::BTreeMap;
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

use crate::{DittoError, Result};

pub(super) fn header_map_from_pairs(headers: &BTreeMap<String, String>) -> Result<HeaderMap> {
    let mut out = HeaderMap::new();
    for (name, value) in headers {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|err| {
            DittoError::InvalidResponse(format!("invalid http header name {name:?}: {err}"))
        })?;

        let header_value = HeaderValue::from_str(value).map_err(|err| {
            DittoError::InvalidResponse(format!(
                "invalid http header value for {name:?} (value={value:?}): {err}"
            ))
        })?;

        out.insert(header_name, header_value);
    }
    Ok(out)
}

pub(crate) fn build_http_client(
    timeout: Duration,
    headers: &BTreeMap<String, String>,
) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder().timeout(timeout);
    if !headers.is_empty() {
        builder = builder.default_headers(header_map_from_pairs(headers)?);
    }
    builder.build().map_err(DittoError::Http)
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
