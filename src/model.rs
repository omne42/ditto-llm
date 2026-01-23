use async_trait::async_trait;
use futures_util::stream::BoxStream;

use crate::Result;
use crate::types::{GenerateRequest, GenerateResponse, StreamChunk};

pub type StreamResult = BoxStream<'static, Result<StreamChunk>>;

#[async_trait]
pub trait LanguageModel: Send + Sync {
    fn provider(&self) -> &str;
    fn model_id(&self) -> &str;

    async fn generate(&self, request: GenerateRequest) -> Result<GenerateResponse>;

    async fn stream(&self, request: GenerateRequest) -> Result<StreamResult>;
}
