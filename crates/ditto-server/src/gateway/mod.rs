//! Gateway module (feature-gated).

pub mod adapters;
pub mod application;
#[doc(hidden)]
pub mod budget;
#[doc(hidden)]
pub mod cache;
pub mod config;
pub mod contracts;
#[cfg(feature = "gateway-costing")]
pub mod costing;
pub mod domain;
#[doc(hidden)]
pub mod guardrails;
#[doc(hidden)]
pub mod http;
#[doc(hidden)]
pub mod http_backend;
mod interop;
#[doc(hidden)]
pub mod limits;
#[cfg(feature = "gateway-config-yaml")]
pub mod litellm_config;
#[cfg(feature = "gateway-metrics-prometheus")]
#[doc(hidden)]
pub mod metrics_prometheus;
mod multipart;
#[cfg(feature = "gateway-store-mysql")]
#[doc(hidden)]
pub mod mysql_store;
pub mod observability;
#[cfg(feature = "gateway-otel")]
pub mod otel;
pub mod passthrough;
#[cfg(feature = "gateway-store-postgres")]
#[doc(hidden)]
pub mod postgres_store;
#[doc(hidden)]
pub mod proxy_backend;
#[cfg(feature = "gateway-proxy-cache")]
#[doc(hidden)]
pub mod proxy_cache;
#[cfg(feature = "gateway-routing-advanced")]
pub mod proxy_routing;
mod redaction;
#[cfg(feature = "gateway-store-redis")]
#[doc(hidden)]
pub mod redis_store;
mod responses_shim;
#[doc(hidden)]
pub mod router;
#[cfg(feature = "gateway-store-sqlite")]
#[doc(hidden)]
pub mod sqlite_store;
#[doc(hidden)]
pub mod state_file;
#[doc(hidden)]
pub mod store_types;
#[cfg(feature = "gateway-tokenizer")]
pub mod token_count;
#[cfg(feature = "gateway-translation")]
#[doc(hidden)]
pub mod translation;
pub mod transport;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

use async_trait::async_trait;
use omne_integrity_primitives::Sha256Hasher;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use self::config::normalize_virtual_key_token_key;
use domain::RateLimiter;
use domain::{BudgetTracker, ResponseCache, Router};
use observability::{Observability, ObservabilitySnapshot};

pub(crate) fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

pub(crate) fn read_unpoisoned<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(|poison| poison.into_inner())
}

pub(crate) fn write_unpoisoned<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(|poison| poison.into_inner())
}

pub use adapters::backend::{HttpBackend, ProxyBackend};
#[cfg(feature = "gateway-proxy-cache")]
pub use adapters::cache::{
    CachedProxyResponse, ProxyCacheConfig, ProxyCacheEntryMetadata, ProxyCachePurgeSelector,
    ProxyCacheStoredResponse, ProxyResponseCache,
};
pub use adapters::state::{GatewayStateFile, GatewayStateFileError};
#[cfg(feature = "gateway-store-mysql")]
pub use adapters::store::{MySqlStore, MySqlStoreError};
#[cfg(feature = "gateway-store-postgres")]
pub use adapters::store::{PostgresStore, PostgresStoreError};
#[cfg(feature = "gateway-store-redis")]
pub use adapters::store::{RedisStore, RedisStoreError};
#[cfg(feature = "gateway-store-sqlite")]
pub use adapters::store::{SqliteStore, SqliteStoreError};
#[cfg(feature = "gateway-translation")]
pub use application::translation::TranslationBackend;
pub use config::{
    BackendConfig, GatewayConfig, GatewayObservabilityConfig, GatewayRedactionConfig,
    GatewaySamplingConfig, VirtualKeyConfig,
};
#[cfg(feature = "gateway-costing")]
pub use costing::{PricingTable, PricingTableError};
pub use domain::{
    AuditLogRecord, BudgetConfig, BudgetLedgerRecord, CacheConfig, CostLedgerRecord,
    GuardrailsConfig, LimitsConfig, ProxyRequestFingerprint, ProxyRequestIdempotencyBeginOutcome,
    ProxyRequestIdempotencyRecord, ProxyRequestIdempotencyState, ProxyRequestIdempotencyStore,
    ProxyRequestIdempotencyStoreError, ProxyRequestReplayError, ProxyRequestReplayOutcome,
    ProxyRequestReplayResponse, RouteBackend, RouteRule, RouterConfig, StoredHttpHeader,
};
pub use passthrough::PassthroughConfig;
#[cfg(feature = "gateway-routing-advanced")]
pub use proxy_routing::{
    BackendHealthSnapshot, ProxyCircuitBreakerConfig, ProxyRetryConfig, ProxyRoutingConfig,
};
pub use transport::http::GatewayHttpState;

