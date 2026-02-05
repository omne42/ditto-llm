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
    let prompt_hash = hash64_fnv1a(request.prompt.as_bytes());
    format!(
        "ditto-gateway-cache-v1|{key_id}|{}|{}|{}|{:016x}|{}",
        request.model,
        request.input_tokens,
        request.max_output_tokens,
        prompt_hash,
        request.prompt.len()
    )
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
    backends: HashMap<String, Arc<dyn Backend>>,
    limits: RateLimiter,
    cache: ResponseCache,
    budget: BudgetTracker,
    router: Router,
    observability: Observability,
    clock: Box<dyn Clock>,
}

impl Gateway {
    pub fn new(config: GatewayConfig) -> Self {
        Self::with_clock(config, Box::new(SystemClock))
    }

    pub fn with_clock(config: GatewayConfig, clock: Box<dyn Clock>) -> Self {
        let router = Router::new(config.router.clone());
        Self {
            config,
            backends: HashMap::new(),
            limits: RateLimiter::default(),
            cache: ResponseCache::default(),
            budget: BudgetTracker::default(),
            router,
            observability: Observability::default(),
            clock,
        }
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

    pub fn upsert_virtual_key(&mut self, key: VirtualKeyConfig) -> bool {
        if let Some(existing) = self.config.virtual_keys.iter_mut().find(|k| k.id == key.id) {
            *existing = key;
            self.prune_internal_scopes();
            false
        } else {
            self.config.virtual_keys.push(key);
            self.prune_internal_scopes();
            true
        }
    }

    pub fn remove_virtual_key(&mut self, id: &str) -> Option<VirtualKeyConfig> {
        let index = self.config.virtual_keys.iter().position(|k| k.id == id)?;
        let removed = self.config.virtual_keys.remove(index);
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

    pub async fn handle(
        &mut self,
        request: GatewayRequest,
    ) -> Result<GatewayResponse, GatewayError> {
        self.observability.record_request();

        let key = self
            .config
            .virtual_key(&request.virtual_key)
            .ok_or(GatewayError::Unauthorized)?;

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
            .rule_for_model(&request.model, Some(key))
            .and_then(|rule| rule.guardrails.as_ref())
            .unwrap_or(&key.guardrails);

        if let Err(err) = guardrails.check(&request) {
            self.observability.record_guardrail_blocked();
            return Err(err);
        }

        if let Err(err) = key.passthrough.validate(&request) {
            self.observability.record_guardrail_blocked();
            return Err(err);
        }

        let backend_name = self.router.select_backend(&request, key)?;

        let bypass_cache = key.passthrough.bypass_cache(&request);
        let cache_key = (key.cache.enabled && !bypass_cache)
            .then(|| control_plane_cache_key(&key.id, &request));
        if let Some(cache_key) = cache_key.as_deref() {
            if let Some(mut cached) = self.cache.get(&key.id, cache_key, now) {
                cached.cached = true;
                self.observability.record_cache_hit();
                return Ok(cached);
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

        let backend =
            self.backends
                .get(&backend_name)
                .cloned()
                .ok_or(GatewayError::BackendNotFound {
                    name: backend_name.clone(),
                })?;

        self.observability.record_backend_call();
        let mut response = backend.call(&request).await?;
        response.backend = backend_name.clone();
        response.cached = false;

        self.budget.spend(&key.id, &key.budget, u64::from(tokens));
        if let (Some(tenant_id), Some(tenant_budget)) = (
            key.tenant_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty()),
            key.tenant_budget.as_ref(),
        ) {
            let scope = format!("tenant:{tenant_id}");
            self.budget.spend(&scope, tenant_budget, u64::from(tokens));
        }
        if let (Some(project_id), Some(project_budget)) = (
            key.project_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty()),
            key.project_budget.as_ref(),
        ) {
            let scope = format!("project:{project_id}");
            self.budget.spend(&scope, project_budget, u64::from(tokens));
        }
        if let (Some(user_id), Some(user_budget)) = (
            key.user_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty()),
            key.user_budget.as_ref(),
        ) {
            let scope = format!("user:{user_id}");
            self.budget.spend(&scope, user_budget, u64::from(tokens));
        }

        if let Some(cache_key) = cache_key {
            self.cache
                .insert(&key.id, cache_key, response.clone(), &key.cache, now);
        }

        Ok(response)
    }
}

impl GatewayConfig {
    pub fn virtual_key(&self, token: &str) -> Option<&VirtualKeyConfig> {
        self.virtual_keys.iter().find(|key| key.token == token)
    }
}
