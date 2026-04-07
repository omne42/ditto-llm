// Transport HTTP implementation.
mod a2a;
mod admin;
mod admin_auth;
mod admin_persistence;
mod anthropic;
mod config_versions;
mod control_plane;
mod google_genai;
mod litellm_keys;
mod mcp;
mod openai_compat_proxy_cost_budget;
mod openai_compat_proxy_costing;
mod openai_compat_proxy_handler;
mod openai_compat_proxy_mcp;
mod openai_compat_proxy_path_normalize;
mod openai_compat_proxy_preamble;
mod openai_compat_proxy_proxy_cache_hit;
mod openai_compat_proxy_proxy_failure;
mod openai_compat_proxy_rate_limit;
mod openai_compat_proxy_request_dedup;
mod openai_compat_proxy_request_schema;
mod openai_compat_proxy_streaming_multipart;
mod openai_models;
mod proxy_backend;
mod proxy_bounded_body;
mod proxy_budget_reservations;
mod proxy_gateway_context;
mod proxy_map_openai_gateway_error;
mod request_extractors;
mod router;
mod translation_backend;
pub use self::a2a::A2aAgentState;
use self::admin::{error_response, map_gateway_error};
use self::admin_auth::{ensure_admin_read, ensure_admin_secret_access, ensure_admin_write};
use self::config_versions::{
    ConfigVersionHistory, ConfigVersionInfo, diff_config_versions, export_config,
    get_config_version, get_config_version_by_id, list_config_versions, rollback_config_version,
    upsert_config_router, validate_config_payload,
};
use self::control_plane::GatewayControlPlaneSnapshot;
pub use self::mcp::McpServerState;
use self::mcp::{mcp_call_tool, mcp_list_tools};
#[cfg(feature = "gateway-costing")]
use self::openai_compat_proxy_cost_budget::{
    CostBudgetEndpointPolicy, cost_budget_endpoint_policy,
};
#[cfg(feature = "gateway-costing")]
use self::openai_compat_proxy_costing::estimate_charge_cost_usd_micros;
use self::openai_compat_proxy_handler::handle_openai_compat_proxy;
use self::openai_compat_proxy_mcp::maybe_handle_mcp_tools_chat_completions;
use self::openai_compat_proxy_path_normalize::normalize_openai_compat_path_and_query;
#[allow(unused_imports)]
use self::openai_compat_proxy_preamble::{
    BackendAttemptOutcome, ProxyAttemptParams, validate_openai_multipart_request_schema,
};
#[cfg(feature = "gateway-proxy-cache")]
use self::openai_compat_proxy_proxy_cache_hit::maybe_handle_proxy_cache_hit;
use self::openai_compat_proxy_proxy_failure::{
    ProxyFailureContext, finalize_openai_compat_proxy_failure,
};
#[cfg(feature = "gateway-store-redis")]
use self::openai_compat_proxy_rate_limit::{normalize_rate_limit_route, redis_rate_limit_scopes};
use self::openai_compat_proxy_request_dedup::{
    LocalProxyRequestIdempotencyStore, PrepareProxyRequestDedupInput, ProxyRequestDedupDecision,
    finish_proxy_request_dedup_result, prepare_proxy_request_dedup,
};
#[allow(unused_imports)]
use self::openai_compat_proxy_request_schema::{
    extract_max_output_tokens, validate_openai_request_schema,
};
use self::openai_compat_proxy_streaming_multipart::{
    handle_openai_compat_proxy_streaming_multipart, should_stream_large_multipart_request,
};
use self::proxy_backend::attempt_proxy_backend;
use self::proxy_bounded_body::read_reqwest_body_bytes_bounded_with_content_length;
#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
use self::proxy_budget_reservations::{
    ProxyBudgetReservationParams, reserve_proxy_token_budgets_for_request,
    rollback_proxy_token_budget_reservations, settle_proxy_token_budget_reservations_checked,
};
use self::proxy_budget_reservations::{ProxyPermitOutcome, try_acquire_proxy_permits};
#[cfg(all(
    feature = "gateway-costing",
    any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ),
))]
use self::proxy_budget_reservations::{
    record_proxy_spent_cost_usd_micros_checked, reserve_proxy_cost_budgets_for_request,
    rollback_proxy_cost_budget_reservations, settle_proxy_cost_budget_reservations_checked,
};
use self::proxy_gateway_context::{
    ResolveOpenAiCompatProxyGatewayContextRequest, ResolvedGatewayContext,
    resolve_openai_compat_proxy_gateway_context,
};
use self::proxy_map_openai_gateway_error::map_openai_gateway_error;
use self::request_extractors::{
    extract_bearer, extract_header, extract_litellm_api_key, extract_query_param,
    extract_virtual_key,
};
pub use self::router::router;
#[cfg(feature = "gateway-translation")]
use self::translation_backend::attempt_translation_backend;
#[cfg(feature = "gateway-proxy-cache")]
use omne_integrity_primitives::Sha256Hasher;

// inlined from ../../http/core.rs
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path as StdPath, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, RwLock};
use std::task::{Context, Poll};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(feature = "gateway-metrics-prometheus")]
use std::time::Instant;

use axum::Json;
use axum::body::{Body, to_bytes};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use bytes::Bytes;
use futures_util::StreamExt;
#[allow(unused_imports)]
use futures_util::stream;
use futures_util::stream::BoxStream;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};

#[cfg(feature = "gateway-routing-advanced")]
use ditto_core::utils::task::AbortOnDrop;

#[cfg(feature = "gateway-translation")]
#[derive(Clone, Copy, Debug)]
struct ProxySpend {
    tokens: u64,
    cost_usd_micros: Option<u64>,
}

#[cfg(feature = "sdk")]
use ditto_core::sdk::devtools::DevtoolsLogger;

#[cfg(feature = "gateway-costing")]
use super::costing::PricingTable;

#[cfg(feature = "gateway-proxy-cache")]
use super::proxy_cache::{
    CachedProxyResponse, ProxyCacheConfig, ProxyCacheEntryMetadata, ProxyCachePurgeSelector,
    ProxyCacheStreamRecorder, ProxyResponseCache,
};

#[cfg(feature = "gateway-metrics-prometheus")]
use super::metrics_prometheus::{PrometheusMetrics, PrometheusMetricsConfig};

#[cfg(feature = "gateway-routing-advanced")]
use super::proxy_routing::{BackendHealth, BackendHealthSnapshot, FailureKind, ProxyRoutingConfig};

#[cfg(feature = "gateway-store-sqlite")]
use super::{SqliteStore, SqliteStoreError};

#[cfg(feature = "gateway-store-postgres")]
use super::{PostgresStore, PostgresStoreError};

#[cfg(feature = "gateway-store-mysql")]
use super::{MySqlStore, MySqlStoreError};

#[cfg(feature = "gateway-store-redis")]
use super::{RedisStore, RedisStoreError};

#[cfg(feature = "gateway-tokenizer")]
use super::token_count;

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
use super::AuditLogRecord;

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
use super::BudgetLedgerRecord;

#[cfg(all(
    feature = "gateway-costing",
    any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ),
))]
use super::CostLedgerRecord;

use super::budget::BudgetTracker;
use super::interop;
use super::limits::RateLimiter;
use super::observability::{GatewayObservabilityPolicy, GatewayObservabilitySink, Observability};
use super::redaction::GatewayRedactor;
use super::responses_shim;
#[cfg(feature = "gateway-translation")]
use super::translation;
use super::{
    BudgetConfig, Gateway, GatewayError, GatewayPreparedRequest, GatewayRequest, GatewayResponse,
    GatewayStateFile, LimitsConfig, ObservabilitySnapshot, ProxyBackend, RouterConfig,
    VirtualKeyConfig, lock_unpoisoned,
};
use crate::gateway::ProxyRequestIdempotencyStore;

static REQUEST_ID_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
struct AdminTenantToken {
    tenant_id: String,
    token: String,
    read_only: bool,
}

#[derive(Clone, Default)]
struct GatewayAdminState {
    admin_token: Option<String>,
    admin_read_token: Option<String>,
    admin_tenant_tokens: Vec<AdminTenantToken>,
    state_file: Option<PathBuf>,
    json_logs: bool,
    #[cfg(feature = "sdk")]
    devtools: Option<DevtoolsLogger>,
}

#[derive(Clone)]
struct GatewayRuntimeBackends {
    proxy_backends: Arc<HashMap<String, ProxyBackend>>,
    a2a_agents: Arc<HashMap<String, A2aAgentState>>,
    mcp_servers: Arc<HashMap<String, McpServerState>>,
    #[cfg(feature = "gateway-translation")]
    translation_backends: Arc<HashMap<String, super::TranslationBackend>>,
}

impl Default for GatewayRuntimeBackends {
    fn default() -> Self {
        Self {
            proxy_backends: Arc::new(HashMap::new()),
            a2a_agents: Arc::new(HashMap::new()),
            mcp_servers: Arc::new(HashMap::new()),
            #[cfg(feature = "gateway-translation")]
            translation_backends: Arc::new(HashMap::new()),
        }
    }
}

#[derive(Clone, Default)]
struct GatewayPersistenceState {
    #[cfg(feature = "gateway-store-sqlite")]
    sqlite: Option<SqliteStore>,
    #[cfg(feature = "gateway-store-postgres")]
    postgres: Option<PostgresStore>,
    #[cfg(feature = "gateway-store-mysql")]
    mysql: Option<MySqlStore>,
    #[cfg(feature = "gateway-store-redis")]
    redis: Option<RedisStore>,
}

#[derive(Clone)]
struct GatewayProxyRuntimeState {
    #[cfg(feature = "gateway-costing")]
    pricing: Option<Arc<PricingTable>>,
    #[cfg(feature = "gateway-proxy-cache")]
    cache: Option<Arc<Mutex<ProxyResponseCache>>>,
    #[cfg(feature = "gateway-proxy-cache")]
    cache_config: Option<ProxyCacheConfig>,
    max_body_bytes: usize,
    usage_max_body_bytes: usize,
    backpressure: Option<Arc<Semaphore>>,
    backend_backpressure: Arc<HashMap<String, Arc<Semaphore>>>,
    #[cfg(feature = "gateway-metrics-prometheus")]
    metrics: Option<Arc<Mutex<PrometheusMetrics>>>,
    #[cfg(feature = "gateway-routing-advanced")]
    routing: Option<ProxyRoutingConfig>,
    #[cfg(feature = "gateway-routing-advanced")]
    backend_health: Option<Arc<Mutex<HashMap<String, BackendHealth>>>>,
    #[cfg(feature = "gateway-routing-advanced")]
    health_check_task: Option<Arc<AbortOnDrop>>,
    request_dedup: Arc<LocalProxyRequestIdempotencyStore>,
}

impl GatewayProxyRuntimeState {
    fn new(backend_backpressure: HashMap<String, Arc<Semaphore>>) -> Self {
        Self {
            #[cfg(feature = "gateway-costing")]
            pricing: None,
            #[cfg(feature = "gateway-proxy-cache")]
            cache: None,
            #[cfg(feature = "gateway-proxy-cache")]
            cache_config: None,
            max_body_bytes: 64 * 1024 * 1024,
            usage_max_body_bytes: 1024 * 1024,
            backpressure: None,
            backend_backpressure: Arc::new(backend_backpressure),
            #[cfg(feature = "gateway-metrics-prometheus")]
            metrics: None,
            #[cfg(feature = "gateway-routing-advanced")]
            routing: None,
            #[cfg(feature = "gateway-routing-advanced")]
            backend_health: None,
            #[cfg(feature = "gateway-routing-advanced")]
            health_check_task: None,
            request_dedup: Arc::new(LocalProxyRequestIdempotencyStore::default()),
        }
    }
}

