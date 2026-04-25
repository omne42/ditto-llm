//! Gateway persistence adapters.

mod memory_request_idempotency;

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
use async_trait::async_trait;

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
use super::super::{
    AuditLogRecord, BudgetLedgerRecord, CostLedgerRecord, ProxyRequestFingerprint,
    ProxyRequestIdempotencyBeginOutcome, ProxyRequestIdempotencyRecord,
    ProxyRequestIdempotencyState, ProxyRequestIdempotencyStore, ProxyRequestIdempotencyStoreError,
    ProxyRequestReplayOutcome, RouterConfig, VirtualKeyConfig,
};
#[cfg(all(feature = "gateway-proxy-cache", feature = "gateway-store-redis"))]
use super::super::{
    CachedProxyResponse, ProxyCacheEntryMetadata, ProxyCachePurgeSelector, ProxyCacheStoredResponse,
};
#[cfg(feature = "gateway-store-redis")]
use super::super::{GatewayError, LimitsConfig};

#[cfg(feature = "gateway-store-mysql")]
pub mod mysql;
#[cfg(feature = "gateway-store-postgres")]
pub mod postgres;
#[cfg(feature = "gateway-store-redis")]
pub mod redis;
#[cfg(feature = "gateway-store-sqlite")]
pub mod sqlite;

pub(crate) use memory_request_idempotency::LocalProxyRequestIdempotencyStore;
#[cfg(feature = "gateway-store-mysql")]
pub use mysql::{MySqlStore, MySqlStoreError};
#[cfg(feature = "gateway-store-postgres")]
pub use postgres::{PostgresStore, PostgresStoreError};
#[cfg(feature = "gateway-store-redis")]
pub use redis::{RedisStore, RedisStoreError};
#[cfg(feature = "gateway-store-sqlite")]
pub use sqlite::{SqliteStore, SqliteStoreError};

#[cfg(feature = "gateway-store-sqlite")]
#[async_trait]
impl ProxyRequestIdempotencyStore for SqliteStore {
    async fn begin_proxy_request_idempotency(
        &self,
        request_id: &str,
        fingerprint: &ProxyRequestFingerprint,
        fingerprint_key: &str,
        owner_token: &str,
        now_ms: u64,
        lease_ttl_ms: u64,
    ) -> Result<ProxyRequestIdempotencyBeginOutcome, ProxyRequestIdempotencyStoreError> {
        SqliteStore::begin_proxy_request_idempotency(
            self,
            request_id,
            fingerprint,
            fingerprint_key,
            owner_token,
            now_ms,
            lease_ttl_ms,
        )
        .await
        .map_err(|err| ProxyRequestIdempotencyStoreError::new(err.to_string()))
    }

    async fn get_proxy_request_idempotency(
        &self,
        request_id: &str,
        now_ms: u64,
    ) -> Result<Option<ProxyRequestIdempotencyRecord>, ProxyRequestIdempotencyStoreError> {
        SqliteStore::get_proxy_request_idempotency(self, request_id, now_ms)
            .await
            .map_err(|err| ProxyRequestIdempotencyStoreError::new(err.to_string()))
    }

    async fn refresh_proxy_request_idempotency_lease(
        &self,
        request_id: &str,
        owner_token: &str,
        now_ms: u64,
        lease_ttl_ms: u64,
    ) -> Result<bool, ProxyRequestIdempotencyStoreError> {
        SqliteStore::refresh_proxy_request_idempotency_lease(
            self,
            request_id,
            owner_token,
            now_ms,
            lease_ttl_ms,
        )
        .await
        .map_err(|err| ProxyRequestIdempotencyStoreError::new(err.to_string()))
    }

    async fn complete_proxy_request_idempotency(
        &self,
        request_id: &str,
        owner_token: &str,
        outcome: &ProxyRequestReplayOutcome,
        now_ms: u64,
        replay_ttl_ms: u64,
    ) -> Result<bool, ProxyRequestIdempotencyStoreError> {
        SqliteStore::complete_proxy_request_idempotency(
            self,
            request_id,
            owner_token,
            outcome,
            now_ms,
            replay_ttl_ms,
        )
        .await
        .map_err(|err| ProxyRequestIdempotencyStoreError::new(err.to_string()))
    }

