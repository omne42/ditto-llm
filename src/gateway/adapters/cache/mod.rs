//! Gateway cache adapters.

mod local_lru;
#[cfg(feature = "gateway-proxy-cache")]
pub mod proxy_cache;

#[cfg(feature = "gateway-translation")]
pub(crate) use local_lru::LocalLruCache;
#[cfg(feature = "gateway-proxy-cache")]
pub use proxy_cache::{
    CachedProxyResponse, ProxyCacheConfig, ProxyCacheEntryMetadata, ProxyCachePurgeSelector,
    ProxyCacheStoredResponse, ProxyResponseCache,
};