#[derive(Clone)]
pub struct GatewayHttpState {
    gateway: Arc<Gateway>,
    control_plane: Arc<RwLock<GatewayControlPlaneSnapshot>>,
    control_plane_writes: Arc<Mutex<()>>,
    limits: Arc<StdMutex<RateLimiter>>,
    budget: Arc<StdMutex<BudgetTracker>>,
    observability: Arc<StdMutex<Observability>>,
    config_versions: Arc<Mutex<ConfigVersionHistory>>,
    #[allow(dead_code)]
    redactor: Arc<GatewayRedactor>,
    observability_policy: Arc<GatewayObservabilityPolicy>,
    backends: GatewayRuntimeBackends,
    admin: GatewayAdminState,
    #[allow(dead_code)]
    stores: GatewayPersistenceState,
    proxy: GatewayProxyRuntimeState,
}

impl GatewayHttpState {
    pub fn new(gateway: Gateway) -> Self {
        let initial_config = gateway.config_snapshot();
        let initial_virtual_keys = initial_config.virtual_keys.clone();
        let initial_router = initial_config.router.clone();
        let runtime_backends = GatewayRuntimeBackends::default();
        let control_plane =
            GatewayControlPlaneSnapshot::from_gateway_state(&gateway, &runtime_backends);
        let limits = gateway.limits.clone();
        let budget = gateway.budget.clone();
        let observability = gateway.observability.clone();
        let redactor = Arc::new(GatewayRedactor::from_config(
            &initial_config.observability.redaction,
        ));
        let observability_policy = Arc::new(GatewayObservabilityPolicy::new(
            redactor.clone(),
            &initial_config.observability.sampling,
        ));
        let mut proxy_backend_backpressure: HashMap<String, Arc<Semaphore>> = HashMap::new();
        for backend in &initial_config.backends {
            let Some(max_in_flight) = backend.max_in_flight else {
                continue;
            };
            proxy_backend_backpressure.insert(
                backend.name.clone(),
                Arc::new(Semaphore::new(max_in_flight.max(1))),
            );
        }

        Self {
            gateway: Arc::new(gateway),
            control_plane: Arc::new(RwLock::new(control_plane)),
            control_plane_writes: Arc::new(Mutex::new(())),
            limits,
            budget,
            observability,
            config_versions: Arc::new(Mutex::new(ConfigVersionHistory::with_bootstrap(
                initial_virtual_keys,
                initial_router,
            ))),
            redactor,
            observability_policy,
            backends: runtime_backends,
            admin: GatewayAdminState::default(),
            stores: GatewayPersistenceState::default(),
            proxy: GatewayProxyRuntimeState::new(proxy_backend_backpressure),
        }
    }

    pub(crate) fn record_request(&self) {
        lock_unpoisoned(&self.observability).record_request();
    }

    #[cfg(feature = "gateway-proxy-cache")]
    pub(crate) fn record_cache_hit(&self) {
        lock_unpoisoned(&self.observability).record_cache_hit();
    }

    pub(crate) fn record_rate_limited(&self) {
        lock_unpoisoned(&self.observability).record_rate_limited();
    }

    pub(crate) fn record_guardrail_blocked(&self) {
        lock_unpoisoned(&self.observability).record_guardrail_blocked();
    }

    pub(crate) fn record_budget_exceeded(&self) {
        lock_unpoisoned(&self.observability).record_budget_exceeded();
    }

    pub(crate) fn record_backend_call(&self) {
        lock_unpoisoned(&self.observability).record_backend_call();
    }

    pub(super) async fn lock_control_plane_writes(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.control_plane_writes.lock().await
    }

    pub(crate) fn observability_snapshot(&self) -> ObservabilitySnapshot {
        lock_unpoisoned(&self.observability).snapshot()
    }

    pub(crate) fn prepare_observability_event(
        &self,
        sink: GatewayObservabilitySink,
        payload: Value,
    ) -> Option<Value> {
        self.observability_policy.prepare_event(sink, payload)
    }

    #[cfg(feature = "gateway-metrics-prometheus")]
    pub(crate) fn redact_observability_prometheus_metrics(&self, rendered: &str) -> String {
        self.observability_policy.redact_prometheus_render(rendered)
    }

    pub(crate) fn check_and_consume_rate_limits<'a, I>(
        &self,
        scopes: I,
        tokens: u32,
        minute: u64,
    ) -> Result<(), GatewayError>
    where
        I: IntoIterator<Item = (&'a str, &'a super::LimitsConfig)>,
    {
        lock_unpoisoned(&self.limits).check_and_consume_many(scopes, tokens, minute)
    }

    pub(crate) fn reserve_budget_tokens<'a, I>(
        &self,
        scopes: I,
        tokens: u64,
    ) -> Result<(), GatewayError>
    where
        I: IntoIterator<Item = (&'a str, &'a super::BudgetConfig)>,
    {
        lock_unpoisoned(&self.budget).reserve_many(scopes, tokens)
    }

    pub(crate) fn settle_budget_tokens<'a, I>(
        &self,
        scopes: I,
        reserved_tokens: u64,
        actual_tokens: u64,
    ) where
        I: IntoIterator<Item = (&'a str, &'a super::BudgetConfig)>,
    {
        lock_unpoisoned(&self.budget).settle_many(scopes, reserved_tokens, actual_tokens);
    }

    pub(crate) fn rollback_budget_tokens<'a, I>(&self, scopes: I, reserved_tokens: u64)
    where
        I: IntoIterator<Item = (&'a str, &'a super::BudgetConfig)>,
    {
        lock_unpoisoned(&self.budget).refund_many(scopes, reserved_tokens);
    }

    #[cfg(feature = "gateway-costing")]
    pub(crate) fn reserve_budget_cost<'a, I>(
        &self,
        scopes: I,
        usd_micros: u64,
    ) -> Result<(), GatewayError>
    where
        I: IntoIterator<Item = (&'a str, &'a super::BudgetConfig)>,
    {
        lock_unpoisoned(&self.budget).reserve_cost_many(scopes, usd_micros)
    }

    #[cfg(feature = "gateway-costing")]
    pub(crate) fn settle_budget_cost<'a, I>(
        &self,
        scopes: I,
        reserved_usd_micros: u64,
        actual_usd_micros: Option<u64>,
    ) where
        I: IntoIterator<Item = (&'a str, &'a super::BudgetConfig)>,
    {
        lock_unpoisoned(&self.budget).settle_cost_many(
            scopes,
            reserved_usd_micros,
            actual_usd_micros,
        );
    }

    #[cfg(feature = "gateway-costing")]
    pub(crate) fn rollback_budget_cost<'a, I>(&self, scopes: I, reserved_usd_micros: u64)
    where
        I: IntoIterator<Item = (&'a str, &'a super::BudgetConfig)>,
    {
        lock_unpoisoned(&self.budget).refund_cost_many(scopes, reserved_usd_micros);
    }

    fn has_any_admin_tokens(&self) -> bool {
        self.admin.admin_token.is_some()
            || self.admin.admin_read_token.is_some()
            || !self.admin.admin_tenant_tokens.is_empty()
    }

    fn has_admin_write_tokens(&self) -> bool {
        self.admin.admin_token.is_some()
            || self
                .admin
                .admin_tenant_tokens
                .iter()
                .any(|binding| !binding.read_only)
    }

    pub fn with_admin_token(mut self, token: impl Into<String>) -> Self {
        self.admin.admin_token = Some(token.into());
        self
    }

    pub fn with_admin_read_token(mut self, token: impl Into<String>) -> Self {
        self.admin.admin_read_token = Some(token.into());
        self
    }

    pub fn with_admin_tenant_token(
        mut self,
        tenant_id: impl Into<String>,
        token: impl Into<String>,
    ) -> Self {
        self.admin.admin_tenant_tokens.push(AdminTenantToken {
            tenant_id: tenant_id.into(),
            token: token.into(),
            read_only: false,
        });
        self
    }

    pub fn with_admin_tenant_read_token(
        mut self,
        tenant_id: impl Into<String>,
        token: impl Into<String>,
    ) -> Self {
        self.admin.admin_tenant_tokens.push(AdminTenantToken {
            tenant_id: tenant_id.into(),
            token: token.into(),
            read_only: true,
        });
        self
    }

    pub fn with_proxy_backends(mut self, backends: HashMap<String, ProxyBackend>) -> Self {
        self.backends.proxy_backends = Arc::new(backends);
        self.sync_control_plane_from_gateway();
        self
    }

    pub fn with_a2a_agents(mut self, agents: HashMap<String, A2aAgentState>) -> Self {
        self.backends.a2a_agents = Arc::new(agents);
        self.sync_control_plane_from_gateway();
        self
    }

    pub fn with_mcp_servers(mut self, servers: HashMap<String, McpServerState>) -> Self {
        self.backends.mcp_servers = Arc::new(servers);
        self.sync_control_plane_from_gateway();
        self
    }

    pub fn with_proxy_max_body_bytes(mut self, max_body_bytes: usize) -> Self {
        self.proxy.max_body_bytes = max_body_bytes.max(1);
        self
    }

    pub fn with_proxy_usage_max_body_bytes(mut self, max_body_bytes: usize) -> Self {
        self.proxy.usage_max_body_bytes = max_body_bytes;
        self
    }

    #[cfg(feature = "gateway-translation")]
    pub fn with_translation_backends(
        mut self,
        backends: HashMap<String, super::TranslationBackend>,
    ) -> Self {
        self.backends.translation_backends = Arc::new(backends);
        self.sync_control_plane_from_gateway();
        self
    }

    pub fn with_state_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.admin.state_file = Some(path.into());
        self
    }

    fn proxy_request_idempotency_store(&self) -> Arc<dyn ProxyRequestIdempotencyStore> {
        #[cfg(feature = "gateway-store-redis")]
        if let Some(store) = self.stores.redis.as_ref() {
            return Arc::new(store.clone());
        }
        #[cfg(feature = "gateway-store-postgres")]
        if let Some(store) = self.stores.postgres.as_ref() {
            return Arc::new(store.clone());
        }
        #[cfg(feature = "gateway-store-mysql")]
        if let Some(store) = self.stores.mysql.as_ref() {
            return Arc::new(store.clone());
        }
        #[cfg(feature = "gateway-store-sqlite")]
        if let Some(store) = self.stores.sqlite.as_ref() {
            return Arc::new(store.clone());
        }

        self.proxy.request_dedup.clone()
    }

    #[cfg(feature = "gateway-store-sqlite")]
    pub fn with_sqlite_store(mut self, store: SqliteStore) -> Self {
        self.stores.sqlite = Some(store);
        self
    }

    #[cfg(feature = "gateway-store-postgres")]
    pub fn with_postgres_store(mut self, store: PostgresStore) -> Self {
        self.stores.postgres = Some(store);
        self
    }

    #[cfg(feature = "gateway-store-mysql")]
    pub fn with_mysql_store(mut self, store: MySqlStore) -> Self {
        self.stores.mysql = Some(store);
        self
    }

    #[cfg(feature = "gateway-store-redis")]
    pub fn with_redis_store(mut self, store: RedisStore) -> Self {
        self.stores.redis = Some(store);
        self
    }

    #[cfg(feature = "gateway-costing")]
    pub fn with_pricing_table(mut self, pricing: PricingTable) -> Self {
        self.proxy.pricing = Some(Arc::new(pricing));
        self
    }

    pub fn with_json_logs(mut self) -> Self {
        self.admin.json_logs = true;
        self
    }

    #[cfg(feature = "gateway-proxy-cache")]
    pub fn with_proxy_cache(mut self, config: ProxyCacheConfig) -> Self {
        self.proxy.cache = Some(Arc::new(Mutex::new(ProxyResponseCache::new(
            config.clone(),
        ))));
        self.proxy.cache_config = Some(config);
        self
    }

    pub fn with_proxy_max_in_flight(mut self, max_in_flight: usize) -> Self {
        self.proxy.backpressure = Some(Arc::new(Semaphore::new(max_in_flight.max(1))));
        self
    }

    #[cfg(feature = "gateway-metrics-prometheus")]
    pub fn with_prometheus_metrics(mut self, config: PrometheusMetricsConfig) -> Self {
        self.proxy.metrics = Some(Arc::new(Mutex::new(PrometheusMetrics::new(config))));
        self
    }

    #[cfg(feature = "gateway-routing-advanced")]
    pub fn with_proxy_routing(mut self, config: ProxyRoutingConfig) -> Self {
        self.proxy.routing = Some(config);
        self.proxy.backend_health = Some(Arc::new(Mutex::new(HashMap::new())));
        self
    }

    #[cfg(feature = "sdk")]
    pub fn with_devtools_logger(mut self, logger: DevtoolsLogger) -> Self {
        self.admin.devtools = Some(logger);
        self
    }
}

