use async_trait::async_trait;

use crate::Result;

#[async_trait]
pub trait EmbeddingModel: Send + Sync {
    fn provider(&self) -> &str;
    fn model_id(&self) -> &str;

    async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>>;

    async fn embed_single(&self, text: String) -> Result<Vec<f32>> {
        let embeddings = self.embed(vec![text]).await?;
        embeddings
            .into_iter()
            .next()
            .ok_or_else(|| crate::DittoError::InvalidResponse("embedding response is empty".into()))
    }
}
