use async_trait::async_trait;

use crate::Result;
use crate::types::{RerankRequest, RerankResponse};

#[async_trait]
pub trait RerankModel: Send + Sync {
    fn provider(&self) -> &str;
    fn model_id(&self) -> &str;

    async fn rerank(&self, request: RerankRequest) -> Result<RerankResponse>;
}