#[derive(Debug, Deserialize)]
struct GatewayHttpRequest {
    #[serde(default)]
    virtual_key: Option<String>,
    model: String,
    prompt: String,
    input_tokens: u32,
    max_output_tokens: u32,
    #[serde(default)]
    passthrough: bool,
}

#[derive(Debug, Serialize)]
struct ErrorDetail {
    code: &'static str,
    message: String,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: ErrorDetail,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
}

#[derive(Debug, Serialize)]
struct ReadinessCheck {
    name: String,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

#[derive(Debug, Serialize)]
struct ReadinessResponse {
    status: &'static str,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    checks: Vec<ReadinessCheck>,
}

// inlined from core/diagnostics.rs
async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn ready(State(state): State<GatewayHttpState>) -> (StatusCode, Json<ReadinessResponse>) {
    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis",
        feature = "gateway-routing-advanced"
    ))]
    let mut checks: Vec<ReadinessCheck> = Vec::new();
    #[cfg(not(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis",
        feature = "gateway-routing-advanced"
    )))]
    let checks: Vec<ReadinessCheck> = Vec::new();

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.stores.sqlite.as_ref() {
        match store.ping().await {
            Ok(()) => checks.push(ReadinessCheck {
                name: "store.sqlite".to_string(),
                status: "ok",
                detail: None,
            }),
            Err(err) => checks.push(ReadinessCheck {
                name: "store.sqlite".to_string(),
                status: "error",
                detail: Some(err.to_string()),
            }),
        }
    }

    #[cfg(feature = "gateway-store-postgres")]
    if let Some(store) = state.stores.postgres.as_ref() {
        match store.ping().await {
            Ok(()) => checks.push(ReadinessCheck {
                name: "store.postgres".to_string(),
                status: "ok",
                detail: None,
            }),
            Err(err) => checks.push(ReadinessCheck {
                name: "store.postgres".to_string(),
                status: "error",
                detail: Some(err.to_string()),
            }),
        }
    }

    #[cfg(feature = "gateway-store-mysql")]
    if let Some(store) = state.stores.mysql.as_ref() {
        match store.ping().await {
            Ok(()) => checks.push(ReadinessCheck {
                name: "store.mysql".to_string(),
                status: "ok",
                detail: None,
            }),
            Err(err) => checks.push(ReadinessCheck {
                name: "store.mysql".to_string(),
                status: "error",
                detail: Some(err.to_string()),
            }),
        }
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.stores.redis.as_ref() {
        match store.ping().await {
            Ok(()) => checks.push(ReadinessCheck {
                name: "store.redis".to_string(),
                status: "ok",
                detail: None,
            }),
            Err(err) => checks.push(ReadinessCheck {
                name: "store.redis".to_string(),
                status: "error",
                detail: Some(err.to_string()),
            }),
        }
    }

    #[cfg(feature = "gateway-routing-advanced")]
    if let Some(config) = state.proxy.routing.as_ref()
        && config.health_check.enabled
    {
        let mut backend_names: Vec<String> =
            state.backends.proxy_backends.keys().cloned().collect();
        backend_names.sort();

        if let Some(health) = state.proxy.backend_health.as_ref() {
            let snapshots = {
                let health = health.lock().await;
                backend_names
                    .iter()
                    .map(|backend_name| {
                        (
                            backend_name.clone(),
                            health
                                .get(backend_name)
                                .map(|entry| entry.snapshot(backend_name)),
                        )
                    })
                    .collect::<Vec<_>>()
            };

            for (backend_name, snapshot) in snapshots {
                let check_name = format!("backend.{backend_name}");
                match snapshot {
                    Some(snapshot) if snapshot.health_check_healthy == Some(true) => {
                        checks.push(ReadinessCheck {
                            name: check_name,
                            status: "ok",
                            detail: None,
                        });
                    }
                    Some(snapshot) => checks.push(ReadinessCheck {
                        name: check_name,
                        status: "error",
                        detail: Some(snapshot.health_check_last_error.unwrap_or_else(|| {
                            "backend health check reported unhealthy".to_string()
                        })),
                    }),
                    None => checks.push(ReadinessCheck {
                        name: check_name,
                        status: "error",
                        detail: Some("backend health check has not completed yet".to_string()),
                    }),
                }
            }
        } else {
            checks.push(ReadinessCheck {
                name: "backend_health".to_string(),
                status: "error",
                detail: Some("backend health tracking is not initialized".to_string()),
            });
        }
    }

    #[cfg(not(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis",
        feature = "gateway-routing-advanced"
    )))]
    let _ = &state;

    let is_ready = checks.iter().all(|check| check.status == "ok");
    let status = if is_ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    let body = ReadinessResponse {
        status: if is_ready { "ready" } else { "not_ready" },
        checks,
    };

    (status, Json(body))
}

async fn metrics(State(state): State<GatewayHttpState>) -> Json<ObservabilitySnapshot> {
    Json(state.observability_snapshot())
}

#[cfg(feature = "gateway-proxy-cache")]
#[derive(Debug, Deserialize)]
struct PurgeProxyCacheRequest {
    #[serde(default)]
    all: bool,
    #[serde(flatten)]
    selector: ProxyCachePurgeSelector,
}

#[cfg(feature = "gateway-proxy-cache")]
#[derive(Debug, Serialize)]
struct PurgeProxyCacheResponse {
    cleared_memory: bool,
    deleted_memory: u64,
    deleted_redis: Option<u64>,
}
// end inline: core/diagnostics.rs

#[cfg(feature = "gateway-metrics-prometheus")]
async fn metrics_prometheus(
    State(state): State<GatewayHttpState>,
) -> Result<(StatusCode, HeaderMap, String), (StatusCode, Json<ErrorResponse>)> {
    let Some(metrics) = state.proxy.metrics.as_ref() else {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            "not_configured",
            "prometheus metrics not enabled",
        ));
    };

    let rendered = {
        let metrics = metrics.lock().await;
        metrics.render()
    };
    let rendered = state.redact_observability_prometheus_metrics(&rendered);
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("text/plain; version=0.0.4"),
    );
    Ok((StatusCode::OK, headers, rendered))
}

