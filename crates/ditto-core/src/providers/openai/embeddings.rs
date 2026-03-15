use async_trait::async_trait;

use crate::capabilities::embedding::EmbeddingModel;
use crate::config::{Env, ProviderConfig};
use crate::error::{DittoError, Result};
use crate::providers::openai_like;

#[derive(Clone)]
pub struct OpenAIEmbeddings {
    client: openai_like::OpenAiLikeClient,
}

impl OpenAIEmbeddings {
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

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.client = self.client.with_model(model);
        self
    }

    pub async fn from_config(config: &ProviderConfig, env: &Env) -> Result<Self> {
        const DEFAULT_KEYS: &[&str] = &["OPENAI_API_KEY"];
        Ok(Self {
            client: openai_like::OpenAiLikeClient::from_config_required(config, env, DEFAULT_KEYS)
                .await?,
        })
    }

    fn resolve_model(&self) -> Result<&str> {
        if !self.client.model.trim().is_empty() {
            return Ok(self.client.model.as_str());
        }
        Err(DittoError::provider_model_missing(
            "openai embedding",
            "set OpenAIEmbeddings::with_model",
        ))
    }
}

#[async_trait]
impl EmbeddingModel for OpenAIEmbeddings {
    fn provider(&self) -> &str {
        "openai"
    }

    fn model_id(&self) -> &str {
        self.client.model.as_str()
    }

    async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let model = self.resolve_model()?;
        crate::providers::openai_embeddings_common::embed(&self.client, model, texts).await
    }
}
