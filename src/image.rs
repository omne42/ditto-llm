use async_trait::async_trait;

use crate::Result;
use crate::types::{ImageGenerationRequest, ImageGenerationResponse};

#[async_trait]
pub trait ImageGenerationModel: Send + Sync {
    fn provider(&self) -> &str;
    fn model_id(&self) -> &str;

    async fn generate(&self, request: ImageGenerationRequest) -> Result<ImageGenerationResponse>;
}