async fn handle_gateway(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Json(payload): Json<GatewayHttpRequest>,
) -> Result<Json<GatewayResponse>, (StatusCode, Json<ErrorResponse>)> {
    let virtual_key = payload
        .virtual_key
        .or_else(|| extract_virtual_key(&headers))
        .ok_or_else(|| {
            error_response(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "missing virtual key",
            )
        })?;

    let request = GatewayRequest {
        virtual_key,
        model: payload.model,
        prompt: payload.prompt,
        input_tokens: payload.input_tokens,
        max_output_tokens: payload.max_output_tokens,
        passthrough: payload.passthrough,
    };

    let request_id = extract_header(&headers, "x-request-id").unwrap_or_else(generate_request_id);
    let model = request.model.clone();
    let tokens = u64::from(request.total_tokens());
    let passthrough = request.passthrough;

    emit_json_log(
        &state,
        "gateway.request",
        serde_json::json!({
            "request_id": &request_id,
            "model": &model,
            "tokens": tokens,
            "passthrough": passthrough,
        }),
    );

    let prepared = state.gateway.prepare_handle_request(&request);
    let (_virtual_key_id, result) = match prepared {
        Ok(GatewayPreparedRequest::Cached { key_id, response }) => (Some(key_id), Ok(response)),
        Ok(GatewayPreparedRequest::Call(prepared)) => {
            let virtual_key_id = Some(prepared.key_id.clone());
            match prepared.backend.call(&request).await {
                Ok(mut response) => {
                    response.backend = prepared.backend_name.clone();
                    response.cached = false;
                    state.gateway.complete_handle_success(&prepared, &response);
                    (virtual_key_id, Ok(response))
                }
                Err(err) => {
                    state.gateway.complete_handle_failure(&prepared);
                    (virtual_key_id, Err(err))
                }
            }
        }
        Err(err) => (None, Err(err)),
    };

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    {
        let input_tokens = u64::from(request.input_tokens);
        let spent_tokens = result
            .as_ref()
            .ok()
            .map(|response| input_tokens.saturating_add(u64::from(response.output_tokens)));
        if let Some(spent_tokens) = spent_tokens {
            #[cfg(feature = "gateway-store-sqlite")]
            if let Some(store) = state.stores.sqlite.as_ref()
                && let Some(virtual_key_id) = _virtual_key_id.as_deref()
                && let Err(err) = store
                    .record_spent_tokens(virtual_key_id, spent_tokens)
                    .await
            {
                emit_json_log(
                    &state,
                    "gateway.warning",
                    serde_json::json!({
                        "request_id": &request_id,
                        "virtual_key_id": virtual_key_id,
                        "warning": "store_record_spent_tokens_failed",
                        "store": "sqlite",
                        "error": err.to_string(),
                    }),
                );
            }

            #[cfg(feature = "gateway-store-postgres")]
            if let Some(store) = state.stores.postgres.as_ref()
                && let Some(virtual_key_id) = _virtual_key_id.as_deref()
                && let Err(err) = store
                    .record_spent_tokens(virtual_key_id, spent_tokens)
                    .await
            {
                emit_json_log(
                    &state,
                    "gateway.warning",
                    serde_json::json!({
                        "request_id": &request_id,
                        "virtual_key_id": virtual_key_id,
                        "warning": "store_record_spent_tokens_failed",
                        "store": "postgres",
                        "error": err.to_string(),
                    }),
                );
            }

            #[cfg(feature = "gateway-store-mysql")]
            if let Some(store) = state.stores.mysql.as_ref()
                && let Some(virtual_key_id) = _virtual_key_id.as_deref()
                && let Err(err) = store
                    .record_spent_tokens(virtual_key_id, spent_tokens)
                    .await
            {
                emit_json_log(
                    &state,
                    "gateway.warning",
                    serde_json::json!({
                        "request_id": &request_id,
                        "virtual_key_id": virtual_key_id,
                        "warning": "store_record_spent_tokens_failed",
                        "store": "mysql",
                        "error": err.to_string(),
                    }),
                );
            }

            #[cfg(feature = "gateway-store-redis")]
            if let Some(store) = state.stores.redis.as_ref()
                && let Some(virtual_key_id) = _virtual_key_id.as_deref()
                && let Err(err) = store
                    .record_spent_tokens(virtual_key_id, spent_tokens)
                    .await
            {
                emit_json_log(
                    &state,
                    "gateway.warning",
                    serde_json::json!({
                        "request_id": &request_id,
                        "virtual_key_id": virtual_key_id,
                        "warning": "store_record_spent_tokens_failed",
                        "store": "redis",
                        "error": err.to_string(),
                    }),
                );
            }
        }
    }

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    if let Some(virtual_key_id) = _virtual_key_id.as_deref() {
        append_audit_log(
            &state,
            "gateway",
            serde_json::json!({
                "request_id": &request_id,
                "virtual_key_id": virtual_key_id,
                "model": &model,
                "tokens": tokens,
                "ok": result.is_ok(),
            }),
        )
        .await
        .map_err(|err| error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage_error", err))?;
    }

    match result {
        Ok(response) => {
            emit_json_log(
                &state,
                "gateway.response",
                serde_json::json!({
                    "request_id": &request_id,
                    "backend": &response.backend,
                    "cached": response.cached,
                }),
            );
            Ok(Json(response))
        }
        Err(err) => {
            emit_json_log(
                &state,
                "gateway.error",
                serde_json::json!({
                    "request_id": &request_id,
                    "error": err.to_string(),
                }),
            );
            Err(map_gateway_error(err))
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct OpenAiErrorDetail {
    message: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct OpenAiErrorResponse {
    error: OpenAiErrorDetail,
}

fn openai_error(
    status: StatusCode,
    kind: impl Into<String>,
    code: Option<&str>,
    message: impl std::fmt::Display,
) -> (StatusCode, Json<OpenAiErrorResponse>) {
    (
        status,
        Json(OpenAiErrorResponse {
            error: OpenAiErrorDetail {
                message: message.to_string(),
                kind: kind.into(),
                code: code.map(ToString::to_string),
            },
        }),
    )
}

fn collect_budget_scopes<'a>(
    virtual_key_id: Option<&'a str>,
    budget: Option<&'a super::BudgetConfig>,
    tenant_budget_scope: &'a Option<(String, super::BudgetConfig)>,
    project_budget_scope: &'a Option<(String, super::BudgetConfig)>,
    user_budget_scope: &'a Option<(String, super::BudgetConfig)>,
) -> Vec<(&'a str, &'a super::BudgetConfig)> {
    let mut scopes = Vec::new();
    if let (Some(scope), Some(budget)) = (virtual_key_id, budget) {
        scopes.push((scope, budget));
    }
    if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
        scopes.push((scope.as_str(), budget));
    }
    if let Some((scope, budget)) = project_budget_scope.as_ref() {
        scopes.push((scope.as_str(), budget));
    }
    if let Some((scope, budget)) = user_budget_scope.as_ref() {
        scopes.push((scope.as_str(), budget));
    }
    scopes
}

#[cfg(feature = "gateway-costing")]
fn max_option_u64(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

#[derive(Debug, Serialize)]
struct GoogleApiErrorDetail {
    code: u16,
    message: String,
    status: &'static str,
}

#[derive(Debug, Serialize)]
struct GoogleApiErrorResponse {
    error: GoogleApiErrorDetail,
}

fn google_error(
    status: StatusCode,
    message: impl Into<String>,
) -> (StatusCode, Json<GoogleApiErrorResponse>) {
    (
        status,
        Json(GoogleApiErrorResponse {
            error: GoogleApiErrorDetail {
                code: status.as_u16(),
                message: message.into(),
                status: "INVALID_ARGUMENT",
            },
        }),
    )
}

fn gateway_uses_virtual_keys(state: &GatewayHttpState) -> bool {
    state.uses_virtual_keys()
}

#[derive(Clone, Copy, Debug)]
struct InternalUpstreamAuthPassthrough;

fn enable_internal_upstream_auth_passthrough(req: &mut axum::http::Request<Body>) {
    req.extensions_mut().insert(InternalUpstreamAuthPassthrough);
}

fn internal_upstream_auth_passthrough_enabled(parts: &axum::http::request::Parts) -> bool {
    parts
        .extensions
        .get::<InternalUpstreamAuthPassthrough>()
        .is_some()
}

fn synthesize_bearer_header(token: &str) -> Option<axum::http::HeaderValue> {
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    axum::http::HeaderValue::from_str(&format!("Bearer {token}")).ok()
}
// end inline: ../../http/core.rs
// inlined from ../../http/openai_compat_proxy.rs
// This file is intentionally split to keep each staged Rust file under the pre-commit size limit.

// inlined from ../../http/proxy.rs
// inlined from proxy/core.rs
// This file is intentionally split to keep each staged Rust file under the pre-commit size limit.
// inlined from core/schema.rs
#[cfg(feature = "gateway-routing-advanced")]
use std::time::Duration;

#[allow(dead_code)]
fn clamp_u64_to_u32(value: u64) -> u32 {
    if value > u64::from(u32::MAX) {
        u32::MAX
    } else {
        value as u32
    }
}

fn estimate_tokens_from_bytes(body: &Bytes) -> u32 {
    let len = body.len();
    if len == 0 {
        return 0;
    }
    let estimate = (len.saturating_add(3) / 4) as u64;
    if estimate > u64::from(u32::MAX) {
        u32::MAX
    } else {
        estimate as u32
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ObservedUsage {
    input_tokens: Option<u64>,
    cache_input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    reasoning_tokens: Option<u64>,
    total_tokens: Option<u64>,
}

#[derive(serde::Deserialize)]
struct OpenAiUsageEnvelope {
    usage: Option<OpenAiUsagePayload>,
}

#[derive(serde::Deserialize)]
struct OpenAiUsagePayload {
    #[serde(default)]
    total_tokens: Option<u64>,
    #[serde(default, alias = "prompt_tokens")]
    input_tokens: Option<u64>,
    #[serde(default, alias = "completion_tokens")]
    output_tokens: Option<u64>,
    #[serde(default)]
    reasoning_tokens: Option<u64>,
    #[serde(default, alias = "prompt_tokens_details")]
    input_tokens_details: Option<OpenAiInputTokenDetails>,
    #[serde(default)]
    output_tokens_details: Option<OpenAiOutputTokenDetails>,
    #[serde(default)]
    completion_tokens_details: Option<OpenAiOutputTokenDetails>,
    #[serde(default)]
    cache_creation_input_tokens: Option<u64>,
}

#[derive(serde::Deserialize)]
struct OpenAiInputTokenDetails {
    #[serde(default)]
    cached_tokens: Option<u64>,
    #[serde(default, alias = "cache_creation_tokens")]
    cache_creation_tokens: Option<u64>,
}

#[derive(serde::Deserialize)]
struct OpenAiOutputTokenDetails {
    #[serde(default)]
    reasoning_tokens: Option<u64>,
}

fn extract_openai_usage_from_bytes(bytes: &Bytes) -> Option<ObservedUsage> {
    extract_openai_usage_from_slice(bytes.as_ref())
}

fn extract_openai_usage_from_slice(bytes: &[u8]) -> Option<ObservedUsage> {
    let usage = serde_json::from_slice::<OpenAiUsageEnvelope>(bytes)
        .ok()?
        .usage?;

    let input_tokens = usage.input_tokens;
    let output_tokens = usage.output_tokens;
    let reasoning_tokens = usage.reasoning_tokens.or_else(|| {
        usage
            .output_tokens_details
            .as_ref()
            .and_then(|details| details.reasoning_tokens)
            .or_else(|| {
                usage
                    .completion_tokens_details
                    .as_ref()
                    .and_then(|details| details.reasoning_tokens)
            })
    });
    let total_tokens = usage.total_tokens.or_else(|| {
        input_tokens.and_then(|input| output_tokens.map(|output| input.saturating_add(output)))
    });
    let cache_input_tokens = usage
        .input_tokens_details
        .as_ref()
        .and_then(|details| details.cached_tokens);
    let cache_creation_input_tokens = usage.cache_creation_input_tokens.or_else(|| {
        usage
            .input_tokens_details
            .as_ref()
            .and_then(|details| details.cache_creation_tokens)
    });

    Some(ObservedUsage {
        input_tokens,
        cache_input_tokens,
        cache_creation_input_tokens,
        output_tokens,
        reasoning_tokens,
        total_tokens,
    })
}

fn sanitize_proxy_headers(headers: &mut HeaderMap, strip_authorization: bool) {
    if strip_authorization {
        headers.remove("authorization");
        headers.remove("x-api-key");
        headers.remove("x-litellm-api-key");
    }
    headers.remove("proxy-authorization");
    headers.remove("x-forwarded-authorization");
    headers.remove("connection");
    headers.remove("keep-alive");
    headers.remove("proxy-authenticate");
    headers.remove("proxy-connection");
    headers.remove("te");
    headers.remove("trailer");
    headers.remove("transfer-encoding");
    headers.remove("upgrade");
    headers.remove("x-ditto-virtual-key");
    headers.remove("x-ditto-protocol");
    headers.remove("x-ditto-cache-bypass");
    headers.remove("x-ditto-bypass-cache");
    headers.remove("content-length");
}

fn apply_backend_headers(headers: &mut HeaderMap, backend_headers: &HeaderMap) {
    for (name, value) in backend_headers.iter() {
        headers.insert(name, value.clone());
    }
}

fn generate_request_id() -> String {
    let seq = REQUEST_ID_SEQ.fetch_add(1, Ordering::Relaxed);
    let ts_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    format!("ditto-{ts_ms}-{seq}")
}

fn insert_request_id(headers: &mut HeaderMap, request_id: &str) {
    let value = match axum::http::HeaderValue::from_str(request_id) {
        Ok(value) => value,
        Err(_) => return,
    };
    headers.insert("x-request-id", value);
}

fn emit_json_log(state: &GatewayHttpState, event: &str, payload: serde_json::Value) {
    if !state.admin.json_logs {
        return;
    }

    let Some(payload) = state.prepare_observability_event(
        crate::gateway::observability::GatewayObservabilitySink::JsonLogs,
        payload,
    ) else {
        return;
    };
    let record = serde_json::json!({
        "ts_ms": SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0),
        "event": event,
        "payload": payload,
    });
    eprintln!("{record}");
}

#[cfg(feature = "sdk")]
fn emit_devtools_log(state: &GatewayHttpState, kind: &str, payload: serde_json::Value) {
    let Some(logger) = state.admin.devtools.as_ref() else {
        return;
    };
    let Some(payload) = state.prepare_observability_event(
        crate::gateway::observability::GatewayObservabilitySink::Devtools,
        payload,
    ) else {
        return;
    };
    let _ = logger.log_event(kind, payload);
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
fn enrich_audit_payload_virtual_key_scope(state: &GatewayHttpState, payload: &mut Value) {
    let Some(fields) = payload.as_object_mut() else {
        return;
    };
    let Some(virtual_key_id) = fields
        .get("virtual_key_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
    };

    let Some(key) = state
        .list_virtual_keys_snapshot()
        .into_iter()
        .find(|candidate| candidate.id == virtual_key_id)
    else {
        return;
    };

    if !fields.contains_key("tenant_id")
        && let Some(tenant_id) = key.tenant_id
    {
        fields.insert("tenant_id".to_string(), Value::String(tenant_id));
    }
    if !fields.contains_key("project_id")
        && let Some(project_id) = key.project_id
    {
        fields.insert("project_id".to_string(), Value::String(project_id));
    }
    if !fields.contains_key("user_id")
        && let Some(user_id) = key.user_id
    {
        fields.insert("user_id".to_string(), Value::String(user_id));
    }
}

fn ensure_virtual_key_tokens_exportable(
    keys: &[VirtualKeyConfig],
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if keys
        .iter()
        .any(|key| crate::gateway::config::virtual_key_token_is_persisted_hash(&key.token))
    {
        return Err(error_response(
            StatusCode::CONFLICT,
            "secret_unavailable",
            "include_tokens=true is unavailable after reloading virtual keys from hashed persistence",
        ));
    }

    Ok(())
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
fn report_proxy_audit_append_failure(store: &str, kind: &str, err: &impl std::fmt::Display) {
    eprintln!("failed to append {store} proxy audit log `{kind}`: {err}");
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
pub(super) fn openai_storage_error_response(
    message: impl std::fmt::Display,
) -> (StatusCode, Json<OpenAiErrorResponse>) {
    openai_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "api_error",
        Some("storage_error"),
        message,
    )
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
async fn append_audit_log(
    state: &GatewayHttpState,
    kind: &str,
    mut payload: serde_json::Value,
) -> Result<(), String> {
    enrich_audit_payload_virtual_key_scope(state, &mut payload);
    let Some(payload) = state.prepare_observability_event(
        crate::gateway::observability::GatewayObservabilitySink::Audit,
        payload,
    ) else {
        return Ok(());
    };

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.stores.sqlite.as_ref()
        && let Err(err) = store.append_audit_log(kind, payload.clone()).await
    {
        report_proxy_audit_append_failure("sqlite", kind, &err);
        return Err(format!(
            "failed to append sqlite proxy audit log `{kind}`: {err}"
        ));
    }

    #[cfg(feature = "gateway-store-postgres")]
    if let Some(store) = state.stores.postgres.as_ref()
        && let Err(err) = store.append_audit_log(kind, payload.clone()).await
    {
        report_proxy_audit_append_failure("postgres", kind, &err);
        return Err(format!(
            "failed to append postgres proxy audit log `{kind}`: {err}"
        ));
    }

    #[cfg(feature = "gateway-store-mysql")]
    if let Some(store) = state.stores.mysql.as_ref()
        && let Err(err) = store.append_audit_log(kind, payload.clone()).await
    {
        report_proxy_audit_append_failure("mysql", kind, &err);
        return Err(format!(
            "failed to append mysql proxy audit log `{kind}`: {err}"
        ));
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.stores.redis.as_ref()
        && let Err(err) = store.append_audit_log(kind, payload).await
    {
        report_proxy_audit_append_failure("redis", kind, &err);
        return Err(format!(
            "failed to append redis proxy audit log `{kind}`: {err}"
        ));
    }

    Ok(())
}

type ProxyBodyStream = BoxStream<'static, Result<Bytes, std::io::Error>>;

#[derive(Default)]
struct ProxyPermits {
    _proxy: Option<OwnedSemaphorePermit>,
    _backend: Option<OwnedSemaphorePermit>,
}

impl ProxyPermits {
    fn new(proxy: Option<OwnedSemaphorePermit>, backend: Option<OwnedSemaphorePermit>) -> Self {
        Self {
            _proxy: proxy,
            _backend: backend,
        }
    }

    fn is_empty(&self) -> bool {
        self._proxy.is_none() && self._backend.is_none()
    }

    fn take(&mut self) -> Self {
        Self {
            _proxy: self._proxy.take(),
            _backend: self._backend.take(),
        }
    }
}

struct ProxyBodyStreamWithPermit {
    inner: ProxyBodyStream,
    _permits: ProxyPermits,
}

impl futures_util::Stream for ProxyBodyStreamWithPermit {
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        this.inner.as_mut().poll_next(cx)
    }
}

fn proxy_body_from_bytes_with_permit(bytes: Bytes, proxy_permits: ProxyPermits) -> Body {
    if proxy_permits.is_empty() {
        return Body::from(bytes);
    };

    let stream =
        futures_util::stream::once(async move { Ok::<Bytes, std::io::Error>(bytes) }).boxed();
    let stream = ProxyBodyStreamWithPermit {
        inner: stream,
        _permits: proxy_permits,
    };
    Body::from_stream(stream)
}
// end inline: core/schema.rs
// inlined from core/response.rs
#[derive(Clone, Copy, Debug)]
enum ProxyStreamEnd {
    Completed,
    Error,
    #[cfg(feature = "gateway-metrics-prometheus")]
    Aborted,
}

#[cfg(feature = "gateway-metrics-prometheus")]
struct ProxyStreamFinalizer {
    metrics: Option<Arc<Mutex<super::metrics_prometheus::PrometheusMetrics>>>,
    backend: String,
    path: String,
}

#[cfg(feature = "gateway-metrics-prometheus")]
impl ProxyStreamFinalizer {
    async fn finalize(self, end: ProxyStreamEnd, stream_bytes: u64) {
        let Some(metrics) = self.metrics else {
            return;
        };
        let mut metrics = metrics.lock().await;
        metrics.record_proxy_stream_close(&self.backend, &self.path);
        metrics.record_proxy_stream_bytes(&self.backend, &self.path, stream_bytes);
        match end {
            ProxyStreamEnd::Completed => {
                metrics.record_proxy_stream_completed(&self.backend, &self.path);
            }
            ProxyStreamEnd::Error => {
                metrics.record_proxy_stream_error(&self.backend, &self.path);
            }
            ProxyStreamEnd::Aborted => {
                metrics.record_proxy_stream_aborted(&self.backend, &self.path);
            }
        }
    }
}

#[cfg(feature = "gateway-metrics-prometheus")]
const PROXY_STREAM_ABORT_FINALIZER_WORKERS: usize = 2;

#[cfg(feature = "gateway-metrics-prometheus")]
const PROXY_STREAM_ABORT_FINALIZER_QUEUE_CAPACITY: usize = 1024;

#[cfg(feature = "gateway-metrics-prometheus")]
struct ProxyStreamAbortFinalizeJob {
    finalizer: ProxyStreamFinalizer,
    bytes_sent: u64,
}

#[cfg(feature = "gateway-metrics-prometheus")]
struct ProxyStreamAbortFinalizerPool {
    senders: Vec<std::sync::mpsc::SyncSender<ProxyStreamAbortFinalizeJob>>,
    next_sender: std::sync::atomic::AtomicUsize,
}

#[cfg(feature = "gateway-metrics-prometheus")]
fn proxy_stream_abort_finalizer_pool() -> &'static ProxyStreamAbortFinalizerPool {
    static POOL: std::sync::OnceLock<ProxyStreamAbortFinalizerPool> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        let workers = PROXY_STREAM_ABORT_FINALIZER_WORKERS.max(1);
        let capacity = PROXY_STREAM_ABORT_FINALIZER_QUEUE_CAPACITY.max(1);
        let mut senders = Vec::with_capacity(workers);

        for worker in 0..workers {
            let (tx, rx) = std::sync::mpsc::sync_channel::<ProxyStreamAbortFinalizeJob>(capacity);
            let thread_name = format!("ditto-proxy-stream-finalizer-{worker}");
            let spawn_result = std::thread::Builder::new()
                .name(thread_name)
                .spawn(move || {
                    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    else {
                        return;
                    };
                    while let Ok(job) = rx.recv() {
                        runtime.block_on(async move {
                            job.finalizer
                                .finalize(ProxyStreamEnd::Aborted, job.bytes_sent)
                                .await;
                        });
                    }
                });
            if spawn_result.is_ok() {
                senders.push(tx);
            }
        }

        ProxyStreamAbortFinalizerPool {
            senders,
            next_sender: std::sync::atomic::AtomicUsize::new(0),
        }
    })
}

#[cfg(feature = "gateway-metrics-prometheus")]
fn spawn_proxy_stream_abort_finalize(job: ProxyStreamAbortFinalizeJob) {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            handle.spawn(async move {
                job.finalizer
                    .finalize(ProxyStreamEnd::Aborted, job.bytes_sent)
                    .await;
            });
        }
        Err(_) => {
            let _ = std::thread::Builder::new()
                .name("ditto-proxy-stream-finalizer-fallback".to_string())
                .spawn(move || {
                    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    else {
                        return;
                    };
                    runtime.block_on(async move {
                        job.finalizer
                            .finalize(ProxyStreamEnd::Aborted, job.bytes_sent)
                            .await;
                    });
                });
        }
    }
}

#[cfg(feature = "gateway-metrics-prometheus")]
fn enqueue_proxy_stream_abort_finalize(finalizer: ProxyStreamFinalizer, bytes_sent: u64) {
    let job = ProxyStreamAbortFinalizeJob {
        finalizer,
        bytes_sent,
    };
    let pool = proxy_stream_abort_finalizer_pool();

    if pool.senders.is_empty() {
        spawn_proxy_stream_abort_finalize(job);
        return;
    }

    let idx = pool
        .next_sender
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        % pool.senders.len();
    if let Err(err) = pool.senders[idx].try_send(job) {
        let job = match err {
            std::sync::mpsc::TrySendError::Full(job) => job,
            std::sync::mpsc::TrySendError::Disconnected(job) => job,
        };
        spawn_proxy_stream_abort_finalize(job);
    }
}

#[derive(Clone, Copy)]
struct ProxyResponseContext<'a> {
    state: &'a GatewayHttpState,
    backend: &'a str,
    request_id: &'a str,
    #[cfg(feature = "gateway-metrics-prometheus")]
    metrics_path: &'a str,
    cache_key: Option<&'a str>,
    #[cfg(feature = "gateway-proxy-cache")]
    cache_metadata: Option<&'a ProxyCacheEntryMetadata>,
}

#[cfg(feature = "gateway-proxy-cache")]
struct ProxyCompletedStreamCacheWrite {
    state: GatewayHttpState,
    backend: String,
    status: StatusCode,
    headers: HeaderMap,
    cache_key: String,
    cache_metadata: ProxyCacheEntryMetadata,
    recorder: ProxyCacheStreamRecorder,
}

#[cfg(feature = "gateway-proxy-cache")]
impl ProxyCompletedStreamCacheWrite {
    fn new(
        state: &GatewayHttpState,
        backend: &str,
        status: StatusCode,
        headers: &HeaderMap,
        cache_key: Option<&str>,
        cache_metadata: Option<&ProxyCacheEntryMetadata>,
    ) -> Option<Self> {
        if !status.is_success() {
            return None;
        }

        let (Some(config), Some(cache_key), Some(cache_metadata)) =
            (state.proxy.cache_config.as_ref(), cache_key, cache_metadata)
        else {
            return None;
        };

        Some(Self {
            state: state.clone(),
            backend: backend.to_string(),
            status,
            headers: headers.clone(),
            cache_key: cache_key.to_string(),
            cache_metadata: cache_metadata.clone(),
            recorder: config.stream_recorder()?,
        })
    }

    fn ingest(&mut self, chunk: &Bytes) {
        self.recorder.ingest(chunk);
    }

    async fn finish(self) {
        store_completed_stream_proxy_cache(
            &self.state,
            &self.cache_key,
            &self.cache_metadata,
            self.status,
            &self.headers,
            &self.backend,
            self.recorder,
        )
        .await;
    }
}

#[cfg(feature = "gateway-proxy-cache")]
async fn store_completed_stream_proxy_cache(
    state: &GatewayHttpState,
    cache_key: &str,
    metadata: &ProxyCacheEntryMetadata,
    status: StatusCode,
    headers: &HeaderMap,
    backend: &str,
    recorder: ProxyCacheStreamRecorder,
) {
    let Some(body) = recorder.finish() else {
        return;
    };

    let cached = CachedProxyResponse {
        status: status.as_u16(),
        headers: headers.clone(),
        body,
        backend: backend.to_string(),
    };
    store_proxy_cache_response(state, cache_key, cached, metadata, now_epoch_seconds()).await;
}

async fn proxy_response(
    ctx: ProxyResponseContext<'_>,
    upstream: reqwest::Response,
    proxy_permits: ProxyPermits,
) -> axum::response::Response {
    let _state = ctx.state;
    let backend = ctx.backend.to_string();
    let request_id = ctx.request_id.to_string();
    #[cfg(feature = "gateway-metrics-prometheus")]
    let metrics_path = ctx.metrics_path;
    let _cache_key = ctx.cache_key;
    #[cfg(feature = "gateway-proxy-cache")]
    let _cache_metadata = ctx.cache_metadata;
    let status = upstream.status();
    let upstream_headers = upstream.headers().clone();
    let content_type = upstream_headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if content_type.starts_with("text/event-stream") {
        let mut headers = upstream_headers;
        apply_proxy_response_headers(&mut headers, &backend, &request_id, false);
        if let Some(cache_key) = _cache_key
            && let Ok(value) = axum::http::HeaderValue::from_str(cache_key)
        {
            headers.insert("x-ditto-cache-key", value);
        }

        let upstream_stream: ProxyBodyStream = upstream
            .bytes_stream()
            .map(|chunk| chunk.map_err(std::io::Error::other))
            .boxed();

        struct StreamState {
            upstream: ProxyBodyStream,
            bytes_sent: u64,
            #[cfg(feature = "gateway-metrics-prometheus")]
            finalizer: Option<ProxyStreamFinalizer>,
            #[cfg(feature = "gateway-proxy-cache")]
            cache_completion: Option<ProxyCompletedStreamCacheWrite>,
            _permits: ProxyPermits,
        }

        impl Drop for StreamState {
            fn drop(&mut self) {
                #[cfg(feature = "gateway-metrics-prometheus")]
                {
                    let Some(finalizer) = self.finalizer.take() else {
                        return;
                    };
                    let bytes_sent = self.bytes_sent;
                    enqueue_proxy_stream_abort_finalize(finalizer, bytes_sent);
                }
            }
        }

        impl StreamState {
            async fn finalize(&mut self, end: ProxyStreamEnd) {
                #[cfg(not(any(
                    feature = "gateway-proxy-cache",
                    feature = "gateway-metrics-prometheus"
                )))]
                let _ = end;

                #[cfg(feature = "gateway-proxy-cache")]
                if matches!(end, ProxyStreamEnd::Completed)
                    && let Some(cache_completion) = self.cache_completion.take()
                {
                    cache_completion.finish().await;
                }

                #[cfg(feature = "gateway-metrics-prometheus")]
                {
                    let Some(finalizer) = self.finalizer.take() else {
                        return;
                    };
                    let bytes_sent = self.bytes_sent;
                    finalizer.finalize(end, bytes_sent).await;
                }
            }
        }

        #[cfg(feature = "gateway-metrics-prometheus")]
        let metrics = _state.proxy.metrics.clone();
        #[cfg(feature = "gateway-metrics-prometheus")]
        if let Some(metrics) = metrics.as_ref() {
            metrics
                .lock()
                .await
                .record_proxy_stream_open(&backend, metrics_path);
        }

        #[cfg(feature = "gateway-metrics-prometheus")]
        let finalizer = ProxyStreamFinalizer {
            metrics,
            backend: backend.clone(),
            path: metrics_path.to_string(),
        };

        let state = StreamState {
            upstream: upstream_stream,
            bytes_sent: 0,
            #[cfg(feature = "gateway-metrics-prometheus")]
            finalizer: Some(finalizer),
            #[cfg(feature = "gateway-proxy-cache")]
            cache_completion: ProxyCompletedStreamCacheWrite::new(
                _state,
                &backend,
                status,
                &headers,
                _cache_key,
                _cache_metadata,
            ),
            _permits: proxy_permits,
        };

        let stream = futures_util::stream::try_unfold(state, |mut state| async move {
            match state.upstream.next().await {
                Some(Ok(chunk)) => {
                    state.bytes_sent = state.bytes_sent.saturating_add(chunk.len() as u64);
                    #[cfg(feature = "gateway-proxy-cache")]
                    if let Some(cache_completion) = state.cache_completion.as_mut() {
                        cache_completion.ingest(&chunk);
                    }
                    Ok(Some((chunk, state)))
                }
                Some(Err(err)) => {
                    state.finalize(ProxyStreamEnd::Error).await;
                    Err(err)
                }
                None => {
                    state.finalize(ProxyStreamEnd::Completed).await;
                    Ok(None)
                }
            }
        });

        let mut response = axum::response::Response::new(Body::from_stream(stream));
        *response.status_mut() = status;
        *response.headers_mut() = headers;
        response
    } else {
        let cacheable = status.is_success() && _cache_key.is_some();
        let should_buffer = cacheable && {
            #[cfg(feature = "gateway-proxy-cache")]
            {
                let content_length = upstream_headers
                    .get("content-length")
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse::<usize>().ok());
                _state.proxy.cache_config.as_ref().is_some_and(|config| {
                    content_length.is_some_and(|len| len <= config.max_body_bytes)
                })
            }
            #[cfg(not(feature = "gateway-proxy-cache"))]
            {
                false
            }
        };
        let mut headers = upstream_headers;
        apply_proxy_response_headers(&mut headers, &backend, &request_id, false);
        if let Some(cache_key) = _cache_key
            && let Ok(value) = axum::http::HeaderValue::from_str(cache_key)
        {
            headers.insert("x-ditto-cache-key", value);
        }
        if should_buffer {
            let max_body_bytes = {
                #[cfg(feature = "gateway-proxy-cache")]
                {
                    _state
                        .proxy
                        .cache_config
                        .as_ref()
                        .map(|c| c.max_body_bytes)
                        .unwrap_or(1)
                }
                #[cfg(not(feature = "gateway-proxy-cache"))]
                {
                    1
                }
            };
            let bytes = match read_reqwest_body_bytes_bounded_with_content_length(
                upstream,
                &headers,
                max_body_bytes,
            )
            .await
            {
                Ok(bytes) => bytes,
                Err(err) => {
                    return openai_error(
                        StatusCode::BAD_GATEWAY,
                        "api_error",
                        Some("invalid_backend_response"),
                        format_args!(
                            "backend response too large to buffer/cache (max={max_body_bytes}): {err}; disable proxy cache or use streaming"
                        ),
                    )
                    .into_response();
                }
            };

            #[cfg(feature = "gateway-proxy-cache")]
            if status.is_success()
                && let (Some(cache_key), Some(cache_metadata)) = (_cache_key, _cache_metadata)
            {
                let cached = CachedProxyResponse {
                    status: status.as_u16(),
                    headers: headers.clone(),
                    body: bytes.clone(),
                    backend: backend.clone(),
                };
                store_proxy_cache_response(
                    _state,
                    cache_key,
                    cached,
                    cache_metadata,
                    now_epoch_seconds(),
                )
                .await;
            }

            let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits);
            let mut response = axum::response::Response::new(body);
            *response.status_mut() = status;
            *response.headers_mut() = headers;
            return response;
        }

        headers.remove("content-length");
        let stream = upstream
            .bytes_stream()
            .map(|chunk| chunk.map_err(std::io::Error::other))
            .boxed();
        let stream = ProxyBodyStreamWithPermit {
            inner: stream,
            _permits: proxy_permits,
        };
        let mut response = axum::response::Response::new(Body::from_stream(stream));
        *response.status_mut() = status;
        *response.headers_mut() = headers;
        response
    }
}