#[derive(Clone, Serialize, Deserialize)]
pub struct GatewayRequest {
    pub virtual_key: String,
    pub model: String,
    pub prompt: String,
    pub input_tokens: u32,
    pub max_output_tokens: u32,
    pub passthrough: bool,
}

impl std::fmt::Debug for GatewayRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        struct HexU64(u64);

        impl std::fmt::Debug for HexU64 {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{:016x}", self.0)
            }
        }

        f.debug_struct("GatewayRequest")
            .field("virtual_key", &"<redacted>")
            .field("virtual_key_len", &self.virtual_key.len())
            .field(
                "virtual_key_hash",
                &HexU64(hash64_fnv1a(self.virtual_key.as_bytes())),
            )
            .field("model", &self.model)
            .field("prompt", &"<redacted>")
            .field("prompt_len", &self.prompt.len())
            .field("prompt_hash", &HexU64(hash64_fnv1a(self.prompt.as_bytes())))
            .field("input_tokens", &self.input_tokens)
            .field("max_output_tokens", &self.max_output_tokens)
            .field("passthrough", &self.passthrough)
            .finish()
    }
}

impl GatewayRequest {
    pub fn total_tokens(&self) -> u32 {
        self.input_tokens.saturating_add(self.max_output_tokens)
    }

    pub fn cache_key_hash(&self) -> u64 {
        let mut hash = fnv1a64_init();
        hash = fnv1a64_update(hash, b"ditto-gateway-request-cache-key-v1|");
        hash = fnv1a64_update(hash, self.virtual_key.as_bytes());
        hash = fnv1a64_update(hash, b"|");
        hash = fnv1a64_update(hash, self.model.as_bytes());
        hash = fnv1a64_update(hash, b"|");
        hash = fnv1a64_update(hash, &u64::from(self.input_tokens).to_le_bytes());
        hash = fnv1a64_update(hash, b"|");
        hash = fnv1a64_update(hash, &u64::from(self.max_output_tokens).to_le_bytes());
        hash = fnv1a64_update(hash, b"|");
        hash = fnv1a64_update(hash, self.prompt.as_bytes());
        hash
    }

    pub(crate) fn route_seed_hash(&self, key_id: &str) -> u64 {
        let mut hash = fnv1a64_init();
        hash = fnv1a64_update(hash, b"ditto-gateway-route-seed-v1|");
        hash = fnv1a64_update(hash, key_id.as_bytes());
        hash = fnv1a64_update(hash, b"|");
        hash = fnv1a64_update(hash, self.model.as_bytes());
        hash = fnv1a64_update(hash, b"|");
        hash = fnv1a64_update(hash, &u64::from(self.input_tokens).to_le_bytes());
        hash = fnv1a64_update(hash, b"|");
        hash = fnv1a64_update(hash, &u64::from(self.max_output_tokens).to_le_bytes());
        hash = fnv1a64_update(hash, b"|");
        hash = fnv1a64_update(hash, self.prompt.as_bytes());
        hash
    }

    #[deprecated(
        note = "cache_key() previously returned raw request fields (including the virtual key and prompt); it now returns a stable, non-sensitive hex digest. Prefer `cache_key_hash()`."
    )]
    pub fn cache_key(&self) -> String {
        format!("{:016x}", self.cache_key_hash())
    }
}

fn control_plane_cache_key(key_id: &str, request: &GatewayRequest) -> String {
    let mut hasher = Sha256Hasher::new();
    hasher.update(b"ditto-gateway-cache-v2|");
    hasher.update(key_id.as_bytes());
    hasher.update(b"|");
    hasher.update(request.model.as_bytes());
    hasher.update(b"|");
    hasher.update(u64::from(request.input_tokens).to_le_bytes());
    hasher.update(b"|");
    hasher.update(u64::from(request.max_output_tokens).to_le_bytes());
    hasher.update(b"|");
    hasher.update(request.prompt.as_bytes());
    hasher.update(b"|");
    hasher.update(
        u64::try_from(request.prompt.len())
            .unwrap_or(u64::MAX)
            .to_le_bytes(),
    );

    hasher.finalize().to_string()
}

fn hash64_fnv1a(bytes: &[u8]) -> u64 {
    fnv1a64_update(fnv1a64_init(), bytes)
}

const FNV1A64_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV1A64_PRIME: u64 = 0x100000001b3;

fn fnv1a64_init() -> u64 {
    FNV1A64_OFFSET_BASIS
}

