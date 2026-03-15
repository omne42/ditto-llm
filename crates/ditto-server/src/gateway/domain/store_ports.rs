use async_trait::async_trait;
use thiserror::Error;

use super::{
    ProxyRequestFingerprint, ProxyRequestIdempotencyBeginOutcome, ProxyRequestIdempotencyRecord,
    ProxyRequestReplayOutcome,
};

#[derive(Clone, Debug, Error)]
#[error("{message}")]
pub struct ProxyRequestIdempotencyStoreError {
    message: String,
}

impl ProxyRequestIdempotencyStoreError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl From<String> for ProxyRequestIdempotencyStoreError {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for ProxyRequestIdempotencyStoreError {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

#[async_trait]
pub trait ProxyRequestIdempotencyStore: Send + Sync {
    async fn begin_proxy_request_idempotency(
        &self,
        request_id: &str,
        fingerprint: &ProxyRequestFingerprint,
        fingerprint_key: &str,
        owner_token: &str,
        now_ms: u64,
        lease_ttl_ms: u64,
    ) -> Result<ProxyRequestIdempotencyBeginOutcome, ProxyRequestIdempotencyStoreError>;

    async fn get_proxy_request_idempotency(
        &self,
        request_id: &str,
        now_ms: u64,
    ) -> Result<Option<ProxyRequestIdempotencyRecord>, ProxyRequestIdempotencyStoreError>;

    async fn refresh_proxy_request_idempotency_lease(
        &self,
        request_id: &str,
        owner_token: &str,
        now_ms: u64,
        lease_ttl_ms: u64,
    ) -> Result<bool, ProxyRequestIdempotencyStoreError>;

    async fn complete_proxy_request_idempotency(
        &self,
        request_id: &str,
        owner_token: &str,
        outcome: &ProxyRequestReplayOutcome,
        now_ms: u64,
        replay_ttl_ms: u64,
    ) -> Result<bool, ProxyRequestIdempotencyStoreError>;

    async fn release_proxy_request_idempotency(
        &self,
        request_id: &str,
        owner_token: &str,
    ) -> Result<bool, ProxyRequestIdempotencyStoreError>;
}