async fn responses_shim_response(
    ctx: ProxyResponseContext<'_>,
    upstream: reqwest::Response,
    proxy_permits: ProxyPermits,
) -> Result<axum::response::Response, (StatusCode, Json<OpenAiErrorResponse>)> {
    let _state = ctx.state;
    let backend = ctx.backend.to_string();
    let request_id = ctx.request_id.to_string();
    #[cfg(feature = "gateway-metrics-prometheus")]
    let metrics_path = ctx.metrics_path;
    let _cache_key = ctx.cache_key;
    #[cfg(feature = "gateway-proxy-cache")]
    let _cache_metadata = ctx.cache_metadata;
    let status = upstream.status();
    let upstream_headers = upstream.headers().clone();
    let content_type = upstream_headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if content_type.starts_with("text/event-stream") {
        let data_stream = ditto_core::session_transport::sse_data_stream_from_response(upstream);
        let stream =
            responses_shim::chat_completions_sse_to_responses_sse(data_stream, request_id.clone());
        let upstream_stream: ProxyBodyStream = stream.boxed();
        let mut headers = upstream_headers;
        headers.insert(
            "x-ditto-shim",
            axum::http::HeaderValue::from_static("responses_via_chat_completions"),
        );
        headers.insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("text/event-stream"),
        );
        headers.remove("content-length");
        apply_proxy_response_headers(&mut headers, &backend, &request_id, false);
        if let Some(cache_key) = _cache_key
            && let Ok(value) = axum::http::HeaderValue::from_str(cache_key)
        {
            headers.insert("x-ditto-cache-key", value);
        }

        struct StreamState {
            upstream: ProxyBodyStream,
            bytes_sent: u64,
            #[cfg(feature = "gateway-metrics-prometheus")]
            finalizer: Option<ProxyStreamFinalizer>,
            #[cfg(feature = "gateway-proxy-cache")]
            cache_completion: Option<ProxyCompletedStreamCacheWrite>,
            _permits: ProxyPermits,
        }

        impl Drop for StreamState {
            fn drop(&mut self) {
                #[cfg(feature = "gateway-metrics-prometheus")]
                {
                    let Some(finalizer) = self.finalizer.take() else {
                        return;
                    };
                    let bytes_sent = self.bytes_sent;
                    enqueue_proxy_stream_abort_finalize(finalizer, bytes_sent);
                }
            }
        }

        impl StreamState {
            async fn finalize(&mut self, end: ProxyStreamEnd) {
                #[cfg(not(any(
                    feature = "gateway-proxy-cache",
                    feature = "gateway-metrics-prometheus"
                )))]
                let _ = end;

                #[cfg(feature = "gateway-proxy-cache")]
                if matches!(end, ProxyStreamEnd::Completed)
                    && let Some(cache_completion) = self.cache_completion.take()
                {
                    cache_completion.finish().await;
                }

                #[cfg(feature = "gateway-metrics-prometheus")]
                {
                    let Some(finalizer) = self.finalizer.take() else {
                        return;
                    };
                    let bytes_sent = self.bytes_sent;
                    finalizer.finalize(end, bytes_sent).await;
                }
            }
        }

        #[cfg(feature = "gateway-metrics-prometheus")]
        let metrics = _state.proxy.metrics.clone();
        #[cfg(feature = "gateway-metrics-prometheus")]
        if let Some(metrics) = metrics.as_ref() {
            metrics
                .lock()
                .await
                .record_proxy_stream_open(&backend, metrics_path);
        }

        #[cfg(feature = "gateway-metrics-prometheus")]
        let finalizer = ProxyStreamFinalizer {
            metrics,
            backend: backend.clone(),
            path: metrics_path.to_string(),
        };

        let state = StreamState {
            upstream: upstream_stream,
            bytes_sent: 0,
            #[cfg(feature = "gateway-metrics-prometheus")]
            finalizer: Some(finalizer),
            #[cfg(feature = "gateway-proxy-cache")]
            cache_completion: ProxyCompletedStreamCacheWrite::new(
                _state,
                &backend,
                status,
                &headers,
                _cache_key,
                _cache_metadata,
            ),
            _permits: proxy_permits,
        };

        let stream = futures_util::stream::try_unfold(state, |mut state| async move {
            match state.upstream.next().await {
                Some(Ok(chunk)) => {
                    state.bytes_sent = state.bytes_sent.saturating_add(chunk.len() as u64);
                    #[cfg(feature = "gateway-proxy-cache")]
                    if let Some(cache_completion) = state.cache_completion.as_mut() {
                        cache_completion.ingest(&chunk);
                    }
                    Ok(Some((chunk, state)))
                }
                Some(Err(err)) => {
                    state.finalize(ProxyStreamEnd::Error).await;
                    Err(err)
                }
                None => {
                    state.finalize(ProxyStreamEnd::Completed).await;
                    Ok(None)
                }
            }
        });

        let mut response = axum::response::Response::new(Body::from_stream(stream));
        *response.status_mut() = status;
        *response.headers_mut() = headers;
        Ok(response)
    } else {
        let max_body_bytes = 8 * 1024 * 1024;
        let bytes = read_reqwest_body_bytes_bounded_with_content_length(
            upstream,
            &upstream_headers,
            max_body_bytes,
        )
            .await
            .map_err(|err| {
                openai_error(
                    StatusCode::BAD_GATEWAY,
                    "api_error",
                    Some("invalid_backend_response"),
                    format!(
                        "chat/completions response too large to shim (max={max_body_bytes}): {err}; use streaming or call /v1/chat/completions directly"
                    ),
                )
            })?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|err| {
            openai_error(
                StatusCode::BAD_GATEWAY,
                "api_error",
                Some("invalid_backend_response"),
                format!("invalid chat/completions response: {err}"),
            )
        })?;
        let mapped =
            responses_shim::chat_completions_response_to_responses(&value).ok_or_else(|| {
                openai_error(
                    StatusCode::BAD_GATEWAY,
                    "api_error",
                    Some("invalid_backend_response"),
                    "chat/completions response cannot be mapped to /responses",
                )
            })?;
        let mapped_bytes = serde_json::to_vec(&mapped)
            .map(Bytes::from)
            .unwrap_or_else(|_| Bytes::from(mapped.to_string()));

        let mut headers = upstream_headers;
        headers.insert(
            "x-ditto-shim",
            axum::http::HeaderValue::from_static("responses_via_chat_completions"),
        );
        headers.insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("application/json"),
        );
        headers.remove("content-length");

        #[cfg(feature = "gateway-proxy-cache")]
        if status.is_success()
            && let (Some(cache_key), Some(cache_metadata)) = (_cache_key, _cache_metadata)
        {
            let cached = CachedProxyResponse {
                status: status.as_u16(),
                headers: headers.clone(),
                body: mapped_bytes.clone(),
                backend: backend.clone(),
            };
            store_proxy_cache_response(
                _state,
                cache_key,
                cached,
                cache_metadata,
                now_epoch_seconds(),
            )
            .await;
        }

        apply_proxy_response_headers(&mut headers, &backend, &request_id, false);
        if let Some(cache_key) = _cache_key
            && let Ok(value) = axum::http::HeaderValue::from_str(cache_key)
        {
            headers.insert("x-ditto-cache-key", value);
        }
        let body = proxy_body_from_bytes_with_permit(mapped_bytes, proxy_permits);
        let mut response = axum::response::Response::new(body);
        *response.status_mut() = status;
        *response.headers_mut() = headers;
        Ok(response)
    }
}

