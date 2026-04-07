//! Gateway cache adapters.

#[cfg(feature = "gateway-proxy-cache")]
pub mod proxy_cache;

#[cfg(feature = "gateway-proxy-cache")]
pub use proxy_cache::{
    CachedProxyResponse, ProxyCacheConfig, ProxyCacheEntryMetadata, ProxyCachePurgeSelector,
    ProxyCacheStoredResponse, ProxyResponseCache,
};
