use async_trait::async_trait;

use crate::Result;
use crate::types::{BatchCreateRequest, BatchListResponse, BatchResponse};

#[async_trait]
pub trait BatchClient: Send + Sync {
    fn provider(&self) -> &str;

    async fn create(&self, request: BatchCreateRequest) -> Result<BatchResponse>;
    async fn retrieve(&self, batch_id: &str) -> Result<BatchResponse>;
    async fn cancel(&self, batch_id: &str) -> Result<BatchResponse>;
    async fn list(&self, limit: Option<u32>, after: Option<String>) -> Result<BatchListResponse>;
}