fn apply_proxy_response_headers(
    headers: &mut HeaderMap,
    backend: &str,
    request_id: &str,
    cache_hit: bool,
) {
    headers.insert(
        "x-ditto-backend",
        axum::http::HeaderValue::from_str(backend)
            .unwrap_or_else(|_| axum::http::HeaderValue::from_static("unknown")),
    );
    if cache_hit {
        headers.insert("x-ditto-cache", axum::http::HeaderValue::from_static("hit"));
    } else {
        headers.remove("x-ditto-cache");
    }
    if let Ok(value) = axum::http::HeaderValue::from_str(request_id) {
        headers.insert("x-ditto-request-id", value.clone());
        headers.insert("x-request-id", value);
    }
}

#[cfg(feature = "gateway-proxy-cache")]
fn cached_proxy_response(
    cached: CachedProxyResponse,
    request_id: String,
) -> axum::response::Response {
    let status = StatusCode::from_u16(cached.status).unwrap_or(StatusCode::OK);
    let mut headers = cached.headers.clone();
    apply_proxy_response_headers(&mut headers, &cached.backend, &request_id, true);
    let mut response = axum::response::Response::new(Body::from(cached.body));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
}

#[cfg(feature = "gateway-proxy-cache")]
async fn store_proxy_cache_response(
    state: &GatewayHttpState,
    cache_key: &str,
    cached: CachedProxyResponse,
    metadata: &ProxyCacheEntryMetadata,
    now_epoch_seconds: u64,
) {
    if let Some(config) = state.proxy.cache_config.as_ref() {
        let max_body_bytes = config.max_body_bytes_for_headers(&cached.headers);
        if max_body_bytes == 0 || cached.body.len() > max_body_bytes {
            return;
        }
    }

    #[cfg(feature = "gateway-metrics-prometheus")]
    let mut redis_store_error: Option<bool> = None;

    #[cfg(feature = "gateway-store-redis")]
    if let (Some(store), Some(config)) = (
        state.stores.redis.as_ref(),
        state.proxy.cache_config.as_ref(),
    ) {
        #[cfg(feature = "gateway-metrics-prometheus")]
        {
            let result = store
                .set_proxy_cache_response(cache_key, &cached, metadata, config.ttl_seconds)
                .await;
            redis_store_error = Some(result.is_err());
        }

        #[cfg(not(feature = "gateway-metrics-prometheus"))]
        {
            let _ = store
                .set_proxy_cache_response(cache_key, &cached, metadata, config.ttl_seconds)
                .await;
        }
    }

    if let Some(cache) = state.proxy.cache.as_ref() {
        let mut cache = cache.lock().await;
        cache.insert_with_metadata(
            cache_key.to_string(),
            cached,
            metadata.clone(),
            now_epoch_seconds,
        );
    }

    #[cfg(feature = "gateway-metrics-prometheus")]
    if let Some(metrics) = state.proxy.metrics.as_ref() {
        let mut metrics = metrics.lock().await;
        metrics.record_proxy_cache_store("memory");
        if let Some(redis_error) = redis_store_error {
            metrics.record_proxy_cache_store("redis");
            if redis_error {
                metrics.record_proxy_cache_store_error("redis");
            }
        }
    }
}

