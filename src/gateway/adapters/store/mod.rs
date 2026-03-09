//! Gateway persistence adapters.

use super::super::{
    AuditLogRecord, BudgetLedgerRecord, CostLedgerRecord, GatewayError, LimitsConfig, RouterConfig,
    VirtualKeyConfig,
};
#[cfg(feature = "gateway-proxy-cache")]
use super::super::{
    CachedProxyResponse, ProxyCacheEntryMetadata, ProxyCachePurgeSelector, ProxyCacheStoredResponse,
};
#[cfg(feature = "gateway-store-mysql")]
pub mod mysql;
#[cfg(feature = "gateway-store-postgres")]
pub mod postgres;
#[cfg(feature = "gateway-store-redis")]
pub mod redis;
#[cfg(feature = "gateway-store-sqlite")]
pub mod sqlite;

#[cfg(feature = "gateway-store-mysql")]
pub use mysql::{MySqlStore, MySqlStoreError};
#[cfg(feature = "gateway-store-postgres")]
pub use postgres::{PostgresStore, PostgresStoreError};
#[cfg(feature = "gateway-store-redis")]
pub use redis::{RedisStore, RedisStoreError};
#[cfg(feature = "gateway-store-sqlite")]
pub use sqlite::{SqliteStore, SqliteStoreError};
