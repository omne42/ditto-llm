use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

use crate::batch::BatchClient;
use crate::profile::{
    Env, HttpAuth, ProviderConfig, RequestAuth, apply_http_query_params,
    resolve_request_auth_with_default_keys,
};
use crate::types::{Batch, BatchCreateRequest, BatchListResponse, BatchResponse, Warning};
use crate::{DittoError, Result};

#[derive(Clone)]
pub struct OpenAICompatibleBatches {
    http: reqwest::Client,
    base_url: String,
    auth: Option<RequestAuth>,
    http_query_params: BTreeMap<String, String>,
}

impl OpenAICompatibleBatches {
    pub fn new(api_key: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        let api_key = api_key.into();
        let auth = if api_key.trim().is_empty() {
            None
        } else {
            HttpAuth::bearer(&api_key).ok().map(RequestAuth::Http)
        };

        Self {
            http,
            base_url: "https://api.openai.com/v1".to_string(),
            auth,
            http_query_params: BTreeMap::new(),
        }
    }

    pub fn with_http_client(mut self, http: reqwest::Client) -> Self {
        self.http = http;
        self
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub async fn from_config(config: &ProviderConfig, env: &Env) -> Result<Self> {
        const DEFAULT_KEYS: &[&str] = &[
            "OPENAI_COMPAT_API_KEY",
            "OPENAI_API_KEY",
            "CODE_PM_OPENAI_API_KEY",
        ];

        let auth = match config.auth.clone() {
            Some(auth) => Some(
                resolve_request_auth_with_default_keys(
                    &auth,
                    env,
                    DEFAULT_KEYS,
                    "authorization",
                    Some("Bearer "),
                )
                .await?,
            ),
            None => DEFAULT_KEYS
                .iter()
                .find_map(|key| env.get(key))
                .and_then(|token| HttpAuth::bearer(&token).ok().map(RequestAuth::Http)),
        };

        let mut out = Self::new("");
        out.auth = auth;
        out.http_query_params = config.http_query_params.clone();
        if !config.http_headers.is_empty() {
            out = out.with_http_client(crate::profile::build_http_client(
                std::time::Duration::from_secs(300),
                &config.http_headers,
            )?);
        }
        if let Some(base_url) = config.base_url.as_deref().filter(|s| !s.trim().is_empty()) {
            out = out.with_base_url(base_url);
        }
        Ok(out)
    }

    fn apply_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let req = match self.auth.as_ref() {
            Some(auth) => auth.apply(req),
            None => req,
        };
        apply_http_query_params(req, &self.http_query_params)
    }

    fn batches_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/batches") {
            base.to_string()
        } else {
            format!("{base}/batches")
        }
    }

    fn batch_url(&self, batch_id: &str) -> String {
        let url = self.batches_url();
        let base = url.trim_end_matches('/');
        format!("{base}/{batch_id}")
    }

    fn batch_cancel_url(&self, batch_id: &str) -> String {
        let url = self.batch_url(batch_id);
        let base = url.trim_end_matches('/');
        format!("{base}/cancel")
    }

    async fn parse_batch_response(&self, response: reqwest::Response) -> Result<(Batch, Value)> {
        let raw = response.json::<Value>().await?;
        let batch = serde_json::from_value::<Batch>(raw.clone())?;
        Ok((batch, raw))
    }
}

#[derive(Debug, Deserialize, Default)]
struct BatchListObject {
    #[serde(default)]
    data: Vec<Value>,
    #[serde(default)]
    has_more: Option<bool>,
    #[serde(default)]
    last_id: Option<String>,
}

#[async_trait]
impl BatchClient for OpenAICompatibleBatches {
    fn provider(&self) -> &str {
        "openai-compatible"
    }

    async fn create(&self, request: BatchCreateRequest) -> Result<BatchResponse> {
        let selected_provider_options = crate::types::select_provider_options_value(
            request.provider_options.as_ref(),
            self.provider(),
        )?;
        let mut warnings = Vec::<Warning>::new();

        let mut body = Map::<String, Value>::new();
        body.insert(
            "input_file_id".to_string(),
            Value::String(request.input_file_id),
        );
        body.insert("endpoint".to_string(), Value::String(request.endpoint));
        body.insert(
            "completion_window".to_string(),
            Value::String(request.completion_window),
        );
        if let Some(metadata) = request.metadata {
            body.insert("metadata".to_string(), serde_json::to_value(metadata)?);
        }

        crate::types::merge_provider_options_into_body(
            &mut body,
            selected_provider_options.as_ref(),
            &[],
            "batches.create.provider_options",
            &mut warnings,
        );

        let url = self.batches_url();
        let response = self
            .apply_auth(self.http.post(url).json(&body))
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(DittoError::Api { status, body: text });
        }