fn fnv1a64_update(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV1A64_PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestBackend;

    #[async_trait]
    impl Backend for TestBackend {
        async fn call(&self, _request: &GatewayRequest) -> Result<GatewayResponse, GatewayError> {
            Ok(GatewayResponse {
                content: "ok".to_string(),
                output_tokens: 1,
                backend: "primary".to_string(),
                cached: false,
            })
        }
    }

    #[test]
    fn gateway_request_cache_key_is_not_raw() {
        let request = GatewayRequest {
            virtual_key: "sk-test-secret".to_string(),
            model: "gpt-test".to_string(),
            prompt: "hello secret prompt".to_string(),
            input_tokens: 1,
            max_output_tokens: 2,
            passthrough: false,
        };

        #[allow(deprecated)]
        let key = request.cache_key();
        assert_eq!(key.len(), 16);
        assert!(key.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(!key.contains("sk-test-secret"));
        assert!(!key.contains("hello secret prompt"));
    }

    #[test]
    fn control_plane_cache_key_changes_when_prompt_changes() {
        let key_id = "vk_1";
        let request_a = GatewayRequest {
            virtual_key: "sk-test-secret".to_string(),
            model: "gpt-test".to_string(),
            prompt: "hello prompt a".to_string(),
            input_tokens: 1,
            max_output_tokens: 2,
            passthrough: false,
        };
        let request_b = GatewayRequest {
            prompt: "hello prompt b".to_string(),
            ..request_a.clone()
        };

        let cache_key_a = control_plane_cache_key(key_id, &request_a);
        let cache_key_b = control_plane_cache_key(key_id, &request_b);

        assert_eq!(cache_key_a.len(), 64);
        assert_eq!(cache_key_b.len(), 64);
        assert_ne!(cache_key_a, cache_key_b);
        assert!(!cache_key_a.contains("hello prompt a"));
        assert!(!cache_key_b.contains("hello prompt b"));
    }

    #[test]
    fn virtual_key_token_index_updates_after_upsert() {
        let config = GatewayConfig {
            backends: Vec::new(),
            virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-old")],
            router: router::RouterConfig {
                default_backends: Vec::new(),
                rules: Vec::new(),
            },
            a2a_agents: Vec::new(),
            mcp_servers: Vec::new(),
            observability: Default::default(),
        };
        let gateway = Gateway::new(config);
        assert!(gateway.virtual_key_by_token("vk-old").is_some());

        let mut updated = VirtualKeyConfig::new("key-1", "vk-new");
        updated.enabled = true;
        gateway.upsert_virtual_key(updated);

        assert!(gateway.virtual_key_by_token("vk-old").is_none());
        assert!(gateway.virtual_key_by_token("vk-new").is_some());
    }

    #[test]
    fn prepare_handle_request_reserves_budget_until_completed_or_rolled_back() {
        let mut key = VirtualKeyConfig::new("key-1", "vk-1");
        key.budget.total_tokens = Some(3);

        let config = GatewayConfig {
            backends: Vec::new(),
            virtual_keys: vec![key],
            router: router::RouterConfig {
                default_backends: vec![RouteBackend {
                    backend: "primary".to_string(),
                    weight: 1.0,
                }],
                rules: Vec::new(),
            },
            a2a_agents: Vec::new(),
            mcp_servers: Vec::new(),
            observability: Default::default(),
        };
        let mut gateway = Gateway::new(config);
        gateway.register_backend("primary", TestBackend);

        let request = GatewayRequest {
            virtual_key: "vk-1".to_string(),
            model: "gpt-test".to_string(),
            prompt: "hello".to_string(),
            input_tokens: 1,
            max_output_tokens: 2,
            passthrough: false,
        };

        let prepared = match gateway
            .prepare_handle_request(&request)
            .expect("first request should pass")
        {
            GatewayPreparedRequest::Call(prepared) => prepared,
            GatewayPreparedRequest::Cached { .. } => panic!("expected backend call"),
        };

        let second = gateway.prepare_handle_request(&request);
        assert!(matches!(second, Err(GatewayError::BudgetExceeded { .. })));

        gateway.complete_handle_failure(&prepared);

        let third = gateway.prepare_handle_request(&request);
        assert!(matches!(third, Ok(GatewayPreparedRequest::Call(_))));
    }

    #[test]
    fn in_flight_request_does_not_repopulate_cache_after_cache_disable() {
        let mut key = VirtualKeyConfig::new("key-1", "vk-1");
        key.cache.enabled = true;
        key.cache.ttl_seconds = Some(60);

        let config = GatewayConfig {
            backends: Vec::new(),
            virtual_keys: vec![key.clone()],
            router: router::RouterConfig {
                default_backends: vec![RouteBackend {
                    backend: "primary".to_string(),
                    weight: 1.0,
                }],
                rules: Vec::new(),
            },
            a2a_agents: Vec::new(),
            mcp_servers: Vec::new(),
            observability: Default::default(),
        };
        let mut gateway = Gateway::new(config);
        gateway.register_backend("primary", TestBackend);

        let request = GatewayRequest {
            virtual_key: "vk-1".to_string(),
            model: "gpt-test".to_string(),
            prompt: "hello".to_string(),
            input_tokens: 1,
            max_output_tokens: 2,
            passthrough: false,
        };

        let prepared = match gateway
            .prepare_handle_request(&request)
            .expect("request should pass")
        {
            GatewayPreparedRequest::Call(prepared) => prepared,
            GatewayPreparedRequest::Cached { .. } => panic!("expected backend call"),
        };
        let cache_key = prepared
            .cache_key
            .clone()
            .expect("cache should be enabled for prepared call");

        let mut updated = key;
        updated.cache.enabled = false;
        gateway.upsert_virtual_key(updated);

        let response = GatewayResponse {
            content: "ok".to_string(),
            output_tokens: 1,
            backend: "primary".to_string(),
            cached: false,
        };
        gateway.complete_handle_success(&prepared, &response);

        let now = gateway.clock.now_epoch_seconds();
        assert!(
            lock_unpoisoned(gateway.cache.as_ref())
                .get("key-1", &cache_key, now)
                .is_none()
        );
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GatewayResponse {
    pub content: String,
    pub output_tokens: u32,
    pub backend: String,
    pub cached: bool,
}

#[derive(Debug, Error)]
pub enum GatewayError {
    #[error("unauthorized virtual key")]
    Unauthorized,
    #[error("rate limit exceeded: {limit}")]
    RateLimited { limit: String },
    #[error("guardrail rejected: {reason}")]
    GuardrailRejected { reason: String },
    #[error("budget exceeded: limit={limit} attempted={attempted}")]
    BudgetExceeded { limit: u64, attempted: u64 },
    #[error(
        "cost budget exceeded: limit_usd_micros={limit_usd_micros} attempted_usd_micros={attempted_usd_micros}"
    )]
    CostBudgetExceeded {
        limit_usd_micros: u64,
        attempted_usd_micros: u64,
    },
    #[error("backend not found: {name}")]
    BackendNotFound { name: String },
    #[error("backend error: {message}")]
    Backend { message: String },
    #[error("backend timeout: {message}")]
    BackendTimeout { message: String },
    #[error("invalid request: {reason}")]
    InvalidRequest { reason: String },
}

#[async_trait]
pub trait Backend: Send + Sync {
    async fn call(&self, request: &GatewayRequest) -> Result<GatewayResponse, GatewayError>;
}

pub trait Clock: Send + Sync {
    fn now_epoch_seconds(&self) -> u64;
}

#[derive(Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_epoch_seconds(&self) -> u64 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_else(|_| std::time::Duration::from_secs(0));
        now.as_secs()
    }
}

