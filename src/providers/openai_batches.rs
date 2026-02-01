use async_trait::async_trait;

use super::openai_batches_common;
use super::openai_like;

use crate::Result;
use crate::batch::BatchClient;
use crate::profile::{Env, ProviderConfig};
use crate::types::{BatchCreateRequest, BatchListResponse, BatchResponse};

#[derive(Clone)]
pub struct OpenAIBatches {
    client: openai_like::OpenAiLikeClient,
}

impl OpenAIBatches {
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
        const DEFAULT_KEYS: &[&str] = &["OPENAI_API_KEY", "CODE_PM_OPENAI_API_KEY"];
        Ok(Self {
            client: openai_like::OpenAiLikeClient::from_config_required(config, env, DEFAULT_KEYS)
                .await?,
        })
    }
}

#[async_trait]
impl BatchClient for OpenAIBatches {
    fn provider(&self) -> &str {
        "openai"
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