    async fn release_proxy_request_idempotency(
        &self,
        request_id: &str,
        owner_token: &str,
    ) -> Result<bool, ProxyRequestIdempotencyStoreError> {
        SqliteStore::release_proxy_request_idempotency(self, request_id, owner_token)
            .await
            .map_err(|err| ProxyRequestIdempotencyStoreError::new(err.to_string()))
    }
}

#[cfg(feature = "gateway-store-postgres")]
#[async_trait]
impl ProxyRequestIdempotencyStore for PostgresStore {
    async fn begin_proxy_request_idempotency(
        &self,
        request_id: &str,
        fingerprint: &ProxyRequestFingerprint,
        fingerprint_key: &str,
        owner_token: &str,
        now_ms: u64,
        lease_ttl_ms: u64,
    ) -> Result<ProxyRequestIdempotencyBeginOutcome, ProxyRequestIdempotencyStoreError> {
        PostgresStore::begin_proxy_request_idempotency(
            self,
            request_id,
            fingerprint,
            fingerprint_key,
            owner_token,
            now_ms,
            lease_ttl_ms,
        )
        .await
        .map_err(|err| ProxyRequestIdempotencyStoreError::new(err.to_string()))
    }

    async fn get_proxy_request_idempotency(
        &self,
        request_id: &str,
        now_ms: u64,
    ) -> Result<Option<ProxyRequestIdempotencyRecord>, ProxyRequestIdempotencyStoreError> {
        PostgresStore::get_proxy_request_idempotency(self, request_id, now_ms)
            .await
            .map_err(|err| ProxyRequestIdempotencyStoreError::new(err.to_string()))
    }

    async fn refresh_proxy_request_idempotency_lease(
        &self,
        request_id: &str,
        owner_token: &str,
        now_ms: u64,
        lease_ttl_ms: u64,
    ) -> Result<bool, ProxyRequestIdempotencyStoreError> {
        PostgresStore::refresh_proxy_request_idempotency_lease(
            self,
            request_id,
            owner_token,
            now_ms,
            lease_ttl_ms,
        )
        .await
        .map_err(|err| ProxyRequestIdempotencyStoreError::new(err.to_string()))
    }

    async fn complete_proxy_request_idempotency(
        &self,
        request_id: &str,
        owner_token: &str,
        outcome: &ProxyRequestReplayOutcome,
        now_ms: u64,
        replay_ttl_ms: u64,
    ) -> Result<bool, ProxyRequestIdempotencyStoreError> {
        PostgresStore::complete_proxy_request_idempotency(
            self,
            request_id,
            owner_token,
            outcome,
            now_ms,
            replay_ttl_ms,
        )
        .await
        .map_err(|err| ProxyRequestIdempotencyStoreError::new(err.to_string()))
    }

    async fn release_proxy_request_idempotency(
        &self,
        request_id: &str,
        owner_token: &str,
    ) -> Result<bool, ProxyRequestIdempotencyStoreError> {
        PostgresStore::release_proxy_request_idempotency(self, request_id, owner_token)
            .await
            .map_err(|err| ProxyRequestIdempotencyStoreError::new(err.to_string()))
    }
}

#[cfg(feature = "gateway-store-mysql")]
#[async_trait]
impl ProxyRequestIdempotencyStore for MySqlStore {
    async fn begin_proxy_request_idempotency(
        &self,
        request_id: &str,
        fingerprint: &ProxyRequestFingerprint,
        fingerprint_key: &str,
        owner_token: &str,
        now_ms: u64,
        lease_ttl_ms: u64,
    ) -> Result<ProxyRequestIdempotencyBeginOutcome, ProxyRequestIdempotencyStoreError> {
        MySqlStore::begin_proxy_request_idempotency(
            self,
            request_id,
            fingerprint,
            fingerprint_key,
            owner_token,
            now_ms,
            lease_ttl_ms,
        )
        .await
        .map_err(|err| ProxyRequestIdempotencyStoreError::new(err.to_string()))
    }

    async fn get_proxy_request_idempotency(
        &self,
        request_id: &str,
        now_ms: u64,
    ) -> Result<Option<ProxyRequestIdempotencyRecord>, ProxyRequestIdempotencyStoreError> {
        MySqlStore::get_proxy_request_idempotency(self, request_id, now_ms)
            .await
            .map_err(|err| ProxyRequestIdempotencyStoreError::new(err.to_string()))
    }