struct GatewayControlPlane {
    config: GatewayConfig,
    virtual_key_token_index: HashMap<String, usize>,
    backends: HashMap<String, Arc<dyn Backend>>,
    router: Router,
}

impl GatewayControlPlane {
    fn new(config: GatewayConfig) -> Self {
        let router = Router::new(config.router.clone());
        let mut control_plane = Self {
            config,
            virtual_key_token_index: HashMap::new(),
            backends: HashMap::new(),
            router,
        };
        control_plane.rebuild_virtual_key_token_index();
        control_plane
    }

    fn list_virtual_keys(&self) -> Vec<VirtualKeyConfig> {
        self.config.virtual_keys.clone()
    }

    fn replace_virtual_keys(&mut self, keys: Vec<VirtualKeyConfig>) {
        self.config.virtual_keys = keys;
        self.rebuild_virtual_key_token_index();
    }

    fn router_config(&self) -> RouterConfig {
        self.config.router.clone()
    }

    fn replace_router_config(&mut self, router: RouterConfig) {
        self.config.router = router.clone();
        self.router = Router::new(router);
    }

    fn backend_names(&self) -> Vec<String> {
        let mut names = std::collections::BTreeSet::new();
        for backend in &self.config.backends {
            names.insert(backend.name.clone());
        }
        names.extend(self.backends.keys().cloned());
        names.into_iter().collect()
    }

    fn backend_model_maps(&self) -> HashMap<String, BTreeMap<String, String>> {
        self.config
            .backends
            .iter()
            .map(|backend| (backend.name.clone(), backend.model_map.clone()))
            .collect()
    }

    fn config_snapshot(&self) -> GatewayConfig {
        self.config.clone()
    }