        let (batch, raw) = self.parse_batch_response(response).await?;
        Ok(BatchResponse {
            batch,
            warnings,
            provider_metadata: Some(raw),
        })
    }

    async fn retrieve(&self, batch_id: &str) -> Result<BatchResponse> {
        let url = self.batch_url(batch_id);
        let response = self.apply_auth(self.http.get(url)).send().await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(DittoError::Api { status, body: text });
        }

        let (batch, raw) = self.parse_batch_response(response).await?;
        Ok(BatchResponse {
            batch,
            warnings: Vec::new(),
            provider_metadata: Some(raw),
        })
    }

    async fn cancel(&self, batch_id: &str) -> Result<BatchResponse> {
        let url = self.batch_cancel_url(batch_id);
        let response = self.apply_auth(self.http.post(url)).send().await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(DittoError::Api { status, body: text });
        }

        let (batch, raw) = self.parse_batch_response(response).await?;
        Ok(BatchResponse {
            batch,
            warnings: Vec::new(),
            provider_metadata: Some(raw),
        })
    }

    async fn list(&self, limit: Option<u32>, after: Option<String>) -> Result<BatchListResponse> {
        let url = self.batches_url();
        let mut req = self.http.get(url);
        if let Some(limit) = limit {
            req = req.query(&[("limit", limit)]);
        }
        if let Some(after) = after.as_deref().filter(|s| !s.trim().is_empty()) {
            req = req.query(&[("after", after)]);
        }

        let response = self.apply_auth(req).send().await?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(DittoError::Api { status, body: text });
        }

        let raw = response.json::<Value>().await?;
        let parsed = serde_json::from_value::<BatchListObject>(raw.clone())?;
        let mut batches = Vec::<Batch>::new();
        for item in parsed.data {
            batches.push(serde_json::from_value::<Batch>(item)?);
        }

        Ok(BatchListResponse {
            batches,
            after: parsed.last_id,
            has_more: parsed.has_more,
            warnings: Vec::new(),
            provider_metadata: Some(raw),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::Method::GET;
    use httpmock::Method::POST;
    use httpmock::MockServer;
    use serde_json::json;

    #[tokio::test]
    async fn create_posts_to_batches_and_merges_provider_options() -> Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }

        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v1/batches")
                    .body_includes("\"input_file_id\":\"file_123\"")
                    .body_includes("\"endpoint\":\"/v1/chat/completions\"")
                    .body_includes("\"completion_window\":\"24h\"")
                    .body_includes("\"metadata\"")
                    .body_includes("\"extra\":\"ok\"");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(
                        json!({
                            "id": "batch_123",
                            "object": "batch",
                            "endpoint": "/v1/chat/completions",
                            "input_file_id": "file_123",
                            "completion_window": "24h",
                            "status": "validating",
                            "created_at": 123,
                            "request_counts": { "total": 10, "completed": 0, "failed": 0 }
                        })
                        .to_string(),
                    );
            })
            .await;

        let client = OpenAICompatibleBatches::new("sk-test").with_base_url(server.url("/v1"));

        let request = BatchCreateRequest {
            input_file_id: "file_123".to_string(),
            endpoint: "/v1/chat/completions".to_string(),
            completion_window: "24h".to_string(),
            metadata: Some(BTreeMap::from([("run".to_string(), "yes".to_string())])),
            provider_options: Some(json!({
                "openai-compatible": { "extra": "ok", "input_file_id": "ignore" }
            })),
        };

        let resp = client.create(request).await?;
        mock.assert_async().await;

        assert_eq!(resp.batch.id, "batch_123");
        assert_eq!(resp.batch.status, crate::types::BatchStatus::Validating);
        assert!(resp.warnings.iter().any(|w| matches!(
            w,
            Warning::Compatibility { feature, details }
                if feature == "batches.create.provider_options" && details.contains("overrides input_file_id")
        )));

        Ok(())
    }

    #[tokio::test]
    async fn list_gets_batches() -> Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }

        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/v1/batches")
                    .query_param("limit", "2")
                    .query_param("after", "batch_111");
                then.status(200).header("content-type", "application/json").body(
                    json!({
                        "object": "list",
                        "data": [
                            { "id": "batch_222", "status": "in_progress" },
                            { "id": "batch_333", "status": "completed", "output_file_id": "file_out" }
                        ],
                        "has_more": false,
                        "last_id": "batch_333"
                    })
                    .to_string(),
                );
            })
            .await;

        let client = OpenAICompatibleBatches::new("sk-test").with_base_url(server.url("/v1"));
        let resp = client.list(Some(2), Some("batch_111".to_string())).await?;
        mock.assert_async().await;

        assert_eq!(resp.batches.len(), 2);
        assert_eq!(resp.batches[0].id, "batch_222");
        assert_eq!(resp.batches[1].output_file_id.as_deref(), Some("file_out"));
        assert_eq!(resp.after.as_deref(), Some("batch_333"));
        Ok(())
    }
}