    async fn refresh_proxy_request_idempotency_lease(
        &self,
        request_id: &str,
        owner_token: &str,
        now_ms: u64,
        lease_ttl_ms: u64,
    ) -> Result<bool, ProxyRequestIdempotencyStoreError> {
        MySqlStore::refresh_proxy_request_idempotency_lease(
            self,
            request_id,
            owner_token,
            now_ms,
            lease_ttl_ms,
        )
        .await
        .map_err(|err| ProxyRequestIdempotencyStoreError::new(err.to_string()))
    }

    async fn complete_proxy_request_idempotency(
        &self,
        request_id: &str,
        owner_token: &str,
        outcome: &ProxyRequestReplayOutcome,
        now_ms: u64,
        replay_ttl_ms: u64,
    ) -> Result<bool, ProxyRequestIdempotencyStoreError> {
        MySqlStore::complete_proxy_request_idempotency(
            self,
            request_id,
            owner_token,
            outcome,
            now_ms,
            replay_ttl_ms,
        )
        .await
        .map_err(|err| ProxyRequestIdempotencyStoreError::new(err.to_string()))
    }

    async fn release_proxy_request_idempotency(
        &self,
        request_id: &str,
        owner_token: &str,
    ) -> Result<bool, ProxyRequestIdempotencyStoreError> {
        MySqlStore::release_proxy_request_idempotency(self, request_id, owner_token)
            .await
            .map_err(|err| ProxyRequestIdempotencyStoreError::new(err.to_string()))
    }
}

#[cfg(feature = "gateway-store-redis")]
#[async_trait]
impl ProxyRequestIdempotencyStore for RedisStore {
    async fn begin_proxy_request_idempotency(
        &self,
        request_id: &str,
        fingerprint: &ProxyRequestFingerprint,
        fingerprint_key: &str,
        owner_token: &str,
        now_ms: u64,
        lease_ttl_ms: u64,
    ) -> Result<ProxyRequestIdempotencyBeginOutcome, ProxyRequestIdempotencyStoreError> {
        RedisStore::begin_proxy_request_idempotency(
            self,
            request_id,
            fingerprint,
            fingerprint_key,
            owner_token,
            now_ms,
            lease_ttl_ms,
        )
        .await
        .map_err(|err| ProxyRequestIdempotencyStoreError::new(err.to_string()))
    }

    async fn get_proxy_request_idempotency(
        &self,
        request_id: &str,
        now_ms: u64,
    ) -> Result<Option<ProxyRequestIdempotencyRecord>, ProxyRequestIdempotencyStoreError> {
        RedisStore::get_proxy_request_idempotency(self, request_id, now_ms)
            .await
            .map_err(|err| ProxyRequestIdempotencyStoreError::new(err.to_string()))
    }

    async fn refresh_proxy_request_idempotency_lease(
        &self,
        request_id: &str,
        owner_token: &str,
        now_ms: u64,
        lease_ttl_ms: u64,
    ) -> Result<bool, ProxyRequestIdempotencyStoreError> {
        RedisStore::refresh_proxy_request_idempotency_lease(
            self,
            request_id,
            owner_token,
            now_ms,
            lease_ttl_ms,
        )
        .await
        .map_err(|err| ProxyRequestIdempotencyStoreError::new(err.to_string()))
    }

    async fn complete_proxy_request_idempotency(
        &self,
        request_id: &str,
        owner_token: &str,
        outcome: &ProxyRequestReplayOutcome,
        now_ms: u64,
        replay_ttl_ms: u64,
    ) -> Result<bool, ProxyRequestIdempotencyStoreError> {
        RedisStore::complete_proxy_request_idempotency(
            self,
            request_id,
            owner_token,
            outcome,
            now_ms,
            replay_ttl_ms,
        )
        .await
        .map_err(|err| ProxyRequestIdempotencyStoreError::new(err.to_string()))
    }

    async fn release_proxy_request_idempotency(
        &self,
        request_id: &str,
        owner_token: &str,
    ) -> Result<bool, ProxyRequestIdempotencyStoreError> {
        RedisStore::release_proxy_request_idempotency(self, request_id, owner_token)
            .await
            .map_err(|err| ProxyRequestIdempotencyStoreError::new(err.to_string()))
    }
}
