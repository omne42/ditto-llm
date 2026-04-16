use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use http_kit::read_reqwest_body_bytes_truncated;

use super::{Backend, GatewayError, GatewayRequest, GatewayResponse};

const MAX_BACKEND_ERROR_BODY_BYTES: usize = 64 * 1024;

#[derive(Clone)]
pub struct HttpBackend {
    url: String,
    client: reqwest::Client,
    headers: BTreeMap<String, String>,
}

impl HttpBackend {
    pub fn new(url: impl Into<String>) -> Result<Self, GatewayError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .map_err(|err| GatewayError::Backend {
                message: format!("backend http client error: {err}"),
            })?;
        Ok(Self {
            url: url.into(),
            client,
            headers: BTreeMap::new(),
        })
    }

    pub fn with_headers(mut self, headers: BTreeMap<String, String>) -> Self {
        self.headers = headers;
        self
    }
}

#[async_trait]
impl Backend for HttpBackend {
    async fn call(&self, request: &GatewayRequest) -> Result<GatewayResponse, GatewayError> {
        let mut req = self.client.post(&self.url).json(request);
        for (name, value) in &self.headers {
            req = req.header(name, value);
        }

        let response = req.send().await.map_err(|err| GatewayError::Backend {
            message: format!("backend request failed: {err}"),
        })?;

        let status = response.status();
        if !status.is_success() {
            let body = response_text_truncated(response, MAX_BACKEND_ERROR_BODY_BYTES).await;
            return Err(GatewayError::Backend {
                message: format!("backend status {status}: {body}"),
            });
        }

        response
            .json::<GatewayResponse>()
            .await
            .map_err(|err| GatewayError::Backend {
                message: format!("backend response decode error: {err}"),
            })
    }
}

async fn response_text_truncated(response: reqwest::Response, max_bytes: usize) -> String {
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
