//! Gateway module (feature-gated).

pub mod budget;
pub mod cache;
pub mod config;
pub mod guardrails;
pub mod limits;
pub mod observability;
pub mod passthrough;
pub mod router;

use std::collections::HashMap;
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
pub use config::{GatewayConfig, VirtualKeyConfig};
pub use guardrails::GuardrailsConfig;
pub use limits::LimitsConfig;
pub use passthrough::PassthroughConfig;
pub use router::{RouteRule, RouterConfig};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GatewayRequest {
    pub virtual_key: String,
    pub model: String,
    pub prompt: String,
    pub input_tokens: u32,
    pub max_output_tokens: u32,
    pub passthrough: bool,
}

impl GatewayRequest {
    pub fn total_tokens(&self) -> u32 {
        self.input_tokens.saturating_add(self.max_output_tokens)
    }

    pub fn cache_key(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}",
            self.virtual_key, self.model, self.input_tokens, self.max_output_tokens, self.prompt
        )
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

        if let Err(err) = key.guardrails.check(&request) {
            self.observability.record_guardrail_blocked();
            return Err(err);
        }

        if let Err(err) = key.passthrough.validate(&request) {
            self.observability.record_guardrail_blocked();
            return Err(err);
        }

        let backend_name = self.router.select_backend(&request, key)?;

        let bypass_cache = key.passthrough.bypass_cache(&request);
        if key.cache.enabled && !bypass_cache {
            if let Some(mut cached) = self.cache.get(&request.cache_key(), now) {
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

        if key.cache.enabled && !bypass_cache {
            self.cache.insert(
                request.cache_key(),
                response.clone(),
                key.cache.ttl_seconds,
                now,
            );
        }

        Ok(response)
    }
}

impl GatewayConfig {
    pub fn virtual_key(&self, token: &str) -> Option<&VirtualKeyConfig> {
        self.virtual_keys.iter().find(|key| key.token == token)
    }
}
