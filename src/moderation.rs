use async_trait::async_trait;

use crate::Result;
use crate::types::{ModerationRequest, ModerationResponse};

#[async_trait]
pub trait ModerationModel: Send + Sync {
    fn provider(&self) -> &str;
    fn model_id(&self) -> &str;

    async fn moderate(&self, request: ModerationRequest) -> Result<ModerationResponse>;
}