fn now_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(feature = "gateway-routing-advanced")]
async fn filter_backend_candidates_by_health(
    state: &GatewayHttpState,
    candidates: Vec<String>,
    now_epoch_seconds: u64,
) -> Vec<String> {
    let Some(config) = state.proxy.routing.as_ref() else {
        return candidates;
    };
    if !config.circuit_breaker.enabled && !config.health_check.enabled {
        return candidates;
    }
    let Some(health) = state.proxy.backend_health.as_ref() else {
        return candidates;
    };

    let filtered = {
        let health = health.lock().await;
        super::proxy_routing::filter_healthy_backends(&candidates, &health, now_epoch_seconds)
    };

    if filtered.is_empty() {
        candidates
    } else {
        filtered
    }
}

#[cfg(feature = "gateway-routing-advanced")]
async fn record_proxy_backend_failure(
    state: &GatewayHttpState,
    backend: &str,
    now_epoch_seconds: u64,
    kind: FailureKind,
    message: String,
) {
    let Some(config) = state.proxy.routing.as_ref() else {
        return;
    };
    let Some(health) = state.proxy.backend_health.as_ref() else {
        return;
    };

    let mut health = health.lock().await;
    let entry = health.entry(backend.to_string()).or_default();
    entry.record_failure(now_epoch_seconds, &config.circuit_breaker, kind, message);
    drop(health);
}

#[cfg(feature = "gateway-routing-advanced")]
async fn record_proxy_backend_success(state: &GatewayHttpState, backend: &str) {
    let Some(health) = state.proxy.backend_health.as_ref() else {
        return;
    };

    let mut health = health.lock().await;
    health
        .entry(backend.to_string())
        .or_default()
        .record_success();
}

#[cfg(feature = "gateway-routing-advanced")]
fn start_proxy_health_checks(state: &GatewayHttpState) -> Option<Arc<AbortOnDrop>> {
    const PROXY_HEALTH_CHECK_MAX_CONCURRENCY: usize = 8;

    let config = state.proxy.routing.as_ref()?;
    if !config.health_check.enabled {
        return None;
    }
    let health = state.proxy.backend_health.as_ref()?;

    let backend_entries = state
        .backends
        .proxy_backends
        .iter()
        .map(|(backend_name, backend)| (backend_name.clone(), backend.clone()))
        .collect::<Vec<_>>();
    let health = health.clone();
    let path = config.health_check.path.clone();
    let interval = Duration::from_secs(config.health_check.interval_seconds.max(1));
    let timeout = Duration::from_secs(config.health_check.timeout_seconds.max(1));

    let task = tokio::spawn(async move {
        loop {
            let check_stream = stream::iter(backend_entries.iter().cloned())
                .map(|(backend_name, backend)| {
                    let path = path.clone();
                    async move {
                        let mut headers = HeaderMap::new();
                        apply_backend_headers(&mut headers, backend.headers());
                        let result = backend
                            .request_with_timeout(
                                reqwest::Method::GET,
                                &path,
                                headers,
                                None,
                                Some(timeout),
                            )
                            .await;
                        (backend_name, result)
                    }
                })
                .buffer_unordered(PROXY_HEALTH_CHECK_MAX_CONCURRENCY);
            futures_util::pin_mut!(check_stream);

            while let Some((backend_name, result)) = check_stream.next().await {
                let mut health = health.lock().await;
                let entry = health.entry(backend_name).or_default();
                match result {
                    Ok(response) if response.status().is_success() => {
                        entry.record_health_check_success();
                    }
                    Ok(response) => {
                        entry.record_health_check_failure(format!(
                            "health check returned {}",
                            response.status()
                        ));
                    }
                    Err(err) => entry.record_health_check_failure(err.to_string()),
                }
                drop(health);
            }

            tokio::time::sleep(interval).await;
        }
    });
    Some(Arc::new(AbortOnDrop::new(task.abort_handle())))
}

