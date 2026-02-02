use bytes::Bytes;
use futures_util::StreamExt;
use serde::de::DeserializeOwned;

use crate::{DittoError, Result};

const MAX_ERROR_BODY_BYTES: usize = 64 * 1024;
const MAX_RESPONSE_BODY_BYTES: usize = 16 * 1024 * 1024;

pub(crate) async fn response_text_truncated(
    response: reqwest::Response,
    max_bytes: usize,
) -> String {
    let (bytes, truncated) = response_bytes_truncated(response, max_bytes).await;
    let mut body = String::from_utf8_lossy(&bytes).to_string();
    if truncated {
        if !body.is_empty() {
            body.push('\n');
        }
        body.push_str("...(truncated)");
    }
    body
}

async fn response_bytes_truncated(
    response: reqwest::Response,
    max_bytes: usize,
) -> (Vec<u8>, bool) {
    let max_bytes = max_bytes.max(1);
    let mut out = Vec::<u8>::new();
    let mut truncated = false;

    let mut stream = response.bytes_stream();
    while let Some(next) = stream.next().await {
        let Ok(chunk) = next else {
            break;
        };
        let remaining = max_bytes.saturating_sub(out.len());
        if remaining == 0 {
            truncated = true;
            break;
        }
        if chunk.len() <= remaining {
            out.extend_from_slice(chunk.as_ref());
        } else {
            out.extend_from_slice(&chunk.as_ref()[..remaining]);
            truncated = true;
            break;
        }
    }
    (out, truncated)
}

pub(crate) async fn send_checked(req: reqwest::RequestBuilder) -> Result<reqwest::Response> {
    let response = req.send().await?;
    let status = response.status();
    if !status.is_success() {
        let body = response_text_truncated(response, MAX_ERROR_BODY_BYTES).await;
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
    let (bytes, truncated) = response_bytes_truncated(response, MAX_RESPONSE_BODY_BYTES).await;
    let bytes = Bytes::from(bytes);
    if !status.is_success() {
        let body = String::from_utf8_lossy(&bytes).to_string();
        if truncated {
            return Err(DittoError::Api {
                status,
                body: format!("{body}\n...(truncated)"),
            });
        }
        return Err(DittoError::Api { status, body });
    }
    if truncated {
        return Err(DittoError::InvalidResponse(format!(
            "response exceeded max bytes ({MAX_RESPONSE_BODY_BYTES})"
        )));
    }
    Ok(bytes)
}
