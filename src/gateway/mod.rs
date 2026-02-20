//! Gateway module (feature-gated).

pub mod budget;
pub mod cache;
pub mod config;
#[cfg(feature = "gateway-costing")]
pub mod costing;
pub mod guardrails;
pub mod http;
pub mod http_backend;
mod interop;
pub mod limits;
#[cfg(feature = "gateway-config-yaml")]
pub mod litellm_config;
#[cfg(feature = "gateway-metrics-prometheus")]
pub mod metrics_prometheus;
mod multipart;
pub mod observability;
#[cfg(feature = "gateway-otel")]
pub mod otel;
pub mod passthrough;
pub mod proxy_backend;
#[cfg(feature = "gateway-proxy-cache")]
pub mod proxy_cache;
#[cfg(feature = "gateway-routing-advanced")]
pub mod proxy_routing;
mod redaction;
#[cfg(feature = "gateway-store-redis")]
pub mod redis_store;
mod responses_shim;
pub mod router;
#[cfg(feature = "gateway-store-sqlite")]
pub mod sqlite_store;
pub mod state_file;
pub mod store_types;
#[cfg(feature = "gateway-tokenizer")]
pub mod token_count;
#[cfg(feature = "gateway-translation")]
pub mod translation;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use budget::BudgetTracker;
use cache::ResponseCache;
use limits::RateLimiter;
use observability::{Observability, ObservabilitySnapshot};
use router::Router;

pub use budget::BudgetConfig;
pub use cache::CacheConfig;
pub use config::{
    BackendConfig, GatewayConfig, GatewayObservabilityConfig, GatewayRedactionConfig,
    VirtualKeyConfig,
};
#[cfg(feature = "gateway-costing")]
pub use costing::{PricingTable, PricingTableError};
pub use guardrails::GuardrailsConfig;
pub use http::GatewayHttpState;
pub use http_backend::HttpBackend;
pub use limits::LimitsConfig;
pub use passthrough::PassthroughConfig;
pub use proxy_backend::ProxyBackend;
#[cfg(feature = "gateway-proxy-cache")]
pub use proxy_cache::{CachedProxyResponse, ProxyCacheConfig, ProxyResponseCache};
#[cfg(feature = "gateway-routing-advanced")]
pub use proxy_routing::{
    BackendHealthSnapshot, ProxyCircuitBreakerConfig, ProxyRetryConfig, ProxyRoutingConfig,
};
#[cfg(feature = "gateway-store-redis")]
pub use redis_store::{RedisStore, RedisStoreError};
pub use router::{RouteBackend, RouteRule, RouterConfig};
#[cfg(feature = "gateway-store-sqlite")]
pub use sqlite_store::{SqliteStore, SqliteStoreError};
pub use state_file::{GatewayStateFile, GatewayStateFileError};
pub use store_types::{AuditLogRecord, BudgetLedgerRecord, CostLedgerRecord};
#[cfg(feature = "gateway-translation")]
pub use translation::TranslationBackend;

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
    use sha2::Digest as _;

    let mut hasher = sha2::Sha256::new();
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

    hex_lower(&hasher.finalize())
}

