use async_trait::async_trait;

use crate::error::Result;

#[async_trait]
pub trait EmbeddingModel: Send + Sync {
    fn provider(&self) -> &str;
    fn model_id(&self) -> &str;

    async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>>;

    async fn embed_single(&self, text: String) -> Result<Vec<f32>> {
        let embeddings = self.embed(vec![text]).await?;
        embeddings.into_iter().next().ok_or_else(|| {
            crate::error::DittoError::invalid_response_text("embedding response is empty")
        })
    }
}

#[async_trait]
pub trait EmbeddingModelExt: EmbeddingModel {
    async fn embed_many(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        self.embed(texts).await
    }

    async fn embed_one(&self, text: String) -> Result<Vec<f32>> {
        self.embed_single(text).await
    }
}

impl<T> EmbeddingModelExt for T where T: EmbeddingModel + ?Sized {}
