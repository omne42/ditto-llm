#![allow(dead_code)]

use bytes::Bytes;
#[cfg(any(
    feature = "gateway",
    feature = "provider-openai",
    feature = "provider-openai-compatible"
))]
use reqwest::header::HeaderMap;
use serde::de::DeserializeOwned;

use crate::error::{DittoError, Result};
use http_kit::{
    ReadReqwestBodyBytesError, read_reqwest_body_bytes_limited, read_reqwest_body_bytes_truncated,
};

use super::policy::HttpResponseBodyPolicy;

pub async fn response_text_truncated(response: reqwest::Response, max_bytes: usize) -> String {
    let (bytes, truncated) = match read_reqwest_body_bytes_truncated(response, max_bytes).await {
        Ok((bytes, truncated)) => (bytes, truncated),
        Err(err) => return format!("failed to read response body: {err}"),
    };
    let mut body = String::from_utf8_lossy(&bytes).to_string();
    if truncated {
        if !body.is_empty() {
            body.push('\n');
        }
        body.push_str("...(truncated)");
    }
    body
}

#[cfg(any(
    feature = "gateway",
    feature = "provider-openai",
    feature = "provider-openai-compatible"
))]
pub(crate) async fn read_reqwest_body_bytes_bounded_with_content_length(
    response: reqwest::Response,
    _headers: &HeaderMap,
    max_bytes: usize,
) -> Result<Bytes> {
    read_reqwest_body_bytes_bounded(response, max_bytes).await
}

pub(crate) async fn send_checked(req: reqwest::RequestBuilder) -> Result<reqwest::Response> {
    send_checked_with_policy(req, HttpResponseBodyPolicy::default()).await
}

pub(crate) async fn send_checked_with_policy(
    req: reqwest::RequestBuilder,
    policy: HttpResponseBodyPolicy,
) -> Result<reqwest::Response> {
    let response = req.send().await?;
    let status = response.status();
    if !status.is_success() {
        let body = response_text_truncated(response, policy.max_error_body_bytes).await;
        return Err(DittoError::Api { status, body });
    }
    Ok(response)
}

pub(crate) async fn send_checked_json<T: DeserializeOwned>(
    req: reqwest::RequestBuilder,
) -> Result<T> {
    send_checked_json_with_policy(req, HttpResponseBodyPolicy::default()).await
}

pub(crate) async fn send_checked_json_with_policy<T: DeserializeOwned>(
    req: reqwest::RequestBuilder,
    policy: HttpResponseBodyPolicy,
) -> Result<T> {
    let response = send_checked_with_policy(req, policy).await?;
    let bytes = read_reqwest_body_bytes_bounded(response, policy.max_response_body_bytes).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

#[allow(dead_code)]
pub(crate) async fn send_checked_bytes(req: reqwest::RequestBuilder) -> Result<Bytes> {
    send_checked_bytes_with_policy(req, HttpResponseBodyPolicy::default()).await
}

#[allow(dead_code)]
pub(crate) async fn send_checked_bytes_with_policy(
    req: reqwest::RequestBuilder,
    policy: HttpResponseBodyPolicy,
) -> Result<Bytes> {
    let response = req.send().await?;
    let status = response.status();
    if !status.is_success() {
        let body = response_text_truncated(response, policy.max_response_body_bytes).await;
        return Err(DittoError::Api { status, body });
    }
    read_reqwest_body_bytes_bounded(response, policy.max_response_body_bytes).await
}

async fn read_reqwest_body_bytes_bounded(
    response: reqwest::Response,
    max_bytes: usize,
) -> Result<Bytes> {
    match read_reqwest_body_bytes_limited(response, max_bytes.max(1)).await {
        Ok(bytes) => Ok(Bytes::from(bytes)),
        Err(ReadReqwestBodyBytesError::ContentLengthExceedsLimit {
            content_length,
            max_bytes,
        }) => Err(crate::invalid_response!(
            "error_detail.http.content_length_exceeds_max_bytes",
            "content_length" => content_length.to_string(),
            "max_bytes" => max_bytes.to_string()
        )),
        Err(ReadReqwestBodyBytesError::ResponseExceededLimit { max_bytes }) => {
            Err(crate::invalid_response!(
                "error_detail.http.response_exceeded_max_bytes",
                "max_bytes" => max_bytes.to_string()
            ))
        }
        Err(ReadReqwestBodyBytesError::Transport(err)) => Err(err.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::HttpResponseBodyPolicy;
    use crate::error::DittoError;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn send_checked_bytes_errors_on_truncated_http_body() {
        if crate::utils::test_support::should_skip_httpmock() {
            return;
        }
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut req_buf = [0u8; 1024];
            let _ = socket.read(&mut req_buf).await;
            let response = b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: 100\r\nConnection: close\r\n\r\nabc";
            socket.write_all(response).await.expect("write response");
            socket.shutdown().await.expect("shutdown");
        });

        let client = reqwest::Client::new();
        let result = super::send_checked_bytes(client.get(format!("http://{addr}/"))).await;
        assert!(result.is_err(), "truncated body should return error");
        let _ = server.await;
    }

    #[tokio::test]
    async fn send_checked_bytes_rejects_success_content_length_over_limit() {
        if crate::utils::test_support::should_skip_httpmock() {
            return;
        }
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let oversized = HttpResponseBodyPolicy::default().max_response_body_bytes + 1;
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut req_buf = [0u8; 1024];
            let _ = socket.read(&mut req_buf).await;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {oversized}\r\nConnection: close\r\n\r\nabc"
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write response");
            socket.shutdown().await.expect("shutdown");
        });

        let client = reqwest::Client::new();
        let result = super::send_checked_bytes(client.get(format!("http://{addr}/"))).await;
        match result {
            Err(DittoError::InvalidResponse(message)) => {
                let catalog = message
                    .as_catalog()
                    .expect("http invalid response should be catalog-backed");
                assert_eq!(
                    catalog.code(),
                    "error_detail.http.content_length_exceeds_max_bytes"
                );
                assert_eq!(
                    catalog.text_arg("content_length").map(str::to_owned),
                    Some(oversized.to_string())
                );
            }
            other => panic!("unexpected result: {other:?}"),
        }
        let _ = server.await;
    }
}
