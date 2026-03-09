use async_trait::async_trait;

use crate::Result;
use crate::types::{ImageEditRequest, ImageEditResponse};

#[async_trait]
pub trait ImageEditModel: Send + Sync {
    fn provider(&self) -> &str;
    fn model_id(&self) -> &str;

    async fn edit(&self, request: ImageEditRequest) -> Result<ImageEditResponse>;
}