    fn virtual_key_by_token(&self, token: &str) -> Option<&VirtualKeyConfig> {
        if let Some(token_key) = normalize_virtual_key_token_key(token)
            && let Some(index) = self.virtual_key_token_index.get(&token_key).copied()
            && let Some(key) = self.config.virtual_keys.get(index)
            && key.matches_token(token)
        {
            return Some(key);
        }
        self.config
            .virtual_keys
            .iter()
            .find(|key| key.matches_token(token))
    }

    fn upsert_virtual_key(&mut self, key: VirtualKeyConfig) -> bool {
        if let Some(existing) = self
            .config
            .virtual_keys
            .iter_mut()
            .find(|candidate| candidate.id == key.id)
        {
            *existing = key;
            self.rebuild_virtual_key_token_index();
            false
        } else {
            self.config.virtual_keys.push(key);
            self.rebuild_virtual_key_token_index();
            true
        }
    }

    fn remove_virtual_key(&mut self, id: &str) -> Option<VirtualKeyConfig> {
        let index = self
            .config
            .virtual_keys
            .iter()
            .position(|key| key.id == id)?;
        let removed = self.config.virtual_keys.remove(index);
        self.rebuild_virtual_key_token_index();
        Some(removed)
    }

    fn rebuild_virtual_key_token_index(&mut self) {
        self.virtual_key_token_index.clear();
        for (idx, key) in self.config.virtual_keys.iter().enumerate() {
            if let Some(token_key) = key.token_lookup_key() {
                self.virtual_key_token_index.entry(token_key).or_insert(idx);
            }
        }
    }
}

pub struct Gateway {
    control_plane: RwLock<GatewayControlPlane>,
    limits: Arc<Mutex<RateLimiter>>,
    cache: Arc<Mutex<ResponseCache>>,
    budget: Arc<Mutex<BudgetTracker>>,
    observability: Arc<Mutex<Observability>>,
    clock: Box<dyn Clock>,
}

#[derive(Clone)]
pub(crate) struct GatewayRuntimeSnapshot {
    virtual_keys: Vec<VirtualKeyConfig>,
    router: RouterConfig,
    limits: RateLimiter,
    cache: ResponseCache,
    budget: BudgetTracker,
}

pub(crate) struct GatewayMutation<'a> {
    control_plane: &'a mut GatewayControlPlane,
    cache: &'a Mutex<ResponseCache>,
}

impl GatewayMutation<'_> {
    pub(crate) fn list_virtual_keys(&self) -> Vec<VirtualKeyConfig> {
        self.control_plane.list_virtual_keys()
    }

    pub(crate) fn replace_virtual_keys(&mut self, keys: Vec<VirtualKeyConfig>) {
        self.control_plane.replace_virtual_keys(keys);
    }

    pub(crate) fn replace_router_config(&mut self, router: RouterConfig) {
        self.control_plane.replace_router_config(router);
    }

    pub(crate) fn upsert_virtual_key(&mut self, key: VirtualKeyConfig) -> bool {
        if self
            .control_plane
            .config
            .virtual_keys
            .iter()
            .find(|candidate| candidate.id == key.id)
            .is_some_and(|existing| existing.cache != key.cache)
        {
            lock_unpoisoned(self.cache).remove_scope(&key.id);
        }
        self.control_plane.upsert_virtual_key(key)
    }

    pub(crate) fn remove_virtual_key(&mut self, id: &str) -> Option<VirtualKeyConfig> {
        self.control_plane.remove_virtual_key(id)
    }
}

pub(crate) struct GatewayPreparedCall {
    pub(crate) key_id: String,
    pub(crate) backend: Arc<dyn Backend>,
    pub(crate) backend_name: String,
    tokens: u64,
    key_budget: BudgetConfig,
    tenant_budget_scope: Option<(String, BudgetConfig)>,
    project_budget_scope: Option<(String, BudgetConfig)>,
    user_budget_scope: Option<(String, BudgetConfig)>,
    cache_key: Option<String>,
}

pub(crate) enum GatewayPreparedRequest {
    Cached {
        key_id: String,
        response: GatewayResponse,
    },
    Call(Box<GatewayPreparedCall>),
}

impl Gateway {
    pub fn new(config: GatewayConfig) -> Self {
        Self::with_clock(config, Box::new(SystemClock))
    }

    pub fn with_clock(config: GatewayConfig, clock: Box<dyn Clock>) -> Self {
        Self {
            control_plane: RwLock::new(GatewayControlPlane::new(config)),
            limits: Arc::new(Mutex::new(RateLimiter::default())),
            cache: Arc::new(Mutex::new(ResponseCache::default())),
            budget: Arc::new(Mutex::new(BudgetTracker::default())),
            observability: Arc::new(Mutex::new(Observability::default())),
            clock,
        }
    }

