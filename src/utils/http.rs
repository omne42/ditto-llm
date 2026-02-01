use bytes::Bytes;
use serde::de::DeserializeOwned;

use crate::{DittoError, Result};

pub(crate) async fn send_checked(req: reqwest::RequestBuilder) -> Result<reqwest::Response> {
    let response = req.send().await?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(DittoError::Api { status, body });
    }
    Ok(response)
}

pub(crate) async fn send_checked_json<T: DeserializeOwned>(
    req: reqwest::RequestBuilder,
) -> Result<T> {
    let response = send_checked(req).await?;
    Ok(response.json::<T>().await?)
}

#[allow(dead_code)]
pub(crate) async fn send_checked_bytes(req: reqwest::RequestBuilder) -> Result<Bytes> {
    let response = req.send().await?;
    let status = response.status();
    let bytes = response.bytes().await?;
    if !status.is_success() {
        let body = String::from_utf8_lossy(&bytes).to_string();
        return Err(DittoError::Api { status, body });
    }
    Ok(bytes)
}
