use async_trait::async_trait;

use super::openai_batches_common;
use super::openai_like;

use crate::Result;
use crate::batch::BatchClient;
use crate::profile::{Env, ProviderConfig};
use crate::types::{BatchCreateRequest, BatchListResponse, BatchResponse};

#[derive(Clone)]
pub struct OpenAICompatibleBatches {
    client: openai_like::OpenAiLikeClient,
}

impl OpenAICompatibleBatches {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: openai_like::OpenAiLikeClient::new(api_key),
        }
    }

    pub fn with_http_client(mut self, http: reqwest::Client) -> Self {
        self.client = self.client.with_http_client(http);
        self
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.client = self.client.with_base_url(base_url);
        self
    }

    pub async fn from_config(config: &ProviderConfig, env: &Env) -> Result<Self> {
        const DEFAULT_KEYS: &[&str] = &[
            "OPENAI_COMPAT_API_KEY",
            "OPENAI_API_KEY",
            "CODE_PM_OPENAI_API_KEY",
        ];
        Ok(Self {
            client: openai_like::OpenAiLikeClient::from_config_optional(config, env, DEFAULT_KEYS)
                .await?,
        })
    }
}

#[async_trait]
impl BatchClient for OpenAICompatibleBatches {
    fn provider(&self) -> &str {
        "openai-compatible"
    }

    async fn create(&self, request: BatchCreateRequest) -> Result<BatchResponse> {
        openai_batches_common::create(self.provider(), &self.client, request).await
    }

    async fn retrieve(&self, batch_id: &str) -> Result<BatchResponse> {
        openai_batches_common::retrieve(&self.client, batch_id).await
    }

    async fn cancel(&self, batch_id: &str) -> Result<BatchResponse> {
        openai_batches_common::cancel(&self.client, batch_id).await
    }

    async fn list(&self, limit: Option<u32>, after: Option<String>) -> Result<BatchListResponse> {
        openai_batches_common::list(&self.client, limit, after).await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::types::Warning;
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