    fn with_control_plane<R>(&self, f: impl FnOnce(&GatewayControlPlane) -> R) -> R {
        let control_plane = read_unpoisoned(&self.control_plane);
        f(&control_plane)
    }

    pub(crate) fn mutate_control_plane<R>(
        &self,
        f: impl FnOnce(&mut GatewayMutation<'_>) -> R,
    ) -> R {
        let result = {
            let mut control_plane = write_unpoisoned(&self.control_plane);
            let mut mutation = GatewayMutation {
                control_plane: &mut control_plane,
                cache: &self.cache,
            };
            f(&mut mutation)
        };
        self.prune_internal_scopes();
        result
    }

    pub fn register_backend(&mut self, name: impl Into<String>, backend: impl Backend + 'static) {
        let control_plane = self
            .control_plane
            .get_mut()
            .unwrap_or_else(|poison| poison.into_inner());
        control_plane
            .backends
            .insert(name.into(), Arc::new(backend));
    }

    pub fn observability(&self) -> ObservabilitySnapshot {
        lock_unpoisoned(&self.observability).snapshot()
    }

    pub(crate) fn config_snapshot(&self) -> GatewayConfig {
        self.with_control_plane(GatewayControlPlane::config_snapshot)
    }

    pub(crate) fn runtime_snapshot(&self) -> GatewayRuntimeSnapshot {
        self.with_control_plane(|control_plane| GatewayRuntimeSnapshot {
            virtual_keys: control_plane.list_virtual_keys(),
            router: control_plane.router_config(),
            limits: lock_unpoisoned(&self.limits).clone(),
            cache: lock_unpoisoned(&self.cache).clone(),
            budget: lock_unpoisoned(&self.budget).clone(),
        })
    }

    pub(crate) fn backend_model_maps(&self) -> HashMap<String, BTreeMap<String, String>> {
        self.with_control_plane(GatewayControlPlane::backend_model_maps)
    }

    pub fn list_virtual_keys(&self) -> Vec<VirtualKeyConfig> {
        self.with_control_plane(GatewayControlPlane::list_virtual_keys)
    }

    pub fn replace_virtual_keys(&self, keys: Vec<VirtualKeyConfig>) {
        self.mutate_control_plane(|mutation| mutation.replace_virtual_keys(keys));
    }

    pub fn router_config(&self) -> RouterConfig {
        self.with_control_plane(GatewayControlPlane::router_config)
    }

    pub fn replace_router_config(&self, router: RouterConfig) {
        self.mutate_control_plane(|mutation| mutation.replace_router_config(router));
    }

    pub fn backend_names(&self) -> Vec<String> {
        self.with_control_plane(GatewayControlPlane::backend_names)
    }

    #[cfg(test)]
    pub(crate) fn virtual_key_by_token(&self, token: &str) -> Option<VirtualKeyConfig> {
        self.with_control_plane(|control_plane| control_plane.virtual_key_by_token(token).cloned())
    }

    pub fn upsert_virtual_key(&self, key: VirtualKeyConfig) -> bool {
        self.mutate_control_plane(|mutation| mutation.upsert_virtual_key(key))
    }

    pub fn remove_virtual_key(&self, id: &str) -> Option<VirtualKeyConfig> {
        self.mutate_control_plane(|mutation| mutation.remove_virtual_key(id))
    }

    pub(crate) fn restore_runtime_snapshot(&self, snapshot: &GatewayRuntimeSnapshot) {
        {
            let mut control_plane = write_unpoisoned(&self.control_plane);
            control_plane.replace_virtual_keys(snapshot.virtual_keys.clone());
            control_plane.replace_router_config(snapshot.router.clone());
        }
        *lock_unpoisoned(&self.limits) = snapshot.limits.clone();
        *lock_unpoisoned(&self.cache) = snapshot.cache.clone();
        *lock_unpoisoned(&self.budget) = snapshot.budget.clone();
    }

    fn prune_internal_scopes(&self) {
        let virtual_keys = self.list_virtual_keys();
        let mut scopes = HashSet::<String>::new();

        for key in &virtual_keys {
            scopes.insert(key.id.clone());

            if let Some(tenant_id) = key
                .tenant_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
            {
                scopes.insert(format!("tenant:{tenant_id}"));
            }

            if let Some(project_id) = key
                .project_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
            {
                scopes.insert(format!("project:{project_id}"));
            }

            if let Some(user_id) = key
                .user_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
            {
                scopes.insert(format!("user:{user_id}"));
            }
        }

        lock_unpoisoned(&self.limits).retain_scopes(&scopes);
        lock_unpoisoned(&self.budget).retain_scopes(&scopes);
        lock_unpoisoned(&self.cache).retain_scopes(&scopes);
    }

    pub(crate) fn prepare_handle_request(
        &self,
        request: &GatewayRequest,
    ) -> Result<GatewayPreparedRequest, GatewayError> {
        lock_unpoisoned(&self.observability).record_request();

        let control_plane = read_unpoisoned(&self.control_plane);
        let key = control_plane
            .virtual_key_by_token(&request.virtual_key)
            .ok_or(GatewayError::Unauthorized)?
            .clone();

        if !key.enabled {
            return Err(GatewayError::Unauthorized);
        }

        let now = self.clock.now_epoch_seconds();
        let minute = now / 60;
        let tokens = request.total_tokens();

        if let Err(err) =
            lock_unpoisoned(&self.limits).check_and_consume(&key.id, &key.limits, tokens, minute)
        {
            lock_unpoisoned(&self.observability).record_rate_limited();
            return Err(err);
        }

        if let (Some(tenant_id), Some(tenant_limits)) = (
            key.tenant_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty()),
            key.tenant_limits.as_ref(),
        ) {
            let scope = format!("tenant:{tenant_id}");
            if let Err(err) = lock_unpoisoned(&self.limits).check_and_consume(
                &scope,
                tenant_limits,
                tokens,
                minute,
            ) {
                lock_unpoisoned(&self.observability).record_rate_limited();
                return Err(err);
            }
        }

        if let (Some(project_id), Some(project_limits)) = (
            key.project_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty()),
            key.project_limits.as_ref(),
        ) {
            let scope = format!("project:{project_id}");
            if let Err(err) = lock_unpoisoned(&self.limits).check_and_consume(
                &scope,
                project_limits,
                tokens,
                minute,
            ) {
                lock_unpoisoned(&self.observability).record_rate_limited();
                return Err(err);
            }
        }

