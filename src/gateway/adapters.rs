//! Gateway adapter facades.

pub mod backend {
    pub use super::super::{HttpBackend, ProxyBackend};
}

pub mod cache {
    pub use super::super::CacheConfig;
    #[cfg(feature = "gateway-proxy-cache")]
    pub use super::super::{
        CachedProxyResponse, ProxyCacheConfig, ProxyCacheEntryMetadata, ProxyCachePurgeSelector,
        ProxyCacheStoredResponse, ProxyResponseCache,
    };
}

pub mod state {
    pub use super::super::{GatewayStateFile, GatewayStateFileError};
}

pub mod store {
    pub use super::super::store_types::{AuditLogRecord, BudgetLedgerRecord, CostLedgerRecord};
    #[cfg(feature = "gateway-store-mysql")]
    pub use super::super::{MySqlStore, MySqlStoreError};
    #[cfg(feature = "gateway-store-postgres")]
    pub use super::super::{PostgresStore, PostgresStoreError};
    #[cfg(feature = "gateway-store-redis")]
    pub use super::super::{RedisStore, RedisStoreError};
    #[cfg(feature = "gateway-store-sqlite")]
    pub use super::super::{SqliteStore, SqliteStoreError};
}