fn hash64_fnv1a(bytes: &[u8]) -> u64 {
    fnv1a64_update(fnv1a64_init(), bytes)
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
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
        let mut gateway = Gateway::new(config);
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
        assert!(gateway.cache.get("key-1", &cache_key, now).is_none());
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

pub struct Gateway {
    config: GatewayConfig,
    virtual_key_token_index: HashMap<String, usize>,
    backends: HashMap<String, Arc<dyn Backend>>,
    limits: RateLimiter,
    cache: ResponseCache,
    budget: BudgetTracker,
    router: Router,
    observability: Observability,
    clock: Box<dyn Clock>,
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
        let router = Router::new(config.router.clone());
        let mut gateway = Self {
            config,
            virtual_key_token_index: HashMap::new(),
            backends: HashMap::new(),
            limits: RateLimiter::default(),
            cache: ResponseCache::default(),
            budget: BudgetTracker::default(),
            router,
            observability: Observability::default(),
            clock,
        };
        gateway.rebuild_virtual_key_token_index();
        gateway
    }

    pub fn register_backend(&mut self, name: impl Into<String>, backend: impl Backend + 'static) {
        self.backends.insert(name.into(), Arc::new(backend));
    }

    pub fn observability(&self) -> ObservabilitySnapshot {
        self.observability.snapshot()
    }

    pub fn list_virtual_keys(&self) -> Vec<VirtualKeyConfig> {
        self.config.virtual_keys.clone()
    }

    pub(crate) fn virtual_key_by_token(&self, token: &str) -> Option<&VirtualKeyConfig> {
        if let Some(index) = self.virtual_key_token_index.get(token).copied() {
            if let Some(key) = self.config.virtual_keys.get(index) {
                if key.token == token {
                    return Some(key);
                }
            }
        }
        self.config
            .virtual_keys
            .iter()
            .find(|key| key.token == token)
    }

    fn rebuild_virtual_key_token_index(&mut self) {
        self.virtual_key_token_index.clear();
        for (idx, key) in self.config.virtual_keys.iter().enumerate() {
            self.virtual_key_token_index
                .entry(key.token.clone())
                .or_insert(idx);
        }
    }

    pub fn upsert_virtual_key(&mut self, key: VirtualKeyConfig) -> bool {
        if let Some(existing) = self.config.virtual_keys.iter_mut().find(|k| k.id == key.id) {
            let cache_changed = existing.cache != key.cache;
            if cache_changed {
                self.cache.remove_scope(&key.id);
            }
            *existing = key;
            self.rebuild_virtual_key_token_index();
            self.prune_internal_scopes();
            false
        } else {
            self.config.virtual_keys.push(key);
            self.rebuild_virtual_key_token_index();
            self.prune_internal_scopes();
            true
        }
    }

    pub fn remove_virtual_key(&mut self, id: &str) -> Option<VirtualKeyConfig> {
        let index = self.config.virtual_keys.iter().position(|k| k.id == id)?;
        let removed = self.config.virtual_keys.remove(index);
        self.rebuild_virtual_key_token_index();
        self.prune_internal_scopes();
        Some(removed)
    }

    fn prune_internal_scopes(&mut self) {
        let mut scopes = HashSet::<String>::new();

        for key in &self.config.virtual_keys {
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

        self.limits.retain_scopes(&scopes);
        self.budget.retain_scopes(&scopes);
        self.cache.retain_scopes(&scopes);
    }

    pub(crate) fn prepare_handle_request(
        &mut self,
        request: &GatewayRequest,
    ) -> Result<GatewayPreparedRequest, GatewayError> {
        self.observability.record_request();

        let key = self
            .virtual_key_by_token(&request.virtual_key)
            .ok_or(GatewayError::Unauthorized)?
            .clone();

        if !key.enabled {
            return Err(GatewayError::Unauthorized);
        }

        let now = self.clock.now_epoch_seconds();
        let minute = now / 60;
        let tokens = request.total_tokens();

        if let Err(err) = self
            .limits
            .check_and_consume(&key.id, &key.limits, tokens, minute)
        {
            self.observability.record_rate_limited();
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
            if let Err(err) = self
                .limits
                .check_and_consume(&scope, tenant_limits, tokens, minute)
            {
                self.observability.record_rate_limited();
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
            if let Err(err) = self
                .limits
                .check_and_consume(&scope, project_limits, tokens, minute)
            {
                self.observability.record_rate_limited();
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
            if let Err(err) = self
                .limits
                .check_and_consume(&scope, user_limits, tokens, minute)
            {
                self.observability.record_rate_limited();
                return Err(err);
            }
        }

        let guardrails = self
            .router
            .rule_for_model(&request.model, Some(&key))
            .and_then(|rule| rule.guardrails.as_ref())
            .unwrap_or(&key.guardrails);

        if let Err(err) = guardrails.check(request) {
            self.observability.record_guardrail_blocked();
            return Err(err);
        }

        if let Err(err) = key.passthrough.validate(request) {
            self.observability.record_guardrail_blocked();
            return Err(err);
        }

        let backend_name = self.router.select_backend(request, &key)?;

        let bypass_cache = key.passthrough.bypass_cache(request);
        let cache_key =
            (key.cache.enabled && !bypass_cache).then(|| control_plane_cache_key(&key.id, request));
        if let Some(cache_key) = cache_key.as_deref() {
            if let Some(mut cached) = self.cache.get(&key.id, cache_key, now) {
                cached.cached = true;
                self.observability.record_cache_hit();
                return Ok(GatewayPreparedRequest::Cached {
                    key_id: key.id.clone(),
                    response: cached,
                });
            }
        }

        if let Err(err) = self
            .budget
            .can_spend(&key.id, &key.budget, u64::from(tokens))
        {
            self.observability.record_budget_exceeded();
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
            if let Err(err) = self
                .budget
                .can_spend(&scope, tenant_budget, u64::from(tokens))
            {
                self.observability.record_budget_exceeded();
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
            if let Err(err) = self
                .budget
                .can_spend(&scope, project_budget, u64::from(tokens))
            {
                self.observability.record_budget_exceeded();
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
            if let Err(err) = self
                .budget
                .can_spend(&scope, user_budget, u64::from(tokens))
            {
                self.observability.record_budget_exceeded();
                return Err(err);
            }
        }

        let backend = self.backends.get(&backend_name).cloned().ok_or_else(|| {
            GatewayError::BackendNotFound {
                name: backend_name.clone(),
            }
        })?;

        self.observability.record_backend_call();

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

        // Reserve token budgets before releasing the gateway lock so concurrent
        // requests cannot pass budget checks against stale spent counters.
        self.budget.spend(&key.id, &key.budget, u64::from(tokens));
        if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
            self.budget.spend(scope, budget, u64::from(tokens));
        }
        if let Some((scope, budget)) = project_budget_scope.as_ref() {
            self.budget.spend(scope, budget, u64::from(tokens));
        }
        if let Some((scope, budget)) = user_budget_scope.as_ref() {
            self.budget.spend(scope, budget, u64::from(tokens));
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
        &mut self,
        prepared: &GatewayPreparedCall,
        response: &GatewayResponse,
    ) {
        if let Some(cache_key) = prepared.cache_key.as_ref() {
            // Use the latest key config so in-flight requests do not re-populate
            // cache entries after cache has been disabled or key has been removed.
            let Some(key) = self
                .config
                .virtual_keys
                .iter()
                .find(|k| k.id == prepared.key_id)
            else {
                return;
            };
            if !key.enabled || !key.cache.enabled {
                return;
            }
            let now = self.clock.now_epoch_seconds();
            self.cache.insert(
                &prepared.key_id,
                cache_key.clone(),
                response.clone(),
                &key.cache,
                now,
            );
        }
    }

    pub(crate) fn complete_handle_failure(&mut self, prepared: &GatewayPreparedCall) {
        self.budget
            .refund(&prepared.key_id, &prepared.key_budget, prepared.tokens);
        if let Some((scope, budget)) = prepared.tenant_budget_scope.as_ref() {
            self.budget.refund(scope, budget, prepared.tokens);
        }
        if let Some((scope, budget)) = prepared.project_budget_scope.as_ref() {
            self.budget.refund(scope, budget, prepared.tokens);
        }
        if let Some((scope, budget)) = prepared.user_budget_scope.as_ref() {
            self.budget.refund(scope, budget, prepared.tokens);
        }
    }

    pub async fn handle(
        &mut self,
        request: GatewayRequest,
    ) -> Result<GatewayResponse, GatewayError> {
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
        self.virtual_keys.iter().find(|key| key.token == token)
    }
}