        if let (Some(user_id), Some(user_limits)) = (
            key.user_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty()),
            key.user_limits.as_ref(),
        ) {
            let scope = format!("user:{user_id}");
            if let Err(err) =
                lock_unpoisoned(&self.limits).check_and_consume(&scope, user_limits, tokens, minute)
            {
                lock_unpoisoned(&self.observability).record_rate_limited();
                return Err(err);
            }
        }

        let guardrails = control_plane
            .router
            .rule_for_model(&request.model, Some(&key))
            .and_then(|rule| rule.guardrails.as_ref())
            .unwrap_or(&key.guardrails);

        if let Err(err) = guardrails.check(request) {
            lock_unpoisoned(&self.observability).record_guardrail_blocked();
            return Err(err);
        }

        if let Err(err) = key.passthrough.validate(request) {
            lock_unpoisoned(&self.observability).record_guardrail_blocked();
            return Err(err);
        }

        let backend_name = control_plane.router.select_backend(request, &key)?;

        let bypass_cache = key.passthrough.bypass_cache(request);
        let cache_key =
            (key.cache.enabled && !bypass_cache).then(|| control_plane_cache_key(&key.id, request));
        if let Some(cache_key) = cache_key.as_deref()
            && let Some(mut cached) = lock_unpoisoned(&self.cache).get(&key.id, cache_key, now)
        {
            cached.cached = true;
            lock_unpoisoned(&self.observability).record_cache_hit();
            return Ok(GatewayPreparedRequest::Cached {
                key_id: key.id.clone(),
                response: cached,
            });
        }

        if let Err(err) =
            lock_unpoisoned(&self.budget).can_spend(&key.id, &key.budget, u64::from(tokens))
        {
            lock_unpoisoned(&self.observability).record_budget_exceeded();
            return Err(err);
        }

        if let (Some(tenant_id), Some(tenant_budget)) = (
            key.tenant_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty()),
            key.tenant_budget.as_ref(),
        ) {
            let scope = format!("tenant:{tenant_id}");
            if let Err(err) =
                lock_unpoisoned(&self.budget).can_spend(&scope, tenant_budget, u64::from(tokens))
            {
                lock_unpoisoned(&self.observability).record_budget_exceeded();
                return Err(err);
            }
        }

        if let (Some(project_id), Some(project_budget)) = (
            key.project_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty()),
            key.project_budget.as_ref(),
        ) {
            let scope = format!("project:{project_id}");
            if let Err(err) =
                lock_unpoisoned(&self.budget).can_spend(&scope, project_budget, u64::from(tokens))
            {
                lock_unpoisoned(&self.observability).record_budget_exceeded();
                return Err(err);
            }
        }
        if let (Some(user_id), Some(user_budget)) = (
            key.user_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty()),
            key.user_budget.as_ref(),
        ) {
            let scope = format!("user:{user_id}");
            if let Err(err) =
                lock_unpoisoned(&self.budget).can_spend(&scope, user_budget, u64::from(tokens))
            {
                lock_unpoisoned(&self.observability).record_budget_exceeded();
                return Err(err);
            }
        }

        let backend = control_plane
            .backends
            .get(&backend_name)
            .cloned()
            .ok_or_else(|| GatewayError::BackendNotFound {
                name: backend_name.clone(),
            })?;

        lock_unpoisoned(&self.observability).record_backend_call();

        let tenant_budget_scope = key
            .tenant_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .and_then(|tenant_id| {
                key.tenant_budget
                    .as_ref()
                    .map(|budget| (format!("tenant:{tenant_id}"), budget.clone()))
            });
        let project_budget_scope = key
            .project_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .and_then(|project_id| {
                key.project_budget
                    .as_ref()
                    .map(|budget| (format!("project:{project_id}"), budget.clone()))
            });
        let user_budget_scope = key
            .user_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .and_then(|user_id| {
                key.user_budget
                    .as_ref()
                    .map(|budget| (format!("user:{user_id}"), budget.clone()))
            });

        // Reserve token budgets before returning so concurrent requests cannot
        // pass budget checks against stale spent counters.
        lock_unpoisoned(&self.budget).spend(&key.id, &key.budget, u64::from(tokens));
        if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
            lock_unpoisoned(&self.budget).spend(scope, budget, u64::from(tokens));
        }
        if let Some((scope, budget)) = project_budget_scope.as_ref() {
            lock_unpoisoned(&self.budget).spend(scope, budget, u64::from(tokens));
        }
        if let Some((scope, budget)) = user_budget_scope.as_ref() {
            lock_unpoisoned(&self.budget).spend(scope, budget, u64::from(tokens));
        }

        Ok(GatewayPreparedRequest::Call(Box::new(
            GatewayPreparedCall {
                key_id: key.id,
                backend,
                backend_name,
                tokens: u64::from(tokens),
                key_budget: key.budget,
                tenant_budget_scope,
                project_budget_scope,
                user_budget_scope,
                cache_key,
            },
        )))
    }

    pub(crate) fn complete_handle_success(
        &self,
        prepared: &GatewayPreparedCall,
        response: &GatewayResponse,
    ) {
        let Some(cache_key) = prepared.cache_key.as_ref() else {
            return;
        };
        let Some(cache_config) = self.with_control_plane(|control_plane| {
            control_plane
                .config
                .virtual_keys
                .iter()
                .find(|key| key.id == prepared.key_id)
                .and_then(|key| {
                    if key.enabled && key.cache.enabled {
                        Some(key.cache.clone())
                    } else {
                        None
                    }
                })
        }) else {
            return;
        };

        let now = self.clock.now_epoch_seconds();
        lock_unpoisoned(&self.cache).insert(
            &prepared.key_id,
            cache_key.clone(),
            response.clone(),
            &cache_config,
            now,
        );
    }

    pub(crate) fn complete_handle_failure(&self, prepared: &GatewayPreparedCall) {
        lock_unpoisoned(&self.budget).refund(
            &prepared.key_id,
            &prepared.key_budget,
            prepared.tokens,
        );
        if let Some((scope, budget)) = prepared.tenant_budget_scope.as_ref() {
            lock_unpoisoned(&self.budget).refund(scope, budget, prepared.tokens);
        }
        if let Some((scope, budget)) = prepared.project_budget_scope.as_ref() {
            lock_unpoisoned(&self.budget).refund(scope, budget, prepared.tokens);
        }
        if let Some((scope, budget)) = prepared.user_budget_scope.as_ref() {
            lock_unpoisoned(&self.budget).refund(scope, budget, prepared.tokens);
        }
    }

    pub async fn handle(&self, request: GatewayRequest) -> Result<GatewayResponse, GatewayError> {
        match self.prepare_handle_request(&request)? {
            GatewayPreparedRequest::Cached { response, .. } => Ok(response),
            GatewayPreparedRequest::Call(prepared) => {
                let mut response = match prepared.backend.call(&request).await {
                    Ok(response) => response,
                    Err(err) => {
                        self.complete_handle_failure(&prepared);
                        return Err(err);
                    }
                };
                response.backend = prepared.backend_name.clone();
                response.cached = false;
                self.complete_handle_success(&prepared, &response);
                Ok(response)
            }
        }
    }
}

impl GatewayConfig {
    pub fn virtual_key(&self, token: &str) -> Option<&VirtualKeyConfig> {
        self.virtual_keys
            .iter()
            .find(|key| key.matches_token(token))
    }
}