#[cfg(feature = "gateway-proxy-cache")]
fn proxy_cache_can_read(method: &axum::http::Method) -> bool {
    *method == axum::http::Method::GET || *method == axum::http::Method::POST
}

#[cfg(feature = "gateway-proxy-cache")]
fn proxy_cache_bypass(headers: &HeaderMap) -> bool {
    if headers.get("x-ditto-cache-bypass").is_some()
        || headers.get("x-ditto-bypass-cache").is_some()
    {
        return true;
    }

    headers
        .get("cache-control")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            let lowered = value.to_ascii_lowercase();
            lowered.contains("no-store") || lowered.contains("no-cache")
        })
}

#[cfg(feature = "gateway-proxy-cache")]
fn proxy_cache_scope(virtual_key_id: Option<&str>, headers: &HeaderMap) -> String {
    if let Some(virtual_key_id) = virtual_key_id {
        return format!("vk:{virtual_key_id}");
    }

    if let Some(authorization) = extract_header(headers, "authorization") {
        let hash = hash64_fnv1a(authorization.as_bytes());
        return format!("auth:{hash:016x}");
    }

    if let Some(api_key) = extract_header(headers, "x-api-key") {
        let hash = hash64_fnv1a(api_key.as_bytes());
        return format!("x-api-key:{hash:016x}");
    }

    "public".to_string()
}

#[cfg(feature = "gateway-proxy-cache")]
fn proxy_cache_header_affects_upstream(header: &str) -> bool {
    !matches!(
        header,
        "authorization"
            | "x-api-key"
            | "x-litellm-api-key"
            | "proxy-authorization"
            | "x-forwarded-authorization"
            | "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-connection"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "x-ditto-virtual-key"
            | "x-ditto-protocol"
            | "x-ditto-cache-bypass"
            | "x-ditto-bypass-cache"
            | "content-length"
            | "x-request-id"
            | "traceparent"
            | "tracestate"
            | "baggage"
    )
}

#[cfg(feature = "gateway-proxy-cache")]
fn proxy_cache_key(
    method: &axum::http::Method,
    path: &str,
    body: &Bytes,
    scope: &str,
    route_partition: &str,
    headers: &HeaderMap,
) -> String {
    let mut header_names: Vec<&str> = headers
        .keys()
        .map(|name| name.as_str())
        .filter(|name| proxy_cache_header_affects_upstream(name))
        .collect();
    header_names.sort_unstable();
    header_names.dedup();

    let mut hasher = Sha256Hasher::new();
    hasher.update(b"ditto-proxy-cache-v3|");
    hasher.update(method.as_str().as_bytes());
    hasher.update(b"|");
    hasher.update(path.as_bytes());
    hasher.update(b"|");
    hasher.update(scope.as_bytes());
    hasher.update(b"|");
    hasher.update(route_partition.as_bytes());
    hasher.update(b"|");
    for name in header_names {
        hasher.update(name.as_bytes());
        hasher.update(b":");
        for value in headers.get_all(name).iter() {
            hasher.update(value.as_bytes());
            hasher.update(b"\x1f");
        }
        hasher.update(b"\n");
    }
    hasher.update(b"|");
    hasher.update(body.as_ref());
    format!("ditto-proxy-cache-v3-{}", hasher.finalize())
}

#[cfg(feature = "gateway-proxy-cache")]
fn proxy_cache_route_partition(backend_candidates: &[String]) -> String {
    let mut hasher = Sha256Hasher::new();
    hasher.update(b"ditto-proxy-route-v1|");
    for backend in backend_candidates
        .iter()
        .map(|backend| backend.trim())
        .filter(|backend| !backend.is_empty())
    {
        hasher.update(backend.as_bytes());
        hasher.update(b"\x1f");
    }
    hasher.finalize().to_string()
}

#[cfg(feature = "gateway-proxy-cache")]
fn hash64_fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
// end inline: core/response.rs
// end inline: proxy/core.rs

// inlined from proxy/sanitize_proxy_headers_tests.rs
#[cfg(test)]
mod sanitize_proxy_headers_tests {
    use super::{HeaderMap, sanitize_proxy_headers};

    #[test]
    fn removes_hop_by_hop_and_proxy_auth_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "proxy-authorization",
            axum::http::HeaderValue::from_static("proxy-secret"),
        );
        headers.insert(
            "x-forwarded-authorization",
            axum::http::HeaderValue::from_static("forwarded-secret"),
        );
        headers.insert(
            "connection",
            axum::http::HeaderValue::from_static("keep-alive"),
        );
        headers.insert(
            "keep-alive",
            axum::http::HeaderValue::from_static("timeout=5"),
        );
        headers.insert(
            "proxy-authenticate",
            axum::http::HeaderValue::from_static("Basic realm=\"test\""),
        );
        headers.insert(
            "proxy-connection",
            axum::http::HeaderValue::from_static("keep-alive"),
        );
        headers.insert("te", axum::http::HeaderValue::from_static("trailers"));
        headers.insert(
            "trailer",
            axum::http::HeaderValue::from_static("some-trailer"),
        );
        headers.insert(
            "transfer-encoding",
            axum::http::HeaderValue::from_static("chunked"),
        );
        headers.insert("upgrade", axum::http::HeaderValue::from_static("websocket"));
        headers.insert(
            "content-length",
            axum::http::HeaderValue::from_static("123"),
        );
        headers.insert(
            "authorization",
            axum::http::HeaderValue::from_static("Bearer abc"),
        );
        headers.insert("x-api-key", axum::http::HeaderValue::from_static("abc"));
        headers.insert(
            "x-litellm-api-key",
            axum::http::HeaderValue::from_static("Bearer abc"),
        );
        headers.insert(
            "x-ditto-protocol",
            axum::http::HeaderValue::from_static("google"),
        );
        headers.insert("x-test", axum::http::HeaderValue::from_static("ok"));

        sanitize_proxy_headers(&mut headers, false);

        for name in [
            "proxy-authorization",
            "x-forwarded-authorization",
            "connection",
            "keep-alive",
            "proxy-authenticate",
            "proxy-connection",
            "te",
            "trailer",
            "transfer-encoding",
            "upgrade",
            "content-length",
            "x-ditto-protocol",
        ] {
            assert!(headers.get(name).is_none(), "{name} should be removed");
        }

        assert!(headers.get("authorization").is_some());
        assert!(headers.get("x-api-key").is_some());
        assert!(headers.get("x-litellm-api-key").is_some());
        assert_eq!(headers.get("x-test").unwrap().to_str().unwrap(), "ok");
    }

    #[test]
    fn strips_authorization_and_api_key_when_requested() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            axum::http::HeaderValue::from_static("Bearer abc"),
        );
        headers.insert("x-api-key", axum::http::HeaderValue::from_static("abc"));
        headers.insert(
            "x-litellm-api-key",
            axum::http::HeaderValue::from_static("Bearer abc"),
        );
        headers.insert("x-test", axum::http::HeaderValue::from_static("ok"));

        sanitize_proxy_headers(&mut headers, true);

        assert!(headers.get("authorization").is_none());
        assert!(headers.get("x-api-key").is_none());
        assert!(headers.get("x-litellm-api-key").is_none());
        assert_eq!(headers.get("x-test").unwrap().to_str().unwrap(), "ok");
    }
}

#[cfg(all(test, feature = "gateway-proxy-cache"))]
mod proxy_cache_key_tests {
    use super::{HeaderMap, proxy_cache_key, proxy_cache_route_partition};
    use axum::http::Method;
    use bytes::Bytes;

    #[test]
    fn proxy_cache_key_changes_when_route_partition_changes() {
        let headers = HeaderMap::new();
        let body = Bytes::from_static(
            br#"{"model":"gpt-test","messages":[{"role":"user","content":"hi"}]}"#,
        );

        let primary = proxy_cache_key(
            &Method::POST,
            "/v1/chat/completions",
            &body,
            "vk:key-1",
            &proxy_cache_route_partition(&["primary".to_string()]),
            &headers,
        );
        let secondary = proxy_cache_key(
            &Method::POST,
            "/v1/chat/completions",
            &body,
            "vk:key-1",
            &proxy_cache_route_partition(&["secondary".to_string()]),
            &headers,
        );

        assert_ne!(primary, secondary);
    }

    #[test]
    fn proxy_cache_route_partition_ignores_blank_candidates() {
        let compact =
            proxy_cache_route_partition(&["primary".to_string(), "secondary".to_string()]);
        let noisy = proxy_cache_route_partition(&[
            " primary ".to_string(),
            String::new(),
            "secondary".to_string(),
            "   ".to_string(),
        ]);

        assert_eq!(compact, noisy);
    }
}
// end inline: proxy/sanitize_proxy_headers_tests.rs
// inlined from proxy/usage_parsing_tests.rs
#[cfg(test)]
mod usage_parsing_tests {
    use super::extract_openai_usage_from_bytes;
    use bytes::Bytes;
    use serde_json::json;

    #[test]
    fn parses_openai_usage_with_cache_and_reasoning_details() {
        let response = json!({
            "id": "chatcmpl_test",
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 3,
                "total_tokens": 13,
                "prompt_tokens_details": {
                    "cached_tokens": 8,
                    "cache_creation_tokens": 2
                },
                "completion_tokens_details": {
                    "reasoning_tokens": 1
                }
            }
        });

        let bytes = Bytes::from(response.to_string());
        let usage = extract_openai_usage_from_bytes(&bytes).expect("usage");
        assert_eq!(usage.input_tokens, Some(10));
        assert_eq!(usage.cache_input_tokens, Some(8));
        assert_eq!(usage.cache_creation_input_tokens, Some(2));
        assert_eq!(usage.output_tokens, Some(3));
        assert_eq!(usage.reasoning_tokens, Some(1));
        assert_eq!(usage.total_tokens, Some(13));
    }

    #[test]
    fn computes_total_tokens_when_missing() {
        let response = json!({
            "usage": {
                "input_tokens": 4,
                "output_tokens": 5
            }
        });

        let bytes = Bytes::from(response.to_string());
        let usage = extract_openai_usage_from_bytes(&bytes).expect("usage");
        assert_eq!(usage.input_tokens, Some(4));
        assert_eq!(usage.output_tokens, Some(5));
        assert_eq!(usage.total_tokens, Some(9));
    }
}
// end inline: proxy/usage_parsing_tests.rs

#[cfg(all(test, feature = "gateway-store-sqlite"))]
mod audit_log_tests {
    use super::*;
    use crate::gateway::{Gateway, GatewayConfig, RouterConfig};

    #[tokio::test]
    async fn append_audit_log_returns_storage_error_for_broken_sqlite_store() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let broken_path = tmp.path().join("missing-parent").join("gateway.sqlite");
        let state = GatewayHttpState::new(Gateway::new(GatewayConfig {
            backends: Vec::new(),
            virtual_keys: Vec::new(),
            router: RouterConfig::default(),
            a2a_agents: Vec::new(),
            mcp_servers: Vec::new(),
            observability: Default::default(),
        }))
        .with_sqlite_store(SqliteStore::new(broken_path));

        let err = append_audit_log(&state, "proxy", serde_json::json!({"ok": true}))
            .await
            .expect_err("audit append should fail");
        assert!(err.contains("sqlite proxy audit log `proxy`"));
    }
}
// end inline: ../../http/proxy.rs
