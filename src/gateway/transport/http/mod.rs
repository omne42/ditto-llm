// Transport HTTP implementation.
mod control_plane;
use self::control_plane::GatewayControlPlaneSnapshot;

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
use futures_util::stream;
use futures_util::stream::BoxStream;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};

#[cfg(feature = "gateway-routing-advanced")]
use crate::utils::task::AbortOnDrop;

#[cfg(feature = "gateway-translation")]
#[derive(Clone, Copy, Debug)]
struct ProxySpend {
    tokens: u64,
    cost_usd_micros: Option<u64>,
}

#[cfg(feature = "sdk")]
use crate::sdk::devtools::DevtoolsLogger;

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
    Gateway, GatewayError, GatewayPreparedRequest, GatewayRequest, GatewayResponse,
    GatewayStateFile, ObservabilitySnapshot, ProxyBackend, RouterConfig, VirtualKeyConfig,
    lock_unpoisoned,
};

static REQUEST_ID_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
struct AdminTenantToken {
    tenant_id: String,
    token: String,
    read_only: bool,
}

#[derive(Clone, Debug, Serialize)]
struct ConfigVersionInfo {
    version_id: String,
    created_at_ms: u64,
    reason: String,
    virtual_key_count: usize,
    virtual_keys_sha256: String,
    router_default_backend_count: usize,
    router_rule_count: usize,
    router_sha256: String,
}

#[derive(Clone, Debug)]
struct ConfigVersionSnapshot {
    info: ConfigVersionInfo,
    virtual_keys: Vec<VirtualKeyConfig>,
    router: RouterConfig,
}

#[derive(Debug)]
struct ConfigVersionHistory {
    max_entries: usize,
    next_sequence: u64,
    entries: VecDeque<ConfigVersionSnapshot>,
}

impl ConfigVersionHistory {
    fn with_bootstrap(virtual_keys: Vec<VirtualKeyConfig>, router: RouterConfig) -> Self {
        let mut history = Self {
            max_entries: 100,
            next_sequence: 1,
            entries: VecDeque::new(),
        };
        let _ = history.push_snapshot(virtual_keys, router, "bootstrap");
        history
    }

    fn push_snapshot(
        &mut self,
        virtual_keys: Vec<VirtualKeyConfig>,
        router: RouterConfig,
        reason: impl Into<String>,
    ) -> ConfigVersionInfo {
        let info = ConfigVersionInfo {
            version_id: format!("cfgv-{:020}", self.next_sequence),
            created_at_ms: now_epoch_millis_u64(),
            reason: reason.into(),
            virtual_key_count: virtual_keys.len(),
            virtual_keys_sha256: virtual_keys_sha256(&virtual_keys),
            router_default_backend_count: router.default_backends.len(),
            router_rule_count: router.rules.len(),
            router_sha256: router_sha256(&router),
        };
        self.next_sequence = self.next_sequence.saturating_add(1);

        self.entries.push_back(ConfigVersionSnapshot {
            info: info.clone(),
            virtual_keys,
            router,
        });
        while self.entries.len() > self.max_entries {
            let _ = self.entries.pop_front();
        }

        info
    }

    fn current_info(&self) -> Option<ConfigVersionInfo> {
        self.entries.back().map(|snapshot| snapshot.info.clone())
    }

    fn list_infos_desc(&self) -> Vec<ConfigVersionInfo> {
        self.entries
            .iter()
            .rev()
            .map(|snapshot| snapshot.info.clone())
            .collect()
    }

    fn find_snapshot(&self, version_id: &str) -> Option<ConfigVersionSnapshot> {
        self.entries
            .iter()
            .find(|snapshot| snapshot.info.version_id == version_id)
            .cloned()
    }
}

fn now_epoch_millis_u64() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn virtual_keys_sha256(virtual_keys: &[VirtualKeyConfig]) -> String {
    use sha2::Digest as _;

    let payload = serde_json::to_vec(virtual_keys).unwrap_or_default();
    let mut hasher = sha2::Sha256::new();
    hasher.update(b"ditto-gateway-config-version-v1|");
    hasher.update(payload);

    hex_lower_bytes(&hasher.finalize())
}

fn router_sha256(router: &RouterConfig) -> String {
    use sha2::Digest as _;

    let payload = serde_json::to_vec(router).unwrap_or_default();
    let mut hasher = sha2::Sha256::new();
    hasher.update(b"ditto-gateway-router-version-v1|");
    hasher.update(payload);

    hex_lower_bytes(&hasher.finalize())
}

fn hex_lower_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
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
        }
    }
}

#[derive(Clone)]
pub struct GatewayHttpState {
    gateway: Arc<Gateway>,
    control_plane: Arc<RwLock<GatewayControlPlaneSnapshot>>,
    limits: Arc<StdMutex<RateLimiter>>,
    budget: Arc<StdMutex<BudgetTracker>>,
    observability: Arc<StdMutex<Observability>>,
    config_versions: Arc<Mutex<ConfigVersionHistory>>,
    redactor: Arc<GatewayRedactor>,
    observability_policy: Arc<GatewayObservabilityPolicy>,
    backends: GatewayRuntimeBackends,
    admin: GatewayAdminState,
    stores: GatewayPersistenceState,
    proxy: GatewayProxyRuntimeState,
}

impl GatewayHttpState {
    pub fn new(gateway: Gateway) -> Self {
        let initial_config = gateway.config_snapshot();
        let initial_virtual_keys = initial_config.virtual_keys.clone();
        let initial_router = initial_config.router.clone();
        let control_plane = GatewayControlPlaneSnapshot::from_gateway(&gateway);
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
            limits,
            budget,
            observability,
            config_versions: Arc::new(Mutex::new(ConfigVersionHistory::with_bootstrap(
                initial_virtual_keys,
                initial_router,
            ))),
            redactor,
            observability_policy,
            backends: GatewayRuntimeBackends::default(),
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

    pub(crate) fn check_and_consume_rate_limit(
        &self,
        scope: &str,
        limits: &super::LimitsConfig,
        tokens: u32,
        minute: u64,
    ) -> Result<(), GatewayError> {
        lock_unpoisoned(&self.limits).check_and_consume(scope, limits, tokens, minute)
    }

    pub(crate) fn can_spend_budget_tokens(
        &self,
        scope: &str,
        budget: &super::BudgetConfig,
        tokens: u64,
    ) -> Result<(), GatewayError> {
        lock_unpoisoned(&self.budget).can_spend(scope, budget, tokens)
    }

    #[cfg(feature = "gateway-costing")]
    pub(crate) fn can_spend_budget_cost(
        &self,
        scope: &str,
        budget: &super::BudgetConfig,
        usd_micros: u64,
    ) -> Result<(), GatewayError> {
        lock_unpoisoned(&self.budget).can_spend_cost_usd_micros(scope, budget, usd_micros)
    }

    pub(crate) fn spend_budget_tokens(
        &self,
        scope: &str,
        budget: &super::BudgetConfig,
        tokens: u64,
    ) {
        lock_unpoisoned(&self.budget).spend(scope, budget, tokens);
    }

    #[cfg(feature = "gateway-costing")]
    pub(crate) fn spend_budget_cost(
        &self,
        scope: &str,
        budget: &super::BudgetConfig,
        usd_micros: u64,
    ) {
        lock_unpoisoned(&self.budget).spend_cost_usd_micros(scope, budget, usd_micros);
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
        self
    }

    pub fn with_a2a_agents(mut self, agents: HashMap<String, A2aAgentState>) -> Self {
        self.backends.a2a_agents = Arc::new(agents);
        self
    }

    pub fn with_mcp_servers(mut self, servers: HashMap<String, McpServerState>) -> Self {
        self.backends.mcp_servers = Arc::new(servers);
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
        self
    }

    pub fn with_state_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.admin.state_file = Some(path.into());
        self
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

// inlined from core/diagnostics.rs
async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
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
            if let Some(store) = state.stores.sqlite.as_ref() {
                if let Some(virtual_key_id) = _virtual_key_id.as_deref() {
                    if let Err(err) = store
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
                }
            }

            #[cfg(feature = "gateway-store-postgres")]
            if let Some(store) = state.stores.postgres.as_ref() {
                if let Some(virtual_key_id) = _virtual_key_id.as_deref() {
                    if let Err(err) = store
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
                }
            }

            #[cfg(feature = "gateway-store-mysql")]
            if let Some(store) = state.stores.mysql.as_ref() {
                if let Some(virtual_key_id) = _virtual_key_id.as_deref() {
                    if let Err(err) = store
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
                }
            }

            #[cfg(feature = "gateway-store-redis")]
            if let Some(store) = state.stores.redis.as_ref() {
                if let Some(virtual_key_id) = _virtual_key_id.as_deref() {
                    if let Err(err) = store
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
        .await;
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

#[derive(Debug, Serialize)]
struct OpenAiErrorDetail {
    message: String,
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<&'static str>,
}

#[derive(Debug, Serialize)]
struct OpenAiErrorResponse {
    error: OpenAiErrorDetail,
}

fn openai_error(
    status: StatusCode,
    kind: &'static str,
    code: Option<&'static str>,
    message: impl std::fmt::Display,
) -> (StatusCode, Json<OpenAiErrorResponse>) {
    (
        status,
        Json(OpenAiErrorResponse {
            error: OpenAiErrorDetail {
                message: message.to_string(),
                kind,
                code,
            },
        }),
    )
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

fn synthesize_bearer_header(token: &str) -> Option<axum::http::HeaderValue> {
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    axum::http::HeaderValue::from_str(&format!("Bearer {token}")).ok()
}
// end inline: ../../http/core.rs
// inlined from ../../http/anthropic.rs
#[derive(Debug, Serialize)]
struct AnthropicErrorDetail {
    #[serde(rename = "type")]
    kind: &'static str,
    message: String,
}

#[derive(Debug, Serialize)]
struct AnthropicErrorResponse {
    #[serde(rename = "type")]
    kind: &'static str,
    error: AnthropicErrorDetail,
}

fn anthropic_error(
    status: StatusCode,
    kind: &'static str,
    message: impl Into<String>,
) -> (StatusCode, Json<AnthropicErrorResponse>) {
    (
        status,
        Json(AnthropicErrorResponse {
            kind: "error",
            error: AnthropicErrorDetail {
                kind,
                message: message.into(),
            },
        }),
    )
}

async fn handle_anthropic_messages(
    State(state): State<GatewayHttpState>,
    req: axum::http::Request<Body>,
) -> Result<axum::response::Response, (StatusCode, Json<AnthropicErrorResponse>)> {
    const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;

    let (parts, body) = req.into_parts();
    let body = to_bytes(body, MAX_BODY_BYTES).await.map_err(|err| {
        anthropic_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            err.to_string(),
        )
    })?;

    let request_json: serde_json::Value = serde_json::from_slice(&body).map_err(|err| {
        anthropic_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            format!("invalid JSON: {err}"),
        )
    })?;

    let openai_request = interop::anthropic_messages_request_to_openai_chat_completions(
        &request_json,
    )
    .map_err(|err| anthropic_error(StatusCode::BAD_REQUEST, "invalid_request_error", err))?;

    let stream_requested = openai_request
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    #[cfg(not(feature = "streaming"))]
    if stream_requested {
        return Err(anthropic_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "streaming is not enabled",
        ));
    }

    let openai_bytes = serde_json::to_vec(&openai_request).map_err(|err| {
        anthropic_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            format!("failed to serialize request: {err}"),
        )
    })?;

    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/json"),
    );
    headers.insert(
        "x-ditto-protocol",
        axum::http::HeaderValue::from_static("anthropic"),
    );
    if stream_requested {
        headers.insert(
            axum::http::header::ACCEPT,
            axum::http::HeaderValue::from_static("text/event-stream"),
        );
    }
    if let Some(value) = parts.headers.get("authorization") {
        headers.insert("authorization", value.clone());
    }
    if !headers.contains_key("authorization") {
        if let Some(token) = extract_virtual_key(&parts.headers)
            .as_deref()
            .and_then(synthesize_bearer_header)
        {
            headers.insert("authorization", token);
        }
    }
    if let Some(value) = parts.headers.get("x-request-id") {
        headers.insert("x-request-id", value.clone());
    }

    let mut openai_req = axum::http::Request::builder()
        .method(axum::http::Method::POST)
        .uri("/v1/chat/completions")
        .body(Body::from(openai_bytes))
        .map_err(|err| {
            anthropic_error(
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                err.to_string(),
            )
        })?;
    *openai_req.headers_mut() = headers;

    let openai_resp = handle_openai_compat_proxy(
        State(state.clone()),
        Path("chat/completions".to_string()),
        openai_req,
    )
    .await
    .map_err(|(status, err)| anthropic_error(status, "api_error", err.0.error.message))?;

    let status = openai_resp.status();
    let content_type = openai_resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if content_type.starts_with("text/event-stream") {
        #[cfg(feature = "streaming")]
        {
            use tokio_util::io::StreamReader;

            let (mut parts, body) = openai_resp.into_parts();
            parts.headers.insert(
                axum::http::header::CONTENT_TYPE,
                axum::http::HeaderValue::from_static("text/event-stream"),
            );
            parts.headers.remove("content-length");

            let data_stream = body
                .into_data_stream()
                .map(|result| result.map_err(|err| std::io::Error::other(err.to_string())));
            let reader = StreamReader::new(data_stream);
            let reader = tokio::io::BufReader::new(reader);
            let data_stream = crate::utils::sse::sse_data_stream_from_reader(reader);

            let fallback_id =
                extract_header(&parts.headers, "x-request-id").unwrap_or_else(generate_request_id);
            let encoder = Some(interop::AnthropicSseEncoder::new(fallback_id));

            let stream = stream::unfold(
                (
                    data_stream,
                    encoder,
                    VecDeque::<Result<Bytes, std::io::Error>>::new(),
                    false,
                ),
                |(mut data_stream, mut encoder, mut buffer, mut done)| async move {
                    loop {
                        if let Some(item) = buffer.pop_front() {
                            return Some((item, (data_stream, encoder, buffer, done)));
                        }
                        if done {
                            return None;
                        }

                        match data_stream.next().await {
                            Some(Ok(data)) => {
                                let Some(encoder_ref) = encoder.as_mut() else {
                                    done = true;
                                    continue;
                                };
                                match encoder_ref.push_openai_chunk(&data) {
                                    Ok(chunks) => {
                                        for chunk in chunks {
                                            buffer.push_back(Ok(chunk));
                                        }
                                    }
                                    Err(err) => {
                                        done = true;
                                        buffer.push_back(Err(std::io::Error::other(err)));
                                    }
                                }
                            }
                            Some(Err(err)) => {
                                done = true;
                                buffer.push_back(Err(std::io::Error::other(err.to_string())));
                            }
                            None => {
                                if let Some(encoder) = encoder.take() {
                                    for chunk in encoder.finish() {
                                        buffer.push_back(Ok(chunk));
                                    }
                                }
                                done = true;
                            }
                        }
                    }
                },
            );

            let mut response = axum::response::Response::new(Body::from_stream(stream));
            *response.status_mut() = status;
            *response.headers_mut() = parts.headers;
            return Ok(response);
        }
        #[cfg(not(feature = "streaming"))]
        {
            return Err(anthropic_error(
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                "streaming is not enabled",
            ));
        }
    }

    let (openai_parts, openai_body) = openai_resp.into_parts();
    let status = openai_parts.status;
    let request_id_header = openai_parts.headers.get("x-ditto-request-id").cloned();
    let bytes = to_bytes(openai_body, MAX_BODY_BYTES)
        .await
        .unwrap_or_default();

    if !status.is_success() {
        let message = serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|value| {
                value
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(Value::as_str)
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| String::from_utf8_lossy(&bytes).to_string());
        return Err(anthropic_error(status, "api_error", message));
    }

    let openai_json: serde_json::Value = serde_json::from_slice(&bytes).map_err(|err| {
        anthropic_error(
            StatusCode::BAD_GATEWAY,
            "api_error",
            format!("invalid backend JSON: {err}"),
        )
    })?;

    let anthropic_json =
        interop::openai_chat_completions_response_to_anthropic_message(&openai_json)
            .map_err(|err| anthropic_error(StatusCode::BAD_GATEWAY, "api_error", err))?;
    let out_bytes = serde_json::to_vec(&anthropic_json)
        .unwrap_or_else(|_| anthropic_json.to_string().into_bytes());

    let mut response = axum::response::Response::new(Body::from(out_bytes));
    *response.status_mut() = status;
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/json"),
    );
    if let Some(value) = request_id_header {
        response.headers_mut().insert("x-ditto-request-id", value);
    }
    Ok(response)
}

#[derive(Debug, Serialize)]
struct AnthropicCountTokensResponse {
    input_tokens: u32,
}

async fn handle_anthropic_count_tokens(
    State(state): State<GatewayHttpState>,
    req: axum::http::Request<Body>,
) -> Result<Json<AnthropicCountTokensResponse>, (StatusCode, Json<AnthropicErrorResponse>)> {
    const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

    let (parts, body) = req.into_parts();
    if gateway_uses_virtual_keys(&state) {
        let token = extract_virtual_key(&parts.headers).ok_or_else(|| {
            anthropic_error(
                StatusCode::UNAUTHORIZED,
                "authentication_error",
                "missing api key",
            )
        })?;
        let authorized = state
            .virtual_key_by_token(&token)
            .is_some_and(|key| key.enabled);
        if !authorized {
            return Err(anthropic_error(
                StatusCode::UNAUTHORIZED,
                "authentication_error",
                "unauthorized api key",
            ));
        }
    }

    let body = to_bytes(body, MAX_BODY_BYTES).await.map_err(|err| {
        anthropic_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            err.to_string(),
        )
    })?;
    #[cfg(feature = "gateway-tokenizer")]
    let request_json: serde_json::Value = serde_json::from_slice(&body).map_err(|err| {
        anthropic_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            format!("invalid JSON: {err}"),
        )
    })?;

    #[cfg(not(feature = "gateway-tokenizer"))]
    let _request_json: serde_json::Value = serde_json::from_slice(&body).map_err(|err| {
        anthropic_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            format!("invalid JSON: {err}"),
        )
    })?;

    #[cfg(feature = "gateway-tokenizer")]
    let openai_request = interop::anthropic_messages_request_to_openai_chat_completions(
        &request_json,
    )
    .map_err(|err| anthropic_error(StatusCode::BAD_REQUEST, "invalid_request_error", err))?;

    #[cfg(feature = "gateway-tokenizer")]
    let input_tokens = {
        let model = openai_request
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default();
        token_count::estimate_input_tokens("/v1/chat/completions", model, &openai_request)
            .unwrap_or_else(|| estimate_tokens_from_bytes(&body))
    };

    #[cfg(not(feature = "gateway-tokenizer"))]
    let input_tokens = estimate_tokens_from_bytes(&body);

    Ok(Json(AnthropicCountTokensResponse { input_tokens }))
}
// end inline: ../../http/anthropic.rs
// inlined from ../../http/google_genai.rs
async fn handle_google_genai(
    State(state): State<GatewayHttpState>,
    Path(path): Path<String>,
    req: axum::http::Request<Body>,
) -> Result<axum::response::Response, (StatusCode, Json<GoogleApiErrorResponse>)> {
    const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;

    let (model_raw, action) = path
        .rsplit_once(':')
        .ok_or_else(|| google_error(StatusCode::NOT_FOUND, "unsupported endpoint"))?;
    let model = model_raw.trim().trim_start_matches("models/").to_string();
    let stream_requested = action.starts_with("streamGenerateContent");

    #[cfg(not(feature = "streaming"))]
    if stream_requested {
        return Err(google_error(
            StatusCode::BAD_REQUEST,
            "streaming is not enabled",
        ));
    }

    let (parts, body) = req.into_parts();
    let body = to_bytes(body, MAX_BODY_BYTES)
        .await
        .map_err(|err| google_error(StatusCode::BAD_REQUEST, err.to_string()))?;
    let request_json: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|err| google_error(StatusCode::BAD_REQUEST, format!("invalid JSON: {err}")))?;

    let openai_request = interop::google_generate_content_request_to_openai_chat_completions(
        &model,
        &request_json,
        stream_requested,
    )
    .map_err(|err| google_error(StatusCode::BAD_REQUEST, err))?;

    let openai_bytes = serde_json::to_vec(&openai_request).map_err(|err| {
        google_error(
            StatusCode::BAD_REQUEST,
            format!("failed to serialize request: {err}"),
        )
    })?;

    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/json"),
    );
    headers.insert(
        "x-ditto-protocol",
        axum::http::HeaderValue::from_static("google"),
    );
    if stream_requested {
        headers.insert(
            axum::http::header::ACCEPT,
            axum::http::HeaderValue::from_static("text/event-stream"),
        );
    }
    if let Some(value) = parts.headers.get("authorization") {
        headers.insert("authorization", value.clone());
    }
    if !headers.contains_key("authorization") {
        let token = extract_header(&parts.headers, "x-ditto-virtual-key")
            .or_else(|| extract_header(&parts.headers, "x-goog-api-key"))
            .or_else(|| extract_query_param(&parts.uri, "key"))
            .or_else(|| extract_litellm_api_key(&parts.headers))
            .or_else(|| extract_bearer(&parts.headers));
        if let Some(token) = token.as_deref().and_then(synthesize_bearer_header) {
            headers.insert("authorization", token);
        }
    }
    if let Some(value) = parts.headers.get("x-request-id") {
        headers.insert("x-request-id", value.clone());
    }

    let mut openai_req = axum::http::Request::builder()
        .method(axum::http::Method::POST)
        .uri("/v1/chat/completions")
        .body(Body::from(openai_bytes))
        .map_err(|err| google_error(StatusCode::BAD_REQUEST, err.to_string()))?;
    *openai_req.headers_mut() = headers;

    let openai_resp = handle_openai_compat_proxy(
        State(state.clone()),
        Path("chat/completions".to_string()),
        openai_req,
    )
    .await
    .map_err(|(status, err)| google_error(status, err.0.error.message))?;

    let status = openai_resp.status();
    let content_type = openai_resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if content_type.starts_with("text/event-stream") {
        #[cfg(feature = "streaming")]
        {
            use tokio_util::io::StreamReader;

            let (mut parts, body) = openai_resp.into_parts();
            parts.headers.insert(
                axum::http::header::CONTENT_TYPE,
                axum::http::HeaderValue::from_static("text/event-stream"),
            );
            parts.headers.remove("content-length");

            let data_stream = body
                .into_data_stream()
                .map(|result| result.map_err(|err| std::io::Error::other(err.to_string())));
            let reader = StreamReader::new(data_stream);
            let reader = tokio::io::BufReader::new(reader);
            let data_stream = crate::utils::sse::sse_data_stream_from_reader(reader);

            let fallback_id =
                extract_header(&parts.headers, "x-request-id").unwrap_or_else(generate_request_id);
            let encoder = Some(interop::GoogleSseEncoder::new(fallback_id, false));

            let stream = stream::unfold(
                (
                    data_stream,
                    encoder,
                    VecDeque::<Result<Bytes, std::io::Error>>::new(),
                    false,
                ),
                |(mut data_stream, mut encoder, mut buffer, mut done)| async move {
                    loop {
                        if let Some(item) = buffer.pop_front() {
                            return Some((item, (data_stream, encoder, buffer, done)));
                        }
                        if done {
                            return None;
                        }

                        match data_stream.next().await {
                            Some(Ok(data)) => {
                                let Some(encoder_ref) = encoder.as_mut() else {
                                    done = true;
                                    continue;
                                };
                                match encoder_ref.push_openai_chunk(&data) {
                                    Ok(chunks) => {
                                        for chunk in chunks {
                                            buffer.push_back(Ok(chunk));
                                        }
                                    }
                                    Err(err) => {
                                        done = true;
                                        buffer.push_back(Err(std::io::Error::other(err)));
                                    }
                                }
                            }
                            Some(Err(err)) => {
                                done = true;
                                buffer.push_back(Err(std::io::Error::other(err.to_string())));
                            }
                            None => {
                                if let Some(encoder) = encoder.take() {
                                    buffer.push_back(Ok(encoder.finish()));
                                }
                                done = true;
                            }
                        }
                    }
                },
            );

            let mut response = axum::response::Response::new(Body::from_stream(stream));
            *response.status_mut() = status;
            *response.headers_mut() = parts.headers;
            return Ok(response);
        }
        #[cfg(not(feature = "streaming"))]
        {
            return Err(google_error(
                StatusCode::BAD_REQUEST,
                "streaming is not enabled",
            ));
        }
    }

    let bytes = to_bytes(openai_resp.into_body(), MAX_BODY_BYTES)
        .await
        .unwrap_or_default();

    if !status.is_success() {
        let message = String::from_utf8_lossy(&bytes).to_string();
        return Err(google_error(status, message));
    }

    let openai_json: serde_json::Value = serde_json::from_slice(&bytes).map_err(|err| {
        google_error(
            StatusCode::BAD_GATEWAY,
            format!("invalid backend JSON: {err}"),
        )
    })?;
    let google_json =
        interop::openai_chat_completions_response_to_google_generate_content(&openai_json)
            .map_err(|err| google_error(StatusCode::BAD_GATEWAY, err))?;
    let out_bytes =
        serde_json::to_vec(&google_json).unwrap_or_else(|_| google_json.to_string().into_bytes());

    let mut response = axum::response::Response::new(Body::from(out_bytes));
    *response.status_mut() = status;
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/json"),
    );
    Ok(response)
}

async fn handle_fallback(
    State(state): State<GatewayHttpState>,
    req: axum::http::Request<Body>,
) -> axum::response::Response {
    match req.uri().path() {
        "/v1internal:generateContent" => {
            match handle_cloudcode_generate_content_inner(state.clone(), req, false).await {
                Ok(response) => response,
                Err(err) => err.into_response(),
            }
        }
        "/v1internal:streamGenerateContent" => {
            match handle_cloudcode_generate_content_inner(state.clone(), req, true).await {
                Ok(response) => response,
                Err(err) => err.into_response(),
            }
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn handle_cloudcode_generate_content_inner(
    state: GatewayHttpState,
    req: axum::http::Request<Body>,
    stream_requested: bool,
) -> Result<axum::response::Response, (StatusCode, Json<GoogleApiErrorResponse>)> {
    const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;

    #[cfg(not(feature = "streaming"))]
    if stream_requested {
        return Err(google_error(
            StatusCode::BAD_REQUEST,
            "streaming is not enabled",
        ));
    }

    let (parts, body) = req.into_parts();
    let body = to_bytes(body, MAX_BODY_BYTES)
        .await
        .map_err(|err| google_error(StatusCode::BAD_REQUEST, err.to_string()))?;
    let request_json: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|err| google_error(StatusCode::BAD_REQUEST, format!("invalid JSON: {err}")))?;
    let obj = request_json.as_object().ok_or_else(|| {
        google_error(
            StatusCode::BAD_REQUEST,
            "request body must be a JSON object",
        )
    })?;
    let model = obj
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| google_error(StatusCode::BAD_REQUEST, "missing field `model`"))?
        .to_string();
    let inner_request = obj
        .get("request")
        .ok_or_else(|| google_error(StatusCode::BAD_REQUEST, "missing field `request`"))?;

    let openai_request = interop::google_generate_content_request_to_openai_chat_completions(
        &model,
        inner_request,
        stream_requested,
    )
    .map_err(|err| google_error(StatusCode::BAD_REQUEST, err))?;

    let openai_bytes = serde_json::to_vec(&openai_request).map_err(|err| {
        google_error(
            StatusCode::BAD_REQUEST,
            format!("failed to serialize request: {err}"),
        )
    })?;

    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/json"),
    );
    headers.insert(
        "x-ditto-protocol",
        axum::http::HeaderValue::from_static("google"),
    );
    if stream_requested {
        headers.insert(
            axum::http::header::ACCEPT,
            axum::http::HeaderValue::from_static("text/event-stream"),
        );
    }
    if let Some(value) = parts.headers.get("authorization") {
        headers.insert("authorization", value.clone());
    }
    if !headers.contains_key("authorization") {
        let token = extract_header(&parts.headers, "x-ditto-virtual-key")
            .or_else(|| extract_header(&parts.headers, "x-goog-api-key"))
            .or_else(|| extract_query_param(&parts.uri, "key"))
            .or_else(|| extract_litellm_api_key(&parts.headers))
            .or_else(|| extract_bearer(&parts.headers));
        if let Some(token) = token.as_deref().and_then(synthesize_bearer_header) {
            headers.insert("authorization", token);
        }
    }
    if let Some(value) = parts.headers.get("x-request-id") {
        headers.insert("x-request-id", value.clone());
    }

    let mut openai_req = axum::http::Request::builder()
        .method(axum::http::Method::POST)
        .uri("/v1/chat/completions")
        .body(Body::from(openai_bytes))
        .map_err(|err| google_error(StatusCode::BAD_REQUEST, err.to_string()))?;
    *openai_req.headers_mut() = headers;

    let openai_resp = handle_openai_compat_proxy(
        State(state.clone()),
        Path("chat/completions".to_string()),
        openai_req,
    )
    .await
    .map_err(|(status, err)| google_error(status, err.0.error.message))?;

    let status = openai_resp.status();
    let content_type = openai_resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if content_type.starts_with("text/event-stream") {
        #[cfg(feature = "streaming")]
        {
            use tokio_util::io::StreamReader;

            let (mut parts, body) = openai_resp.into_parts();
            parts.headers.insert(
                axum::http::header::CONTENT_TYPE,
                axum::http::HeaderValue::from_static("text/event-stream"),
            );
            parts.headers.remove("content-length");

            let data_stream = body
                .into_data_stream()
                .map(|result| result.map_err(|err| std::io::Error::other(err.to_string())));
            let reader = StreamReader::new(data_stream);
            let reader = tokio::io::BufReader::new(reader);
            let data_stream = crate::utils::sse::sse_data_stream_from_reader(reader);

            let fallback_id =
                extract_header(&parts.headers, "x-request-id").unwrap_or_else(generate_request_id);
            let encoder = Some(interop::GoogleSseEncoder::new(fallback_id, true));

            let stream = stream::unfold(
                (
                    data_stream,
                    encoder,
                    VecDeque::<Result<Bytes, std::io::Error>>::new(),
                    false,
                ),
                |(mut data_stream, mut encoder, mut buffer, mut done)| async move {
                    loop {
                        if let Some(item) = buffer.pop_front() {
                            return Some((item, (data_stream, encoder, buffer, done)));
                        }
                        if done {
                            return None;
                        }

                        match data_stream.next().await {
                            Some(Ok(data)) => {
                                let Some(encoder_ref) = encoder.as_mut() else {
                                    done = true;
                                    continue;
                                };
                                match encoder_ref.push_openai_chunk(&data) {
                                    Ok(chunks) => {
                                        for chunk in chunks {
                                            buffer.push_back(Ok(chunk));
                                        }
                                    }
                                    Err(err) => {
                                        done = true;
                                        buffer.push_back(Err(std::io::Error::other(err)));
                                    }
                                }
                            }
                            Some(Err(err)) => {
                                done = true;
                                buffer.push_back(Err(std::io::Error::other(err.to_string())));
                            }
                            None => {
                                if let Some(encoder) = encoder.take() {
                                    buffer.push_back(Ok(encoder.finish()));
                                }
                                done = true;
                            }
                        }
                    }
                },
            );

            let mut response = axum::response::Response::new(Body::from_stream(stream));
            *response.status_mut() = status;
            *response.headers_mut() = parts.headers;
            return Ok(response);
        }
        #[cfg(not(feature = "streaming"))]
        {
            return Err(google_error(
                StatusCode::BAD_REQUEST,
                "streaming is not enabled",
            ));
        }
    }

    let bytes = to_bytes(openai_resp.into_body(), MAX_BODY_BYTES)
        .await
        .unwrap_or_default();

    if !status.is_success() {
        let message = String::from_utf8_lossy(&bytes).to_string();
        return Err(google_error(status, message));
    }

    let openai_json: serde_json::Value = serde_json::from_slice(&bytes).map_err(|err| {
        google_error(
            StatusCode::BAD_GATEWAY,
            format!("invalid backend JSON: {err}"),
        )
    })?;
    let cloudcode_json =
        interop::openai_chat_completions_response_to_cloudcode_generate_content(&openai_json)
            .map_err(|err| google_error(StatusCode::BAD_GATEWAY, err))?;
    let out_bytes = serde_json::to_vec(&cloudcode_json)
        .unwrap_or_else(|_| cloudcode_json.to_string().into_bytes());

    let mut response = axum::response::Response::new(Body::from(out_bytes));
    *response.status_mut() = status;
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/json"),
    );
    Ok(response)
}
// end inline: ../../http/google_genai.rs
// inlined from ../../http/openai_compat_proxy.rs
// This file is intentionally split to keep each staged Rust file under the pre-commit size limit.
// inlined from openai_compat_proxy/preamble.rs
#[derive(Clone, Copy)]
struct ProxyAttemptParams<'a> {
    state: &'a GatewayHttpState,
    parts: &'a axum::http::request::Parts,
    body: &'a Bytes,
    parsed_json: &'a Option<serde_json::Value>,
    model: &'a Option<String>,
    service_tier: &'a Option<String>,
    request_id: &'a str,
    path_and_query: &'a str,
    now_epoch_seconds: u64,
    charge_tokens: u32,
    stream_requested: bool,
    strip_authorization: bool,
    use_persistent_budget: bool,
    virtual_key_id: &'a Option<String>,
    budget: &'a Option<super::BudgetConfig>,
    tenant_budget_scope: &'a Option<(String, super::BudgetConfig)>,
    project_budget_scope: &'a Option<(String, super::BudgetConfig)>,
    user_budget_scope: &'a Option<(String, super::BudgetConfig)>,
    charge_cost_usd_micros: Option<u64>,
    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    token_budget_reservation_ids: &'a [String],
    cost_budget_reserved: bool,
    #[cfg(all(
        feature = "gateway-costing",
        any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        ),
    ))]
    cost_budget_reservation_ids: &'a [String],
    max_attempts: usize,
    #[cfg(feature = "gateway-routing-advanced")]
    retry_config: &'a super::ProxyRetryConfig,
    #[cfg(feature = "gateway-proxy-cache")]
    proxy_cache_key: &'a Option<String>,
    #[cfg(feature = "gateway-proxy-cache")]
    proxy_cache_metadata: &'a Option<ProxyCacheEntryMetadata>,
    #[cfg(feature = "gateway-metrics-prometheus")]
    metrics_path: &'a str,
    #[cfg(feature = "gateway-metrics-prometheus")]
    metrics_timer_start: Instant,
}

enum BackendAttemptOutcome {
    Response(axum::response::Response),
    Continue(Option<(StatusCode, Json<OpenAiErrorResponse>)>),
    Stop((StatusCode, Json<OpenAiErrorResponse>)),
}

// inlined from multipart_schema.rs
fn validate_openai_multipart_request_schema(
    path_and_query: &str,
    content_type: Option<&str>,
    body: &Bytes,
) -> Option<String> {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query)
        .trim_end_matches('/');

    let endpoint = if path == "/v1/audio/transcriptions" {
        "audio/transcriptions"
    } else if path == "/v1/audio/translations" {
        "audio/translations"
    } else if path == "/v1/files" {
        "files"
    } else {
        return None;
    };

    let Some(content_type) = content_type else {
        return Some(format!("{endpoint} request missing content-type"));
    };
    if !content_type
        .to_ascii_lowercase()
        .starts_with("multipart/form-data")
    {
        return Some(format!("{endpoint} request must be multipart/form-data"));
    }

    let parts = match super::multipart::parse_multipart_form(content_type, body) {
        Ok(parts) => parts,
        Err(err) => return Some(err),
    };

    if endpoint.starts_with("audio/") {
        let mut has_file = false;
        let mut has_model = false;
        for part in parts {
            match part.name.as_str() {
                "file" => has_file = true,
                "model" if part.filename.is_none() => {
                    let value = String::from_utf8_lossy(part.data.as_ref())
                        .trim()
                        .to_string();
                    if !value.is_empty() {
                        has_model = true;
                    }
                }
                _ => {}
            }
        }

        if !has_file {
            return Some(format!("{endpoint} request missing file"));
        }
        if !has_model {
            return Some(format!("{endpoint} request missing model"));
        }
        return None;
    }

    let mut has_file = false;
    let mut has_purpose = false;
    for part in parts {
        match part.name.as_str() {
            "file" => has_file = true,
            "purpose" if part.filename.is_none() => {
                let value = String::from_utf8_lossy(part.data.as_ref())
                    .trim()
                    .to_string();
                if !value.is_empty() {
                    has_purpose = true;
                }
            }
            _ => {}
        }
    }

    if !has_file {
        return Some("files request missing file".to_string());
    }
    if !has_purpose {
        return Some("files request missing purpose".to_string());
    }
    None
}
// end inline: multipart_schema.rs
// end inline: openai_compat_proxy/preamble.rs
// inlined from openai_compat_proxy/cost_budget.rs
#[cfg(feature = "gateway-costing")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CostBudgetEndpointPolicy {
    TokenBased,
    Free,
    Unsupported,
}

#[cfg(feature = "gateway-costing")]
fn cost_budget_endpoint_policy(
    method: &axum::http::Method,
    path_and_query: &str,
) -> CostBudgetEndpointPolicy {
    if *method != axum::http::Method::POST {
        return CostBudgetEndpointPolicy::Free;
    }

    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query)
        .trim_end_matches('/');

    if path == "/v1/chat/completions"
        || path == "/v1/completions"
        || path == "/v1/embeddings"
        || path == "/v1/moderations"
        || path == "/v1/rerank"
        || path.starts_with("/v1/responses")
    {
        return CostBudgetEndpointPolicy::TokenBased;
    }

    if path == "/v1/files" {
        return CostBudgetEndpointPolicy::Free;
    }

    CostBudgetEndpointPolicy::Unsupported
}
// end inline: openai_compat_proxy/cost_budget.rs
// inlined from openai_compat_proxy/costing.rs
#[cfg(feature = "gateway-costing")]
fn estimate_charge_cost_usd_micros(
    state: &GatewayHttpState,
    request_model: Option<&str>,
    input_tokens_estimate: u32,
    max_output_tokens: u32,
    service_tier: Option<&str>,
    backends: &[String],
) -> Option<u64> {
    let request_model = request_model?;
    let pricing = state.proxy.pricing.as_ref()?;

    let mut cost = pricing.estimate_cost_usd_micros_for_service_tier(
        request_model,
        input_tokens_estimate,
        max_output_tokens,
        service_tier,
    );

    for backend_name in backends {
        if !state.backends.proxy_backends.contains_key(backend_name) {
            continue;
        }

        let mapped_model = state.mapped_backend_model(backend_name, request_model);

        if let Some(mapped_model) = mapped_model.as_deref() {
            cost = max_option_u64(
                cost,
                pricing.estimate_cost_usd_micros_for_service_tier(
                    mapped_model,
                    input_tokens_estimate,
                    max_output_tokens,
                    service_tier,
                ),
            );
        }
    }

    cost
}
// end inline: openai_compat_proxy/costing.rs
// inlined from openai_compat_proxy/rate_limit.rs
#[cfg(feature = "gateway-store-redis")]
fn normalize_rate_limit_route(path_and_query: &str) -> String {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    let path = path.strip_suffix('/').unwrap_or(path);

    match path {
        "/v1/chat/completions"
        | "/v1/completions"
        | "/v1/embeddings"
        | "/v1/moderations"
        | "/v1/images/generations"
        | "/v1/audio/transcriptions"
        | "/v1/audio/translations"
        | "/v1/audio/speech"
        | "/v1/files"
        | "/v1/rerank"
        | "/v1/batches"
        | "/v1/models"
        | "/v1/responses"
        | "/v1/responses/compact" => path.to_string(),
        _ => {
            if path.starts_with("/v1/models/") {
                return "/v1/models/*".to_string();
            }
            if path.starts_with("/v1/batches/") {
                if path.ends_with("/cancel") {
                    return "/v1/batches/*/cancel".to_string();
                }
                return "/v1/batches/*".to_string();
            }
            if path.starts_with("/v1/files/") {
                if path.ends_with("/content") {
                    return "/v1/files/*/content".to_string();
                }
                return "/v1/files/*".to_string();
            }
            if path.starts_with("/v1/responses/") {
                return "/v1/responses/*".to_string();
            }

            "/v1/*".to_string()
        }
    }
}
// end inline: openai_compat_proxy/rate_limit.rs
// inlined from openai_compat_proxy/resolve_gateway_context.rs
#[derive(Debug, Clone)]
struct ResolvedGatewayContext {
    virtual_key_id: Option<String>,
    limits: Option<super::LimitsConfig>,
    budget: Option<super::BudgetConfig>,
    tenant_budget_scope: Option<(String, super::BudgetConfig)>,
    project_budget_scope: Option<(String, super::BudgetConfig)>,
    user_budget_scope: Option<(String, super::BudgetConfig)>,
    tenant_limits_scope: Option<(String, super::LimitsConfig)>,
    project_limits_scope: Option<(String, super::LimitsConfig)>,
    user_limits_scope: Option<(String, super::LimitsConfig)>,
    backend_candidates: Vec<String>,
    strip_authorization: bool,
    charge_cost_usd_micros: Option<u64>,
}

struct ResolveOpenAiCompatProxyGatewayContextRequest<'a> {
    state: &'a GatewayHttpState,
    parts: &'a axum::http::request::Parts,
    body: &'a Bytes,
    parsed_json: &'a Option<serde_json::Value>,
    request_id: &'a str,
    path_and_query: &'a str,
    model: &'a Option<String>,
    service_tier: &'a Option<String>,
    input_tokens_estimate: u32,
    max_output_tokens: u32,
    charge_tokens: u32,
    minute: u64,
    use_redis_budget: bool,
    use_persistent_budget: bool,
    #[cfg(feature = "gateway-metrics-prometheus")]
    metrics_path: &'a str,
    #[cfg(feature = "gateway-metrics-prometheus")]
    metrics_timer_start: Instant,
}

#[derive(Debug, Clone)]
struct OpenAiCompatProxyGatewayPreamble {
    strip_authorization: bool,
    key: Option<super::VirtualKeyConfig>,
}

async fn resolve_openai_compat_proxy_gateway_preamble(
    state: &GatewayHttpState,
    parts: &axum::http::request::Parts,
) -> Result<OpenAiCompatProxyGatewayPreamble, (StatusCode, Json<OpenAiErrorResponse>)> {
    state.record_request();

    let strip_authorization = state.uses_virtual_keys();
    let key = if strip_authorization {
        let token = extract_virtual_key(&parts.headers).ok_or_else(|| {
            openai_error(
                StatusCode::UNAUTHORIZED,
                "authentication_error",
                Some("invalid_api_key"),
                "missing virtual key",
            )
        })?;
        let key = state.virtual_key_by_token(&token).ok_or_else(|| {
            openai_error(
                StatusCode::UNAUTHORIZED,
                "authentication_error",
                Some("invalid_api_key"),
                "unauthorized virtual key",
            )
        })?;
        if !key.enabled {
            return Err(openai_error(
                StatusCode::UNAUTHORIZED,
                "authentication_error",
                Some("invalid_api_key"),
                "virtual key disabled",
            ));
        }
        Some(key)
    } else {
        None
    };

    Ok(OpenAiCompatProxyGatewayPreamble {
        strip_authorization,
        key,
    })
}

async fn resolve_openai_compat_proxy_gateway_context(
    request: ResolveOpenAiCompatProxyGatewayContextRequest<'_>,
) -> Result<ResolvedGatewayContext, (StatusCode, Json<OpenAiErrorResponse>)> {
    let ResolveOpenAiCompatProxyGatewayContextRequest {
        state,
        parts,
        body,
        parsed_json,
        request_id,
        path_and_query,
        model,
        service_tier,
        input_tokens_estimate,
        max_output_tokens,
        charge_tokens,
        minute,
        use_redis_budget,
        use_persistent_budget,
        #[cfg(feature = "gateway-metrics-prometheus")]
        metrics_path,
        #[cfg(feature = "gateway-metrics-prometheus")]
        metrics_timer_start,
    } = request;
    #[cfg(not(feature = "gateway-costing"))]
    let _ = (&service_tier, max_output_tokens);

    let gateway_preamble = resolve_openai_compat_proxy_gateway_preamble(state, parts).await?;
    let strip_authorization = gateway_preamble.strip_authorization;
    let key = gateway_preamble.key;

    let (
        virtual_key_id,
        limits,
        budget,
        tenant_budget_scope,
        project_budget_scope,
        user_budget_scope,
        tenant_limits_scope,
        project_limits_scope,
        user_limits_scope,
        backend_candidates,
        charge_cost_usd_micros,
    ) = {
        if let Some(key) = key.as_ref() {
            let virtual_key_id = Some(key.id.clone());
            let limits = Some(key.limits.clone());

            let tenant_scope = key
                .tenant_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(|id| format!("tenant:{id}"));
            let tenant_budget_scope = tenant_scope.as_ref().and_then(|scope| {
                key.tenant_budget
                    .as_ref()
                    .map(|budget| (scope.clone(), budget.clone()))
            });
            let tenant_limits_scope = tenant_scope.as_ref().and_then(|scope| {
                key.tenant_limits
                    .as_ref()
                    .map(|limits| (scope.clone(), limits.clone()))
            });

            let project_scope = key
                .project_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(|id| format!("project:{id}"));
            let project_budget_scope = project_scope.as_ref().and_then(|scope| {
                key.project_budget
                    .as_ref()
                    .map(|budget| (scope.clone(), budget.clone()))
            });
            let project_limits_scope = project_scope.as_ref().and_then(|scope| {
                key.project_limits
                    .as_ref()
                    .map(|limits| (scope.clone(), limits.clone()))
            });

            let user_scope = key
                .user_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(|id| format!("user:{id}"));
            let user_budget_scope = user_scope.as_ref().and_then(|scope| {
                key.user_budget
                    .as_ref()
                    .map(|budget| (scope.clone(), budget.clone()))
            });
            let user_limits_scope = user_scope.as_ref().and_then(|scope| {
                key.user_limits
                    .as_ref()
                    .map(|limits| (scope.clone(), limits.clone()))
            });

            #[cfg(feature = "gateway-costing")]
            let (has_cost_budget, cost_budget_policy) = {
                let has_cost_budget = key.budget.total_usd_micros.is_some()
                    || tenant_budget_scope
                        .as_ref()
                        .is_some_and(|(_, budget)| budget.total_usd_micros.is_some())
                    || project_budget_scope
                        .as_ref()
                        .is_some_and(|(_, budget)| budget.total_usd_micros.is_some())
                    || user_budget_scope
                        .as_ref()
                        .is_some_and(|(_, budget)| budget.total_usd_micros.is_some());

                let cost_budget_policy = if has_cost_budget {
                    Some(cost_budget_endpoint_policy(&parts.method, path_and_query))
                } else {
                    None
                };

                (has_cost_budget, cost_budget_policy)
            };

            #[cfg(feature = "gateway-costing")]
            if has_cost_budget
                && matches!(
                    cost_budget_policy,
                    Some(CostBudgetEndpointPolicy::Unsupported)
                )
            {
                let path = path_and_query
                    .split_once('?')
                    .map(|(path, _)| path)
                    .unwrap_or(path_and_query)
                    .trim_end_matches('/');
                return Err(openai_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request_error",
                    Some("cost_budget_unsupported_endpoint"),
                    format!(
                        "cost budgets are token-based and do not support {path} (disable total_usd_micros or use token budgets)"
                    ),
                ));
            }

            if !use_redis_budget {
                if let Err(err) =
                    state.check_and_consume_rate_limit(&key.id, &key.limits, charge_tokens, minute)
                {
                    state.record_rate_limited();
                    let mapped = map_openai_gateway_error(err);
                    #[cfg(feature = "gateway-metrics-prometheus")]
                    if let Some(metrics) = state.proxy.metrics.as_ref() {
                        let duration = metrics_timer_start.elapsed();
                        let status = mapped.0.as_u16();
                        let mut metrics = metrics.lock().await;
                        metrics.record_proxy_request(Some(&key.id), model.as_deref(), metrics_path);
                        metrics.record_proxy_rate_limited(
                            Some(&key.id),
                            model.as_deref(),
                            metrics_path,
                        );
                        metrics.record_proxy_response_status_by_path(metrics_path, status);
                        if let Some(model) = model.as_deref() {
                            metrics.record_proxy_response_status_by_model(model, status);
                            metrics.observe_proxy_request_duration_by_model(model, duration);
                        }
                        metrics.observe_proxy_request_duration(metrics_path, duration);
                    }
                    return Err(mapped);
                }
                if let Some((scope, limits)) = tenant_limits_scope.as_ref() {
                    if let Err(err) =
                        state.check_and_consume_rate_limit(scope, limits, charge_tokens, minute)
                    {
                        state.record_rate_limited();
                        let mapped = map_openai_gateway_error(err);
                        #[cfg(feature = "gateway-metrics-prometheus")]
                        if let Some(metrics) = state.proxy.metrics.as_ref() {
                            let duration = metrics_timer_start.elapsed();
                            let status = mapped.0.as_u16();
                            let mut metrics = metrics.lock().await;
                            metrics.record_proxy_request(
                                Some(&key.id),
                                model.as_deref(),
                                metrics_path,
                            );
                            metrics.record_proxy_rate_limited(
                                Some(&key.id),
                                model.as_deref(),
                                metrics_path,
                            );
                            metrics.record_proxy_response_status_by_path(metrics_path, status);
                            if let Some(model) = model.as_deref() {
                                metrics.record_proxy_response_status_by_model(model, status);
                                metrics.observe_proxy_request_duration_by_model(model, duration);
                            }
                            metrics.observe_proxy_request_duration(metrics_path, duration);
                        }
                        return Err(mapped);
                    }
                }
                if let Some((scope, limits)) = project_limits_scope.as_ref() {
                    if let Err(err) =
                        state.check_and_consume_rate_limit(scope, limits, charge_tokens, minute)
                    {
                        state.record_rate_limited();
                        let mapped = map_openai_gateway_error(err);
                        #[cfg(feature = "gateway-metrics-prometheus")]
                        if let Some(metrics) = state.proxy.metrics.as_ref() {
                            let duration = metrics_timer_start.elapsed();
                            let status = mapped.0.as_u16();
                            let mut metrics = metrics.lock().await;
                            metrics.record_proxy_request(
                                Some(&key.id),
                                model.as_deref(),
                                metrics_path,
                            );
                            metrics.record_proxy_rate_limited(
                                Some(&key.id),
                                model.as_deref(),
                                metrics_path,
                            );
                            metrics.record_proxy_response_status_by_path(metrics_path, status);
                            if let Some(model) = model.as_deref() {
                                metrics.record_proxy_response_status_by_model(model, status);
                                metrics.observe_proxy_request_duration_by_model(model, duration);
                            }
                            metrics.observe_proxy_request_duration(metrics_path, duration);
                        }
                        return Err(mapped);
                    }
                }
                if let Some((scope, limits)) = user_limits_scope.as_ref() {
                    if let Err(err) =
                        state.check_and_consume_rate_limit(scope, limits, charge_tokens, minute)
                    {
                        state.record_rate_limited();
                        let mapped = map_openai_gateway_error(err);
                        #[cfg(feature = "gateway-metrics-prometheus")]
                        if let Some(metrics) = state.proxy.metrics.as_ref() {
                            let duration = metrics_timer_start.elapsed();
                            let status = mapped.0.as_u16();
                            let mut metrics = metrics.lock().await;
                            metrics.record_proxy_request(
                                Some(&key.id),
                                model.as_deref(),
                                metrics_path,
                            );
                            metrics.record_proxy_rate_limited(
                                Some(&key.id),
                                model.as_deref(),
                                metrics_path,
                            );
                            metrics.record_proxy_response_status_by_path(metrics_path, status);
                            if let Some(model) = model.as_deref() {
                                metrics.record_proxy_response_status_by_model(model, status);
                                metrics.observe_proxy_request_duration_by_model(model, duration);
                            }
                            metrics.observe_proxy_request_duration(metrics_path, duration);
                        }
                        return Err(mapped);
                    }
                }
            }

            let guardrails = state.guardrails_for_model(model.as_deref(), key);

            if let Some(model_id) = model.as_deref() {
                if let Some(reason) = guardrails.check_model(model_id) {
                    state.record_guardrail_blocked();
                    let err = openai_error(
                        StatusCode::FORBIDDEN,
                        "policy_error",
                        Some("guardrail_rejected"),
                        reason,
                    );
                    #[cfg(feature = "gateway-metrics-prometheus")]
                    if let Some(metrics) = state.proxy.metrics.as_ref() {
                        let duration = metrics_timer_start.elapsed();
                        let status = err.0.as_u16();
                        let mut metrics = metrics.lock().await;
                        metrics.record_proxy_request(Some(&key.id), model.as_deref(), metrics_path);
                        metrics.record_proxy_guardrail_blocked(
                            Some(&key.id),
                            model.as_deref(),
                            metrics_path,
                        );
                        metrics.record_proxy_response_status_by_path(metrics_path, status);
                        if let Some(model) = model.as_deref() {
                            metrics.record_proxy_response_status_by_model(model, status);
                            metrics.observe_proxy_request_duration_by_model(model, duration);
                        }
                        metrics.observe_proxy_request_duration(metrics_path, duration);
                    }
                    return Err(err);
                }
            }

            if let Some(limit) = guardrails.max_input_tokens {
                if input_tokens_estimate > limit {
                    state.record_guardrail_blocked();
                    let err = openai_error(
                        StatusCode::FORBIDDEN,
                        "policy_error",
                        Some("guardrail_rejected"),
                        format!("input_tokens>{limit}"),
                    );
                    #[cfg(feature = "gateway-metrics-prometheus")]
                    if let Some(metrics) = state.proxy.metrics.as_ref() {
                        let duration = metrics_timer_start.elapsed();
                        let status = err.0.as_u16();
                        let mut metrics = metrics.lock().await;
                        metrics.record_proxy_request(Some(&key.id), model.as_deref(), metrics_path);
                        metrics.record_proxy_guardrail_blocked(
                            Some(&key.id),
                            model.as_deref(),
                            metrics_path,
                        );
                        metrics.record_proxy_response_status_by_path(metrics_path, status);
                        if let Some(model) = model.as_deref() {
                            metrics.record_proxy_response_status_by_model(model, status);
                            metrics.observe_proxy_request_duration_by_model(model, duration);
                        }
                        metrics.observe_proxy_request_duration(metrics_path, duration);
                    }
                    return Err(err);
                }
            }

            if guardrails.validate_schema {
                let reason = if let Some(body_json) = parsed_json.as_ref() {
                    validate_openai_request_schema(path_and_query, body_json)
                } else if parts.method == axum::http::Method::POST {
                    validate_openai_multipart_request_schema(
                        path_and_query,
                        parts
                            .headers
                            .get("content-type")
                            .and_then(|value| value.to_str().ok()),
                        body,
                    )
                } else {
                    None
                };
                if let Some(reason) = reason {
                    state.record_guardrail_blocked();
                    let err = openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        reason,
                    );
                    #[cfg(feature = "gateway-metrics-prometheus")]
                    if let Some(metrics) = state.proxy.metrics.as_ref() {
                        let duration = metrics_timer_start.elapsed();
                        let status = err.0.as_u16();
                        let mut metrics = metrics.lock().await;
                        metrics.record_proxy_request(Some(&key.id), model.as_deref(), metrics_path);
                        metrics.record_proxy_guardrail_blocked(
                            Some(&key.id),
                            model.as_deref(),
                            metrics_path,
                        );
                        metrics.record_proxy_response_status_by_path(metrics_path, status);
                        if let Some(model) = model.as_deref() {
                            metrics.record_proxy_response_status_by_model(model, status);
                            metrics.observe_proxy_request_duration_by_model(model, duration);
                        }
                        metrics.observe_proxy_request_duration(metrics_path, duration);
                    }
                    return Err(err);
                }
            }

            if guardrails.has_text_filters() {
                if let Ok(text) = std::str::from_utf8(body) {
                    if let Some(reason) = guardrails.check_text(text) {
                        state.record_guardrail_blocked();
                        let err = openai_error(
                            StatusCode::FORBIDDEN,
                            "policy_error",
                            Some("guardrail_rejected"),
                            reason,
                        );
                        #[cfg(feature = "gateway-metrics-prometheus")]
                        if let Some(metrics) = state.proxy.metrics.as_ref() {
                            let duration = metrics_timer_start.elapsed();
                            let status = err.0.as_u16();
                            let mut metrics = metrics.lock().await;
                            metrics.record_proxy_request(
                                Some(&key.id),
                                model.as_deref(),
                                metrics_path,
                            );
                            metrics.record_proxy_guardrail_blocked(
                                Some(&key.id),
                                model.as_deref(),
                                metrics_path,
                            );
                            metrics.record_proxy_response_status_by_path(metrics_path, status);
                            if let Some(model) = model.as_deref() {
                                metrics.record_proxy_response_status_by_model(model, status);
                                metrics.observe_proxy_request_duration_by_model(model, duration);
                            }
                            metrics.observe_proxy_request_duration(metrics_path, duration);
                        }
                        return Err(err);
                    }
                }
            }

            if !use_persistent_budget {
                if let Err(err) =
                    state.can_spend_budget_tokens(&key.id, &key.budget, u64::from(charge_tokens))
                {
                    state.record_budget_exceeded();
                    let mapped = map_openai_gateway_error(err);
                    #[cfg(feature = "gateway-metrics-prometheus")]
                    if let Some(metrics) = state.proxy.metrics.as_ref() {
                        let duration = metrics_timer_start.elapsed();
                        let status = mapped.0.as_u16();
                        let mut metrics = metrics.lock().await;
                        metrics.record_proxy_request(Some(&key.id), model.as_deref(), metrics_path);
                        metrics.record_proxy_budget_exceeded(
                            Some(&key.id),
                            model.as_deref(),
                            metrics_path,
                        );
                        metrics.record_proxy_response_status_by_path(metrics_path, status);
                        if let Some(model) = model.as_deref() {
                            metrics.record_proxy_response_status_by_model(model, status);
                            metrics.observe_proxy_request_duration_by_model(model, duration);
                        }
                        metrics.observe_proxy_request_duration(metrics_path, duration);
                    }
                    return Err(mapped);
                }

                if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                    if let Err(err) =
                        state.can_spend_budget_tokens(scope, budget, u64::from(charge_tokens))
                    {
                        state.record_budget_exceeded();
                        let mapped = map_openai_gateway_error(err);
                        #[cfg(feature = "gateway-metrics-prometheus")]
                        if let Some(metrics) = state.proxy.metrics.as_ref() {
                            let duration = metrics_timer_start.elapsed();
                            let status = mapped.0.as_u16();
                            let mut metrics = metrics.lock().await;
                            metrics.record_proxy_request(
                                Some(&key.id),
                                model.as_deref(),
                                metrics_path,
                            );
                            metrics.record_proxy_budget_exceeded(
                                Some(&key.id),
                                model.as_deref(),
                                metrics_path,
                            );
                            metrics.record_proxy_response_status_by_path(metrics_path, status);
                            if let Some(model) = model.as_deref() {
                                metrics.record_proxy_response_status_by_model(model, status);
                                metrics.observe_proxy_request_duration_by_model(model, duration);
                            }
                            metrics.observe_proxy_request_duration(metrics_path, duration);
                        }
                        return Err(mapped);
                    }
                }

                if let Some((scope, budget)) = project_budget_scope.as_ref() {
                    if let Err(err) =
                        state.can_spend_budget_tokens(scope, budget, u64::from(charge_tokens))
                    {
                        state.record_budget_exceeded();
                        let mapped = map_openai_gateway_error(err);
                        #[cfg(feature = "gateway-metrics-prometheus")]
                        if let Some(metrics) = state.proxy.metrics.as_ref() {
                            let duration = metrics_timer_start.elapsed();
                            let status = mapped.0.as_u16();
                            let mut metrics = metrics.lock().await;
                            metrics.record_proxy_request(
                                Some(&key.id),
                                model.as_deref(),
                                metrics_path,
                            );
                            metrics.record_proxy_budget_exceeded(
                                Some(&key.id),
                                model.as_deref(),
                                metrics_path,
                            );
                            metrics.record_proxy_response_status_by_path(metrics_path, status);
                            if let Some(model) = model.as_deref() {
                                metrics.record_proxy_response_status_by_model(model, status);
                                metrics.observe_proxy_request_duration_by_model(model, duration);
                            }
                            metrics.observe_proxy_request_duration(metrics_path, duration);
                        }
                        return Err(mapped);
                    }
                }

                if let Some((scope, budget)) = user_budget_scope.as_ref() {
                    if let Err(err) =
                        state.can_spend_budget_tokens(scope, budget, u64::from(charge_tokens))
                    {
                        state.record_budget_exceeded();
                        let mapped = map_openai_gateway_error(err);
                        #[cfg(feature = "gateway-metrics-prometheus")]
                        if let Some(metrics) = state.proxy.metrics.as_ref() {
                            let duration = metrics_timer_start.elapsed();
                            let status = mapped.0.as_u16();
                            let mut metrics = metrics.lock().await;
                            metrics.record_proxy_request(
                                Some(&key.id),
                                model.as_deref(),
                                metrics_path,
                            );
                            metrics.record_proxy_budget_exceeded(
                                Some(&key.id),
                                model.as_deref(),
                                metrics_path,
                            );
                            metrics.record_proxy_response_status_by_path(metrics_path, status);
                            if let Some(model) = model.as_deref() {
                                metrics.record_proxy_response_status_by_model(model, status);
                                metrics.observe_proxy_request_duration_by_model(model, duration);
                            }
                            metrics.observe_proxy_request_duration(metrics_path, duration);
                        }
                        return Err(mapped);
                    }
                }
            }

            let budget = Some(key.budget.clone());

            let backends = state
                .select_backends_for_model_seeded(
                    model.as_deref().unwrap_or_default(),
                    Some(key),
                    Some(request_id),
                )
                .map_err(map_openai_gateway_error)?;

            #[cfg(feature = "gateway-costing")]
            let charge_cost_usd_micros = {
                if has_cost_budget {
                    match cost_budget_policy.unwrap_or(CostBudgetEndpointPolicy::Unsupported) {
                        CostBudgetEndpointPolicy::Free => Some(0),
                        CostBudgetEndpointPolicy::TokenBased => {
                            if state.proxy.pricing.is_none() {
                                return Err(openai_error(
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    "api_error",
                                    Some("pricing_not_configured"),
                                    "pricing not configured for cost budgets",
                                ));
                            }
                            if model.as_deref().is_none() {
                                return Err(openai_error(
                                    StatusCode::BAD_REQUEST,
                                    "invalid_request_error",
                                    Some("invalid_request"),
                                    "missing field `model`",
                                ));
                            }

                            estimate_charge_cost_usd_micros(
                                state,
                                model.as_deref(),
                                input_tokens_estimate,
                                max_output_tokens,
                                service_tier.as_deref(),
                                &backends,
                            )
                        }
                        CostBudgetEndpointPolicy::Unsupported => {
                            return Err(openai_error(
                                StatusCode::BAD_REQUEST,
                                "invalid_request_error",
                                Some("cost_budget_unsupported_endpoint"),
                                "cost budgets are token-based and do not support this endpoint (disable total_usd_micros or use token budgets)",
                            ));
                        }
                    }
                } else {
                    estimate_charge_cost_usd_micros(
                        state,
                        model.as_deref(),
                        input_tokens_estimate,
                        max_output_tokens,
                        service_tier.as_deref(),
                        &backends,
                    )
                }
            };
            #[cfg(not(feature = "gateway-costing"))]
            let charge_cost_usd_micros: Option<u64> = None;

            if !use_persistent_budget {
                #[cfg(feature = "gateway-costing")]
                if key.budget.total_usd_micros.is_some() {
                    let Some(charge_cost_usd_micros) = charge_cost_usd_micros else {
                        return Err(openai_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "api_error",
                            Some("pricing_not_configured"),
                            "pricing not configured for cost budgets",
                        ));
                    };

                    if let Err(err) =
                        state.can_spend_budget_cost(&key.id, &key.budget, charge_cost_usd_micros)
                    {
                        state.record_budget_exceeded();
                        let mapped = map_openai_gateway_error(err);
                        #[cfg(feature = "gateway-metrics-prometheus")]
                        if let Some(metrics) = state.proxy.metrics.as_ref() {
                            let duration = metrics_timer_start.elapsed();
                            let status = mapped.0.as_u16();
                            let mut metrics = metrics.lock().await;
                            metrics.record_proxy_request(
                                Some(&key.id),
                                model.as_deref(),
                                metrics_path,
                            );
                            metrics.record_proxy_budget_exceeded(
                                Some(&key.id),
                                model.as_deref(),
                                metrics_path,
                            );
                            metrics.record_proxy_response_status_by_path(metrics_path, status);
                            if let Some(model) = model.as_deref() {
                                metrics.record_proxy_response_status_by_model(model, status);
                                metrics.observe_proxy_request_duration_by_model(model, duration);
                            }
                            metrics.observe_proxy_request_duration(metrics_path, duration);
                        }
                        return Err(mapped);
                    }
                }

                #[cfg(feature = "gateway-costing")]
                if tenant_budget_scope
                    .as_ref()
                    .is_some_and(|(_, budget)| budget.total_usd_micros.is_some())
                    || project_budget_scope
                        .as_ref()
                        .is_some_and(|(_, budget)| budget.total_usd_micros.is_some())
                    || user_budget_scope
                        .as_ref()
                        .is_some_and(|(_, budget)| budget.total_usd_micros.is_some())
                {
                    let Some(charge_cost_usd_micros) = charge_cost_usd_micros else {
                        return Err(openai_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "api_error",
                            Some("pricing_not_configured"),
                            "pricing not configured for cost budgets",
                        ));
                    };

                    if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                        if let Some(_limit) = budget.total_usd_micros {
                            if let Err(err) =
                                state.can_spend_budget_cost(scope, budget, charge_cost_usd_micros)
                            {
                                state.record_budget_exceeded();
                                let mapped = map_openai_gateway_error(err);
                                #[cfg(feature = "gateway-metrics-prometheus")]
                                if let Some(metrics) = state.proxy.metrics.as_ref() {
                                    let duration = metrics_timer_start.elapsed();
                                    let status = mapped.0.as_u16();
                                    let mut metrics = metrics.lock().await;
                                    metrics.record_proxy_request(
                                        Some(&key.id),
                                        model.as_deref(),
                                        metrics_path,
                                    );
                                    metrics.record_proxy_budget_exceeded(
                                        Some(&key.id),
                                        model.as_deref(),
                                        metrics_path,
                                    );
                                    metrics
                                        .record_proxy_response_status_by_path(metrics_path, status);
                                    if let Some(model) = model.as_deref() {
                                        metrics
                                            .record_proxy_response_status_by_model(model, status);
                                        metrics.observe_proxy_request_duration_by_model(
                                            model, duration,
                                        );
                                    }
                                    metrics.observe_proxy_request_duration(metrics_path, duration);
                                }
                                return Err(mapped);
                            }
                        }
                    }

                    if let Some((scope, budget)) = project_budget_scope.as_ref() {
                        if let Some(_limit) = budget.total_usd_micros {
                            if let Err(err) =
                                state.can_spend_budget_cost(scope, budget, charge_cost_usd_micros)
                            {
                                state.record_budget_exceeded();
                                let mapped = map_openai_gateway_error(err);
                                #[cfg(feature = "gateway-metrics-prometheus")]
                                if let Some(metrics) = state.proxy.metrics.as_ref() {
                                    let duration = metrics_timer_start.elapsed();
                                    let status = mapped.0.as_u16();
                                    let mut metrics = metrics.lock().await;
                                    metrics.record_proxy_request(
                                        Some(&key.id),
                                        model.as_deref(),
                                        metrics_path,
                                    );
                                    metrics.record_proxy_budget_exceeded(
                                        Some(&key.id),
                                        model.as_deref(),
                                        metrics_path,
                                    );
                                    metrics
                                        .record_proxy_response_status_by_path(metrics_path, status);
                                    if let Some(model) = model.as_deref() {
                                        metrics
                                            .record_proxy_response_status_by_model(model, status);
                                        metrics.observe_proxy_request_duration_by_model(
                                            model, duration,
                                        );
                                    }
                                    metrics.observe_proxy_request_duration(metrics_path, duration);
                                }
                                return Err(mapped);
                            }
                        }
                    }

                    if let Some((scope, budget)) = user_budget_scope.as_ref() {
                        if let Some(_limit) = budget.total_usd_micros {
                            if let Err(err) =
                                state.can_spend_budget_cost(scope, budget, charge_cost_usd_micros)
                            {
                                state.record_budget_exceeded();
                                let mapped = map_openai_gateway_error(err);
                                #[cfg(feature = "gateway-metrics-prometheus")]
                                if let Some(metrics) = state.proxy.metrics.as_ref() {
                                    let duration = metrics_timer_start.elapsed();
                                    let status = mapped.0.as_u16();
                                    let mut metrics = metrics.lock().await;
                                    metrics.record_proxy_request(
                                        Some(&key.id),
                                        model.as_deref(),
                                        metrics_path,
                                    );
                                    metrics.record_proxy_budget_exceeded(
                                        Some(&key.id),
                                        model.as_deref(),
                                        metrics_path,
                                    );
                                    metrics
                                        .record_proxy_response_status_by_path(metrics_path, status);
                                    if let Some(model) = model.as_deref() {
                                        metrics
                                            .record_proxy_response_status_by_model(model, status);
                                        metrics.observe_proxy_request_duration_by_model(
                                            model, duration,
                                        );
                                    }
                                    metrics.observe_proxy_request_duration(metrics_path, duration);
                                }
                                return Err(mapped);
                            }
                        }
                    }
                }
            }

            (
                virtual_key_id,
                limits,
                budget,
                tenant_budget_scope,
                project_budget_scope,
                user_budget_scope,
                tenant_limits_scope,
                project_limits_scope,
                user_limits_scope,
                backends,
                charge_cost_usd_micros,
            )
        } else {
            let backends = state
                .select_backends_for_model_seeded(
                    model.as_deref().unwrap_or_default(),
                    None,
                    Some(request_id),
                )
                .map_err(map_openai_gateway_error)?;

            #[cfg(feature = "gateway-costing")]
            let charge_cost_usd_micros = estimate_charge_cost_usd_micros(
                state,
                model.as_deref(),
                input_tokens_estimate,
                max_output_tokens,
                service_tier.as_deref(),
                &backends,
            );
            #[cfg(not(feature = "gateway-costing"))]
            let charge_cost_usd_micros: Option<u64> = None;

            (
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                backends,
                charge_cost_usd_micros,
            )
        }
    };

    Ok(ResolvedGatewayContext {
        virtual_key_id,
        limits,
        budget,
        tenant_budget_scope,
        project_budget_scope,
        user_budget_scope,
        tenant_limits_scope,
        project_limits_scope,
        user_limits_scope,
        backend_candidates,
        strip_authorization,
        charge_cost_usd_micros,
    })
}
// end inline: openai_compat_proxy/resolve_gateway_context.rs
// inlined from openai_compat_proxy/streaming_multipart.rs
// This file is intentionally split to keep each staged Rust file under the pre-commit size limit.
// inlined from streaming_multipart/preamble.rs
fn should_stream_large_multipart_request(
    parts: &axum::http::request::Parts,
    path_and_query: &str,
    max_body_bytes: usize,
) -> bool {
    if parts.method != axum::http::Method::POST {
        return false;
    }

    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query)
        .trim_end_matches('/');
    if path != "/v1/files" && path != "/v1/audio/transcriptions" && path != "/v1/audio/translations"
    {
        return false;
    }

    let is_multipart = parts
        .headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|ct| ct.to_ascii_lowercase().starts_with("multipart/form-data"));
    if !is_multipart {
        return false;
    }

    let content_length = parts
        .headers
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|raw| raw.parse::<usize>().ok());
    content_length.is_some_and(|len| len > max_body_bytes)
}

fn estimate_tokens_from_length(len: usize) -> u32 {
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
// end inline: streaming_multipart/preamble.rs
// inlined from streaming_multipart/handler.rs
struct ResolvedStreamingMultipartGatewayContext {
    virtual_key_id: Option<String>,
    limits: Option<super::LimitsConfig>,
    budget: Option<super::BudgetConfig>,
    tenant_budget_scope: Option<(String, super::BudgetConfig)>,
    project_budget_scope: Option<(String, super::BudgetConfig)>,
    user_budget_scope: Option<(String, super::BudgetConfig)>,
    tenant_limits_scope: Option<(String, super::LimitsConfig)>,
    project_limits_scope: Option<(String, super::LimitsConfig)>,
    user_limits_scope: Option<(String, super::LimitsConfig)>,
    backend_candidates: Vec<String>,
    strip_authorization: bool,
}

async fn handle_openai_compat_proxy_streaming_multipart(
    state: GatewayHttpState,
    parts: axum::http::request::Parts,
    body: Body,
    request_id: String,
    path_and_query: String,
) -> Result<axum::response::Response, (StatusCode, Json<OpenAiErrorResponse>)> {
    #[cfg(feature = "gateway-metrics-prometheus")]
    let metrics_path = super::metrics_prometheus::normalize_proxy_path_label(&path_and_query);
    #[cfg(feature = "gateway-metrics-prometheus")]
    let metrics_timer_start = Instant::now();
    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    let model: Option<String> = None;
    let content_length = parts
        .headers
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|raw| raw.parse::<usize>().ok())
        .unwrap_or(0);
    let charge_tokens = estimate_tokens_from_length(content_length);

    #[cfg(feature = "gateway-store-sqlite")]
    let use_sqlite_budget = state.stores.sqlite.is_some();
    #[cfg(not(feature = "gateway-store-sqlite"))]
    let use_sqlite_budget = false;

    #[cfg(feature = "gateway-store-postgres")]
    let use_postgres_budget = state.stores.postgres.is_some();
    #[cfg(not(feature = "gateway-store-postgres"))]
    let use_postgres_budget = false;

    #[cfg(feature = "gateway-store-mysql")]
    let use_mysql_budget = state.stores.mysql.is_some();
    #[cfg(not(feature = "gateway-store-mysql"))]
    let use_mysql_budget = false;

    #[cfg(feature = "gateway-store-redis")]
    let use_redis_budget = state.stores.redis.is_some();
    #[cfg(not(feature = "gateway-store-redis"))]
    let use_redis_budget = false;

    let use_persistent_budget =
        use_sqlite_budget || use_postgres_budget || use_mysql_budget || use_redis_budget;

    #[cfg(feature = "gateway-costing")]
    let mut charge_cost_usd_micros: Option<u64> = None;
    #[cfg(not(feature = "gateway-costing"))]
    let charge_cost_usd_micros: Option<u64> = None;

    let now_epoch_seconds = now_epoch_seconds();
    let minute = now_epoch_seconds / 60;
    #[cfg(feature = "gateway-store-redis")]
    let rate_limit_route = normalize_rate_limit_route(&path_and_query);

    let ResolvedStreamingMultipartGatewayContext {
        virtual_key_id,
        limits,
        budget,
        tenant_budget_scope,
        project_budget_scope,
        user_budget_scope,
        tenant_limits_scope,
        project_limits_scope,
        user_limits_scope,
        backend_candidates,
        strip_authorization,
    } = {
        state.record_request();

        let strip_authorization = state.uses_virtual_keys();
        let key = if !strip_authorization {
            None
        } else {
            let token = extract_virtual_key(&parts.headers).ok_or_else(|| {
                openai_error(
                    StatusCode::UNAUTHORIZED,
                    "authentication_error",
                    Some("invalid_api_key"),
                    "missing virtual key",
                )
            })?;
            let key = state.virtual_key_by_token(&token).ok_or_else(|| {
                openai_error(
                    StatusCode::UNAUTHORIZED,
                    "authentication_error",
                    Some("invalid_api_key"),
                    "unauthorized virtual key",
                )
            })?;
            if !key.enabled {
                return Err(openai_error(
                    StatusCode::UNAUTHORIZED,
                    "authentication_error",
                    Some("invalid_api_key"),
                    "virtual key disabled",
                ));
            }
            Some(key)
        };

        let virtual_key_id = key.as_ref().map(|key| key.id.clone());
        let limits = key.as_ref().map(|key| key.limits.clone());

        let tenant_scope = key
            .as_ref()
            .and_then(|key| key.tenant_id.as_deref())
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(|id| format!("tenant:{id}"));
        let tenant_budget_scope = key.as_ref().and_then(|key| {
            tenant_scope.as_ref().and_then(|scope| {
                key.tenant_budget
                    .as_ref()
                    .map(|budget| (scope.clone(), budget.clone()))
            })
        });
        let tenant_limits_scope = key.as_ref().and_then(|key| {
            tenant_scope.as_ref().and_then(|scope| {
                key.tenant_limits
                    .as_ref()
                    .map(|limits| (scope.clone(), limits.clone()))
            })
        });

        let project_scope = key
            .as_ref()
            .and_then(|key| key.project_id.as_deref())
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(|id| format!("project:{id}"));
        let project_budget_scope = key.as_ref().and_then(|key| {
            project_scope.as_ref().and_then(|scope| {
                key.project_budget
                    .as_ref()
                    .map(|budget| (scope.clone(), budget.clone()))
            })
        });
        let project_limits_scope = key.as_ref().and_then(|key| {
            project_scope.as_ref().and_then(|scope| {
                key.project_limits
                    .as_ref()
                    .map(|limits| (scope.clone(), limits.clone()))
            })
        });

        let user_scope = key
            .as_ref()
            .and_then(|key| key.user_id.as_deref())
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(|id| format!("user:{id}"));
        let user_budget_scope = key.as_ref().and_then(|key| {
            user_scope.as_ref().and_then(|scope| {
                key.user_budget
                    .as_ref()
                    .map(|budget| (scope.clone(), budget.clone()))
            })
        });
        let user_limits_scope = key.as_ref().and_then(|key| {
            user_scope.as_ref().and_then(|scope| {
                key.user_limits
                    .as_ref()
                    .map(|limits| (scope.clone(), limits.clone()))
            })
        });

        #[cfg(feature = "gateway-costing")]
        {
            let has_cost_budget = key
                .as_ref()
                .is_some_and(|key| key.budget.total_usd_micros.is_some())
                || tenant_budget_scope
                    .as_ref()
                    .is_some_and(|(_, budget)| budget.total_usd_micros.is_some())
                || project_budget_scope
                    .as_ref()
                    .is_some_and(|(_, budget)| budget.total_usd_micros.is_some())
                || user_budget_scope
                    .as_ref()
                    .is_some_and(|(_, budget)| budget.total_usd_micros.is_some());

            if has_cost_budget {
                match cost_budget_endpoint_policy(&parts.method, &path_and_query) {
                    CostBudgetEndpointPolicy::Free => {
                        charge_cost_usd_micros = Some(0);
                    }
                    CostBudgetEndpointPolicy::TokenBased => {
                        charge_cost_usd_micros = Some(0);
                    }
                    CostBudgetEndpointPolicy::Unsupported => {
                        let path = path_and_query
                            .split_once('?')
                            .map(|(path, _)| path)
                            .unwrap_or(path_and_query.as_str())
                            .trim_end_matches('/');
                        return Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("cost_budget_unsupported_endpoint"),
                            format!(
                                "cost budgets are token-based and do not support {path} (disable total_usd_micros or use token budgets)"
                            ),
                        ));
                    }
                }
            }
        }

        if !use_redis_budget {
            if let (Some(key), Some(limits)) = (key.as_ref(), limits.as_ref()) {
                if let Err(err) =
                    state.check_and_consume_rate_limit(&key.id, limits, charge_tokens, minute)
                {
                    state.record_rate_limited();
                    let mapped = map_openai_gateway_error(err);
                    #[cfg(feature = "gateway-metrics-prometheus")]
                    if let Some(metrics) = state.proxy.metrics.as_ref() {
                        let duration = metrics_timer_start.elapsed();
                        let status = mapped.0.as_u16();
                        let mut metrics = metrics.lock().await;
                        metrics.record_proxy_request(Some(&key.id), None, &metrics_path);
                        metrics.record_proxy_rate_limited(Some(&key.id), None, &metrics_path);
                        metrics.record_proxy_response_status_by_path(&metrics_path, status);
                        metrics.observe_proxy_request_duration(&metrics_path, duration);
                    }
                    return Err(mapped);
                }
            }
            if let Some((scope, limits)) = tenant_limits_scope.as_ref() {
                if let Err(err) =
                    state.check_and_consume_rate_limit(scope, limits, charge_tokens, minute)
                {
                    state.record_rate_limited();
                    let mapped = map_openai_gateway_error(err);
                    #[cfg(feature = "gateway-metrics-prometheus")]
                    if let Some(metrics) = state.proxy.metrics.as_ref() {
                        let duration = metrics_timer_start.elapsed();
                        let status = mapped.0.as_u16();
                        let mut metrics = metrics.lock().await;
                        metrics.record_proxy_request(
                            virtual_key_id.as_deref(),
                            None,
                            &metrics_path,
                        );
                        metrics.record_proxy_rate_limited(
                            virtual_key_id.as_deref(),
                            None,
                            &metrics_path,
                        );
                        metrics.record_proxy_response_status_by_path(&metrics_path, status);
                        metrics.observe_proxy_request_duration(&metrics_path, duration);
                    }
                    return Err(mapped);
                }
            }
            if let Some((scope, limits)) = project_limits_scope.as_ref() {
                if let Err(err) =
                    state.check_and_consume_rate_limit(scope, limits, charge_tokens, minute)
                {
                    state.record_rate_limited();
                    let mapped = map_openai_gateway_error(err);
                    #[cfg(feature = "gateway-metrics-prometheus")]
                    if let Some(metrics) = state.proxy.metrics.as_ref() {
                        let duration = metrics_timer_start.elapsed();
                        let status = mapped.0.as_u16();
                        let mut metrics = metrics.lock().await;
                        metrics.record_proxy_request(
                            virtual_key_id.as_deref(),
                            None,
                            &metrics_path,
                        );
                        metrics.record_proxy_rate_limited(
                            virtual_key_id.as_deref(),
                            None,
                            &metrics_path,
                        );
                        metrics.record_proxy_response_status_by_path(&metrics_path, status);
                        metrics.observe_proxy_request_duration(&metrics_path, duration);
                    }
                    return Err(mapped);
                }
            }
            if let Some((scope, limits)) = user_limits_scope.as_ref() {
                if let Err(err) =
                    state.check_and_consume_rate_limit(scope, limits, charge_tokens, minute)
                {
                    state.record_rate_limited();
                    let mapped = map_openai_gateway_error(err);
                    #[cfg(feature = "gateway-metrics-prometheus")]
                    if let Some(metrics) = state.proxy.metrics.as_ref() {
                        let duration = metrics_timer_start.elapsed();
                        let status = mapped.0.as_u16();
                        let mut metrics = metrics.lock().await;
                        metrics.record_proxy_request(
                            virtual_key_id.as_deref(),
                            None,
                            &metrics_path,
                        );
                        metrics.record_proxy_rate_limited(
                            virtual_key_id.as_deref(),
                            None,
                            &metrics_path,
                        );
                        metrics.record_proxy_response_status_by_path(&metrics_path, status);
                        metrics.observe_proxy_request_duration(&metrics_path, duration);
                    }
                    return Err(mapped);
                }
            }
        }

        if !use_persistent_budget {
            if let (Some(key), Some(budget)) = (key.as_ref(), key.as_ref().map(|key| &key.budget)) {
                if let Err(err) =
                    state.can_spend_budget_tokens(&key.id, budget, u64::from(charge_tokens))
                {
                    state.record_budget_exceeded();
                    let mapped = map_openai_gateway_error(err);
                    #[cfg(feature = "gateway-metrics-prometheus")]
                    if let Some(metrics) = state.proxy.metrics.as_ref() {
                        let duration = metrics_timer_start.elapsed();
                        let status = mapped.0.as_u16();
                        let mut metrics = metrics.lock().await;
                        metrics.record_proxy_request(Some(&key.id), None, &metrics_path);
                        metrics.record_proxy_budget_exceeded(Some(&key.id), None, &metrics_path);
                        metrics.record_proxy_response_status_by_path(&metrics_path, status);
                        metrics.observe_proxy_request_duration(&metrics_path, duration);
                    }
                    return Err(mapped);
                }
            }
            if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                if let Err(err) =
                    state.can_spend_budget_tokens(scope, budget, u64::from(charge_tokens))
                {
                    state.record_budget_exceeded();
                    let mapped = map_openai_gateway_error(err);
                    #[cfg(feature = "gateway-metrics-prometheus")]
                    if let Some(metrics) = state.proxy.metrics.as_ref() {
                        let duration = metrics_timer_start.elapsed();
                        let status = mapped.0.as_u16();
                        let mut metrics = metrics.lock().await;
                        metrics.record_proxy_request(
                            virtual_key_id.as_deref(),
                            None,
                            &metrics_path,
                        );
                        metrics.record_proxy_budget_exceeded(
                            virtual_key_id.as_deref(),
                            None,
                            &metrics_path,
                        );
                        metrics.record_proxy_response_status_by_path(&metrics_path, status);
                        metrics.observe_proxy_request_duration(&metrics_path, duration);
                    }
                    return Err(mapped);
                }
            }
            if let Some((scope, budget)) = project_budget_scope.as_ref() {
                if let Err(err) =
                    state.can_spend_budget_tokens(scope, budget, u64::from(charge_tokens))
                {
                    state.record_budget_exceeded();
                    let mapped = map_openai_gateway_error(err);
                    #[cfg(feature = "gateway-metrics-prometheus")]
                    if let Some(metrics) = state.proxy.metrics.as_ref() {
                        let duration = metrics_timer_start.elapsed();
                        let status = mapped.0.as_u16();
                        let mut metrics = metrics.lock().await;
                        metrics.record_proxy_request(
                            virtual_key_id.as_deref(),
                            None,
                            &metrics_path,
                        );
                        metrics.record_proxy_budget_exceeded(
                            virtual_key_id.as_deref(),
                            None,
                            &metrics_path,
                        );
                        metrics.record_proxy_response_status_by_path(&metrics_path, status);
                        metrics.observe_proxy_request_duration(&metrics_path, duration);
                    }
                    return Err(mapped);
                }
            }
            if let Some((scope, budget)) = user_budget_scope.as_ref() {
                if let Err(err) =
                    state.can_spend_budget_tokens(scope, budget, u64::from(charge_tokens))
                {
                    state.record_budget_exceeded();
                    let mapped = map_openai_gateway_error(err);
                    #[cfg(feature = "gateway-metrics-prometheus")]
                    if let Some(metrics) = state.proxy.metrics.as_ref() {
                        let duration = metrics_timer_start.elapsed();
                        let status = mapped.0.as_u16();
                        let mut metrics = metrics.lock().await;
                        metrics.record_proxy_request(
                            virtual_key_id.as_deref(),
                            None,
                            &metrics_path,
                        );
                        metrics.record_proxy_budget_exceeded(
                            virtual_key_id.as_deref(),
                            None,
                            &metrics_path,
                        );
                        metrics.record_proxy_response_status_by_path(&metrics_path, status);
                        metrics.observe_proxy_request_duration(&metrics_path, duration);
                    }
                    return Err(mapped);
                }
            }
        }

        let budget = key.as_ref().map(|key| key.budget.clone());
        let backends = state
            .select_backends_for_model_seeded("", key.as_ref(), Some(&request_id))
            .map_err(map_openai_gateway_error)?;

        ResolvedStreamingMultipartGatewayContext {
            virtual_key_id,
            limits,
            budget,
            tenant_budget_scope,
            project_budget_scope,
            user_budget_scope,
            tenant_limits_scope,
            project_limits_scope,
            user_limits_scope,
            backend_candidates: backends,
            strip_authorization,
        }
    };

    #[cfg(not(feature = "gateway-store-redis"))]
    let _ = (
        &limits,
        &tenant_limits_scope,
        &project_limits_scope,
        &user_limits_scope,
    );

    #[cfg(feature = "gateway-store-redis")]
    if use_redis_budget {
        if let Some(store) = state.stores.redis.as_ref() {
            if let Some(limits) = limits.as_ref() {
                if let Some(vk_id) = virtual_key_id.as_deref() {
                    if let Err(err) = store
                        .check_and_consume_rate_limits(
                            vk_id,
                            &rate_limit_route,
                            limits,
                            charge_tokens,
                            now_epoch_seconds,
                        )
                        .await
                    {
                        let is_rate_limited = matches!(err, GatewayError::RateLimited { .. });
                        if is_rate_limited {
                            state.record_rate_limited();
                        }
                        let mapped = map_openai_gateway_error(err);
                        #[cfg(feature = "gateway-metrics-prometheus")]
                        if is_rate_limited {
                            if let Some(metrics) = state.proxy.metrics.as_ref() {
                                let duration = metrics_timer_start.elapsed();
                                let status = mapped.0.as_u16();
                                let mut metrics = metrics.lock().await;
                                metrics.record_proxy_request(Some(vk_id), None, &metrics_path);
                                metrics.record_proxy_rate_limited(Some(vk_id), None, &metrics_path);
                                metrics.record_proxy_response_status_by_path(&metrics_path, status);
                                metrics.observe_proxy_request_duration(&metrics_path, duration);
                            }
                        }
                        return Err(mapped);
                    }
                }
            }

            for scope_and_limits in [
                tenant_limits_scope.as_ref(),
                project_limits_scope.as_ref(),
                user_limits_scope.as_ref(),
            ] {
                let Some((scope, limits)) = scope_and_limits else {
                    continue;
                };
                if let Err(err) = store
                    .check_and_consume_rate_limits(
                        scope,
                        &rate_limit_route,
                        limits,
                        charge_tokens,
                        now_epoch_seconds,
                    )
                    .await
                {
                    let is_rate_limited = matches!(err, GatewayError::RateLimited { .. });
                    if is_rate_limited {
                        state.record_rate_limited();
                    }
                    let mapped = map_openai_gateway_error(err);
                    #[cfg(feature = "gateway-metrics-prometheus")]
                    if is_rate_limited {
                        if let Some(metrics) = state.proxy.metrics.as_ref() {
                            let duration = metrics_timer_start.elapsed();
                            let status = mapped.0.as_u16();
                            let mut metrics = metrics.lock().await;
                            metrics.record_proxy_request(
                                virtual_key_id.as_deref(),
                                None,
                                &metrics_path,
                            );
                            metrics.record_proxy_rate_limited(
                                virtual_key_id.as_deref(),
                                None,
                                &metrics_path,
                            );
                            metrics.record_proxy_response_status_by_path(&metrics_path, status);
                            metrics.observe_proxy_request_duration(&metrics_path, duration);
                        }
                    }
                    return Err(mapped);
                }
            }
        }
    }

    #[cfg(feature = "gateway-metrics-prometheus")]
    if let Some(metrics) = state.proxy.metrics.as_ref() {
        metrics
            .lock()
            .await
            .record_proxy_request(virtual_key_id.as_deref(), None, &metrics_path);
    }

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    let budget_reservation_params = ProxyBudgetReservationParams {
        state: &state,
        use_persistent_budget,
        virtual_key_id: virtual_key_id.as_deref(),
        budget: budget.as_ref(),
        tenant_budget_scope: &tenant_budget_scope,
        project_budget_scope: &project_budget_scope,
        user_budget_scope: &user_budget_scope,
        request_id: &request_id,
        path_and_query: &path_and_query,
        model: &model,
        charge_tokens,
    };

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    let (_token_budget_reserved, token_budget_reservation_ids) =
        match reserve_proxy_token_budgets_for_request(budget_reservation_params).await {
            Ok(reserved) => reserved,
            Err(err) => {
                if err.0 == StatusCode::PAYMENT_REQUIRED {
                    state.record_budget_exceeded();
                }

                #[cfg(feature = "gateway-metrics-prometheus")]
                if let Some(metrics) = state.proxy.metrics.as_ref() {
                    let duration = metrics_timer_start.elapsed();
                    let status = err.0.as_u16();
                    let mut metrics = metrics.lock().await;
                    if err.0 == StatusCode::PAYMENT_REQUIRED {
                        metrics.record_proxy_budget_exceeded(
                            virtual_key_id.as_deref(),
                            None,
                            &metrics_path,
                        );
                    }
                    metrics.record_proxy_response_status_by_path(&metrics_path, status);
                    metrics.observe_proxy_request_duration(&metrics_path, duration);
                }

                return Err(err);
            }
        };
    #[cfg(not(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    )))]
    let (_token_budget_reserved, token_budget_reservation_ids): (bool, Vec<String>) =
        (false, Vec::new());

    #[cfg(all(
        feature = "gateway-costing",
        any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        ),
    ))]
    let (cost_budget_reserved, cost_budget_reservation_ids) =
        match reserve_proxy_cost_budgets_for_request(
            budget_reservation_params,
            charge_cost_usd_micros,
            &token_budget_reservation_ids,
        )
        .await
        {
            Ok(reserved) => reserved,
            Err(err) => {
                if err.0 == StatusCode::PAYMENT_REQUIRED {
                    state.record_budget_exceeded();
                }

                #[cfg(feature = "gateway-metrics-prometheus")]
                if let Some(metrics) = state.proxy.metrics.as_ref() {
                    let duration = metrics_timer_start.elapsed();
                    let status = err.0.as_u16();
                    let mut metrics = metrics.lock().await;
                    if err.0 == StatusCode::PAYMENT_REQUIRED {
                        metrics.record_proxy_budget_exceeded(
                            virtual_key_id.as_deref(),
                            None,
                            &metrics_path,
                        );
                    }
                    metrics.record_proxy_response_status_by_path(&metrics_path, status);
                    metrics.observe_proxy_request_duration(&metrics_path, duration);
                }

                return Err(err);
            }
        };
    #[cfg(not(all(
        feature = "gateway-costing",
        any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        ),
    )))]
    let (cost_budget_reserved, cost_budget_reservation_ids): (bool, Vec<String>) =
        (false, Vec::new());

    #[cfg(all(
        not(feature = "gateway-costing"),
        any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        ),
    ))]
    let _ = (&cost_budget_reservation_ids, cost_budget_reserved);

    #[cfg(not(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    )))]
    let _ = (
        &token_budget_reservation_ids,
        &cost_budget_reservation_ids,
        cost_budget_reserved,
    );

    let Some(backend_name) = backend_candidates.first().cloned() else {
        #[cfg(any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        ))]
        rollback_proxy_token_budget_reservations(&state, &token_budget_reservation_ids).await;
        #[cfg(all(
            feature = "gateway-costing",
            any(
                feature = "gateway-store-sqlite",
                feature = "gateway-store-postgres",
                feature = "gateway-store-mysql",
                feature = "gateway-store-redis"
            ),
        ))]
        rollback_proxy_cost_budget_reservations(&state, &cost_budget_reservation_ids).await;
        let err = openai_error(
            StatusCode::BAD_GATEWAY,
            "api_error",
            Some("backend_error"),
            "no backends available",
        );
        #[cfg(feature = "gateway-metrics-prometheus")]
        if let Some(metrics) = state.proxy.metrics.as_ref() {
            let duration = metrics_timer_start.elapsed();
            let status = err.0.as_u16();
            let mut metrics = metrics.lock().await;
            metrics.record_proxy_response_status_by_path(&metrics_path, status);
            metrics.observe_proxy_request_duration(&metrics_path, duration);
        }
        return Err(err);
    };

    #[cfg(feature = "gateway-translation")]
    if state
        .backends
        .translation_backends
        .contains_key(&backend_name)
    {
        #[cfg(any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        ))]
        rollback_proxy_token_budget_reservations(&state, &token_budget_reservation_ids).await;
        #[cfg(all(
            feature = "gateway-costing",
            any(
                feature = "gateway-store-sqlite",
                feature = "gateway-store-postgres",
                feature = "gateway-store-mysql",
                feature = "gateway-store-redis"
            ),
        ))]
        rollback_proxy_cost_budget_reservations(&state, &cost_budget_reservation_ids).await;
        let err = openai_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            Some("request_too_large"),
            "large multipart requests require a proxy backend (not a translation backend)",
        );
        #[cfg(feature = "gateway-metrics-prometheus")]
        if let Some(metrics) = state.proxy.metrics.as_ref() {
            let duration = metrics_timer_start.elapsed();
            let status = err.0.as_u16();
            let mut metrics = metrics.lock().await;
            metrics.record_proxy_response_status_by_path(&metrics_path, status);
            metrics.observe_proxy_request_duration(&metrics_path, duration);
        }
        return Err(err);
    }

    let backend = match state.backends.proxy_backends.get(&backend_name) {
        Some(backend) => backend.clone(),
        None => {
            #[cfg(any(
                feature = "gateway-store-sqlite",
                feature = "gateway-store-postgres",
                feature = "gateway-store-mysql",
                feature = "gateway-store-redis"
            ))]
            rollback_proxy_token_budget_reservations(&state, &token_budget_reservation_ids).await;
            #[cfg(all(
                feature = "gateway-costing",
                any(
                    feature = "gateway-store-sqlite",
                    feature = "gateway-store-postgres",
                    feature = "gateway-store-mysql",
                    feature = "gateway-store-redis"
                ),
            ))]
            rollback_proxy_cost_budget_reservations(&state, &cost_budget_reservation_ids).await;
            let err = openai_error(
                StatusCode::BAD_GATEWAY,
                "api_error",
                Some("backend_not_found"),
                format!("backend not found: {backend_name}"),
            );
            #[cfg(feature = "gateway-metrics-prometheus")]
            if let Some(metrics) = state.proxy.metrics.as_ref() {
                let duration = metrics_timer_start.elapsed();
                let status = err.0.as_u16();
                let mut metrics = metrics.lock().await;
                metrics.record_proxy_response_status_by_path(&metrics_path, status);
                metrics.record_proxy_response_status_by_backend(&backend_name, status);
                metrics.observe_proxy_request_duration(&metrics_path, duration);
            }
            return Err(err);
        }
    };

    let mut proxy_permits = match try_acquire_proxy_permits(&state, &backend_name)? {
        ProxyPermitOutcome::Acquired(permits) => permits,
        ProxyPermitOutcome::BackendRateLimited(err) => {
            #[cfg(feature = "gateway-metrics-prometheus")]
            if let Some(metrics) = state.proxy.metrics.as_ref() {
                let duration = metrics_timer_start.elapsed();
                let status = err.0.as_u16();
                let mut metrics = metrics.lock().await;
                if err.0 == StatusCode::TOO_MANY_REQUESTS {
                    metrics.record_proxy_rate_limited(
                        virtual_key_id.as_deref(),
                        None,
                        &metrics_path,
                    );
                }
                metrics.record_proxy_response_status_by_path(&metrics_path, status);
                metrics.record_proxy_response_status_by_backend(&backend_name, status);
                metrics.observe_proxy_request_duration(&metrics_path, duration);
            }
            return Err(err);
        }
    };

    let mut outgoing_headers = parts.headers.clone();
    sanitize_proxy_headers(&mut outgoing_headers, strip_authorization);
    apply_backend_headers(&mut outgoing_headers, backend.headers());
    insert_request_id(&mut outgoing_headers, &request_id);

    let data_stream = body
        .into_data_stream()
        .map(|result| result.map_err(|err| std::io::Error::other(err.to_string())));
    let outgoing_body = reqwest::Body::wrap_stream(data_stream);

    #[cfg(feature = "gateway-metrics-prometheus")]
    let backend_timer_start = Instant::now();
    #[cfg(feature = "gateway-metrics-prometheus")]
    if let Some(metrics) = state.proxy.metrics.as_ref() {
        let mut metrics = metrics.lock().await;
        metrics.record_proxy_backend_attempt(&backend_name);
        metrics.record_proxy_backend_in_flight_inc(&backend_name);
    }

    let upstream_response = match backend
        .request_stream(
            parts.method.clone(),
            &path_and_query,
            outgoing_headers,
            Some(outgoing_body),
        )
        .await
    {
        Ok(response) => response,
        Err(err) => {
            let mapped = map_openai_gateway_error(err);
            #[cfg(feature = "gateway-metrics-prometheus")]
            if let Some(metrics) = state.proxy.metrics.as_ref() {
                let duration = metrics_timer_start.elapsed();
                let status = mapped.0.as_u16();
                let mut metrics = metrics.lock().await;
                metrics.record_proxy_backend_in_flight_dec(&backend_name);
                metrics.observe_proxy_backend_request_duration(
                    &backend_name,
                    backend_timer_start.elapsed(),
                );
                metrics.record_proxy_backend_failure(&backend_name);
                metrics.record_proxy_response_status_by_path(&metrics_path, status);
                metrics.record_proxy_response_status_by_backend(&backend_name, status);
                metrics.observe_proxy_request_duration(&metrics_path, duration);
            }
            #[cfg(any(
                feature = "gateway-store-sqlite",
                feature = "gateway-store-postgres",
                feature = "gateway-store-mysql",
                feature = "gateway-store-redis"
            ))]
            rollback_proxy_token_budget_reservations(&state, &token_budget_reservation_ids).await;
            #[cfg(all(
                feature = "gateway-costing",
                any(
                    feature = "gateway-store-sqlite",
                    feature = "gateway-store-postgres",
                    feature = "gateway-store-mysql",
                    feature = "gateway-store-redis"
                ),
            ))]
            rollback_proxy_cost_budget_reservations(&state, &cost_budget_reservation_ids).await;
            return Err(mapped);
        }
    };

    let status = upstream_response.status();
    let spend_tokens = status.is_success();
    let spent_tokens = if spend_tokens {
        u64::from(charge_tokens)
    } else {
        0
    };
    let spent_cost_usd_micros = if spend_tokens {
        charge_cost_usd_micros
    } else {
        None
    };

    #[cfg(feature = "gateway-metrics-prometheus")]
    if let Some(metrics) = state.proxy.metrics.as_ref() {
        let duration = metrics_timer_start.elapsed();
        let status_code = status.as_u16();
        let mut metrics = metrics.lock().await;
        metrics.record_proxy_backend_in_flight_dec(&backend_name);
        metrics
            .observe_proxy_backend_request_duration(&backend_name, backend_timer_start.elapsed());
        if spend_tokens {
            metrics.record_proxy_backend_success(&backend_name);
        } else {
            metrics.record_proxy_backend_failure(&backend_name);
        }
        metrics.record_proxy_response_status_by_path(&metrics_path, status_code);
        metrics.record_proxy_response_status_by_backend(&backend_name, status_code);
        metrics.observe_proxy_request_duration(&metrics_path, duration);
    }

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    if !token_budget_reservation_ids.is_empty() {
        settle_proxy_token_budget_reservations(
            &state,
            &token_budget_reservation_ids,
            spend_tokens,
            spent_tokens,
        )
        .await;
    }

    if token_budget_reservation_ids.is_empty() && spend_tokens && !use_persistent_budget {
        if let (Some(virtual_key_id), Some(budget)) = (virtual_key_id.clone(), budget.clone()) {
            state.spend_budget_tokens(&virtual_key_id, &budget, spent_tokens);
            if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                state.spend_budget_tokens(scope, budget, spent_tokens);
            }
            if let Some((scope, budget)) = project_budget_scope.as_ref() {
                state.spend_budget_tokens(scope, budget, spent_tokens);
            }
            if let Some((scope, budget)) = user_budget_scope.as_ref() {
                state.spend_budget_tokens(scope, budget, spent_tokens);
            }

            #[cfg(feature = "gateway-costing")]
            if let Some(spent_cost_usd_micros) = spent_cost_usd_micros {
                state.spend_budget_cost(&virtual_key_id, &budget, spent_cost_usd_micros);
                if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                    state.spend_budget_cost(scope, budget, spent_cost_usd_micros);
                }
                if let Some((scope, budget)) = project_budget_scope.as_ref() {
                    state.spend_budget_cost(scope, budget, spent_cost_usd_micros);
                }
                if let Some((scope, budget)) = user_budget_scope.as_ref() {
                    state.spend_budget_cost(scope, budget, spent_cost_usd_micros);
                }
            }
        }
    }

    #[cfg(not(feature = "gateway-costing"))]
    let _ = &spent_cost_usd_micros;

    #[cfg(all(
        feature = "gateway-costing",
        any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        ),
    ))]
    if !cost_budget_reservation_ids.is_empty() {
        settle_proxy_cost_budget_reservations(
            &state,
            &cost_budget_reservation_ids,
            spend_tokens,
            spent_cost_usd_micros.unwrap_or_default(),
        )
        .await;
    }

    #[cfg(all(
        feature = "gateway-costing",
        any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        ),
    ))]
    if !cost_budget_reserved && use_persistent_budget && spend_tokens {
        if let (Some(virtual_key_id), Some(spent_cost_usd_micros)) =
            (virtual_key_id.as_deref(), spent_cost_usd_micros)
        {
            #[cfg(feature = "gateway-store-sqlite")]
            if let Some(store) = state.stores.sqlite.as_ref() {
                let _ = store
                    .record_spent_cost_usd_micros(virtual_key_id, spent_cost_usd_micros)
                    .await;
            }
            #[cfg(feature = "gateway-store-redis")]
            if let Some(store) = state.stores.redis.as_ref() {
                let _ = store
                    .record_spent_cost_usd_micros(virtual_key_id, spent_cost_usd_micros)
                    .await;
            }
        }
    }

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    {
        let payload = serde_json::json!({
            "request_id": &request_id,
            "virtual_key_id": virtual_key_id.as_deref(),
            "backend": &backend_name,
            "attempted_backends": [&backend_name],
            "method": parts.method.as_str(),
            "path": &path_and_query,
            "model": Value::Null,
            "status": status.as_u16(),
            "charge_tokens": charge_tokens,
            "spent_tokens": spent_tokens,
            "charge_cost_usd_micros": charge_cost_usd_micros,
            "spent_cost_usd_micros": spent_cost_usd_micros,
            "body_len": content_length,
        });
        append_audit_log(&state, "proxy", payload).await;
    }

    emit_json_log(
        &state,
        "proxy.response",
        serde_json::json!({
            "request_id": &request_id,
            "backend": &backend_name,
            "status": status.as_u16(),
        }),
    );

    #[cfg(feature = "gateway-otel")]
    {
        tracing::Span::current().record("cache", tracing::field::display("miss"));
        tracing::Span::current().record("backend", tracing::field::display(&backend_name));
        tracing::Span::current().record("status", tracing::field::display(status.as_u16()));
    }

    Ok(proxy_response(
        ProxyResponseContext {
            state: &state,
            backend: &backend_name,
            request_id: &request_id,
            #[cfg(feature = "gateway-metrics-prometheus")]
            metrics_path: metrics_path.as_str(),
            cache_key: None,
            #[cfg(feature = "gateway-proxy-cache")]
            cache_metadata: None,
        },
        upstream_response,
        proxy_permits.take(),
    )
    .await)
}
// end inline: streaming_multipart/handler.rs
// end inline: openai_compat_proxy/streaming_multipart.rs
// inlined from openai_compat_proxy/path_normalize.rs
fn normalize_openai_compat_path(path: &str) -> std::borrow::Cow<'_, str> {
    if path.starts_with("/v1/") || path == "/v1" {
        return std::borrow::Cow::Borrowed(path);
    }

    match path {
        "/chat/completions" => std::borrow::Cow::Borrowed("/v1/chat/completions"),
        "/completions" => std::borrow::Cow::Borrowed("/v1/completions"),
        "/embeddings" => std::borrow::Cow::Borrowed("/v1/embeddings"),
        "/moderations" => std::borrow::Cow::Borrowed("/v1/moderations"),
        "/images/generations" => std::borrow::Cow::Borrowed("/v1/images/generations"),
        "/audio/transcriptions" => std::borrow::Cow::Borrowed("/v1/audio/transcriptions"),
        "/audio/translations" => std::borrow::Cow::Borrowed("/v1/audio/translations"),
        "/audio/speech" => std::borrow::Cow::Borrowed("/v1/audio/speech"),
        "/files" => std::borrow::Cow::Borrowed("/v1/files"),
        "/rerank" => std::borrow::Cow::Borrowed("/v1/rerank"),
        "/batches" => std::borrow::Cow::Borrowed("/v1/batches"),
        "/models" => std::borrow::Cow::Borrowed("/v1/models"),
        "/responses" => std::borrow::Cow::Borrowed("/v1/responses"),
        "/responses/compact" => std::borrow::Cow::Borrowed("/v1/responses/compact"),
        _ => {
            if path.starts_with("/models/")
                || path.starts_with("/files/")
                || path.starts_with("/batches/")
                || path.starts_with("/responses/")
            {
                return std::borrow::Cow::Owned(format!("/v1{path}"));
            }
            std::borrow::Cow::Borrowed(path)
        }
    }
}

fn normalize_openai_compat_path_and_query(path_and_query: &str) -> std::borrow::Cow<'_, str> {
    let Some((path, query)) = path_and_query.split_once('?') else {
        return normalize_openai_compat_path(path_and_query);
    };

    let normalized_path = normalize_openai_compat_path(path);
    if normalized_path.as_ref() == path {
        std::borrow::Cow::Borrowed(path_and_query)
    } else {
        std::borrow::Cow::Owned(format!("{}?{query}", normalized_path.as_ref()))
    }
}

async fn handle_openai_compat_proxy_root(
    State(state): State<GatewayHttpState>,
    req: axum::http::Request<Body>,
) -> Result<axum::response::Response, (StatusCode, Json<OpenAiErrorResponse>)> {
    handle_openai_compat_proxy(State(state), Path(String::new()), req).await
}
// end inline: openai_compat_proxy/path_normalize.rs
// inlined from openai_compat_proxy/mcp.rs
const MCP_AUTO_EXEC_MAX_TOOL_CALLS_PER_ROUND: usize = 32;
const MCP_AUTO_EXEC_MAX_TOOL_RESULT_TEXT_BYTES: usize = 64 * 1024;
const MCP_AUTO_EXEC_MAX_ADDED_MESSAGES_BYTES: usize = 512 * 1024;

async fn maybe_handle_mcp_tools_chat_completions(
    state: &GatewayHttpState,
    parts: &axum::http::request::Parts,
    parsed_json: &Option<Value>,
    request_id: &str,
    path_and_query: &str,
) -> Result<Option<axum::response::Response>, (StatusCode, Json<OpenAiErrorResponse>)> {
    if parts.headers.contains_key("x-ditto-skip-mcp") {
        return Ok(None);
    }
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);

    match path {
        "/v1/chat/completions" => {
            maybe_handle_mcp_tools_chat_completions_impl(state, parts, parsed_json, request_id)
                .await
        }
        "/v1/responses" => {
            maybe_handle_mcp_tools_responses(state, parts, parsed_json, request_id).await
        }
        _ => Ok(None),
    }
}

async fn maybe_handle_mcp_tools_chat_completions_impl(
    state: &GatewayHttpState,
    parts: &axum::http::request::Parts,
    parsed_json: &Option<Value>,
    request_id: &str,
) -> Result<Option<axum::response::Response>, (StatusCode, Json<OpenAiErrorResponse>)> {
    let Some(request_json) = parsed_json.as_ref() else {
        return Ok(None);
    };

    let tools = request_json.get("tools").and_then(|v| v.as_array());
    let Some(tools) = tools else {
        return Ok(None);
    };

    let (mcp_tool_cfgs, other_tools) = split_mcp_tool_configs(tools)?;
    if mcp_tool_cfgs.is_empty() {
        return Ok(None);
    }

    let original_stream = request_json
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let auto_execute = mcp_tool_cfgs
        .iter()
        .all(|cfg| cfg.require_approval.as_deref() == Some("never"));
    let max_steps = resolve_mcp_max_steps(&mcp_tool_cfgs)?;

    let requested_servers = resolve_mcp_servers_from_tool_cfgs(&mcp_tool_cfgs);
    let server_ids = match requested_servers {
        Some(servers) => servers,
        None => {
            let mut all: Vec<String> = state.backends.mcp_servers.keys().cloned().collect();
            all.sort();
            all
        }
    };

    if server_ids.is_empty() {
        return Err(openai_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            Some("invalid_mcp_server"),
            "no MCP servers selected",
        ));
    }

    let mcp_tools_value = mcp_list_tools(state, &server_ids, None)
        .await
        .map_err(map_openai_gateway_error)?;
    let mut mcp_tools = mcp_tools_value
        .get("tools")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let allowed_tools = collect_allowed_tools(&mcp_tool_cfgs);
    if !allowed_tools.is_empty() {
        mcp_tools.retain(|tool| tool_name_allowed(tool, &allowed_tools));
    }

    let openai_tools: Vec<Value> = mcp_tools
        .iter()
        .filter_map(mcp_tool_to_openai_tool)
        .collect();
    let tools_for_llm: Vec<Value> = openai_tools
        .iter()
        .cloned()
        .chain(other_tools.into_iter())
        .collect();
    let mut request_with_tools = request_json.clone();
    set_json_tools(&mut request_with_tools, tools_for_llm.clone());

    if !auto_execute {
        let response = call_openai_compat_proxy_with_body(
            state,
            parts,
            request_id,
            &request_with_tools,
            original_stream,
        )
        .await?;
        return Ok(Some(response));
    }

    let Some(messages) = request_json
        .get("messages")
        .and_then(|v| v.as_array())
        .cloned()
    else {
        return Err(openai_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            Some("invalid_mcp_request"),
            "missing messages",
        ));
    };
    let mut messages = messages;
    let mut messages_bytes = estimate_messages_bytes(&messages);
    let max_messages_bytes = messages_bytes.saturating_add(MCP_AUTO_EXEC_MAX_ADDED_MESSAGES_BYTES);

    let mut tool_rounds_executed: usize = 0;
    loop {
        if tool_rounds_executed >= max_steps {
            let mut req_json = request_with_tools.clone();
            if let Some(obj) = req_json.as_object_mut() {
                obj.insert("messages".to_string(), Value::Array(messages));
            }
            let response = call_openai_compat_proxy_with_body(
                state,
                parts,
                request_id,
                &req_json,
                original_stream,
            )
            .await?;
            return Ok(Some(response));
        }

        // Non-stream call to extract tool calls.
        let step_request_id = format!("{request_id}-mcp{tool_rounds_executed}");
        let step_response = {
            let mut step_req_json = request_with_tools.clone();
            {
                let Some(obj) = step_req_json.as_object_mut() else {
                    return Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_mcp_request"),
                        "invalid chat/completions request",
                    ));
                };
                obj.insert(
                    "messages".to_string(),
                    Value::Array(std::mem::take(&mut messages)),
                );
            }
            let response = call_openai_compat_proxy_with_body(
                state,
                parts,
                &step_request_id,
                &step_req_json,
                false,
            )
            .await?;
            let restored_messages = {
                let Some(obj) = step_req_json.as_object_mut() else {
                    return Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_mcp_request"),
                        "invalid chat/completions request",
                    ));
                };
                match obj.remove("messages") {
                    Some(Value::Array(restored_messages)) => restored_messages,
                    _ => {
                        return Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_mcp_request"),
                            "invalid chat/completions request",
                        ));
                    }
                }
            };
            messages = restored_messages;
            response
        };

        let (step_status, step_headers, step_body) =
            split_response(step_response, 8 * 1024 * 1024).await?;

        let step_json: Value = match serde_json::from_slice(&step_body) {
            Ok(value) => value,
            Err(_) => {
                return Ok(Some(rebuild_response(step_status, step_headers, step_body)));
            }
        };
        let assistant_message = extract_chat_assistant_message(&step_json);
        let tool_calls = assistant_message
            .as_ref()
            .map(extract_chat_tool_calls_from_message)
            .unwrap_or_default();

        if tool_calls.is_empty() {
            if original_stream {
                let mut req_json = request_with_tools.clone();
                if let Some(obj) = req_json.as_object_mut() {
                    obj.insert("messages".to_string(), Value::Array(messages));
                }
                let response =
                    call_openai_compat_proxy_with_body(state, parts, request_id, &req_json, true)
                        .await?;
                return Ok(Some(response));
            }
            return Ok(Some(rebuild_response(step_status, step_headers, step_body)));
        }
        if tool_calls.len() > MCP_AUTO_EXEC_MAX_TOOL_CALLS_PER_ROUND {
            return Err(openai_error(
                StatusCode::BAD_GATEWAY,
                "api_error",
                Some("mcp_auto_exec_limit_exceeded"),
                format!(
                    "tool call count {} exceeded per-round limit {}",
                    tool_calls.len(),
                    MCP_AUTO_EXEC_MAX_TOOL_CALLS_PER_ROUND
                ),
            ));
        }

        if let Some(message) =
            assistant_message.or_else(|| build_chat_assistant_message_from_tool_calls(&tool_calls))
        {
            push_message_with_limit(
                &mut messages,
                &mut messages_bytes,
                max_messages_bytes,
                message,
            )?;
        }

        for call in &tool_calls {
            let result = mcp_call_tool(state, &server_ids, &call.name, call.arguments.clone())
                .await
                .unwrap_or_else(|err| Value::String(format!("MCP tool call failed: {err}")));
            let content = mcp_tool_result_to_text(&result);
            push_message_with_limit(
                &mut messages,
                &mut messages_bytes,
                max_messages_bytes,
                serde_json::json!({
                    "role": "tool",
                    "tool_call_id": call.id.clone(),
                    "content": content,
                }),
            )?;
        }

        tool_rounds_executed = tool_rounds_executed.saturating_add(1);
    }
}

async fn maybe_handle_mcp_tools_responses(
    state: &GatewayHttpState,
    parts: &axum::http::request::Parts,
    parsed_json: &Option<Value>,
    request_id: &str,
) -> Result<Option<axum::response::Response>, (StatusCode, Json<OpenAiErrorResponse>)> {
    let Some(request_json) = parsed_json.as_ref() else {
        return Ok(None);
    };

    let tools = request_json.get("tools").and_then(|v| v.as_array());
    let Some(tools) = tools else {
        return Ok(None);
    };

    let (mcp_tool_cfgs, other_tools) = split_mcp_tool_configs(tools)?;
    if mcp_tool_cfgs.is_empty() {
        return Ok(None);
    }

    let original_stream = request_json
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let auto_execute = mcp_tool_cfgs
        .iter()
        .all(|cfg| cfg.require_approval.as_deref() == Some("never"));
    let max_steps = resolve_mcp_max_steps(&mcp_tool_cfgs)?;

    let requested_servers = resolve_mcp_servers_from_tool_cfgs(&mcp_tool_cfgs);
    let server_ids = match requested_servers {
        Some(servers) => servers,
        None => {
            let mut all: Vec<String> = state.backends.mcp_servers.keys().cloned().collect();
            all.sort();
            all
        }
    };

    if server_ids.is_empty() {
        return Err(openai_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            Some("invalid_mcp_server"),
            "no MCP servers selected",
        ));
    }

    let mcp_tools_value = mcp_list_tools(state, &server_ids, None)
        .await
        .map_err(map_openai_gateway_error)?;
    let mut mcp_tools = mcp_tools_value
        .get("tools")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let allowed_tools = collect_allowed_tools(&mcp_tool_cfgs);
    if !allowed_tools.is_empty() {
        mcp_tools.retain(|tool| tool_name_allowed(tool, &allowed_tools));
    }

    let openai_tools: Vec<Value> = mcp_tools
        .iter()
        .filter_map(mcp_tool_to_openai_tool)
        .collect();
    let tools_for_llm: Vec<Value> = openai_tools
        .iter()
        .cloned()
        .chain(other_tools.into_iter())
        .collect();
    let mut request_with_tools = request_json.clone();
    set_json_tools(&mut request_with_tools, tools_for_llm.clone());

    if !auto_execute {
        let response = call_openai_compat_proxy_with_body(
            state,
            parts,
            request_id,
            &request_with_tools,
            original_stream,
        )
        .await?;
        return Ok(Some(response));
    }

    // 1) Initial non-stream call to extract tool calls.
    let initial_request_id = format!("{request_id}-mcp0");
    let initial_req_json = request_with_tools.clone();
    let initial_response = call_openai_compat_proxy_with_body(
        state,
        parts,
        &initial_request_id,
        &initial_req_json,
        false,
    )
    .await?;

    let (initial_status, initial_headers, initial_body) =
        split_response(initial_response, 8 * 1024 * 1024).await?;

    let initial_json: Option<Value> = serde_json::from_slice(&initial_body).ok();
    let tool_calls = initial_json
        .as_ref()
        .map(extract_responses_tool_calls)
        .unwrap_or_default();

    if tool_calls.is_empty() {
        if original_stream {
            let response = call_openai_compat_proxy_with_body(
                state,
                parts,
                request_id,
                &request_with_tools,
                true,
            )
            .await?;
            return Ok(Some(response));
        }
        return Ok(Some(rebuild_response(
            initial_status,
            initial_headers,
            initial_body,
        )));
    }

    let tool_results = execute_mcp_tool_calls(state, &server_ids, &tool_calls).await;

    let initial_is_shim = initial_headers.contains_key("x-ditto-shim");
    let response_id = initial_json
        .as_ref()
        .and_then(|value| value.get("id"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());

    if initial_is_shim || response_id.is_none() {
        let response = follow_up_via_chat_completions_to_responses(
            state,
            parts,
            McpResponsesToolLoopParams {
                request_id,
                request_json,
                server_ids: &server_ids,
                tools_for_llm: tools_for_llm.clone(),
                initial_tool_calls: &tool_calls,
                initial_tool_results: &tool_results,
                max_steps,
            },
        )
        .await?;
        return Ok(Some(response));
    }

    let response_id = response_id.unwrap_or_default();

    // Native /responses: multi-step tool loop is safe only for non-stream.
    if original_stream {
        // 2) Follow-up call with tool results via native /responses.
        let mut follow_up = request_with_tools.clone();
        if let Some(obj) = follow_up.as_object_mut() {
            obj.insert(
                "previous_response_id".to_string(),
                Value::String(response_id),
            );
            obj.insert("stream".to_string(), Value::Bool(true));
            let outputs = tool_calls
                .iter()
                .zip(tool_results.iter())
                .map(|(call, output)| {
                    serde_json::json!({
                        "type": "function_call_output",
                        "call_id": call.call_id.clone(),
                        "output": output,
                    })
                })
                .collect::<Vec<_>>();
            obj.insert("input".to_string(), Value::Array(outputs));
            obj.remove("messages");
        }

        let response =
            call_openai_compat_proxy_with_body(state, parts, request_id, &follow_up, true).await?;
        return Ok(Some(response));
    }

    let mut tool_rounds_executed: usize = 1;
    let mut prev_response_id = response_id;
    let mut tool_calls = tool_calls;
    let mut tool_results = tool_results;
    loop {
        if tool_rounds_executed >= max_steps {
            let mut follow_up = request_with_tools.clone();
            if let Some(obj) = follow_up.as_object_mut() {
                obj.insert(
                    "previous_response_id".to_string(),
                    Value::String(prev_response_id.clone()),
                );
                obj.insert("stream".to_string(), Value::Bool(false));
                let outputs = tool_calls
                    .iter()
                    .zip(tool_results.iter())
                    .map(|(call, output)| {
                        serde_json::json!({
                            "type": "function_call_output",
                            "call_id": call.call_id.clone(),
                            "output": output,
                        })
                    })
                    .collect::<Vec<_>>();
                obj.insert("input".to_string(), Value::Array(outputs));
                obj.remove("messages");
            }

            let response =
                call_openai_compat_proxy_with_body(state, parts, request_id, &follow_up, false)
                    .await?;
            return Ok(Some(response));
        }

        let step_request_id = format!("{request_id}-mcp{tool_rounds_executed}");
        let mut follow_up = request_with_tools.clone();
        if let Some(obj) = follow_up.as_object_mut() {
            obj.insert(
                "previous_response_id".to_string(),
                Value::String(prev_response_id.clone()),
            );
            obj.insert("stream".to_string(), Value::Bool(false));
            let outputs = tool_calls
                .iter()
                .zip(tool_results.iter())
                .map(|(call, output)| {
                    serde_json::json!({
                        "type": "function_call_output",
                        "call_id": call.call_id.clone(),
                        "output": output,
                    })
                })
                .collect::<Vec<_>>();
            obj.insert("input".to_string(), Value::Array(outputs));
            obj.remove("messages");
        }

        let response =
            call_openai_compat_proxy_with_body(state, parts, &step_request_id, &follow_up, false)
                .await?;

        let (status, headers, body) = split_response(response, 8 * 1024 * 1024).await?;
        let value: Value = match serde_json::from_slice(&body) {
            Ok(value) => value,
            Err(_) => return Ok(Some(rebuild_response(status, headers, body))),
        };

        let next_tool_calls = extract_responses_tool_calls(&value);
        if next_tool_calls.is_empty() {
            return Ok(Some(rebuild_response(status, headers, body)));
        }

        let next_response_id = value
            .get("id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());
        let Some(next_response_id) = next_response_id else {
            return Ok(Some(rebuild_response(status, headers, body)));
        };

        tool_results = execute_mcp_tool_calls(state, &server_ids, &next_tool_calls).await;
        tool_calls = next_tool_calls;
        prev_response_id = next_response_id;
        tool_rounds_executed = tool_rounds_executed.saturating_add(1);
    }
}

#[derive(Debug, Clone)]
struct McpToolConfig {
    server_url: Option<String>,
    require_approval: Option<String>,
    allowed_tools: Vec<String>,
    max_steps: Option<usize>,
}

const MCP_MAX_STEPS: usize = 8;

type OpenAiCompatProxyError = (StatusCode, Json<OpenAiErrorResponse>);
type OpenAiCompatProxyResult<T> = Result<T, OpenAiCompatProxyError>;
type McpToolSplit = (Vec<McpToolConfig>, Vec<Value>);

fn split_mcp_tool_configs(tools: &[Value]) -> OpenAiCompatProxyResult<McpToolSplit> {
    let mut mcp = Vec::new();
    let mut other = Vec::new();

    for tool in tools {
        let Some(obj) = tool.as_object() else {
            other.push(tool.clone());
            continue;
        };
        let tool_type = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if tool_type != "mcp" {
            other.push(tool.clone());
            continue;
        }

        let server_url = obj
            .get("server_url")
            .and_then(|v| v.as_str())
            .map(|v| v.to_string());
        let require_approval = obj
            .get("require_approval")
            .and_then(|v| v.as_str())
            .map(|v| v.to_string());
        let max_steps_value = obj
            .get("max_steps")
            .or_else(|| obj.get("maxSteps"))
            .cloned();
        let max_steps = if let Some(value) = max_steps_value {
            let Some(steps) = value.as_u64() else {
                return Err(openai_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request_error",
                    Some("invalid_mcp_request"),
                    "max_steps must be a positive integer",
                ));
            };
            if steps == 0 || steps > MCP_MAX_STEPS as u64 {
                return Err(openai_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request_error",
                    Some("invalid_mcp_request"),
                    format!("max_steps must be between 1 and {MCP_MAX_STEPS}"),
                ));
            }
            Some(steps as usize)
        } else {
            None
        };
        let allowed_tools = obj
            .get("allowed_tools")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        mcp.push(McpToolConfig {
            server_url,
            require_approval,
            allowed_tools,
            max_steps,
        });
    }

    Ok((mcp, other))
}

fn resolve_mcp_max_steps(cfgs: &[McpToolConfig]) -> OpenAiCompatProxyResult<usize> {
    let mut max_steps = 1usize;
    for cfg in cfgs {
        if let Some(steps) = cfg.max_steps {
            max_steps = max_steps.max(steps);
        }
    }
    if max_steps == 0 || max_steps > MCP_MAX_STEPS {
        return Err(openai_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            Some("invalid_mcp_request"),
            format!("max_steps must be between 1 and {MCP_MAX_STEPS}"),
        ));
    }
    Ok(max_steps)
}

fn collect_allowed_tools(cfgs: &[McpToolConfig]) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    for cfg in cfgs {
        for tool in &cfg.allowed_tools {
            out.insert(tool.clone());
        }
    }
    out
}

fn tool_name_allowed(tool: &Value, allowed: &std::collections::BTreeSet<String>) -> bool {
    let Some(name) = tool.get("name").and_then(|v| v.as_str()) else {
        return false;
    };
    if allowed.contains(name) {
        return true;
    }
    allowed.iter().any(|candidate| {
        !candidate.is_empty()
            && name.len() > candidate.len() + 1
            && name.ends_with(candidate)
            && name.as_bytes()[name.len() - candidate.len() - 1] == b'-'
    })
}

fn resolve_mcp_servers_from_tool_cfgs(cfgs: &[McpToolConfig]) -> Option<Vec<String>> {
    let mut out = std::collections::BTreeSet::new();
    let mut saw_all = false;

    for cfg in cfgs {
        let Some(url) = cfg.server_url.as_deref() else {
            saw_all = true;
            continue;
        };
        if url.trim().is_empty() {
            saw_all = true;
            continue;
        }
        match parse_mcp_server_selector(url) {
            None => saw_all = true,
            Some(list) => {
                for server in list {
                    out.insert(server);
                }
            }
        }
    }

    if saw_all {
        None
    } else {
        Some(out.into_iter().collect())
    }
}

fn parse_mcp_server_selector(server_url: &str) -> Option<Vec<String>> {
    let trimmed = server_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }

    // LiteLLM tool shortcut: litellm_proxy/mcp/<servers>
    if trimmed == "litellm_proxy" || trimmed.starts_with("litellm_proxy/") {
        let rest = trimmed.trim_start_matches("litellm_proxy");
        let rest = rest.trim_start_matches('/');
        if rest.is_empty() || rest == "mcp" {
            return None;
        }
        let servers = rest.strip_prefix("mcp/").unwrap_or(rest);
        if servers.is_empty() {
            return None;
        }
        return Some(split_csv(servers));
    }

    // URL/path forms: /mcp/<servers> or /<servers>/mcp
    if let Ok(uri) = trimmed.parse::<axum::http::Uri>() {
        if let Some(path) = uri.path_and_query().map(|pq| pq.path()) {
            return parse_mcp_selector_from_path(path);
        }
    }

    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        if let Ok(url) = reqwest::Url::parse(trimmed) {
            return parse_mcp_selector_from_path(url.path());
        }
    }

    parse_mcp_selector_from_path(trimmed)
}

fn parse_mcp_selector_from_path(path: &str) -> Option<Vec<String>> {
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    let path = path.trim_end_matches('/');

    // /mcp => all
    if path == "/mcp" {
        return None;
    }
    // /mcp/<servers>
    if let Some(rest) = path.strip_prefix("/mcp/") {
        if rest.is_empty() {
            return None;
        }
        return Some(split_csv(rest));
    }
    // /<servers>/mcp
    if let Some(rest) = path.strip_suffix("/mcp") {
        let rest = rest.trim_end_matches('/');
        let servers = rest.rsplit('/').next().unwrap_or_default();
        if servers.is_empty() {
            return None;
        }
        return Some(split_csv(servers));
    }
    None
}

fn split_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn mcp_tool_to_openai_tool(tool: &Value) -> Option<Value> {
    let name = tool.get("name")?.as_str()?.to_string();
    let description = tool
        .get("description")
        .and_then(|v| v.as_str())
        .map(|v| v.to_string());
    let input_schema = tool.get("inputSchema").cloned().unwrap_or(Value::Null);

    let mut function = serde_json::json!({
        "name": name,
        "parameters": input_schema,
    });
    if let (Some(desc), Some(obj)) = (description, function.as_object_mut()) {
        obj.insert("description".to_string(), Value::String(desc));
    }

    Some(serde_json::json!({
        "type": "function",
        "function": function,
    }))
}

fn set_json_tools(request: &mut Value, tools: Vec<Value>) {
    let Some(obj) = request.as_object_mut() else {
        return;
    };
    obj.insert("tools".to_string(), Value::Array(tools));
}

async fn call_openai_compat_proxy_with_body(
    state: &GatewayHttpState,
    parts: &axum::http::request::Parts,
    request_id: &str,
    body_json: &Value,
    stream: bool,
) -> Result<axum::response::Response, (StatusCode, Json<OpenAiErrorResponse>)> {
    let mut body_json = body_json.clone();
    if let Some(obj) = body_json.as_object_mut() {
        obj.insert("stream".to_string(), Value::Bool(stream));
    }
    let bytes = match serde_json::to_vec(&body_json) {
        Ok(bytes) => Bytes::from(bytes),
        Err(err) => {
            return Err(openai_error(
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                Some("invalid_json"),
                err,
            ));
        }
    };

    let mut headers = parts.headers.clone();
    headers.insert(
        "x-ditto-skip-mcp",
        axum::http::HeaderValue::from_static("1"),
    );
    if let Ok(value) = axum::http::HeaderValue::from_str(request_id) {
        headers.insert("x-request-id", value);
    }
    headers.insert(
        "content-type",
        axum::http::HeaderValue::from_static("application/json"),
    );

    let mut req = axum::http::Request::new(Body::from(bytes));
    *req.method_mut() = parts.method.clone();
    *req.uri_mut() = parts.uri.clone();
    *req.headers_mut() = headers;

    let fut = Box::pin(handle_openai_compat_proxy(
        State(state.clone()),
        Path(String::new()),
        req,
    ));
    fut.await
}

async fn split_response(
    response: axum::response::Response,
    max_bytes: usize,
) -> Result<(StatusCode, HeaderMap, Bytes), (StatusCode, Json<OpenAiErrorResponse>)> {
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), max_bytes)
        .await
        .map_err(|err| {
            openai_error(
                StatusCode::BAD_GATEWAY,
                "api_error",
                Some("backend_error"),
                err,
            )
        })?;
    Ok((status, headers, bytes))
}

fn rebuild_response(
    status: StatusCode,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    let mut response = axum::response::Response::new(Body::from(body));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
}

// inlined from mcp_responses.rs
#[derive(Debug, Clone)]
struct ResponsesToolCall {
    call_id: String,
    name: String,
    arguments: Value,
}

fn extract_responses_tool_calls(response: &Value) -> Vec<ResponsesToolCall> {
    let mut out = Vec::new();
    let Some(items) = response.get("output").and_then(|v| v.as_array()) else {
        return out;
    };

    for (idx, item) in items.iter().enumerate() {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let item_type = obj.get("type").and_then(|v| v.as_str()).unwrap_or_default();
        if item_type != "function_call" {
            continue;
        }

        let call_id = obj
            .get("call_id")
            .or_else(|| obj.get("id"))
            .and_then(|v| v.as_str())
            .map(|v| v.to_string())
            .unwrap_or_else(|| format!("call_{idx}"));
        let name = obj
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if name.trim().is_empty() {
            continue;
        }

        let arguments = obj.get("arguments").cloned().unwrap_or(Value::Null);
        let arguments = match arguments {
            Value::String(text) => serde_json::from_str(&text).unwrap_or(Value::Null),
            other => other,
        };

        out.push(ResponsesToolCall {
            call_id,
            name,
            arguments,
        });
    }

    out
}

async fn execute_mcp_tool_calls(
    state: &GatewayHttpState,
    server_ids: &[String],
    tool_calls: &[ResponsesToolCall],
) -> Vec<String> {
    let mut out = Vec::with_capacity(tool_calls.len());
    for call in tool_calls {
        let result = mcp_call_tool(state, server_ids, &call.name, call.arguments.clone())
            .await
            .unwrap_or_else(|err| Value::String(format!("MCP tool call failed: {err}")));
        out.push(mcp_tool_result_to_text(&result));
    }
    out
}

struct McpResponsesToolLoopParams<'a> {
    request_id: &'a str,
    request_json: &'a Value,
    server_ids: &'a [String],
    tools_for_llm: Vec<Value>,
    initial_tool_calls: &'a [ResponsesToolCall],
    initial_tool_results: &'a [String],
    max_steps: usize,
}

async fn follow_up_via_chat_completions_to_responses(
    state: &GatewayHttpState,
    parts: &axum::http::request::Parts,
    params: McpResponsesToolLoopParams<'_>,
) -> Result<axum::response::Response, OpenAiCompatProxyError> {
    let original_stream = params
        .request_json
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut request_with_tools = params.request_json.clone();
    set_json_tools(&mut request_with_tools, params.tools_for_llm);

    let chat_req = responses_shim::responses_request_to_chat_completions(&request_with_tools)
        .ok_or_else(|| {
            openai_error(
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                Some("invalid_mcp_request"),
                "missing input/messages",
            )
        })?;
    let mut chat_req = chat_req;

    let Some(messages) = chat_req.get("messages").and_then(|v| v.as_array()).cloned() else {
        return Err(openai_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            Some("invalid_mcp_request"),
            "missing messages",
        ));
    };

    let mut messages = messages;
    let mut messages_bytes = estimate_messages_bytes(&messages);
    let max_messages_bytes = messages_bytes.saturating_add(MCP_AUTO_EXEC_MAX_ADDED_MESSAGES_BYTES);

    let tool_calls_value = params
        .initial_tool_calls
        .iter()
        .map(|call| {
            let args = match &call.arguments {
                Value::Null => "{}".to_string(),
                other => serde_json::to_string(other).unwrap_or_else(|_| "{}".to_string()),
            };
            serde_json::json!({
                "id": call.call_id.clone(),
                "type": "function",
                "function": { "name": call.name.clone(), "arguments": args }
            })
        })
        .collect::<Vec<_>>();

    push_message_with_limit(
        &mut messages,
        &mut messages_bytes,
        max_messages_bytes,
        serde_json::json!({
            "role": "assistant",
            "content": "",
            "tool_calls": tool_calls_value,
        }),
    )?;

    for (call, output) in params
        .initial_tool_calls
        .iter()
        .zip(params.initial_tool_results.iter())
    {
        push_message_with_limit(
            &mut messages,
            &mut messages_bytes,
            max_messages_bytes,
            serde_json::json!({
                "role": "tool",
                "tool_call_id": call.call_id.clone(),
                "content": output,
            }),
        )?;
    }

    let mut tool_rounds_executed: usize = 1;
    loop {
        if tool_rounds_executed >= params.max_steps {
            let Some(obj) = chat_req.as_object_mut() else {
                return Err(openai_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request_error",
                    Some("invalid_mcp_request"),
                    "invalid /responses request",
                ));
            };
            obj.insert("messages".to_string(), Value::Array(messages));
            obj.insert("stream".to_string(), Value::Bool(original_stream));

            let response = call_openai_compat_proxy_with_body_and_path(
                state,
                parts,
                params.request_id,
                &chat_req,
                original_stream,
                "/v1/chat/completions",
            )
            .await?;

            return convert_chat_response_to_responses(response, params.request_id.to_string())
                .await;
        }

        let step_request_id = format!("{}-mcp{tool_rounds_executed}", params.request_id);
        let step_response = {
            let Some(obj) = chat_req.as_object_mut() else {
                return Err(openai_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request_error",
                    Some("invalid_mcp_request"),
                    "invalid /responses request",
                ));
            };
            obj.insert(
                "messages".to_string(),
                Value::Array(std::mem::take(&mut messages)),
            );
            obj.insert("stream".to_string(), Value::Bool(false));

            call_openai_compat_proxy_with_body_and_path(
                state,
                parts,
                &step_request_id,
                &chat_req,
                false,
                "/v1/chat/completions",
            )
            .await?
        };
        let Some(obj) = chat_req.as_object_mut() else {
            return Err(openai_error(
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                Some("invalid_mcp_request"),
                "invalid /responses request",
            ));
        };
        let restored = obj.remove("messages").ok_or_else(|| {
            openai_error(
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                Some("invalid_mcp_request"),
                "invalid /responses request",
            )
        })?;
        let Value::Array(restored_messages) = restored else {
            return Err(openai_error(
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                Some("invalid_mcp_request"),
                "invalid /responses request",
            ));
        };
        messages = restored_messages;

        let (step_status, step_headers, step_body) =
            split_response(step_response, 8 * 1024 * 1024).await?;
        let value: Value = match serde_json::from_slice(&step_body) {
            Ok(value) => value,
            Err(_) => return Ok(rebuild_response(step_status, step_headers, step_body)),
        };

        let assistant_message = extract_chat_assistant_message(&value);
        let tool_calls = assistant_message
            .as_ref()
            .map(extract_chat_tool_calls_from_message)
            .unwrap_or_default();

        if tool_calls.is_empty() {
            if original_stream {
                let Some(obj) = chat_req.as_object_mut() else {
                    return Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_mcp_request"),
                        "invalid /responses request",
                    ));
                };
                obj.insert("messages".to_string(), Value::Array(messages));
                obj.insert("stream".to_string(), Value::Bool(true));
                let response = call_openai_compat_proxy_with_body_and_path(
                    state,
                    parts,
                    params.request_id,
                    &chat_req,
                    true,
                    "/v1/chat/completions",
                )
                .await?;
                return convert_chat_response_to_responses(response, params.request_id.to_string())
                    .await;
            }

            let mapped = responses_shim::chat_completions_response_to_responses(&value)
                .ok_or_else(|| {
                    openai_error(
                        StatusCode::BAD_GATEWAY,
                        "api_error",
                        Some("invalid_backend_response"),
                        "chat/completions response cannot be mapped to /responses",
                    )
                })?;
            let bytes = serde_json::to_vec(&mapped)
                .map(Bytes::from)
                .unwrap_or_else(|_| Bytes::from(mapped.to_string()));

            let mut headers = step_headers;
            headers.insert(
                "x-ditto-shim",
                axum::http::HeaderValue::from_static("responses_via_chat_completions"),
            );
            headers.insert(
                axum::http::header::CONTENT_TYPE,
                axum::http::HeaderValue::from_static("application/json"),
            );
            headers.remove("content-length");

            return Ok(rebuild_response(step_status, headers, bytes));
        }
        if tool_calls.len() > MCP_AUTO_EXEC_MAX_TOOL_CALLS_PER_ROUND {
            return Err(openai_error(
                StatusCode::BAD_GATEWAY,
                "api_error",
                Some("mcp_auto_exec_limit_exceeded"),
                format!(
                    "tool call count {} exceeded per-round limit {}",
                    tool_calls.len(),
                    MCP_AUTO_EXEC_MAX_TOOL_CALLS_PER_ROUND
                ),
            ));
        }

        if let Some(message) =
            assistant_message.or_else(|| build_chat_assistant_message_from_tool_calls(&tool_calls))
        {
            push_message_with_limit(
                &mut messages,
                &mut messages_bytes,
                max_messages_bytes,
                message,
            )?;
        }

        for call in &tool_calls {
            let result =
                mcp_call_tool(state, params.server_ids, &call.name, call.arguments.clone())
                    .await
                    .unwrap_or_else(|err| Value::String(format!("MCP tool call failed: {err}")));
            let content = mcp_tool_result_to_text(&result);
            push_message_with_limit(
                &mut messages,
                &mut messages_bytes,
                max_messages_bytes,
                serde_json::json!({
                    "role": "tool",
                    "tool_call_id": call.id.clone(),
                    "content": content,
                }),
            )?;
        }

        tool_rounds_executed = tool_rounds_executed.saturating_add(1);
    }
}

async fn call_openai_compat_proxy_with_body_and_path(
    state: &GatewayHttpState,
    parts: &axum::http::request::Parts,
    request_id: &str,
    body_json: &Value,
    stream: bool,
    path_and_query: &str,
) -> Result<axum::response::Response, (StatusCode, Json<OpenAiErrorResponse>)> {
    let mut body_json = body_json.clone();
    if let Some(obj) = body_json.as_object_mut() {
        obj.insert("stream".to_string(), Value::Bool(stream));
    }
    let bytes = match serde_json::to_vec(&body_json) {
        Ok(bytes) => Bytes::from(bytes),
        Err(err) => {
            return Err(openai_error(
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                Some("invalid_json"),
                err,
            ));
        }
    };

    let uri = path_and_query.parse::<axum::http::Uri>().map_err(|err| {
        openai_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            Some("invalid_request"),
            format!("invalid proxy uri {path_and_query:?}: {err}"),
        )
    })?;

    let mut headers = parts.headers.clone();
    headers.insert(
        "x-ditto-skip-mcp",
        axum::http::HeaderValue::from_static("1"),
    );
    if let Ok(value) = axum::http::HeaderValue::from_str(request_id) {
        headers.insert("x-request-id", value);
    }
    headers.insert(
        "content-type",
        axum::http::HeaderValue::from_static("application/json"),
    );

    let mut req = axum::http::Request::new(Body::from(bytes));
    *req.method_mut() = parts.method.clone();
    *req.uri_mut() = uri;
    *req.headers_mut() = headers;

    let fut = Box::pin(handle_openai_compat_proxy(
        State(state.clone()),
        Path(String::new()),
        req,
    ));
    fut.await
}

async fn convert_chat_response_to_responses(
    response: axum::response::Response,
    fallback_request_id: String,
) -> Result<axum::response::Response, (StatusCode, Json<OpenAiErrorResponse>)> {
    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if content_type.starts_with("text/event-stream") {
        use tokio_util::io::StreamReader;

        let (mut parts, body) = response.into_parts();
        parts.headers.insert(
            "x-ditto-shim",
            axum::http::HeaderValue::from_static("responses_via_chat_completions"),
        );
        parts.headers.insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("text/event-stream"),
        );
        parts.headers.remove("content-length");

        let data_stream = body
            .into_data_stream()
            .map(|result| result.map_err(|err| std::io::Error::other(err.to_string())));
        let reader = StreamReader::new(data_stream);
        let reader = tokio::io::BufReader::new(reader);
        let data_stream = crate::utils::sse::sse_data_stream_from_reader(reader);

        let stream =
            responses_shim::chat_completions_sse_to_responses_sse(data_stream, fallback_request_id);

        let mut response = axum::response::Response::new(Body::from_stream(stream));
        *response.status_mut() = status;
        *response.headers_mut() = parts.headers;
        return Ok(response);
    }

    let (status, mut headers, body) = split_response(response, 8 * 1024 * 1024).await?;
    let value: Value = serde_json::from_slice(&body).map_err(|err| {
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
    let bytes = serde_json::to_vec(&mapped)
        .map(Bytes::from)
        .unwrap_or_else(|_| Bytes::from(mapped.to_string()));

    headers.insert(
        "x-ditto-shim",
        axum::http::HeaderValue::from_static("responses_via_chat_completions"),
    );
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/json"),
    );
    headers.remove("content-length");

    Ok(rebuild_response(status, headers, bytes))
}
// end inline: mcp_responses.rs

#[derive(Debug, Clone)]
struct ChatToolCall {
    id: String,
    name: String,
    arguments: Value,
}

fn extract_chat_assistant_message(response: &Value) -> Option<Value> {
    response
        .get("choices")
        .and_then(|v| v.as_array())
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .cloned()
}

fn extract_chat_tool_calls_from_message(message: &Value) -> Vec<ChatToolCall> {
    let mut out = Vec::new();

    let tool_calls = message.get("tool_calls").and_then(|v| v.as_array());
    let Some(tool_calls) = tool_calls else {
        return out;
    };

    for (idx, call) in tool_calls.iter().enumerate() {
        let id = call
            .get("id")
            .and_then(|v| v.as_str())
            .map(|v| v.to_string())
            .unwrap_or_else(|| format!("call_{idx}"));
        let name = call
            .get("function")
            .and_then(|v| v.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if name.trim().is_empty() {
            continue;
        }
        let arguments_raw = call
            .get("function")
            .and_then(|v| v.get("arguments"))
            .cloned()
            .unwrap_or(Value::Null);
        let arguments = match arguments_raw {
            Value::String(text) => serde_json::from_str(&text).unwrap_or(Value::Null),
            other => other,
        };
        out.push(ChatToolCall {
            id,
            name,
            arguments,
        });
    }

    out
}

fn build_chat_assistant_message_from_tool_calls(tool_calls: &[ChatToolCall]) -> Option<Value> {
    if tool_calls.is_empty() {
        return None;
    }

    let tool_calls_value = tool_calls
        .iter()
        .map(|call| {
            let args = match &call.arguments {
                Value::Null => "{}".to_string(),
                other => serde_json::to_string(other).unwrap_or_else(|_| "{}".to_string()),
            };
            serde_json::json!({
                "id": call.id.clone(),
                "type": "function",
                "function": { "name": call.name.clone(), "arguments": args }
            })
        })
        .collect::<Vec<_>>();

    Some(serde_json::json!({
        "role": "assistant",
        "content": "",
        "tool_calls": tool_calls_value,
    }))
}

fn mcp_tool_result_to_text(result: &Value) -> String {
    if let Some(text) = result.as_str() {
        return truncate_utf8_with_suffix(text, MCP_AUTO_EXEC_MAX_TOOL_RESULT_TEXT_BYTES);
    }

    if let Some(content) = result.get("content").and_then(|v| v.as_array()) {
        let mut assembled = String::new();
        let raw_limit =
            MCP_AUTO_EXEC_MAX_TOOL_RESULT_TEXT_BYTES.saturating_add(TRUNCATE_SUFFIX.len());
        let mut has_text = false;
        for item in content {
            if item.get("type").and_then(|v| v.as_str()) == Some("text") {
                if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                    has_text = true;
                    if !assembled.is_empty() {
                        append_with_utf8_limit(&mut assembled, "\n", raw_limit);
                    }
                    append_with_utf8_limit(&mut assembled, text, raw_limit);
                    if assembled.len() >= raw_limit {
                        break;
                    }
                }
            }
        }
        if has_text {
            return truncate_utf8_with_suffix(&assembled, MCP_AUTO_EXEC_MAX_TOOL_RESULT_TEXT_BYTES);
        }
    }

    let serialized = serde_json::to_string(result).unwrap_or_else(|_| "tool executed".to_string());
    truncate_utf8_with_suffix(&serialized, MCP_AUTO_EXEC_MAX_TOOL_RESULT_TEXT_BYTES)
}

fn estimate_messages_bytes(messages: &[Value]) -> usize {
    messages.iter().map(json_encoded_size).sum()
}

fn json_encoded_size(value: &Value) -> usize {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .unwrap_or_else(|_| value.to_string().len())
}

fn push_message_with_limit(
    messages: &mut Vec<Value>,
    current_bytes: &mut usize,
    max_bytes: usize,
    message: Value,
) -> OpenAiCompatProxyResult<()> {
    let message_bytes = json_encoded_size(&message);
    let next = current_bytes.saturating_add(message_bytes);
    if next > max_bytes {
        return Err(openai_error(
            StatusCode::BAD_GATEWAY,
            "api_error",
            Some("mcp_auto_exec_limit_exceeded"),
            format!(
                "MCP auto-exec context exceeded byte budget (max={}, next={next})",
                max_bytes
            ),
        ));
    }
    messages.push(message);
    *current_bytes = next;
    Ok(())
}

const TRUNCATE_SUFFIX: &str = "...[truncated]";

fn truncate_utf8_with_suffix(input: &str, max_bytes: usize) -> String {
    if input.len() <= max_bytes {
        return input.to_string();
    }
    if max_bytes == 0 {
        return String::new();
    }
    if max_bytes <= TRUNCATE_SUFFIX.len() {
        let mut end = max_bytes;
        while end > 0 && !input.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        return input[..end].to_string();
    }

    let mut end = max_bytes - TRUNCATE_SUFFIX.len();
    while end > 0 && !input.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    let mut out = String::with_capacity(end.saturating_add(TRUNCATE_SUFFIX.len()));
    out.push_str(&input[..end]);
    out.push_str(TRUNCATE_SUFFIX);
    out
}

fn append_with_utf8_limit(out: &mut String, chunk: &str, max_bytes: usize) {
    if out.len() >= max_bytes {
        return;
    }
    let remaining = max_bytes - out.len();
    if chunk.len() <= remaining {
        out.push_str(chunk);
        return;
    }

    let mut end = remaining;
    while end > 0 && !chunk.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    out.push_str(&chunk[..end]);
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::json;

    use super::{
        MCP_AUTO_EXEC_MAX_TOOL_RESULT_TEXT_BYTES, estimate_messages_bytes, mcp_tool_result_to_text,
        parse_mcp_server_selector, tool_name_allowed, truncate_utf8_with_suffix,
    };

    #[test]
    fn tool_name_allowed_accepts_hyphenated_server_prefix() {
        let allowed = BTreeSet::from(["hello".to_string()]);
        assert!(tool_name_allowed(
            &json!({"name": "alpha-1-hello"}),
            &allowed
        ));
    }

    #[test]
    fn tool_name_allowed_accepts_hyphenated_tool_name() {
        let allowed = BTreeSet::from(["tool-a".to_string()]);
        assert!(tool_name_allowed(
            &json!({"name": "alpha-1-tool-a"}),
            &allowed
        ));
    }

    #[test]
    fn tool_name_allowed_rejects_non_matching_suffix() {
        let allowed = BTreeSet::from(["hello".to_string()]);
        assert!(!tool_name_allowed(
            &json!({"name": "alpha-1-world"}),
            &allowed
        ));
    }

    #[test]
    fn mcp_tool_result_to_text_truncates_large_text() {
        let source = "a".repeat(MCP_AUTO_EXEC_MAX_TOOL_RESULT_TEXT_BYTES.saturating_add(64));
        let out = mcp_tool_result_to_text(&json!(source));
        assert!(out.len() <= MCP_AUTO_EXEC_MAX_TOOL_RESULT_TEXT_BYTES);
        assert!(out.ends_with("...[truncated]"));
    }

    #[test]
    fn truncate_utf8_with_suffix_preserves_char_boundaries() {
        let source = "你好世界".repeat(32);
        let out = truncate_utf8_with_suffix(&source, 17);
        assert!(out.is_char_boundary(out.len()));
    }

    #[test]
    fn estimate_messages_bytes_counts_serialized_size() {
        let messages = vec![
            json!({"role": "user", "content": "hello"}),
            json!({"role": "assistant", "content": "world"}),
        ];
        let size = estimate_messages_bytes(&messages);
        assert!(size > 0);
    }

    #[test]
    fn parse_mcp_server_selector_requires_litellm_proxy_boundary() {
        assert_eq!(
            parse_mcp_server_selector("litellm_proxy/mcp/server-a"),
            Some(vec!["server-a".to_string()])
        );
        assert_eq!(
            parse_mcp_server_selector("litellm_proxyabc/mcp/server-a"),
            None
        );
    }

    #[test]
    fn mcp_tool_result_to_text_truncates_large_content_array_without_joining_all() {
        let large = "a".repeat(MCP_AUTO_EXEC_MAX_TOOL_RESULT_TEXT_BYTES);
        let result = json!({
            "content": [
                { "type": "text", "text": large },
                { "type": "text", "text": large },
                { "type": "text", "text": large }
            ]
        });
        let out = mcp_tool_result_to_text(&result);
        assert!(out.len() <= MCP_AUTO_EXEC_MAX_TOOL_RESULT_TEXT_BYTES);
        assert!(out.ends_with("...[truncated]"));
    }
}
// end inline: openai_compat_proxy/mcp.rs
// inlined from openai_compat_proxy/proxy_cache_hit.rs
#[cfg(feature = "gateway-proxy-cache")]
async fn maybe_handle_proxy_cache_hit(
    state: &GatewayHttpState,
    cache_key: Option<&str>,
    request_id: &str,
    path_and_query: &str,
    now_epoch_seconds: u64,
    _metrics: Option<(&str, std::time::Instant)>,
) -> Option<axum::response::Response> {
    let (Some(cache), Some(cache_key)) = (state.proxy.cache.as_ref(), cache_key) else {
        return None;
    };

    #[cfg(feature = "gateway-metrics-prometheus")]
    if let (Some(metrics), Some((metrics_path, _))) = (state.proxy.metrics.as_ref(), _metrics) {
        metrics.lock().await.record_proxy_cache_lookup(metrics_path);
    }

    #[cfg(feature = "gateway-store-redis")]
    let mut cache_source = "memory";
    #[cfg(not(feature = "gateway-store-redis"))]
    let cache_source = "memory";

    #[cfg(feature = "gateway-store-redis")]
    let mut cached = { cache.lock().await.get(cache_key, now_epoch_seconds) };
    #[cfg(not(feature = "gateway-store-redis"))]
    let cached = { cache.lock().await.get(cache_key, now_epoch_seconds) };

    #[cfg(feature = "gateway-store-redis")]
    if cached.is_none() {
        if let Some(store) = state.stores.redis.as_ref() {
            if let Ok(Some(redis_cached)) = store.get_proxy_cache_response(cache_key).await {
                cache_source = "redis";
                let mut cache = cache.lock().await;
                cache.insert_with_metadata(
                    cache_key.to_string(),
                    redis_cached.response.clone(),
                    redis_cached.metadata,
                    now_epoch_seconds,
                );
                cached = Some(redis_cached.response);
            }
        }
    }

    let Some(cached) = cached else {
        #[cfg(feature = "gateway-metrics-prometheus")]
        if let (Some(metrics), Some((metrics_path, _))) = (state.proxy.metrics.as_ref(), _metrics) {
            metrics.lock().await.record_proxy_cache_miss(metrics_path);
        }
        return None;
    };

    state.record_cache_hit();

    emit_json_log(
        state,
        "proxy.cache_hit",
        serde_json::json!({
            "request_id": request_id,
            "cache": cache_source,
            "backend": &cached.backend,
            "path": path_and_query,
        }),
    );

    #[cfg(feature = "gateway-otel")]
    {
        let span = tracing::Span::current();
        span.record("cache", tracing::field::display("hit"));
        span.record("backend", tracing::field::display(&cached.backend));
        span.record("status", tracing::field::display(cached.status));
    }

    #[cfg(feature = "gateway-metrics-prometheus")]
    if let (Some(metrics), Some((metrics_path, metrics_timer_start))) =
        (state.proxy.metrics.as_ref(), _metrics)
    {
        let mut metrics = metrics.lock().await;
        metrics.record_proxy_cache_hit();
        metrics.record_proxy_cache_hit_by_source(cache_source);
        metrics.record_proxy_cache_hit_by_path(metrics_path);
        metrics.record_proxy_response_status_by_path(metrics_path, cached.status);
        metrics.record_proxy_response_status_by_backend(&cached.backend, cached.status);
        metrics.observe_proxy_request_duration(metrics_path, metrics_timer_start.elapsed());
    }

    let mut response = cached_proxy_response(cached, request_id.to_string());
    if let Ok(value) = axum::http::HeaderValue::from_str(cache_key) {
        response.headers_mut().insert("x-ditto-cache-key", value);
    }
    if let Ok(value) = axum::http::HeaderValue::from_str(cache_source) {
        response.headers_mut().insert("x-ditto-cache-source", value);
    }
    Some(response)
}
// end inline: openai_compat_proxy/proxy_cache_hit.rs
// inlined from openai_compat_proxy/proxy_failure.rs
#[allow(dead_code)]
struct ProxyFailureContext<'a> {
    request_id: &'a str,
    method: &'a axum::http::Method,
    path_and_query: &'a str,
    model: &'a Option<String>,
    virtual_key_id: Option<&'a str>,
    attempted_backends: &'a [String],
    body_len: usize,
    charge_tokens: u32,
    charge_cost_usd_micros: Option<u64>,
    last_err: Option<(StatusCode, Json<OpenAiErrorResponse>)>,
    metrics: Option<(&'a str, std::time::Instant)>,
}

async fn finalize_openai_compat_proxy_failure(
    state: &GatewayHttpState,
    ctx: ProxyFailureContext<'_>,
) -> (StatusCode, Json<OpenAiErrorResponse>) {
    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    {
        let (status, err_kind, err_code, err_message) = match ctx.last_err.as_ref() {
            Some((status, body)) => (
                Some(status.as_u16()),
                Some(body.0.error.kind),
                body.0.error.code,
                Some(body.0.error.message.as_str()),
            ),
            None => (None, None, None, None),
        };

        let payload = serde_json::json!({
            "request_id": ctx.request_id,
            "virtual_key_id": ctx.virtual_key_id,
            "attempted_backends": ctx.attempted_backends,
            "method": ctx.method.as_str(),
            "path": ctx.path_and_query,
            "model": ctx.model,
            "charge_tokens": ctx.charge_tokens,
            "charge_cost_usd_micros": ctx.charge_cost_usd_micros,
            "body_len": ctx.body_len,
            "status": status,
            "error_type": err_kind,
            "error_code": err_code,
            "error_message": err_message,
        });
        append_audit_log(state, "proxy.error", payload).await;
    }

    emit_json_log(
        state,
        "proxy.error",
        serde_json::json!({
            "request_id": ctx.request_id,
            "attempted_backends": ctx.attempted_backends,
            "status": ctx.last_err.as_ref().map(|(status, _)| status.as_u16()),
        }),
    );

    #[cfg(feature = "gateway-metrics-prometheus")]
    if let (Some(metrics), Some((metrics_path, metrics_timer_start))) =
        (state.proxy.metrics.as_ref(), ctx.metrics)
    {
        let status = ctx
            .last_err
            .as_ref()
            .map(|(status, _)| status.as_u16())
            .unwrap_or(StatusCode::BAD_GATEWAY.as_u16());
        let duration = metrics_timer_start.elapsed();
        let mut metrics = metrics.lock().await;
        metrics.record_proxy_response_status_by_path(metrics_path, status);
        if let Some(model) = ctx.model.as_deref() {
            metrics.record_proxy_response_status_by_model(model, status);
            metrics.observe_proxy_request_duration_by_model(model, duration);
        }
        metrics.observe_proxy_request_duration(metrics_path, duration);
    }

    ctx.last_err.unwrap_or_else(|| {
        openai_error(
            StatusCode::BAD_GATEWAY,
            "api_error",
            Some("backend_error"),
            "all backends failed",
        )
    })
}
// end inline: openai_compat_proxy/proxy_failure.rs

fn is_non_billable_openai_meta_path(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    matches!(
        path,
        "/v1/responses/input_tokens" | "/v1/responses/input_tokens/"
    )
}

async fn handle_openai_compat_proxy(
    State(state): State<GatewayHttpState>,
    Path(_path): Path<String>,
    req: axum::http::Request<Body>,
) -> Result<axum::response::Response, (StatusCode, Json<OpenAiErrorResponse>)> {
    let max_body_bytes = state.proxy.max_body_bytes;
    let (parts, incoming_body) = req.into_parts();
    let request_id =
        extract_header(&parts.headers, "x-request-id").unwrap_or_else(generate_request_id);
    let path_and_query = parts
        .uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or_else(|| parts.uri.path());
    let normalized_path_and_query = normalize_openai_compat_path_and_query(path_and_query);
    let path_and_query = normalized_path_and_query.as_ref();
    #[cfg(feature = "gateway-metrics-prometheus")]
    let metrics_path = super::metrics_prometheus::normalize_proxy_path_label(path_and_query);
    #[cfg(feature = "gateway-metrics-prometheus")]
    let metrics_timer_start = Instant::now();
    #[cfg(feature = "gateway-otel")]
    let otel_path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    #[cfg(feature = "gateway-otel")]
    let proxy_span = tracing::info_span!(
        "ditto.gateway.proxy",
        request_id = %request_id,
        method = %parts.method,
        path = %otel_path,
        model = tracing::field::Empty,
        virtual_key_id = tracing::field::Empty,
        backend = tracing::field::Empty,
        status = tracing::field::Empty,
        cache = tracing::field::Empty,
    );
    #[cfg(feature = "gateway-otel")]
    let _proxy_span_guard = proxy_span.enter();
    if should_stream_large_multipart_request(&parts, path_and_query, max_body_bytes) {
        let path_and_query = path_and_query.to_string();
        return handle_openai_compat_proxy_streaming_multipart(
            state,
            parts,
            incoming_body,
            request_id,
            path_and_query,
        )
        .await;
    }
    let body = {
        let _buffering_permit = if let Some(limit) = state.proxy.backpressure.as_ref() {
            Some(limit.clone().try_acquire_owned().map_err(|_| {
                openai_error(
                    StatusCode::TOO_MANY_REQUESTS,
                    "rate_limit_error",
                    Some("inflight_limit"),
                    "too many in-flight proxy requests",
                )
            })?)
        } else {
            None
        };
        to_bytes(incoming_body, max_body_bytes)
            .await
            .map_err(|err| {
                openai_error(StatusCode::BAD_REQUEST, "invalid_request_error", None, err)
            })?
    };

    let content_type_is_json = parts
        .headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|ct| ct.to_ascii_lowercase().starts_with("application/json"));

    let parsed_json: Option<serde_json::Value> = if content_type_is_json {
        if body.is_empty() {
            None
        } else {
            Some(serde_json::from_slice(&body).map_err(|err| {
                openai_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request_error",
                    Some("invalid_json"),
                    err,
                )
            })?)
        }
    } else {
        None
    };

    if let Some(response) = maybe_handle_mcp_tools_chat_completions(
        &state,
        &parts,
        &parsed_json,
        &request_id,
        path_and_query,
    )
    .await?
    {
        return Ok(response);
    }

    let model = parsed_json
        .as_ref()
        .and_then(|value| value.get("model"))
        .and_then(|value| value.as_str())
        .map(|value| value.to_string());

    let service_tier = parsed_json
        .as_ref()
        .and_then(|value| value.get("service_tier"))
        .and_then(|value| value.as_str())
        .map(|value| value.to_string());

    #[cfg(feature = "gateway-otel")]
    if let Some(model) = model.as_deref() {
        proxy_span.record("model", tracing::field::display(model));
    }

    let max_output_tokens = parsed_json
        .as_ref()
        .and_then(|value| extract_max_output_tokens(path_and_query, value))
        .unwrap_or(0);

    let _stream_requested = parsed_json
        .as_ref()
        .and_then(|value| value.get("stream"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

    #[cfg(feature = "gateway-tokenizer")]
    let input_tokens_estimate = parsed_json
        .as_ref()
        .and_then(|json| {
            model
                .as_deref()
                .and_then(|model| token_count::estimate_input_tokens(path_and_query, model, json))
        })
        .unwrap_or_else(|| estimate_tokens_from_bytes(&body));

    #[cfg(not(feature = "gateway-tokenizer"))]
    let input_tokens_estimate = estimate_tokens_from_bytes(&body);
    let charge_tokens = if is_non_billable_openai_meta_path(path_and_query) {
        0
    } else {
        input_tokens_estimate.saturating_add(max_output_tokens)
    };

    #[cfg(feature = "gateway-store-sqlite")]
    let use_sqlite_budget = state.stores.sqlite.is_some();
    #[cfg(not(feature = "gateway-store-sqlite"))]
    let use_sqlite_budget = false;

    #[cfg(feature = "gateway-store-postgres")]
    let use_postgres_budget = state.stores.postgres.is_some();
    #[cfg(not(feature = "gateway-store-postgres"))]
    let use_postgres_budget = false;

    #[cfg(feature = "gateway-store-mysql")]
    let use_mysql_budget = state.stores.mysql.is_some();
    #[cfg(not(feature = "gateway-store-mysql"))]
    let use_mysql_budget = false;

    #[cfg(feature = "gateway-store-redis")]
    let use_redis_budget = state.stores.redis.is_some();
    #[cfg(not(feature = "gateway-store-redis"))]
    let use_redis_budget = false;

    let use_persistent_budget =
        use_sqlite_budget || use_postgres_budget || use_mysql_budget || use_redis_budget;

    let _now_epoch_seconds = now_epoch_seconds();
    let minute = _now_epoch_seconds / 60;
    #[cfg(feature = "gateway-store-redis")]
    let rate_limit_route = normalize_rate_limit_route(path_and_query);

    let ResolvedGatewayContext {
        virtual_key_id,
        limits,
        budget,
        tenant_budget_scope,
        project_budget_scope,
        user_budget_scope,
        tenant_limits_scope,
        project_limits_scope,
        user_limits_scope,
        backend_candidates,
        strip_authorization,
        charge_cost_usd_micros,
    } = resolve_openai_compat_proxy_gateway_context(
        ResolveOpenAiCompatProxyGatewayContextRequest {
            state: &state,
            parts: &parts,
            body: &body,
            parsed_json: &parsed_json,
            request_id: &request_id,
            path_and_query,
            model: &model,
            service_tier: &service_tier,
            input_tokens_estimate,
            max_output_tokens,
            charge_tokens,
            minute,
            use_redis_budget,
            use_persistent_budget,
            #[cfg(feature = "gateway-metrics-prometheus")]
            metrics_path: &metrics_path,
            #[cfg(feature = "gateway-metrics-prometheus")]
            metrics_timer_start,
        },
    )
    .await?;

    #[cfg(not(feature = "gateway-store-redis"))]
    let _ = (
        &limits,
        &tenant_limits_scope,
        &project_limits_scope,
        &user_limits_scope,
    );

    #[cfg(feature = "gateway-store-redis")]
    if let (Some(store), Some(virtual_key_id), Some(limits)) = (
        state.stores.redis.as_ref(),
        virtual_key_id.as_deref(),
        limits.as_ref(),
    ) {
        if let Err(err) = store
            .check_and_consume_rate_limits(
                virtual_key_id,
                &rate_limit_route,
                limits,
                charge_tokens,
                _now_epoch_seconds,
            )
            .await
        {
            let is_rate_limited = matches!(err, GatewayError::RateLimited { .. });
            if is_rate_limited {
                state.record_rate_limited();
            }
            let mapped = map_openai_gateway_error(err);
            #[cfg(feature = "gateway-metrics-prometheus")]
            if is_rate_limited {
                if let Some(metrics) = state.proxy.metrics.as_ref() {
                    let duration = metrics_timer_start.elapsed();
                    let status = mapped.0.as_u16();
                    let mut metrics = metrics.lock().await;
                    metrics.record_proxy_request(
                        Some(virtual_key_id),
                        model.as_deref(),
                        &metrics_path,
                    );
                    metrics.record_proxy_rate_limited(
                        Some(virtual_key_id),
                        model.as_deref(),
                        &metrics_path,
                    );
                    metrics.record_proxy_response_status_by_path(&metrics_path, status);
                    if let Some(model) = model.as_deref() {
                        metrics.record_proxy_response_status_by_model(model, status);
                        metrics.observe_proxy_request_duration_by_model(model, duration);
                    }
                    metrics.observe_proxy_request_duration(&metrics_path, duration);
                }
            }
            return Err(mapped);
        }
    }

    #[cfg(feature = "gateway-store-redis")]
    if let (Some(store), Some((scope, limits))) =
        (state.stores.redis.as_ref(), tenant_limits_scope.as_ref())
    {
        if let Err(err) = store
            .check_and_consume_rate_limits(
                scope,
                &rate_limit_route,
                limits,
                charge_tokens,
                _now_epoch_seconds,
            )
            .await
        {
            let is_rate_limited = matches!(err, GatewayError::RateLimited { .. });
            if is_rate_limited {
                state.record_rate_limited();
            }
            let mapped = map_openai_gateway_error(err);
            #[cfg(feature = "gateway-metrics-prometheus")]
            if is_rate_limited {
                if let Some(metrics) = state.proxy.metrics.as_ref() {
                    let duration = metrics_timer_start.elapsed();
                    let status = mapped.0.as_u16();
                    let mut metrics = metrics.lock().await;
                    metrics.record_proxy_request(
                        virtual_key_id.as_deref(),
                        model.as_deref(),
                        &metrics_path,
                    );
                    metrics.record_proxy_rate_limited(
                        virtual_key_id.as_deref(),
                        model.as_deref(),
                        &metrics_path,
                    );
                    metrics.record_proxy_response_status_by_path(&metrics_path, status);
                    if let Some(model) = model.as_deref() {
                        metrics.record_proxy_response_status_by_model(model, status);
                        metrics.observe_proxy_request_duration_by_model(model, duration);
                    }
                    metrics.observe_proxy_request_duration(&metrics_path, duration);
                }
            }
            return Err(mapped);
        }
    }

    #[cfg(feature = "gateway-store-redis")]
    if let (Some(store), Some((scope, limits))) =
        (state.stores.redis.as_ref(), project_limits_scope.as_ref())
    {
        if let Err(err) = store
            .check_and_consume_rate_limits(
                scope,
                &rate_limit_route,
                limits,
                charge_tokens,
                _now_epoch_seconds,
            )
            .await
        {
            let is_rate_limited = matches!(err, GatewayError::RateLimited { .. });
            if is_rate_limited {
                state.record_rate_limited();
            }
            let mapped = map_openai_gateway_error(err);
            #[cfg(feature = "gateway-metrics-prometheus")]
            if is_rate_limited {
                if let Some(metrics) = state.proxy.metrics.as_ref() {
                    let duration = metrics_timer_start.elapsed();
                    let status = mapped.0.as_u16();
                    let mut metrics = metrics.lock().await;
                    metrics.record_proxy_request(
                        virtual_key_id.as_deref(),
                        model.as_deref(),
                        &metrics_path,
                    );
                    metrics.record_proxy_rate_limited(
                        virtual_key_id.as_deref(),
                        model.as_deref(),
                        &metrics_path,
                    );
                    metrics.record_proxy_response_status_by_path(&metrics_path, status);
                    if let Some(model) = model.as_deref() {
                        metrics.record_proxy_response_status_by_model(model, status);
                        metrics.observe_proxy_request_duration_by_model(model, duration);
                    }
                    metrics.observe_proxy_request_duration(&metrics_path, duration);
                }
            }
            return Err(mapped);
        }
    }

    #[cfg(feature = "gateway-store-redis")]
    if let (Some(store), Some((scope, limits))) =
        (state.stores.redis.as_ref(), user_limits_scope.as_ref())
    {
        if let Err(err) = store
            .check_and_consume_rate_limits(
                scope,
                &rate_limit_route,
                limits,
                charge_tokens,
                _now_epoch_seconds,
            )
            .await
        {
            let is_rate_limited = matches!(err, GatewayError::RateLimited { .. });
            if is_rate_limited {
                state.record_rate_limited();
            }
            let mapped = map_openai_gateway_error(err);
            #[cfg(feature = "gateway-metrics-prometheus")]
            if is_rate_limited {
                if let Some(metrics) = state.proxy.metrics.as_ref() {
                    let duration = metrics_timer_start.elapsed();
                    let status = mapped.0.as_u16();
                    let mut metrics = metrics.lock().await;
                    metrics.record_proxy_request(
                        virtual_key_id.as_deref(),
                        model.as_deref(),
                        &metrics_path,
                    );
                    metrics.record_proxy_rate_limited(
                        virtual_key_id.as_deref(),
                        model.as_deref(),
                        &metrics_path,
                    );
                    metrics.record_proxy_response_status_by_path(&metrics_path, status);
                    if let Some(model) = model.as_deref() {
                        metrics.record_proxy_response_status_by_model(model, status);
                        metrics.observe_proxy_request_duration_by_model(model, duration);
                    }
                    metrics.observe_proxy_request_duration(&metrics_path, duration);
                }
            }
            return Err(mapped);
        }
    }

    #[cfg(feature = "gateway-otel")]
    if let Some(virtual_key_id) = virtual_key_id.as_deref() {
        proxy_span.record("virtual_key_id", tracing::field::display(virtual_key_id));
    }

    #[cfg(feature = "gateway-metrics-prometheus")]
    if let Some(metrics) = state.proxy.metrics.as_ref() {
        metrics.lock().await.record_proxy_request(
            virtual_key_id.as_deref(),
            model.as_deref(),
            &metrics_path,
        );
    }

    #[cfg(feature = "gateway-routing-advanced")]
    let backend_candidates =
        filter_backend_candidates_by_health(&state, backend_candidates, _now_epoch_seconds).await;

    #[cfg(feature = "gateway-proxy-cache")]
    let streaming_cache_enabled = state
        .proxy
        .cache_config
        .as_ref()
        .is_some_and(ProxyCacheConfig::streaming_cache_enabled);

    #[cfg(feature = "gateway-proxy-cache")]
    let (proxy_cache_key, proxy_cache_metadata) = if state.proxy.cache.is_some()
        && proxy_cache_can_read(&parts.method)
        && (!_stream_requested || streaming_cache_enabled)
        && !proxy_cache_bypass(&parts.headers)
        && (parts.method == axum::http::Method::GET || parsed_json.is_some())
    {
        let scope = proxy_cache_scope(virtual_key_id.as_deref(), &parts.headers);
        (
            Some(proxy_cache_key(
                &parts.method,
                path_and_query,
                &body,
                &scope,
                &parts.headers,
            )),
            Some(ProxyCacheEntryMetadata::new(
                scope,
                &parts.method,
                path_and_query,
                model.as_deref(),
            )),
        )
    } else {
        (None, None)
    };

    #[cfg(feature = "gateway-proxy-cache")]
    {
        #[cfg(feature = "gateway-metrics-prometheus")]
        let proxy_metrics = Some((metrics_path.as_str(), metrics_timer_start));
        #[cfg(not(feature = "gateway-metrics-prometheus"))]
        let proxy_metrics = None;

        if let Some(response) = maybe_handle_proxy_cache_hit(
            &state,
            proxy_cache_key.as_deref(),
            &request_id,
            path_and_query,
            _now_epoch_seconds,
            proxy_metrics,
        )
        .await
        {
            return Ok(response);
        }
    }

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    let budget_reservation_params = ProxyBudgetReservationParams {
        state: &state,
        use_persistent_budget,
        virtual_key_id: virtual_key_id.as_deref(),
        budget: budget.as_ref(),
        tenant_budget_scope: &tenant_budget_scope,
        project_budget_scope: &project_budget_scope,
        user_budget_scope: &user_budget_scope,
        request_id: &request_id,
        path_and_query,
        model: &model,
        charge_tokens,
    };

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    let (_token_budget_reserved, token_budget_reservation_ids) =
        match reserve_proxy_token_budgets_for_request(budget_reservation_params).await {
            Ok(reserved) => reserved,
            Err(err) => {
                if err.0 == StatusCode::PAYMENT_REQUIRED {
                    state.record_budget_exceeded();
                }

                #[cfg(feature = "gateway-metrics-prometheus")]
                if let Some(metrics) = state.proxy.metrics.as_ref() {
                    let duration = metrics_timer_start.elapsed();
                    let status = err.0.as_u16();
                    let mut metrics = metrics.lock().await;
                    if err.0 == StatusCode::PAYMENT_REQUIRED {
                        metrics.record_proxy_budget_exceeded(
                            virtual_key_id.as_deref(),
                            model.as_deref(),
                            &metrics_path,
                        );
                    }
                    metrics.record_proxy_response_status_by_path(&metrics_path, status);
                    if let Some(model) = model.as_deref() {
                        metrics.record_proxy_response_status_by_model(model, status);
                        metrics.observe_proxy_request_duration_by_model(model, duration);
                    }
                    metrics.observe_proxy_request_duration(&metrics_path, duration);
                }
                return Err(err);
            }
        };

    #[cfg(not(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    )))]
    let _token_budget_reserved = false;

    #[cfg(all(
        feature = "gateway-costing",
        any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        ),
    ))]
    let (_cost_budget_reserved, cost_budget_reservation_ids) =
        match reserve_proxy_cost_budgets_for_request(
            budget_reservation_params,
            charge_cost_usd_micros,
            &token_budget_reservation_ids,
        )
        .await
        {
            Ok(reserved) => reserved,
            Err(err) => {
                if err.0 == StatusCode::PAYMENT_REQUIRED {
                    state.record_budget_exceeded();
                }

                #[cfg(feature = "gateway-metrics-prometheus")]
                if let Some(metrics) = state.proxy.metrics.as_ref() {
                    let duration = metrics_timer_start.elapsed();
                    let status = err.0.as_u16();
                    let mut metrics = metrics.lock().await;
                    if err.0 == StatusCode::PAYMENT_REQUIRED {
                        metrics.record_proxy_budget_exceeded(
                            virtual_key_id.as_deref(),
                            model.as_deref(),
                            &metrics_path,
                        );
                    }
                    metrics.record_proxy_response_status_by_path(&metrics_path, status);
                    if let Some(model) = model.as_deref() {
                        metrics.record_proxy_response_status_by_model(model, status);
                        metrics.observe_proxy_request_duration_by_model(model, duration);
                    }
                    metrics.observe_proxy_request_duration(&metrics_path, duration);
                }
                return Err(err);
            }
        };

    #[cfg(not(all(
        feature = "gateway-costing",
        any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        ),
    )))]
    let _cost_budget_reserved = false;
    emit_json_log(
        &state,
        "proxy.request",
        serde_json::json!({
            "request_id": &request_id,
            "method": parts.method.as_str(),
            "path": path_and_query,
            "model": &model,
            "virtual_key_id": virtual_key_id.as_deref(),
            "charge_tokens": charge_tokens,
            "charge_cost_usd_micros": charge_cost_usd_micros,
            "body_len": body.len(),
        }),
    );

    #[cfg(feature = "gateway-routing-advanced")]
    let retry_config = state
        .proxy
        .routing
        .as_ref()
        .map(|cfg| cfg.retry.clone())
        .unwrap_or_default();
    #[cfg(feature = "gateway-routing-advanced")]
    let max_attempts = retry_config
        .max_attempts
        .unwrap_or(backend_candidates.len())
        .max(1)
        .min(backend_candidates.len());
    #[cfg(not(feature = "gateway-routing-advanced"))]
    let max_attempts = backend_candidates.len();

    let mut last_err: Option<(StatusCode, Json<OpenAiErrorResponse>)> = None;
    let mut attempted_backends: Vec<String> = Vec::new();

    let attempt_params = ProxyAttemptParams {
        state: &state,
        parts: &parts,
        body: &body,
        parsed_json: &parsed_json,
        model: &model,
        service_tier: &service_tier,
        request_id: &request_id,
        path_and_query,
        now_epoch_seconds: _now_epoch_seconds,
        charge_tokens,
        stream_requested: _stream_requested,
        strip_authorization,
        use_persistent_budget,
        virtual_key_id: &virtual_key_id,
        budget: &budget,
        tenant_budget_scope: &tenant_budget_scope,
        project_budget_scope: &project_budget_scope,
        user_budget_scope: &user_budget_scope,
        charge_cost_usd_micros,
        #[cfg(any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        ))]
        token_budget_reservation_ids: &token_budget_reservation_ids,
        cost_budget_reserved: _cost_budget_reserved,
        #[cfg(all(
            feature = "gateway-costing",
            any(
                feature = "gateway-store-sqlite",
                feature = "gateway-store-postgres",
                feature = "gateway-store-mysql",
                feature = "gateway-store-redis"
            ),
        ))]
        cost_budget_reservation_ids: &cost_budget_reservation_ids,
        max_attempts,
        #[cfg(feature = "gateway-routing-advanced")]
        retry_config: &retry_config,
        #[cfg(feature = "gateway-proxy-cache")]
        proxy_cache_key: &proxy_cache_key,
        #[cfg(feature = "gateway-proxy-cache")]
        proxy_cache_metadata: &proxy_cache_metadata,
        #[cfg(feature = "gateway-metrics-prometheus")]
        metrics_path: &metrics_path,
        #[cfg(feature = "gateway-metrics-prometheus")]
        metrics_timer_start,
    };

    for (idx, backend_name) in backend_candidates.into_iter().enumerate() {
        if idx >= max_attempts {
            break;
        }

        attempted_backends.push(backend_name.clone());

        #[cfg(feature = "gateway-translation")]
        if let Some(translation_backend) = state
            .backends
            .translation_backends
            .get(&backend_name)
            .cloned()
        {
            match attempt_translation_backend(
                attempt_params,
                &backend_name,
                translation_backend,
                &attempted_backends,
            )
            .await?
            {
                BackendAttemptOutcome::Response(response) => return Ok(response),
                BackendAttemptOutcome::Continue(err) => {
                    if let Some(err) = err {
                        last_err = Some(err);
                    }
                    continue;
                }
                BackendAttemptOutcome::Stop(err) => {
                    last_err = Some(err);
                    break;
                }
            }
        }

        match attempt_proxy_backend(attempt_params, &backend_name, idx, &attempted_backends).await?
        {
            BackendAttemptOutcome::Response(response) => return Ok(response),
            BackendAttemptOutcome::Continue(err) => {
                if let Some(err) = err {
                    last_err = Some(err);
                }
                continue;
            }
            BackendAttemptOutcome::Stop(err) => {
                last_err = Some(err);
                break;
            }
        }
    }

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    rollback_proxy_token_budget_reservations(&state, &token_budget_reservation_ids).await;

    #[cfg(all(
        feature = "gateway-costing",
        any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        ),
    ))]
    rollback_proxy_cost_budget_reservations(&state, &cost_budget_reservation_ids).await;

    #[cfg(feature = "gateway-metrics-prometheus")]
    let proxy_metrics = Some((metrics_path.as_str(), metrics_timer_start));
    #[cfg(not(feature = "gateway-metrics-prometheus"))]
    let proxy_metrics = None;

    Err(finalize_openai_compat_proxy_failure(
        &state,
        ProxyFailureContext {
            request_id: &request_id,
            method: &parts.method,
            path_and_query,
            model: &model,
            virtual_key_id: virtual_key_id.as_deref(),
            attempted_backends: &attempted_backends,
            body_len: body.len(),
            charge_tokens,
            charge_cost_usd_micros,
            last_err,
            metrics: proxy_metrics,
        },
    )
    .await)
}
// end inline: ../../http/openai_compat_proxy.rs
// inlined from ../../http/openai_models.rs
async fn handle_openai_models_list(
    State(state): State<GatewayHttpState>,
    req: axum::http::Request<Body>,
) -> Result<axum::response::Response, (StatusCode, Json<OpenAiErrorResponse>)> {
    const PER_BACKEND_TIMEOUT_SECS: u64 = 10;
    const PER_BACKEND_MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

    let (parts, _body) = req.into_parts();

    let request_id =
        extract_header(&parts.headers, "x-request-id").unwrap_or_else(generate_request_id);
    #[cfg(feature = "gateway-translation")]
    let created = now_epoch_seconds();

    let (strip_authorization, key_route) = if !state.uses_virtual_keys() {
        (false, None)
    } else {
        let token = extract_virtual_key(&parts.headers).ok_or_else(|| {
            openai_error(
                StatusCode::UNAUTHORIZED,
                "authentication_error",
                Some("invalid_api_key"),
                "missing virtual key",
            )
        })?;
        let key = state
            .virtual_key_by_token(&token)
            .filter(|key| key.enabled)
            .ok_or_else(|| {
                openai_error(
                    StatusCode::UNAUTHORIZED,
                    "authentication_error",
                    Some("invalid_api_key"),
                    "unauthorized virtual key",
                )
            })?;
        (true, key.route)
    };

    let mut base_headers = parts.headers.clone();
    sanitize_proxy_headers(&mut base_headers, strip_authorization);
    insert_request_id(&mut base_headers, &request_id);

    let mut backends: Vec<(String, ProxyBackend)> = state
        .backends
        .proxy_backends
        .iter()
        .map(|(name, backend)| (name.clone(), backend.clone()))
        .collect();
    backends.sort_by(|(a, _), (b, _)| a.cmp(b));

    if let Some(route) = key_route.as_deref() {
        if let Some((name, backend)) = backends.into_iter().find(|(name, _)| name == route) {
            backends = vec![(name, backend)];
        } else {
            #[cfg(feature = "gateway-translation")]
            if state.backends.translation_backends.contains_key(route) {
                backends = Vec::new();
            } else {
                return Err(openai_error(
                    StatusCode::BAD_GATEWAY,
                    "api_error",
                    Some("backend_not_found"),
                    format!("backend not found: {route}"),
                ));
            }

            #[cfg(not(feature = "gateway-translation"))]
            return Err(openai_error(
                StatusCode::BAD_GATEWAY,
                "api_error",
                Some("backend_not_found"),
                format!("backend not found: {route}"),
            ));
        }
    }

    let had_proxy_backends = !backends.is_empty();
    let results = futures_util::future::join_all(backends.into_iter().map(|(name, backend)| {
        let mut headers = base_headers.clone();
        apply_backend_headers(&mut headers, backend.headers());
        let timeout = std::time::Duration::from_secs(PER_BACKEND_TIMEOUT_SECS);
        async move {
            let response = backend
                .request_with_timeout(
                    reqwest::Method::GET,
                    "/v1/models",
                    headers,
                    None,
                    Some(timeout),
                )
                .await;
            (name, response)
        }
    }))
    .await;

    let mut models_by_id: std::collections::BTreeMap<String, serde_json::Value> =
        std::collections::BTreeMap::new();
    for (backend_name, result) in results {
        let response = match result {
            Ok(response) => response,
            Err(_) => continue,
        };
        if !response.status().is_success() {
            continue;
        }
        let headers = response.headers().clone();
        let bytes = match read_reqwest_body_bytes_bounded_with_content_length(
            response,
            &headers,
            PER_BACKEND_MAX_BODY_BYTES,
        )
        .await
        {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };
        let json: serde_json::Value = match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let models = json
            .get("data")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        for model in models {
            let Some(id) = model.get("id").and_then(serde_json::Value::as_str) else {
                continue;
            };
            models_by_id.entry(id.to_string()).or_insert(model);
        }
        emit_json_log(
            &state,
            "models.backend_ok",
            serde_json::json!({
                "request_id": &request_id,
                "backend": &backend_name,
                "models": models_by_id.len(),
            }),
        );
    }

    #[cfg(feature = "gateway-translation")]
    let has_translation_backends = !state.backends.translation_backends.is_empty();

    #[cfg(feature = "gateway-translation")]
    if has_translation_backends {
        let models = super::translation::collect_models_from_translation_backends(
            state.backends.translation_backends.as_ref(),
        );
        for (id, owned_by) in models {
            models_by_id
                .entry(id.to_string())
                .or_insert_with(|| super::translation::model_to_openai(&id, &owned_by, created));
        }
    }

    if models_by_id.is_empty() {
        if !had_proxy_backends {
            return Err(openai_error(
                StatusCode::BAD_GATEWAY,
                "api_error",
                Some("backend_not_found"),
                "no proxy backends configured",
            ));
        }
        return Err(openai_error(
            StatusCode::BAD_GATEWAY,
            "api_error",
            Some("backend_error"),
            "all upstream /v1/models requests failed",
        ));
    }

    let response_json = serde_json::json!({
        "object": "list",
        "data": models_by_id.into_values().collect::<Vec<_>>(),
    });
    let bytes = serde_json::to_vec(&response_json)
        .unwrap_or_else(|_| response_json.to_string().into_bytes());

    let mut response = axum::response::Response::new(Body::from(bytes));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/json"),
    );
    #[cfg(feature = "gateway-translation")]
    if has_translation_backends {
        response.headers_mut().insert(
            "x-ditto-translation",
            axum::http::HeaderValue::from_static("multi"),
        );
    }
    if let Ok(value) = axum::http::HeaderValue::from_str(&request_id) {
        response.headers_mut().insert("x-request-id", value);
    }
    Ok(response)
}
// end inline: ../../http/openai_models.rs
// inlined from ../../http/proxy.rs
// inlined from proxy/core.rs
// This file is intentionally split to keep each staged Rust file under the pre-commit size limit.
// inlined from core/schema.rs
#[cfg(feature = "gateway-routing-advanced")]
use std::time::Duration;

fn extract_max_output_tokens(path: &str, value: &serde_json::Value) -> Option<u32> {
    let key = if path.starts_with("/v1/responses") {
        "max_output_tokens"
    } else {
        "max_tokens"
    };

    value.get(key).and_then(|v| v.as_u64()).map(|v| {
        if v > u64::from(u32::MAX) {
            u32::MAX
        } else {
            v as u32
        }
    })
}

fn validate_openai_request_schema(
    path_and_query: &str,
    body: &serde_json::Value,
) -> Option<String> {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);

    if path == "/v1/chat/completions" {
        return validate_openai_chat_completions_schema(body);
    }
    if path == "/v1/embeddings" {
        return validate_openai_embeddings_schema(body);
    }
    if path.starts_with("/v1/responses") {
        return validate_openai_responses_schema(body);
    }
    if path == "/v1/completions" {
        return validate_openai_completions_schema(body);
    }
    if path == "/v1/moderations" {
        return validate_openai_moderations_schema(body);
    }
    if path == "/v1/images/generations" {
        return validate_openai_images_generations_schema(body);
    }
    if path == "/v1/audio/speech" {
        return validate_openai_audio_speech_schema(body);
    }
    if path == "/v1/rerank" {
        return validate_openai_rerank_schema(body);
    }
    if path == "/v1/batches" {
        return validate_openai_batches_schema(body);
    }

    None
}

fn validate_openai_chat_completions_schema(body: &serde_json::Value) -> Option<String> {
    let Some(obj) = body.as_object() else {
        return Some("request body must be a JSON object".to_string());
    };

    let model = obj
        .get("model")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if model.is_none() {
        return Some("missing field `model`".to_string());
    }

    let Some(messages) = obj.get("messages").and_then(|value| value.as_array()) else {
        return Some("`messages` must be an array".to_string());
    };

    for (idx, message) in messages.iter().enumerate() {
        let Some(message) = message.as_object() else {
            return Some(format!("messages[{idx}] must be an object"));
        };

        let role = message
            .get("role")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if role.is_none() {
            return Some(format!("messages[{idx}].role must be a non-empty string"));
        }

        if !message.contains_key("content") {
            return Some(format!("messages[{idx}].content is required"));
        }
    }

    None
}

fn validate_openai_responses_schema(body: &serde_json::Value) -> Option<String> {
    let Some(obj) = body.as_object() else {
        return Some("request body must be a JSON object".to_string());
    };

    let model = obj
        .get("model")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if model.is_none() {
        return Some("missing field `model`".to_string());
    }

    let Some(input) = obj.get("input") else {
        return Some("missing field `input`".to_string());
    };
    if !(input.is_string() || input.is_array() || input.is_object()) {
        return Some("`input` must be a string, array, or object".to_string());
    }

    None
}

fn validate_openai_embeddings_schema(body: &serde_json::Value) -> Option<String> {
    let Some(obj) = body.as_object() else {
        return Some("request body must be a JSON object".to_string());
    };

    let model = obj
        .get("model")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if model.is_none() {
        return Some("missing field `model`".to_string());
    }

    let Some(input) = obj.get("input") else {
        return Some("missing field `input`".to_string());
    };
    if !(input.is_string() || input.is_array()) {
        return Some("`input` must be a string or array".to_string());
    }

    None
}

fn validate_openai_completions_schema(body: &serde_json::Value) -> Option<String> {
    let Some(obj) = body.as_object() else {
        return Some("request body must be a JSON object".to_string());
    };

    let model = obj
        .get("model")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if model.is_none() {
        return Some("missing field `model`".to_string());
    }

    let Some(prompt) = obj.get("prompt") else {
        return Some("missing field `prompt`".to_string());
    };
    if !(prompt.is_string() || prompt.is_array()) {
        return Some("`prompt` must be a string or array".to_string());
    }

    None
}

fn validate_openai_moderations_schema(body: &serde_json::Value) -> Option<String> {
    let Some(obj) = body.as_object() else {
        return Some("request body must be a JSON object".to_string());
    };

    let Some(input) = obj.get("input") else {
        return Some("missing field `input`".to_string());
    };
    if input.is_null() {
        return Some("`input` must not be null".to_string());
    }
    if !(input.is_string() || input.is_array() || input.is_object()) {
        return Some("`input` must be a string, array, or object".to_string());
    }

    None
}

fn validate_openai_images_generations_schema(body: &serde_json::Value) -> Option<String> {
    let Some(obj) = body.as_object() else {
        return Some("request body must be a JSON object".to_string());
    };

    match obj.get("prompt") {
        Some(serde_json::Value::String(prompt)) if !prompt.trim().is_empty() => None,
        Some(_) => Some("`prompt` must be a non-empty string".to_string()),
        None => Some("missing field `prompt`".to_string()),
    }
}

fn validate_openai_audio_speech_schema(body: &serde_json::Value) -> Option<String> {
    let Some(obj) = body.as_object() else {
        return Some("request body must be a JSON object".to_string());
    };

    let model = obj
        .get("model")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if model.is_none() {
        return Some("missing field `model`".to_string());
    }

    let input = obj
        .get("input")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if input.is_none() {
        return Some("missing field `input`".to_string());
    }

    let voice = obj
        .get("voice")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if voice.is_none() {
        return Some("missing field `voice`".to_string());
    }

    None
}

fn validate_openai_rerank_schema(body: &serde_json::Value) -> Option<String> {
    let Some(obj) = body.as_object() else {
        return Some("request body must be a JSON object".to_string());
    };

    let query = obj
        .get("query")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if query.is_none() {
        return Some("missing field `query`".to_string());
    }

    let Some(documents) = obj.get("documents") else {
        return Some("missing field `documents`".to_string());
    };
    if !documents.is_array() {
        return Some("`documents` must be an array".to_string());
    }

    None
}

fn validate_openai_batches_schema(body: &serde_json::Value) -> Option<String> {
    let Some(obj) = body.as_object() else {
        return Some("request body must be a JSON object".to_string());
    };

    let input_file_id = obj
        .get("input_file_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if input_file_id.is_none() {
        return Some("missing field `input_file_id`".to_string());
    }

    let endpoint = obj
        .get("endpoint")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if endpoint.is_none() {
        return Some("missing field `endpoint`".to_string());
    }

    let completion_window = obj
        .get("completion_window")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if completion_window.is_none() {
        return Some("missing field `completion_window`".to_string());
    }

    None
}

#[cfg(feature = "gateway-costing")]
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
async fn append_audit_log(state: &GatewayHttpState, kind: &str, payload: serde_json::Value) {
    let Some(payload) = state.prepare_observability_event(
        crate::gateway::observability::GatewayObservabilitySink::Audit,
        payload,
    ) else {
        return;
    };

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.stores.sqlite.as_ref() {
        let _ = store.append_audit_log(kind, payload.clone()).await;
    }

    #[cfg(feature = "gateway-store-postgres")]
    if let Some(store) = state.stores.postgres.as_ref() {
        let _ = store.append_audit_log(kind, payload.clone()).await;
    }

    #[cfg(feature = "gateway-store-mysql")]
    if let Some(store) = state.stores.mysql.as_ref() {
        let _ = store.append_audit_log(kind, payload.clone()).await;
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.stores.redis.as_ref() {
        let _ = store.append_audit_log(kind, payload).await;
    }
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
        if let Some(cache_key) = _cache_key {
            if let Ok(value) = axum::http::HeaderValue::from_str(cache_key) {
                headers.insert("x-ditto-cache-key", value);
            }
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
                #[cfg(feature = "gateway-proxy-cache")]
                if matches!(end, ProxyStreamEnd::Completed) {
                    if let Some(cache_completion) = self.cache_completion.take() {
                        cache_completion.finish().await;
                    }
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
        if let Some(cache_key) = _cache_key {
            if let Ok(value) = axum::http::HeaderValue::from_str(cache_key) {
                headers.insert("x-ditto-cache-key", value);
            }
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
            if status.is_success() {
                if let (Some(cache_key), Some(cache_metadata)) = (_cache_key, _cache_metadata) {
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
        let data_stream = crate::utils::sse::sse_data_stream_from_response(upstream);
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
        if let Some(cache_key) = _cache_key {
            if let Ok(value) = axum::http::HeaderValue::from_str(cache_key) {
                headers.insert("x-ditto-cache-key", value);
            }
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
                #[cfg(feature = "gateway-proxy-cache")]
                if matches!(end, ProxyStreamEnd::Completed) {
                    if let Some(cache_completion) = self.cache_completion.take() {
                        cache_completion.finish().await;
                    }
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
        if status.is_success() {
            if let (Some(cache_key), Some(cache_metadata)) = (_cache_key, _cache_metadata) {
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
        }

        apply_proxy_response_headers(&mut headers, &backend, &request_id, false);
        if let Some(cache_key) = _cache_key {
            if let Ok(value) = axum::http::HeaderValue::from_str(cache_key) {
                headers.insert("x-ditto-cache-key", value);
            }
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
    headers: &HeaderMap,
) -> String {
    use sha2::Digest as _;

    let mut header_names: Vec<&str> = headers
        .keys()
        .map(|name| name.as_str())
        .filter(|name| proxy_cache_header_affects_upstream(name))
        .collect();
    header_names.sort_unstable();
    header_names.dedup();

    let mut hasher = sha2::Sha256::new();
    hasher.update(b"ditto-proxy-cache-v2|");
    hasher.update(method.as_str().as_bytes());
    hasher.update(b"|");
    hasher.update(path.as_bytes());
    hasher.update(b"|");
    hasher.update(scope.as_bytes());
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
    format!(
        "ditto-proxy-cache-v2-{}",
        proxy_cache_hex_lower(&hasher.finalize())
    )
}

#[cfg(feature = "gateway-proxy-cache")]
fn proxy_cache_hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
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
// inlined from proxy/bounded_body.rs
async fn read_reqwest_body_bytes_bounded(
    response: reqwest::Response,
    max_bytes: usize,
    initial_capacity: usize,
) -> Result<Bytes, std::io::Error> {
    let max_bytes = max_bytes.max(1);
    let mut stream = response.bytes_stream();
    let mut buffered = bytes::BytesMut::with_capacity(initial_capacity.min(max_bytes));

    while let Some(next) = stream.next().await {
        match next {
            Ok(chunk) => {
                if buffered.len().saturating_add(chunk.len()) > max_bytes {
                    return Err(std::io::Error::other(format!(
                        "response exceeded max bytes ({max_bytes})"
                    )));
                }
                buffered.extend_from_slice(chunk.as_ref());
            }
            Err(err) => {
                return Err(std::io::Error::other(err));
            }
        }
    }

    Ok(buffered.freeze())
}

async fn read_reqwest_body_bytes_bounded_with_content_length(
    response: reqwest::Response,
    headers: &HeaderMap,
    max_bytes: usize,
) -> Result<Bytes, std::io::Error> {
    let max_bytes = max_bytes.max(1);
    let content_length = headers
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok());
    if content_length.is_some_and(|len| len > max_bytes) {
        return Err(std::io::Error::other(format!(
            "content-length={:?} exceeds max bytes ({max_bytes})",
            content_length
        )));
    }
    let initial_capacity = content_length.map(|len| len.min(max_bytes)).unwrap_or(0);
    read_reqwest_body_bytes_bounded(response, max_bytes, initial_capacity).await
}
// end inline: proxy/bounded_body.rs
// inlined from proxy/map_openai_gateway_error.rs
fn map_openai_gateway_error(err: GatewayError) -> (StatusCode, Json<OpenAiErrorResponse>) {
    match err {
        GatewayError::Unauthorized => openai_error(
            StatusCode::UNAUTHORIZED,
            "authentication_error",
            Some("invalid_api_key"),
            "unauthorized virtual key",
        ),
        GatewayError::RateLimited { limit } => openai_error(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limit_error",
            Some("rate_limited"),
            format!("rate limit exceeded: {limit}"),
        ),
        GatewayError::GuardrailRejected { reason } => openai_error(
            StatusCode::FORBIDDEN,
            "policy_error",
            Some("guardrail_rejected"),
            format!("guardrail rejected: {reason}"),
        ),
        GatewayError::BudgetExceeded { limit, attempted } => openai_error(
            StatusCode::PAYMENT_REQUIRED,
            "insufficient_quota",
            Some("budget_exceeded"),
            format!("budget exceeded: limit={limit} attempted={attempted}"),
        ),
        GatewayError::CostBudgetExceeded {
            limit_usd_micros,
            attempted_usd_micros,
        } => openai_error(
            StatusCode::PAYMENT_REQUIRED,
            "insufficient_quota",
            Some("cost_budget_exceeded"),
            format!(
                "cost budget exceeded: limit_usd_micros={limit_usd_micros} attempted_usd_micros={attempted_usd_micros}"
            ),
        ),
        GatewayError::BackendNotFound { name } => openai_error(
            StatusCode::BAD_GATEWAY,
            "api_error",
            Some("backend_not_found"),
            format!("backend not found: {name}"),
        ),
        GatewayError::Backend { message } => openai_error(
            StatusCode::BAD_GATEWAY,
            "api_error",
            Some("backend_error"),
            message,
        ),
        GatewayError::BackendTimeout { message } => openai_error(
            StatusCode::GATEWAY_TIMEOUT,
            "api_error",
            Some("backend_timeout"),
            message,
        ),
        GatewayError::InvalidRequest { reason } => openai_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            Some("invalid_request"),
            reason,
        ),
    }
}
// end inline: proxy/map_openai_gateway_error.rs
// inlined from proxy/budget_reservations.rs
#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
#[derive(Clone, Copy)]
struct ProxyBudgetReservationParams<'a> {
    state: &'a GatewayHttpState,
    use_persistent_budget: bool,
    virtual_key_id: Option<&'a str>,
    budget: Option<&'a super::BudgetConfig>,
    tenant_budget_scope: &'a Option<(String, super::BudgetConfig)>,
    project_budget_scope: &'a Option<(String, super::BudgetConfig)>,
    user_budget_scope: &'a Option<(String, super::BudgetConfig)>,
    request_id: &'a str,
    path_and_query: &'a str,
    model: &'a Option<String>,
    charge_tokens: u32,
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
async fn reserve_proxy_token_budgets_for_request(
    params: ProxyBudgetReservationParams<'_>,
) -> Result<(bool, Vec<String>), (StatusCode, Json<OpenAiErrorResponse>)> {
    let ProxyBudgetReservationParams {
        state,
        use_persistent_budget,
        virtual_key_id,
        budget,
        tenant_budget_scope,
        project_budget_scope,
        user_budget_scope,
        request_id,
        path_and_query,
        model,
        charge_tokens,
    } = params;

    let token_budget_reserved = if use_persistent_budget {
        if let (Some(virtual_key_id), Some(budget)) = (virtual_key_id, budget) {
            if let Some(limit) = budget.total_tokens {
                let ctx = ProxyBudgetReservationContext {
                    state,
                    reservation_id: request_id,
                    budget_scope: virtual_key_id,
                    request_id,
                    virtual_key_id,
                    path_and_query,
                    model,
                };
                reserve_proxy_token_budget(ctx, limit, charge_tokens).await?;
                true
            } else {
                false
            }
        } else {
            false
        }
    } else {
        false
    };

    let mut token_budget_reservation_ids: Vec<String> = Vec::new();
    if token_budget_reserved {
        token_budget_reservation_ids.push(request_id.to_string());
    }

    if use_persistent_budget {
        if let Some(virtual_key_id) = virtual_key_id {
            if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                if let Some(limit) = budget.total_tokens {
                    let reservation_id = format!("{request_id}::budget::{scope}");
                    let ctx = ProxyBudgetReservationContext {
                        state,
                        reservation_id: &reservation_id,
                        budget_scope: scope,
                        request_id,
                        virtual_key_id,
                        path_and_query,
                        model,
                    };
                    if let Err(err) = reserve_proxy_token_budget(ctx, limit, charge_tokens).await {
                        rollback_proxy_token_budget_reservations(
                            state,
                            &token_budget_reservation_ids,
                        )
                        .await;
                        return Err(err);
                    }
                    token_budget_reservation_ids.push(reservation_id);
                }
            }

            if let Some((scope, budget)) = project_budget_scope.as_ref() {
                if let Some(limit) = budget.total_tokens {
                    let reservation_id = format!("{request_id}::budget::{scope}");
                    let ctx = ProxyBudgetReservationContext {
                        state,
                        reservation_id: &reservation_id,
                        budget_scope: scope,
                        request_id,
                        virtual_key_id,
                        path_and_query,
                        model,
                    };
                    if let Err(err) = reserve_proxy_token_budget(ctx, limit, charge_tokens).await {
                        rollback_proxy_token_budget_reservations(
                            state,
                            &token_budget_reservation_ids,
                        )
                        .await;
                        return Err(err);
                    }
                    token_budget_reservation_ids.push(reservation_id);
                }
            }

            if let Some((scope, budget)) = user_budget_scope.as_ref() {
                if let Some(limit) = budget.total_tokens {
                    let reservation_id = format!("{request_id}::budget::{scope}");
                    let ctx = ProxyBudgetReservationContext {
                        state,
                        reservation_id: &reservation_id,
                        budget_scope: scope,
                        request_id,
                        virtual_key_id,
                        path_and_query,
                        model,
                    };
                    if let Err(err) = reserve_proxy_token_budget(ctx, limit, charge_tokens).await {
                        rollback_proxy_token_budget_reservations(
                            state,
                            &token_budget_reservation_ids,
                        )
                        .await;
                        return Err(err);
                    }
                    token_budget_reservation_ids.push(reservation_id);
                }
            }
        }
    }

    Ok((token_budget_reserved, token_budget_reservation_ids))
}

#[cfg(all(
    feature = "gateway-costing",
    any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ),
))]
async fn reserve_proxy_cost_budgets_for_request(
    params: ProxyBudgetReservationParams<'_>,
    charge_cost_usd_micros: Option<u64>,
    token_budget_reservation_ids: &[String],
) -> Result<(bool, Vec<String>), (StatusCode, Json<OpenAiErrorResponse>)> {
    let ProxyBudgetReservationParams {
        state,
        use_persistent_budget,
        virtual_key_id,
        budget,
        tenant_budget_scope,
        project_budget_scope,
        user_budget_scope,
        request_id,
        path_and_query,
        model,
        charge_tokens: _,
    } = params;

    let cost_budget_reserved = if use_persistent_budget {
        if let (Some(virtual_key_id), Some(budget)) = (virtual_key_id, budget) {
            if let Some(limit_usd_micros) = budget.total_usd_micros {
                let Some(charge_cost_usd_micros) = charge_cost_usd_micros else {
                    rollback_proxy_token_budget_reservations(state, token_budget_reservation_ids)
                        .await;
                    return Err(openai_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "api_error",
                        Some("pricing_not_configured"),
                        "pricing not configured for cost budgets",
                    ));
                };

                let ctx = ProxyBudgetReservationContext {
                    state,
                    reservation_id: request_id,
                    budget_scope: virtual_key_id,
                    request_id,
                    virtual_key_id,
                    path_and_query,
                    model,
                };
                if let Err(err) =
                    reserve_proxy_cost_budget(ctx, limit_usd_micros, charge_cost_usd_micros).await
                {
                    rollback_proxy_token_budget_reservations(state, token_budget_reservation_ids)
                        .await;
                    return Err(err);
                }
                true
            } else {
                false
            }
        } else {
            false
        }
    } else {
        false
    };

    let mut cost_budget_reservation_ids: Vec<String> = Vec::new();
    if cost_budget_reserved {
        cost_budget_reservation_ids.push(request_id.to_string());
    }

    if use_persistent_budget {
        if let Some(virtual_key_id) = virtual_key_id {
            let mut cost_scopes: Vec<(String, u64)> = Vec::new();
            if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                if let Some(limit) = budget.total_usd_micros {
                    cost_scopes.push((scope.clone(), limit));
                }
            }
            if let Some((scope, budget)) = project_budget_scope.as_ref() {
                if let Some(limit) = budget.total_usd_micros {
                    cost_scopes.push((scope.clone(), limit));
                }
            }
            if let Some((scope, budget)) = user_budget_scope.as_ref() {
                if let Some(limit) = budget.total_usd_micros {
                    cost_scopes.push((scope.clone(), limit));
                }
            }

            if !cost_scopes.is_empty() {
                let Some(charge_cost_usd_micros) = charge_cost_usd_micros else {
                    rollback_proxy_cost_budget_reservations(state, &cost_budget_reservation_ids)
                        .await;
                    rollback_proxy_token_budget_reservations(state, token_budget_reservation_ids)
                        .await;
                    return Err(openai_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "api_error",
                        Some("pricing_not_configured"),
                        "pricing not configured for cost budgets",
                    ));
                };

                for (scope, limit_usd_micros) in cost_scopes {
                    let reservation_id = format!("{request_id}::cost::{scope}");
                    let ctx = ProxyBudgetReservationContext {
                        state,
                        reservation_id: &reservation_id,
                        budget_scope: &scope,
                        request_id,
                        virtual_key_id,
                        path_and_query,
                        model,
                    };
                    if let Err(err) =
                        reserve_proxy_cost_budget(ctx, limit_usd_micros, charge_cost_usd_micros)
                            .await
                    {
                        rollback_proxy_cost_budget_reservations(
                            state,
                            &cost_budget_reservation_ids,
                        )
                        .await;
                        rollback_proxy_token_budget_reservations(
                            state,
                            token_budget_reservation_ids,
                        )
                        .await;
                        return Err(err);
                    }
                    cost_budget_reservation_ids.push(reservation_id);
                }
            }
        }
    }

    Ok((cost_budget_reserved, cost_budget_reservation_ids))
}
// end inline: proxy/budget_reservations.rs
// inlined from proxy/budget_reservation.rs
#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
#[derive(Clone, Copy)]
struct ProxyBudgetReservationContext<'a> {
    state: &'a GatewayHttpState,
    reservation_id: &'a str,
    budget_scope: &'a str,
    request_id: &'a str,
    virtual_key_id: &'a str,
    path_and_query: &'a str,
    model: &'a Option<String>,
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
async fn reserve_proxy_token_budget(
    ctx: ProxyBudgetReservationContext<'_>,
    limit: u64,
    charge_tokens: u32,
) -> Result<(), (StatusCode, Json<OpenAiErrorResponse>)> {
    let ProxyBudgetReservationContext {
        state,
        reservation_id,
        budget_scope,
        request_id,
        virtual_key_id,
        path_and_query,
        model,
    } = ctx;
    let charge_tokens_u64 = u64::from(charge_tokens);

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.stores.sqlite.as_ref() {
        match store
            .reserve_budget_tokens(reservation_id, budget_scope, limit, charge_tokens_u64)
            .await
        {
            Ok(()) => return Ok(()),
            Err(SqliteStoreError::BudgetExceeded { limit, attempted }) => {
                append_audit_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
                        "reason": "budget_exceeded",
                        "limit": limit,
                        "attempted": attempted,
                        "charge_tokens": charge_tokens,
                        "path": path_and_query,
                        "model": model,
                    }),
                )
                .await;
                emit_json_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
                        "reason": "budget_exceeded",
                        "limit": limit,
                        "attempted": attempted,
                    }),
                );
                return Err(map_openai_gateway_error(GatewayError::BudgetExceeded {
                    limit,
                    attempted,
                }));
            }
            Err(err) => {
                append_audit_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
                        "reason": "storage_error",
                        "error": err.to_string(),
                        "path": path_and_query,
                        "model": model,
                    }),
                )
                .await;
                return Err(openai_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "api_error",
                    Some("storage_error"),
                    err.to_string(),
                ));
            }
        }
    }

    #[cfg(feature = "gateway-store-postgres")]
    if let Some(store) = state.stores.postgres.as_ref() {
        match store
            .reserve_budget_tokens(reservation_id, budget_scope, limit, charge_tokens_u64)
            .await
        {
            Ok(()) => return Ok(()),
            Err(PostgresStoreError::BudgetExceeded { limit, attempted }) => {
                append_audit_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
                        "reason": "budget_exceeded",
                        "limit": limit,
                        "attempted": attempted,
                        "charge_tokens": charge_tokens,
                        "path": path_and_query,
                        "model": model,
                    }),
                )
                .await;
                emit_json_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
                        "reason": "budget_exceeded",
                        "limit": limit,
                        "attempted": attempted,
                    }),
                );
                return Err(map_openai_gateway_error(GatewayError::BudgetExceeded {
                    limit,
                    attempted,
                }));
            }
            Err(err) => {
                append_audit_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
                        "reason": "storage_error",
                        "error": err.to_string(),
                        "path": path_and_query,
                        "model": model,
                    }),
                )
                .await;
                return Err(openai_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "api_error",
                    Some("storage_error"),
                    err.to_string(),
                ));
            }
        }
    }

    #[cfg(feature = "gateway-store-mysql")]
    if let Some(store) = state.stores.mysql.as_ref() {
        match store
            .reserve_budget_tokens(reservation_id, budget_scope, limit, charge_tokens_u64)
            .await
        {
            Ok(()) => return Ok(()),
            Err(MySqlStoreError::BudgetExceeded { limit, attempted }) => {
                append_audit_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
                        "reason": "budget_exceeded",
                        "limit": limit,
                        "attempted": attempted,
                        "charge_tokens": charge_tokens,
                        "path": path_and_query,
                        "model": model,
                    }),
                )
                .await;
                emit_json_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
                        "reason": "budget_exceeded",
                        "limit": limit,
                        "attempted": attempted,
                    }),
                );
                return Err(map_openai_gateway_error(GatewayError::BudgetExceeded {
                    limit,
                    attempted,
                }));
            }
            Err(err) => {
                append_audit_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
                        "reason": "storage_error",
                        "error": err.to_string(),
                        "path": path_and_query,
                        "model": model,
                    }),
                )
                .await;
                return Err(openai_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "api_error",
                    Some("storage_error"),
                    err.to_string(),
                ));
            }
        }
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.stores.redis.as_ref() {
        match store
            .reserve_budget_tokens(reservation_id, budget_scope, limit, charge_tokens_u64)
            .await
        {
            Ok(()) => return Ok(()),
            Err(RedisStoreError::BudgetExceeded { limit, attempted }) => {
                append_audit_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
                        "reason": "budget_exceeded",
                        "limit": limit,
                        "attempted": attempted,
                        "charge_tokens": charge_tokens,
                        "path": path_and_query,
                        "model": model,
                    }),
                )
                .await;
                emit_json_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
                        "reason": "budget_exceeded",
                        "limit": limit,
                        "attempted": attempted,
                    }),
                );
                return Err(map_openai_gateway_error(GatewayError::BudgetExceeded {
                    limit,
                    attempted,
                }));
            }
            Err(err) => {
                append_audit_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
                        "reason": "storage_error",
                        "error": err.to_string(),
                        "path": path_and_query,
                        "model": model,
                    }),
                )
                .await;
                return Err(openai_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "api_error",
                    Some("storage_error"),
                    err.to_string(),
                ));
            }
        }
    }

    Ok(())
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
async fn rollback_proxy_token_budget_reservations(
    state: &GatewayHttpState,
    reservation_ids: &[String],
) {
    for reservation_id in reservation_ids {
        #[cfg(feature = "gateway-store-sqlite")]
        if let Some(store) = state.stores.sqlite.as_ref() {
            let _ = store.rollback_budget_reservation(reservation_id).await;
        }
        #[cfg(feature = "gateway-store-postgres")]
        if let Some(store) = state.stores.postgres.as_ref() {
            let _ = store.rollback_budget_reservation(reservation_id).await;
        }
        #[cfg(feature = "gateway-store-mysql")]
        if let Some(store) = state.stores.mysql.as_ref() {
            let _ = store.rollback_budget_reservation(reservation_id).await;
        }
        #[cfg(feature = "gateway-store-redis")]
        if let Some(store) = state.stores.redis.as_ref() {
            let _ = store.rollback_budget_reservation(reservation_id).await;
        }
    }
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
async fn settle_proxy_token_budget_reservations(
    state: &GatewayHttpState,
    reservation_ids: &[String],
    spend_tokens: bool,
    spent_tokens: u64,
) {
    if reservation_ids.is_empty() {
        return;
    }
    for reservation_id in reservation_ids {
        #[cfg(feature = "gateway-store-sqlite")]
        if let Some(store) = state.stores.sqlite.as_ref() {
            if spend_tokens {
                let _ = store
                    .commit_budget_reservation_with_tokens(reservation_id, spent_tokens)
                    .await;
            } else {
                let _ = store.rollback_budget_reservation(reservation_id).await;
            }
        }
        #[cfg(feature = "gateway-store-postgres")]
        if let Some(store) = state.stores.postgres.as_ref() {
            if spend_tokens {
                let _ = store
                    .commit_budget_reservation_with_tokens(reservation_id, spent_tokens)
                    .await;
            } else {
                let _ = store.rollback_budget_reservation(reservation_id).await;
            }
        }
        #[cfg(feature = "gateway-store-mysql")]
        if let Some(store) = state.stores.mysql.as_ref() {
            if spend_tokens {
                let _ = store
                    .commit_budget_reservation_with_tokens(reservation_id, spent_tokens)
                    .await;
            } else {
                let _ = store.rollback_budget_reservation(reservation_id).await;
            }
        }
        #[cfg(feature = "gateway-store-redis")]
        if let Some(store) = state.stores.redis.as_ref() {
            if spend_tokens {
                let _ = store
                    .commit_budget_reservation_with_tokens(reservation_id, spent_tokens)
                    .await;
            } else {
                let _ = store.rollback_budget_reservation(reservation_id).await;
            }
        }
    }
}

#[cfg(all(
    feature = "gateway-costing",
    any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ),
))]
async fn reserve_proxy_cost_budget(
    ctx: ProxyBudgetReservationContext<'_>,
    limit_usd_micros: u64,
    charge_cost_usd_micros: u64,
) -> Result<(), (StatusCode, Json<OpenAiErrorResponse>)> {
    let ProxyBudgetReservationContext {
        state,
        reservation_id,
        budget_scope,
        request_id,
        virtual_key_id,
        path_and_query,
        model,
    } = ctx;
    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.stores.sqlite.as_ref() {
        match store
            .reserve_cost_usd_micros(
                reservation_id,
                budget_scope,
                limit_usd_micros,
                charge_cost_usd_micros,
            )
            .await
        {
            Ok(()) => return Ok(()),
            Err(SqliteStoreError::CostBudgetExceeded {
                limit_usd_micros,
                attempted_usd_micros,
            }) => {
                append_audit_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
                        "reason": "cost_budget_exceeded",
                        "limit_usd_micros": limit_usd_micros,
                        "attempted_usd_micros": attempted_usd_micros,
                        "charge_cost_usd_micros": charge_cost_usd_micros,
                        "path": path_and_query,
                        "model": model,
                    }),
                )
                .await;
                emit_json_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
                        "reason": "cost_budget_exceeded",
                        "limit_usd_micros": limit_usd_micros,
                        "attempted_usd_micros": attempted_usd_micros,
                    }),
                );
                return Err(map_openai_gateway_error(GatewayError::CostBudgetExceeded {
                    limit_usd_micros,
                    attempted_usd_micros,
                }));
            }
            Err(err) => {
                append_audit_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
                        "reason": "storage_error",
                        "error": err.to_string(),
                        "path": path_and_query,
                        "model": model,
                    }),
                )
                .await;
                return Err(openai_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "api_error",
                    Some("storage_error"),
                    err.to_string(),
                ));
            }
        }
    }

    #[cfg(feature = "gateway-store-postgres")]
    if let Some(store) = state.stores.postgres.as_ref() {
        match store
            .reserve_cost_usd_micros(
                reservation_id,
                budget_scope,
                limit_usd_micros,
                charge_cost_usd_micros,
            )
            .await
        {
            Ok(()) => return Ok(()),
            Err(PostgresStoreError::CostBudgetExceeded {
                limit_usd_micros,
                attempted_usd_micros,
            }) => {
                append_audit_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
                        "reason": "cost_budget_exceeded",
                        "limit_usd_micros": limit_usd_micros,
                        "attempted_usd_micros": attempted_usd_micros,
                        "charge_cost_usd_micros": charge_cost_usd_micros,
                        "path": path_and_query,
                        "model": model,
                    }),
                )
                .await;
                emit_json_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
                        "reason": "cost_budget_exceeded",
                        "limit_usd_micros": limit_usd_micros,
                        "attempted_usd_micros": attempted_usd_micros,
                    }),
                );
                return Err(map_openai_gateway_error(GatewayError::CostBudgetExceeded {
                    limit_usd_micros,
                    attempted_usd_micros,
                }));
            }
            Err(err) => {
                append_audit_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
                        "reason": "storage_error",
                        "error": err.to_string(),
                        "path": path_and_query,
                        "model": model,
                    }),
                )
                .await;
                return Err(openai_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "api_error",
                    Some("storage_error"),
                    err.to_string(),
                ));
            }
        }
    }

    #[cfg(feature = "gateway-store-mysql")]
    if let Some(store) = state.stores.mysql.as_ref() {
        match store
            .reserve_cost_usd_micros(
                reservation_id,
                budget_scope,
                limit_usd_micros,
                charge_cost_usd_micros,
            )
            .await
        {
            Ok(()) => return Ok(()),
            Err(MySqlStoreError::CostBudgetExceeded {
                limit_usd_micros,
                attempted_usd_micros,
            }) => {
                append_audit_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
                        "reason": "cost_budget_exceeded",
                        "limit_usd_micros": limit_usd_micros,
                        "attempted_usd_micros": attempted_usd_micros,
                        "charge_cost_usd_micros": charge_cost_usd_micros,
                        "path": path_and_query,
                        "model": model,
                    }),
                )
                .await;
                emit_json_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
                        "reason": "cost_budget_exceeded",
                        "limit_usd_micros": limit_usd_micros,
                        "attempted_usd_micros": attempted_usd_micros,
                    }),
                );
                return Err(map_openai_gateway_error(GatewayError::CostBudgetExceeded {
                    limit_usd_micros,
                    attempted_usd_micros,
                }));
            }
            Err(err) => {
                append_audit_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
                        "reason": "storage_error",
                        "error": err.to_string(),
                        "path": path_and_query,
                        "model": model,
                    }),
                )
                .await;
                return Err(openai_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "api_error",
                    Some("storage_error"),
                    err.to_string(),
                ));
            }
        }
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.stores.redis.as_ref() {
        match store
            .reserve_cost_usd_micros(
                reservation_id,
                budget_scope,
                limit_usd_micros,
                charge_cost_usd_micros,
            )
            .await
        {
            Ok(()) => return Ok(()),
            Err(RedisStoreError::CostBudgetExceeded {
                limit_usd_micros,
                attempted_usd_micros,
            }) => {
                append_audit_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
                        "reason": "cost_budget_exceeded",
                        "limit_usd_micros": limit_usd_micros,
                        "attempted_usd_micros": attempted_usd_micros,
                        "charge_cost_usd_micros": charge_cost_usd_micros,
                        "path": path_and_query,
                        "model": model,
                    }),
                )
                .await;
                emit_json_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
                        "reason": "cost_budget_exceeded",
                        "limit_usd_micros": limit_usd_micros,
                        "attempted_usd_micros": attempted_usd_micros,
                    }),
                );
                return Err(map_openai_gateway_error(GatewayError::CostBudgetExceeded {
                    limit_usd_micros,
                    attempted_usd_micros,
                }));
            }
            Err(err) => {
                append_audit_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
                        "reason": "storage_error",
                        "error": err.to_string(),
                        "path": path_and_query,
                        "model": model,
                    }),
                )
                .await;
                return Err(openai_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "api_error",
                    Some("storage_error"),
                    err.to_string(),
                ));
            }
        }
    }

    Ok(())
}

#[cfg(all(
    feature = "gateway-costing",
    any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ),
))]
async fn rollback_proxy_cost_budget_reservations(
    state: &GatewayHttpState,
    reservation_ids: &[String],
) {
    for reservation_id in reservation_ids {
        #[cfg(feature = "gateway-store-sqlite")]
        if let Some(store) = state.stores.sqlite.as_ref() {
            let _ = store.rollback_cost_reservation(reservation_id).await;
        }
        #[cfg(feature = "gateway-store-postgres")]
        if let Some(store) = state.stores.postgres.as_ref() {
            let _ = store.rollback_cost_reservation(reservation_id).await;
        }
        #[cfg(feature = "gateway-store-mysql")]
        if let Some(store) = state.stores.mysql.as_ref() {
            let _ = store.rollback_cost_reservation(reservation_id).await;
        }
        #[cfg(feature = "gateway-store-redis")]
        if let Some(store) = state.stores.redis.as_ref() {
            let _ = store.rollback_cost_reservation(reservation_id).await;
        }
    }
}

#[cfg(all(
    feature = "gateway-costing",
    any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ),
))]
async fn settle_proxy_cost_budget_reservations(
    state: &GatewayHttpState,
    reservation_ids: &[String],
    spend_tokens: bool,
    spent_cost_usd_micros: u64,
) {
    if reservation_ids.is_empty() {
        return;
    }

    for reservation_id in reservation_ids {
        #[cfg(feature = "gateway-store-sqlite")]
        if let Some(store) = state.stores.sqlite.as_ref() {
            if spend_tokens {
                let _ = store
                    .commit_cost_reservation_with_usd_micros(reservation_id, spent_cost_usd_micros)
                    .await;
            } else {
                let _ = store.rollback_cost_reservation(reservation_id).await;
            }
        }
        #[cfg(feature = "gateway-store-postgres")]
        if let Some(store) = state.stores.postgres.as_ref() {
            if spend_tokens {
                let _ = store
                    .commit_cost_reservation_with_usd_micros(reservation_id, spent_cost_usd_micros)
                    .await;
            } else {
                let _ = store.rollback_cost_reservation(reservation_id).await;
            }
        }
        #[cfg(feature = "gateway-store-mysql")]
        if let Some(store) = state.stores.mysql.as_ref() {
            if spend_tokens {
                let _ = store
                    .commit_cost_reservation_with_usd_micros(reservation_id, spent_cost_usd_micros)
                    .await;
            } else {
                let _ = store.rollback_cost_reservation(reservation_id).await;
            }
        }
        #[cfg(feature = "gateway-store-redis")]
        if let Some(store) = state.stores.redis.as_ref() {
            if spend_tokens {
                let _ = store
                    .commit_cost_reservation_with_usd_micros(reservation_id, spent_cost_usd_micros)
                    .await;
            } else {
                let _ = store.rollback_cost_reservation(reservation_id).await;
            }
        }
    }
}

enum ProxyPermitOutcome {
    Acquired(ProxyPermits),
    BackendRateLimited((StatusCode, Json<OpenAiErrorResponse>)),
}

fn try_acquire_proxy_permits(
    state: &GatewayHttpState,
    backend: &str,
) -> Result<ProxyPermitOutcome, (StatusCode, Json<OpenAiErrorResponse>)> {
    let proxy_permit = if let Some(limit) = state.proxy.backpressure.as_ref() {
        Some(limit.clone().try_acquire_owned().map_err(|_| {
            openai_error(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limit_error",
                Some("inflight_limit"),
                "too many in-flight proxy requests",
            )
        })?)
    } else {
        None
    };

    let backend_permit = if let Some(limit) = state.proxy.backend_backpressure.get(backend) {
        match limit.clone().try_acquire_owned() {
            Ok(permit) => Some(permit),
            Err(_) => {
                return Ok(ProxyPermitOutcome::BackendRateLimited(openai_error(
                    StatusCode::TOO_MANY_REQUESTS,
                    "rate_limit_error",
                    Some("inflight_limit_backend"),
                    format!("too many in-flight proxy requests for backend {backend}"),
                )));
            }
        }
    } else {
        None
    };

    Ok(ProxyPermitOutcome::Acquired(ProxyPermits::new(
        proxy_permit,
        backend_permit,
    )))
}
// end inline: proxy/budget_reservation.rs

#[cfg(test)]
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
// end inline: proxy/sanitize_proxy_headers_tests.rs
#[cfg(test)]
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
// end inline: ../../http/proxy.rs
// inlined from ../../http/admin.rs
// inlined from admin/handlers.rs
// inlined from handlers/common.rs
#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
#[derive(Debug, Deserialize)]
struct LedgerQuery {
    #[serde(default)]
    key_prefix: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: usize,
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
fn default_audit_limit() -> usize {
    100
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
fn default_audit_export_limit() -> usize {
    1000
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
const MAX_ADMIN_LEDGER_LIMIT: usize = 10_000;

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
fn apply_admin_list_window<T>(items: &mut Vec<T>, offset: usize, limit: Option<usize>, max: usize) {
    if offset > 0 {
        if offset >= items.len() {
            items.clear();
        } else {
            items.drain(0..offset);
        }
    }

    if let Some(limit) = limit.map(|limit| limit.min(max)) {
        if items.len() > limit {
            items.truncate(limit);
        }
    }
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
fn tenant_allowed_scopes(
    keys: &[VirtualKeyConfig],
    tenant_id: &str,
) -> std::collections::HashSet<String> {
    let tenant_id = tenant_id.trim();
    let mut scopes = std::collections::HashSet::<String>::new();
    if !tenant_id.is_empty() {
        scopes.insert(format!("tenant:{tenant_id}"));
    }

    for key in keys {
        if key.tenant_id.as_deref() != Some(tenant_id) {
            continue;
        }
        scopes.insert(key.id.clone());

        if let Some(project_id) = key
            .project_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            scopes.insert(format!("project:{project_id}"));
        }

        if let Some(user_id) = key
            .user_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            scopes.insert(format!("user:{user_id}"));
        }
    }

    scopes
}
// end inline: handlers/common.rs
// inlined from handlers/proxy_cache.rs
#[cfg(feature = "gateway-proxy-cache")]
async fn purge_proxy_cache(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Json(payload): Json<PurgeProxyCacheRequest>,
) -> Result<Json<PurgeProxyCacheResponse>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_write(&state, &headers)?;
    if admin.tenant_id.is_some() {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "tenant-scoped admin tokens cannot purge the proxy cache",
        ));
    }

    let Some(cache) = state.proxy.cache.as_ref() else {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            "not_configured",
            "proxy cache not enabled",
        ));
    };

    let selector = payload.selector.into_normalized();

    if payload.all {
        let deleted_memory = {
            let mut cache = cache.lock().await;
            let deleted = cache.len() as u64;
            cache.clear();
            deleted
        };

        let deleted_redis = {
            #[cfg(feature = "gateway-store-redis")]
            if let Some(store) = state.stores.redis.as_ref() {
                Some(store.clear_proxy_cache().await.map_err(|err| {
                    error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "storage_error",
                        err.to_string(),
                    )
                })?)
            } else {
                None
            }
            #[cfg(not(feature = "gateway-store-redis"))]
            {
                None
            }
        };

        #[cfg(feature = "gateway-metrics-prometheus")]
        if let Some(metrics) = state.proxy.metrics.as_ref() {
            metrics.lock().await.record_proxy_cache_purge("all");
        }

        #[cfg(any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        ))]
        append_admin_audit_log(
            &state,
            "admin.proxy_cache.purge",
            serde_json::json!({
                "all": true,
                "selector": selector,
                "deleted_memory": deleted_memory,
                "deleted_redis": deleted_redis,
            }),
        )
        .await;

        return Ok(Json(PurgeProxyCacheResponse {
            cleared_memory: true,
            deleted_memory,
            deleted_redis,
        }));
    }

    if selector.is_empty() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "must set all=true or at least one of cache_key/scope/method/path/model",
        ));
    }

    let deleted_memory = {
        let mut cache = cache.lock().await;
        cache.purge_matching(&selector)
    };

    let deleted_redis = {
        #[cfg(feature = "gateway-store-redis")]
        if let Some(store) = state.stores.redis.as_ref() {
            Some(
                store
                    .purge_proxy_cache_matching(&selector)
                    .await
                    .map_err(|err| {
                        error_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "storage_error",
                            err.to_string(),
                        )
                    })?,
            )
        } else {
            None
        }
        #[cfg(not(feature = "gateway-store-redis"))]
        {
            None
        }
    };

    #[cfg(feature = "gateway-metrics-prometheus")]
    if let Some(metrics) = state.proxy.metrics.as_ref() {
        metrics
            .lock()
            .await
            .record_proxy_cache_purge(selector.kind_label());
    }

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    append_admin_audit_log(
        &state,
        "admin.proxy_cache.purge",
        serde_json::json!({
            "all": false,
            "selector": selector,
            "deleted_memory": deleted_memory,
            "deleted_redis": deleted_redis,
        }),
    )
    .await;

    Ok(Json(PurgeProxyCacheResponse {
        cleared_memory: deleted_memory > 0,
        deleted_memory,
        deleted_redis,
    }))
}
// end inline: handlers/proxy_cache.rs
// inlined from handlers/audit.rs
#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
#[derive(Debug, Deserialize)]
struct AuditQuery {
    #[serde(default = "default_audit_limit")]
    limit: usize,
    #[serde(default)]
    since_ts_ms: Option<u64>,
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
#[derive(Debug, Deserialize)]
struct AuditExportQuery {
    #[serde(default)]
    format: Option<String>,
    #[serde(default = "default_audit_export_limit")]
    limit: usize,
    #[serde(default)]
    since_ts_ms: Option<u64>,
    #[serde(default)]
    before_ts_ms: Option<u64>,
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
async fn list_audit_logs(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Query(query): Query<AuditQuery>,
) -> Result<Json<Vec<AuditLogRecord>>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.stores.sqlite.as_ref() {
        let mut logs = store
            .list_audit_logs(query.limit.min(1000), query.since_ts_ms)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;
        if let Some(tenant_id) = admin.tenant_id.as_deref() {
            logs.retain(|log| {
                log.payload
                    .get("tenant_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(tenant_id)
            });
        }
        for log in &mut logs {
            log.payload = state.redactor.redact(std::mem::take(&mut log.payload));
        }
        return Ok(Json(logs));
    }

    #[cfg(feature = "gateway-store-postgres")]
    if let Some(store) = state.stores.postgres.as_ref() {
        let mut logs = store
            .list_audit_logs(query.limit.min(1000), query.since_ts_ms)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;
        if let Some(tenant_id) = admin.tenant_id.as_deref() {
            logs.retain(|log| {
                log.payload
                    .get("tenant_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(tenant_id)
            });
        }
        for log in &mut logs {
            log.payload = state.redactor.redact(std::mem::take(&mut log.payload));
        }
        return Ok(Json(logs));
    }

    #[cfg(feature = "gateway-store-mysql")]
    if let Some(store) = state.stores.mysql.as_ref() {
        let mut logs = store
            .list_audit_logs(query.limit.min(1000), query.since_ts_ms)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;
        if let Some(tenant_id) = admin.tenant_id.as_deref() {
            logs.retain(|log| {
                log.payload
                    .get("tenant_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(tenant_id)
            });
        }
        for log in &mut logs {
            log.payload = state.redactor.redact(std::mem::take(&mut log.payload));
        }
        return Ok(Json(logs));
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.stores.redis.as_ref() {
        let mut logs = store
            .list_audit_logs(query.limit.min(1000), query.since_ts_ms)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;
        if let Some(tenant_id) = admin.tenant_id.as_deref() {
            logs.retain(|log| {
                log.payload
                    .get("tenant_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(tenant_id)
            });
        }
        for log in &mut logs {
            log.payload = state.redactor.redact(std::mem::take(&mut log.payload));
        }
        return Ok(Json(logs));
    }

    Err(error_response(
        StatusCode::BAD_REQUEST,
        "not_configured",
        "store not configured",
    ))
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
fn audit_chain_hash(prev_hash: Option<&str>, record: &AuditLogRecord) -> String {
    use sha2::Digest as _;

    let mut hasher = sha2::Sha256::new();
    if let Some(prev_hash) = prev_hash {
        hasher.update(prev_hash.as_bytes());
    }
    hasher.update(b"\n");
    if let Ok(serialized) = serde_json::to_vec(record) {
        hasher.update(&serialized);
    }
    hex_lower(&hasher.finalize())
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
fn csv_escape(value: &str) -> String {
    if !value.contains([',', '"', '\n', '\r']) {
        return value.to_string();
    }
    let escaped = value.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
#[derive(Debug, Serialize)]
struct AuditExportRecord {
    id: i64,
    ts_ms: u64,
    kind: String,
    payload: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prev_hash: Option<String>,
    hash: String,
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
async fn export_audit_logs(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Query(query): Query<AuditExportQuery>,
) -> Result<axum::response::Response, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;

    let format = query
        .format
        .as_deref()
        .unwrap_or("jsonl")
        .trim()
        .to_ascii_lowercase();
    let limit = query.limit.clamp(1, 10_000);

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.stores.sqlite.as_ref() {
        let mut logs = store
            .list_audit_logs_window(limit, query.since_ts_ms, query.before_ts_ms)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;
        if let Some(tenant_id) = admin.tenant_id.as_deref() {
            logs.retain(|log| {
                log.payload
                    .get("tenant_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(tenant_id)
            });
        }
        for log in &mut logs {
            log.payload = state.redactor.redact(std::mem::take(&mut log.payload));
        }
        return render_audit_export(&format, logs);
    }

    #[cfg(feature = "gateway-store-postgres")]
    if let Some(store) = state.stores.postgres.as_ref() {
        let mut logs = store
            .list_audit_logs_window(limit, query.since_ts_ms, query.before_ts_ms)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;
        if let Some(tenant_id) = admin.tenant_id.as_deref() {
            logs.retain(|log| {
                log.payload
                    .get("tenant_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(tenant_id)
            });
        }
        for log in &mut logs {
            log.payload = state.redactor.redact(std::mem::take(&mut log.payload));
        }
        return render_audit_export(&format, logs);
    }

    #[cfg(feature = "gateway-store-mysql")]
    if let Some(store) = state.stores.mysql.as_ref() {
        let mut logs = store
            .list_audit_logs_window(limit, query.since_ts_ms, query.before_ts_ms)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;
        if let Some(tenant_id) = admin.tenant_id.as_deref() {
            logs.retain(|log| {
                log.payload
                    .get("tenant_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(tenant_id)
            });
        }
        for log in &mut logs {
            log.payload = state.redactor.redact(std::mem::take(&mut log.payload));
        }
        return render_audit_export(&format, logs);
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.stores.redis.as_ref() {
        let mut logs = store
            .list_audit_logs_window(limit, query.since_ts_ms, query.before_ts_ms)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;
        if let Some(tenant_id) = admin.tenant_id.as_deref() {
            logs.retain(|log| {
                log.payload
                    .get("tenant_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(tenant_id)
            });
        }
        for log in &mut logs {
            log.payload = state.redactor.redact(std::mem::take(&mut log.payload));
        }
        return render_audit_export(&format, logs);
    }

    Err(error_response(
        StatusCode::BAD_REQUEST,
        "not_configured",
        "store not configured",
    ))
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
fn render_audit_export(
    format: &str,
    logs: Vec<AuditLogRecord>,
) -> Result<axum::response::Response, (StatusCode, Json<ErrorResponse>)> {
    use axum::body::Body;
    use bytes::Bytes;
    use futures_util::stream;

    let mut prev_hash: Option<String> = None;

    let mut lines = Vec::<String>::with_capacity(logs.len().saturating_add(1));

    match format {
        "jsonl" | "ndjson" => {
            for log in logs {
                let hash = audit_chain_hash(prev_hash.as_deref(), &log);
                let record = AuditExportRecord {
                    id: log.id,
                    ts_ms: log.ts_ms,
                    kind: log.kind,
                    payload: log.payload,
                    prev_hash: prev_hash.clone(),
                    hash: hash.clone(),
                };
                prev_hash = Some(hash);
                let mut line = serde_json::to_string(&record).map_err(|err| {
                    error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "encode_error",
                        err.to_string(),
                    )
                })?;
                line.push('\n');
                lines.push(line);
            }

            let stream = stream::iter(
                lines
                    .into_iter()
                    .map(|line| Ok::<Bytes, std::io::Error>(Bytes::from(line))),
            );
            let mut response = axum::response::Response::new(Body::from_stream(stream));
            response.headers_mut().insert(
                axum::http::header::CONTENT_TYPE,
                axum::http::HeaderValue::from_static("application/x-ndjson"),
            );
            Ok(response)
        }
        "csv" => {
            lines.push("id,ts_ms,kind,payload_json,prev_hash,hash\n".to_string());
            for log in logs {
                let hash = audit_chain_hash(prev_hash.as_deref(), &log);
                let payload_json = serde_json::to_string(&log.payload).unwrap_or_default();
                let line = format!(
                    "{},{},{},{},{},{}\n",
                    log.id,
                    log.ts_ms,
                    csv_escape(&log.kind),
                    csv_escape(&payload_json),
                    csv_escape(prev_hash.as_deref().unwrap_or("")),
                    csv_escape(&hash)
                );
                prev_hash = Some(hash);
                lines.push(line);
            }
            let stream = stream::iter(
                lines
                    .into_iter()
                    .map(|line| Ok::<Bytes, std::io::Error>(Bytes::from(line))),
            );
            let mut response = axum::response::Response::new(Body::from_stream(stream));
            response.headers_mut().insert(
                axum::http::header::CONTENT_TYPE,
                axum::http::HeaderValue::from_static("text/csv"),
            );
            Ok(response)
        }
        _ => Err(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            format!("unsupported export format: {format}"),
        )),
    }
}
// end inline: handlers/audit.rs
// inlined from handlers/budget_ledgers.rs
#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
async fn list_budget_ledgers(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Query(query): Query<LedgerQuery>,
) -> Result<Json<Vec<BudgetLedgerRecord>>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;
    let tenant_scopes = if let Some(tenant_id) = admin.tenant_id.as_deref() {
        let keys = { state.list_virtual_keys_snapshot() };
        Some(tenant_allowed_scopes(&keys, tenant_id))
    } else {
        None
    };

    let key_prefix = query
        .key_prefix
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.stores.sqlite.as_ref() {
        let mut ledgers = store.list_budget_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        if let Some(scopes) = tenant_scopes.as_ref() {
            ledgers.retain(|ledger| scopes.contains(ledger.key_id.as_str()));
        }
        if let Some(key_prefix) = key_prefix {
            ledgers.retain(|ledger| ledger.key_id.starts_with(key_prefix));
        }
        apply_admin_list_window(
            &mut ledgers,
            query.offset,
            query.limit,
            MAX_ADMIN_LEDGER_LIMIT,
        );
        return Ok(Json(ledgers));
    }

    #[cfg(feature = "gateway-store-postgres")]
    if let Some(store) = state.stores.postgres.as_ref() {
        let mut ledgers = store.list_budget_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        if let Some(scopes) = tenant_scopes.as_ref() {
            ledgers.retain(|ledger| scopes.contains(ledger.key_id.as_str()));
        }
        if let Some(key_prefix) = key_prefix {
            ledgers.retain(|ledger| ledger.key_id.starts_with(key_prefix));
        }
        apply_admin_list_window(
            &mut ledgers,
            query.offset,
            query.limit,
            MAX_ADMIN_LEDGER_LIMIT,
        );
        return Ok(Json(ledgers));
    }

    #[cfg(feature = "gateway-store-mysql")]
    if let Some(store) = state.stores.mysql.as_ref() {
        let mut ledgers = store.list_budget_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        if let Some(scopes) = tenant_scopes.as_ref() {
            ledgers.retain(|ledger| scopes.contains(ledger.key_id.as_str()));
        }
        if let Some(key_prefix) = key_prefix {
            ledgers.retain(|ledger| ledger.key_id.starts_with(key_prefix));
        }
        apply_admin_list_window(
            &mut ledgers,
            query.offset,
            query.limit,
            MAX_ADMIN_LEDGER_LIMIT,
        );
        return Ok(Json(ledgers));
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.stores.redis.as_ref() {
        let mut ledgers = store.list_budget_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        if let Some(scopes) = tenant_scopes.as_ref() {
            ledgers.retain(|ledger| scopes.contains(ledger.key_id.as_str()));
        }
        if let Some(key_prefix) = key_prefix {
            ledgers.retain(|ledger| ledger.key_id.starts_with(key_prefix));
        }
        apply_admin_list_window(
            &mut ledgers,
            query.offset,
            query.limit,
            MAX_ADMIN_LEDGER_LIMIT,
        );
        return Ok(Json(ledgers));
    }

    Err(error_response(
        StatusCode::BAD_REQUEST,
        "not_configured",
        "store not configured",
    ))
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
#[derive(Debug, Serialize)]
struct ProjectBudgetLedger {
    project_id: Option<String>,
    spent_tokens: u64,
    reserved_tokens: u64,
    key_count: usize,
    updated_at_ms: u64,
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
#[derive(Debug, Serialize)]
struct UserBudgetLedger {
    user_id: Option<String>,
    spent_tokens: u64,
    reserved_tokens: u64,
    key_count: usize,
    updated_at_ms: u64,
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
#[derive(Debug, Serialize)]
struct TenantBudgetLedger {
    tenant_id: Option<String>,
    spent_tokens: u64,
    reserved_tokens: u64,
    key_count: usize,
    updated_at_ms: u64,
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
fn group_budget_ledgers_by_project(
    ledgers: &[BudgetLedgerRecord],
    keys: &[VirtualKeyConfig],
) -> Vec<ProjectBudgetLedger> {
    use std::collections::BTreeMap;

    let mut key_to_project = std::collections::HashMap::<&str, Option<&str>>::new();
    for key in keys {
        key_to_project.insert(key.id.as_str(), key.project_id.as_deref());
    }

    let mut grouped = BTreeMap::<Option<String>, (u64, u64, usize, u64)>::new();
    for ledger in ledgers {
        let ledger_key_id = ledger.key_id.as_str();
        let project_id = if let Some(project_id) = key_to_project.get(ledger_key_id).copied() {
            project_id.map(|id| id.to_string())
        } else if ledger_key_id.starts_with("tenant:")
            || ledger_key_id.starts_with("project:")
            || ledger_key_id.starts_with("user:")
        {
            continue;
        } else {
            None
        };
        let entry = grouped.entry(project_id).or_insert((0, 0, 0, 0));
        entry.0 = entry.0.saturating_add(ledger.spent_tokens);
        entry.1 = entry.1.saturating_add(ledger.reserved_tokens);
        entry.2 = entry.2.saturating_add(1);
        entry.3 = entry.3.max(ledger.updated_at_ms);
    }

    grouped
        .into_iter()
        .map(
            |(project_id, (spent_tokens, reserved_tokens, key_count, updated_at_ms))| {
                ProjectBudgetLedger {
                    project_id,
                    spent_tokens,
                    reserved_tokens,
                    key_count,
                    updated_at_ms,
                }
            },
        )
        .collect()
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
fn group_budget_ledgers_by_user(
    ledgers: &[BudgetLedgerRecord],
    keys: &[VirtualKeyConfig],
) -> Vec<UserBudgetLedger> {
    use std::collections::BTreeMap;

    let mut key_to_user = std::collections::HashMap::<&str, Option<&str>>::new();
    for key in keys {
        key_to_user.insert(key.id.as_str(), key.user_id.as_deref());
    }

    let mut grouped = BTreeMap::<Option<String>, (u64, u64, usize, u64)>::new();
    for ledger in ledgers {
        let ledger_key_id = ledger.key_id.as_str();
        let user_id = if let Some(user_id) = key_to_user.get(ledger_key_id).copied() {
            user_id.map(|id| id.to_string())
        } else if ledger_key_id.starts_with("tenant:")
            || ledger_key_id.starts_with("project:")
            || ledger_key_id.starts_with("user:")
        {
            continue;
        } else {
            None
        };
        let entry = grouped.entry(user_id).or_insert((0, 0, 0, 0));
        entry.0 = entry.0.saturating_add(ledger.spent_tokens);
        entry.1 = entry.1.saturating_add(ledger.reserved_tokens);
        entry.2 = entry.2.saturating_add(1);
        entry.3 = entry.3.max(ledger.updated_at_ms);
    }

    grouped
        .into_iter()
        .map(
            |(user_id, (spent_tokens, reserved_tokens, key_count, updated_at_ms))| {
                UserBudgetLedger {
                    user_id,
                    spent_tokens,
                    reserved_tokens,
                    key_count,
                    updated_at_ms,
                }
            },
        )
        .collect()
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
fn group_budget_ledgers_by_tenant(
    ledgers: &[BudgetLedgerRecord],
    keys: &[VirtualKeyConfig],
) -> Vec<TenantBudgetLedger> {
    use std::collections::BTreeMap;

    let mut key_to_tenant = std::collections::HashMap::<&str, Option<&str>>::new();
    for key in keys {
        key_to_tenant.insert(key.id.as_str(), key.tenant_id.as_deref());
    }

    let mut grouped = BTreeMap::<Option<String>, (u64, u64, usize, u64)>::new();
    for ledger in ledgers {
        let ledger_key_id = ledger.key_id.as_str();
        let tenant_id = if let Some(tenant_id) = key_to_tenant.get(ledger_key_id).copied() {
            tenant_id.map(|id| id.to_string())
        } else if ledger_key_id.starts_with("tenant:")
            || ledger_key_id.starts_with("project:")
            || ledger_key_id.starts_with("user:")
        {
            continue;
        } else {
            None
        };
        let entry = grouped.entry(tenant_id).or_insert((0, 0, 0, 0));
        entry.0 = entry.0.saturating_add(ledger.spent_tokens);
        entry.1 = entry.1.saturating_add(ledger.reserved_tokens);
        entry.2 = entry.2.saturating_add(1);
        entry.3 = entry.3.max(ledger.updated_at_ms);
    }

    grouped
        .into_iter()
        .map(
            |(tenant_id, (spent_tokens, reserved_tokens, key_count, updated_at_ms))| {
                TenantBudgetLedger {
                    tenant_id,
                    spent_tokens,
                    reserved_tokens,
                    key_count,
                    updated_at_ms,
                }
            },
        )
        .collect()
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
async fn list_project_budget_ledgers(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ProjectBudgetLedger>>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;

    let mut keys = { state.list_virtual_keys_snapshot() };
    if let Some(tenant_id) = admin.tenant_id.as_deref() {
        keys.retain(|key| key.tenant_id.as_deref() == Some(tenant_id));
    }

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.stores.sqlite.as_ref() {
        let ledgers = store.list_budget_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_budget_ledgers_by_project(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-postgres")]
    if let Some(store) = state.stores.postgres.as_ref() {
        let ledgers = store.list_budget_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_budget_ledgers_by_project(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-mysql")]
    if let Some(store) = state.stores.mysql.as_ref() {
        let ledgers = store.list_budget_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_budget_ledgers_by_project(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.stores.redis.as_ref() {
        let ledgers = store.list_budget_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_budget_ledgers_by_project(&ledgers, &keys)));
    }

    Err(error_response(
        StatusCode::BAD_REQUEST,
        "not_configured",
        "store not configured",
    ))
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
async fn list_user_budget_ledgers(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
) -> Result<Json<Vec<UserBudgetLedger>>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;

    let mut keys = { state.list_virtual_keys_snapshot() };
    if let Some(tenant_id) = admin.tenant_id.as_deref() {
        keys.retain(|key| key.tenant_id.as_deref() == Some(tenant_id));
    }

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.stores.sqlite.as_ref() {
        let ledgers = store.list_budget_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_budget_ledgers_by_user(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-postgres")]
    if let Some(store) = state.stores.postgres.as_ref() {
        let ledgers = store.list_budget_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_budget_ledgers_by_user(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-mysql")]
    if let Some(store) = state.stores.mysql.as_ref() {
        let ledgers = store.list_budget_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_budget_ledgers_by_user(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.stores.redis.as_ref() {
        let ledgers = store.list_budget_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_budget_ledgers_by_user(&ledgers, &keys)));
    }

    Err(error_response(
        StatusCode::BAD_REQUEST,
        "not_configured",
        "store not configured",
    ))
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
async fn list_tenant_budget_ledgers(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
) -> Result<Json<Vec<TenantBudgetLedger>>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;

    let mut keys = { state.list_virtual_keys_snapshot() };
    if let Some(tenant_id) = admin.tenant_id.as_deref() {
        keys.retain(|key| key.tenant_id.as_deref() == Some(tenant_id));
    }

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.stores.sqlite.as_ref() {
        let ledgers = store.list_budget_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_budget_ledgers_by_tenant(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-postgres")]
    if let Some(store) = state.stores.postgres.as_ref() {
        let ledgers = store.list_budget_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_budget_ledgers_by_tenant(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-mysql")]
    if let Some(store) = state.stores.mysql.as_ref() {
        let ledgers = store.list_budget_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_budget_ledgers_by_tenant(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.stores.redis.as_ref() {
        let ledgers = store.list_budget_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_budget_ledgers_by_tenant(&ledgers, &keys)));
    }

    Err(error_response(
        StatusCode::BAD_REQUEST,
        "not_configured",
        "store not configured",
    ))
}
// end inline: handlers/budget_ledgers.rs
// inlined from handlers/cost_ledgers.rs
#[cfg(all(
    feature = "gateway-costing",
    any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ),
))]
async fn list_cost_ledgers(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Query(query): Query<LedgerQuery>,
) -> Result<Json<Vec<CostLedgerRecord>>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;
    let tenant_scopes = if let Some(tenant_id) = admin.tenant_id.as_deref() {
        let keys = { state.list_virtual_keys_snapshot() };
        Some(tenant_allowed_scopes(&keys, tenant_id))
    } else {
        None
    };

    let key_prefix = query
        .key_prefix
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.stores.sqlite.as_ref() {
        let mut ledgers = store.list_cost_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        if let Some(scopes) = tenant_scopes.as_ref() {
            ledgers.retain(|ledger| scopes.contains(ledger.key_id.as_str()));
        }
        if let Some(key_prefix) = key_prefix {
            ledgers.retain(|ledger| ledger.key_id.starts_with(key_prefix));
        }
        apply_admin_list_window(
            &mut ledgers,
            query.offset,
            query.limit,
            MAX_ADMIN_LEDGER_LIMIT,
        );
        return Ok(Json(ledgers));
    }

    #[cfg(feature = "gateway-store-postgres")]
    if let Some(store) = state.stores.postgres.as_ref() {
        let mut ledgers = store.list_cost_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        if let Some(scopes) = tenant_scopes.as_ref() {
            ledgers.retain(|ledger| scopes.contains(ledger.key_id.as_str()));
        }
        if let Some(key_prefix) = key_prefix {
            ledgers.retain(|ledger| ledger.key_id.starts_with(key_prefix));
        }
        apply_admin_list_window(
            &mut ledgers,
            query.offset,
            query.limit,
            MAX_ADMIN_LEDGER_LIMIT,
        );
        return Ok(Json(ledgers));
    }

    #[cfg(feature = "gateway-store-mysql")]
    if let Some(store) = state.stores.mysql.as_ref() {
        let mut ledgers = store.list_cost_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        if let Some(scopes) = tenant_scopes.as_ref() {
            ledgers.retain(|ledger| scopes.contains(ledger.key_id.as_str()));
        }
        if let Some(key_prefix) = key_prefix {
            ledgers.retain(|ledger| ledger.key_id.starts_with(key_prefix));
        }
        apply_admin_list_window(
            &mut ledgers,
            query.offset,
            query.limit,
            MAX_ADMIN_LEDGER_LIMIT,
        );
        return Ok(Json(ledgers));
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.stores.redis.as_ref() {
        let mut ledgers = store.list_cost_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        if let Some(scopes) = tenant_scopes.as_ref() {
            ledgers.retain(|ledger| scopes.contains(ledger.key_id.as_str()));
        }
        if let Some(key_prefix) = key_prefix {
            ledgers.retain(|ledger| ledger.key_id.starts_with(key_prefix));
        }
        apply_admin_list_window(
            &mut ledgers,
            query.offset,
            query.limit,
            MAX_ADMIN_LEDGER_LIMIT,
        );
        return Ok(Json(ledgers));
    }

    Err(error_response(
        StatusCode::BAD_REQUEST,
        "not_configured",
        "store not configured",
    ))
}

#[cfg(all(
    feature = "gateway-costing",
    any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ),
))]
#[derive(Debug, Serialize)]
struct ProjectCostLedger {
    project_id: Option<String>,
    spent_usd_micros: u64,
    reserved_usd_micros: u64,
    key_count: usize,
    updated_at_ms: u64,
}

#[cfg(all(
    feature = "gateway-costing",
    any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ),
))]
#[derive(Debug, Serialize)]
struct UserCostLedger {
    user_id: Option<String>,
    spent_usd_micros: u64,
    reserved_usd_micros: u64,
    key_count: usize,
    updated_at_ms: u64,
}

#[cfg(all(
    feature = "gateway-costing",
    any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ),
))]
#[derive(Debug, Serialize)]
struct TenantCostLedger {
    tenant_id: Option<String>,
    spent_usd_micros: u64,
    reserved_usd_micros: u64,
    key_count: usize,
    updated_at_ms: u64,
}

#[cfg(all(
    feature = "gateway-costing",
    any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ),
))]
fn group_cost_ledgers_by_project(
    ledgers: &[CostLedgerRecord],
    keys: &[VirtualKeyConfig],
) -> Vec<ProjectCostLedger> {
    use std::collections::BTreeMap;

    let mut key_to_project = std::collections::HashMap::<&str, Option<&str>>::new();
    for key in keys {
        key_to_project.insert(key.id.as_str(), key.project_id.as_deref());
    }

    let mut grouped = BTreeMap::<Option<String>, (u64, u64, usize, u64)>::new();
    for ledger in ledgers {
        let ledger_key_id = ledger.key_id.as_str();
        let project_id = if let Some(project_id) = key_to_project.get(ledger_key_id).copied() {
            project_id.map(|id| id.to_string())
        } else if ledger_key_id.starts_with("tenant:")
            || ledger_key_id.starts_with("project:")
            || ledger_key_id.starts_with("user:")
        {
            continue;
        } else {
            None
        };
        let entry = grouped.entry(project_id).or_insert((0, 0, 0, 0));
        entry.0 = entry.0.saturating_add(ledger.spent_usd_micros);
        entry.1 = entry.1.saturating_add(ledger.reserved_usd_micros);
        entry.2 = entry.2.saturating_add(1);
        entry.3 = entry.3.max(ledger.updated_at_ms);
    }

    grouped
        .into_iter()
        .map(
            |(project_id, (spent_usd_micros, reserved_usd_micros, key_count, updated_at_ms))| {
                ProjectCostLedger {
                    project_id,
                    spent_usd_micros,
                    reserved_usd_micros,
                    key_count,
                    updated_at_ms,
                }
            },
        )
        .collect()
}

#[cfg(all(
    feature = "gateway-costing",
    any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ),
))]
fn group_cost_ledgers_by_user(
    ledgers: &[CostLedgerRecord],
    keys: &[VirtualKeyConfig],
) -> Vec<UserCostLedger> {
    use std::collections::BTreeMap;

    let mut key_to_user = std::collections::HashMap::<&str, Option<&str>>::new();
    for key in keys {
        key_to_user.insert(key.id.as_str(), key.user_id.as_deref());
    }

    let mut grouped = BTreeMap::<Option<String>, (u64, u64, usize, u64)>::new();
    for ledger in ledgers {
        let ledger_key_id = ledger.key_id.as_str();
        let user_id = if let Some(user_id) = key_to_user.get(ledger_key_id).copied() {
            user_id.map(|id| id.to_string())
        } else if ledger_key_id.starts_with("tenant:")
            || ledger_key_id.starts_with("project:")
            || ledger_key_id.starts_with("user:")
        {
            continue;
        } else {
            None
        };
        let entry = grouped.entry(user_id).or_insert((0, 0, 0, 0));
        entry.0 = entry.0.saturating_add(ledger.spent_usd_micros);
        entry.1 = entry.1.saturating_add(ledger.reserved_usd_micros);
        entry.2 = entry.2.saturating_add(1);
        entry.3 = entry.3.max(ledger.updated_at_ms);
    }

    grouped
        .into_iter()
        .map(
            |(user_id, (spent_usd_micros, reserved_usd_micros, key_count, updated_at_ms))| {
                UserCostLedger {
                    user_id,
                    spent_usd_micros,
                    reserved_usd_micros,
                    key_count,
                    updated_at_ms,
                }
            },
        )
        .collect()
}

#[cfg(all(
    feature = "gateway-costing",
    any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ),
))]
fn group_cost_ledgers_by_tenant(
    ledgers: &[CostLedgerRecord],
    keys: &[VirtualKeyConfig],
) -> Vec<TenantCostLedger> {
    use std::collections::BTreeMap;

    let mut key_to_tenant = std::collections::HashMap::<&str, Option<&str>>::new();
    for key in keys {
        key_to_tenant.insert(key.id.as_str(), key.tenant_id.as_deref());
    }

    let mut grouped = BTreeMap::<Option<String>, (u64, u64, usize, u64)>::new();
    for ledger in ledgers {
        let ledger_key_id = ledger.key_id.as_str();
        let tenant_id = if let Some(tenant_id) = key_to_tenant.get(ledger_key_id).copied() {
            tenant_id.map(|id| id.to_string())
        } else if ledger_key_id.starts_with("tenant:")
            || ledger_key_id.starts_with("project:")
            || ledger_key_id.starts_with("user:")
        {
            continue;
        } else {
            None
        };
        let entry = grouped.entry(tenant_id).or_insert((0, 0, 0, 0));
        entry.0 = entry.0.saturating_add(ledger.spent_usd_micros);
        entry.1 = entry.1.saturating_add(ledger.reserved_usd_micros);
        entry.2 = entry.2.saturating_add(1);
        entry.3 = entry.3.max(ledger.updated_at_ms);
    }

    grouped
        .into_iter()
        .map(
            |(tenant_id, (spent_usd_micros, reserved_usd_micros, key_count, updated_at_ms))| {
                TenantCostLedger {
                    tenant_id,
                    spent_usd_micros,
                    reserved_usd_micros,
                    key_count,
                    updated_at_ms,
                }
            },
        )
        .collect()
}

#[cfg(all(
    feature = "gateway-costing",
    any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ),
))]
async fn list_project_cost_ledgers(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ProjectCostLedger>>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;

    let mut keys = { state.list_virtual_keys_snapshot() };
    if let Some(tenant_id) = admin.tenant_id.as_deref() {
        keys.retain(|key| key.tenant_id.as_deref() == Some(tenant_id));
    }

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.stores.sqlite.as_ref() {
        let ledgers = store.list_cost_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_cost_ledgers_by_project(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-postgres")]
    if let Some(store) = state.stores.postgres.as_ref() {
        let ledgers = store.list_cost_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_cost_ledgers_by_project(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-mysql")]
    if let Some(store) = state.stores.mysql.as_ref() {
        let ledgers = store.list_cost_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_cost_ledgers_by_project(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.stores.redis.as_ref() {
        let ledgers = store.list_cost_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_cost_ledgers_by_project(&ledgers, &keys)));
    }

    Err(error_response(
        StatusCode::BAD_REQUEST,
        "not_configured",
        "store not configured",
    ))
}

#[cfg(all(
    feature = "gateway-costing",
    any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ),
))]
async fn list_user_cost_ledgers(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
) -> Result<Json<Vec<UserCostLedger>>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;

    let mut keys = { state.list_virtual_keys_snapshot() };
    if let Some(tenant_id) = admin.tenant_id.as_deref() {
        keys.retain(|key| key.tenant_id.as_deref() == Some(tenant_id));
    }

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.stores.sqlite.as_ref() {
        let ledgers = store.list_cost_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_cost_ledgers_by_user(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-postgres")]
    if let Some(store) = state.stores.postgres.as_ref() {
        let ledgers = store.list_cost_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_cost_ledgers_by_user(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-mysql")]
    if let Some(store) = state.stores.mysql.as_ref() {
        let ledgers = store.list_cost_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_cost_ledgers_by_user(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.stores.redis.as_ref() {
        let ledgers = store.list_cost_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_cost_ledgers_by_user(&ledgers, &keys)));
    }

    Err(error_response(
        StatusCode::BAD_REQUEST,
        "not_configured",
        "store not configured",
    ))
}

#[cfg(all(
    feature = "gateway-costing",
    any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ),
))]
async fn list_tenant_cost_ledgers(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
) -> Result<Json<Vec<TenantCostLedger>>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;

    let mut keys = { state.list_virtual_keys_snapshot() };
    if let Some(tenant_id) = admin.tenant_id.as_deref() {
        keys.retain(|key| key.tenant_id.as_deref() == Some(tenant_id));
    }

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.stores.sqlite.as_ref() {
        let ledgers = store.list_cost_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_cost_ledgers_by_tenant(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-postgres")]
    if let Some(store) = state.stores.postgres.as_ref() {
        let ledgers = store.list_cost_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_cost_ledgers_by_tenant(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-mysql")]
    if let Some(store) = state.stores.mysql.as_ref() {
        let ledgers = store.list_cost_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_cost_ledgers_by_tenant(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.stores.redis.as_ref() {
        let ledgers = store.list_cost_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_cost_ledgers_by_tenant(&ledgers, &keys)));
    }

    Err(error_response(
        StatusCode::BAD_REQUEST,
        "not_configured",
        "store not configured",
    ))
}
// end inline: handlers/cost_ledgers.rs
// end inline: admin/handlers.rs
// inlined from admin/backends.rs
#[cfg(feature = "gateway-routing-advanced")]
async fn list_backends(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
) -> Result<Json<Vec<BackendHealthSnapshot>>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;
    if admin.tenant_id.is_some() {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "tenant-scoped admin tokens cannot access backend health",
        ));
    }

    let Some(health) = state.proxy.backend_health.as_ref() else {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "not_configured",
            "proxy routing not enabled",
        ));
    };

    let mut names: Vec<String> = state.backends.proxy_backends.keys().cloned().collect();
    names.sort();

    let mut out = Vec::with_capacity(names.len());
    {
        let health = health.lock().await;
        for name in names {
            let snapshot = health
                .get(name.as_str())
                .map(|entry| entry.snapshot(&name))
                .unwrap_or_else(|| BackendHealth::default().snapshot(&name));
            out.push(snapshot);
        }
        drop(health);
    }

    Ok(Json(out))
}

#[cfg(feature = "gateway-routing-advanced")]
async fn reset_backend(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<Json<BackendHealthSnapshot>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_write(&state, &headers)?;
    if admin.tenant_id.is_some() {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "tenant-scoped admin tokens cannot reset backends",
        ));
    }

    let Some(health) = state.proxy.backend_health.as_ref() else {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "not_configured",
            "proxy routing not enabled",
        ));
    };

    let mut health = health.lock().await;
    health.remove(name.as_str());
    drop(health);

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    append_admin_audit_log(
        &state,
        "admin.backend.reset",
        serde_json::json!({
            "backend": &name,
        }),
    )
    .await;

    Ok(Json(BackendHealth::default().snapshot(&name)))
}
// end inline: admin/backends.rs
// inlined from admin/config_versions.rs
const MAX_CONFIG_VERSIONS_LIMIT: usize = 1_000;

#[derive(Debug, Deserialize)]
struct ListConfigVersionsQuery {
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: usize,
}

#[derive(Debug, Deserialize)]
struct GetConfigVersionQuery {
    #[serde(default)]
    include_tokens: bool,
}

#[derive(Debug, Deserialize)]
struct ConfigDiffQuery {
    from_version_id: String,
    to_version_id: String,
    #[serde(default)]
    include_tokens: bool,
}

#[derive(Debug, Deserialize)]
struct ExportConfigQuery {
    #[serde(default)]
    version_id: Option<String>,
    #[serde(default)]
    include_tokens: bool,
}

#[derive(Debug, Deserialize)]
struct ConfigRollbackRequest {
    version_id: String,
    #[serde(default)]
    dry_run: bool,
}

#[derive(Debug, Deserialize)]
struct ConfigRouterUpsertRequest {
    router: RouterConfig,
    #[serde(default)]
    dry_run: bool,
}

#[derive(Debug, Serialize)]
struct ConfigVersionDetailResponse {
    #[serde(flatten)]
    info: ConfigVersionInfo,
    virtual_keys: Vec<VirtualKeyConfig>,
    router: RouterConfig,
}

#[derive(Debug, Serialize)]
struct ConfigRollbackResponse {
    dry_run: bool,
    noop: bool,
    rolled_back_to_version_id: String,
    target_version: ConfigVersionInfo,
    current_version: ConfigVersionInfo,
}

#[derive(Debug, Serialize)]
struct ConfigVersionDiffSummary {
    from_virtual_key_count: usize,
    to_virtual_key_count: usize,
    added: usize,
    removed: usize,
    changed: usize,
    unchanged: usize,
    router_changed: bool,
}

#[derive(Debug, Serialize)]
struct ConfigVersionDiffEntry {
    id: String,
    before: VirtualKeyConfig,
    after: VirtualKeyConfig,
}

#[derive(Debug, Serialize)]
struct ConfigVersionDiffResponse {
    from_version: ConfigVersionInfo,
    to_version: ConfigVersionInfo,
    summary: ConfigVersionDiffSummary,
    added: Vec<VirtualKeyConfig>,
    removed: Vec<VirtualKeyConfig>,
    changed: Vec<ConfigVersionDiffEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    router_before: Option<RouterConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    router_after: Option<RouterConfig>,
}

#[derive(Debug, Deserialize)]
struct ConfigValidateRequest {
    #[serde(default)]
    virtual_keys: Vec<VirtualKeyConfig>,
    #[serde(default)]
    router: Option<RouterConfig>,
    #[serde(default)]
    expected_virtual_keys_sha256: Option<String>,
    #[serde(default)]
    expected_router_sha256: Option<String>,
}

#[derive(Debug, Serialize)]
struct ConfigValidationIssue {
    code: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
}

#[derive(Debug, Serialize)]
struct ConfigValidateResponse {
    valid: bool,
    virtual_key_count: usize,
    computed_virtual_keys_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    router_default_backend_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    router_rule_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    computed_router_sha256: Option<String>,
    issues: Vec<ConfigValidationIssue>,
}

#[derive(Debug, Serialize)]
struct ConfigRouterUpsertResponse {
    dry_run: bool,
    noop: bool,
    router_changed: bool,
    target_router_sha256: String,
    previous_version: ConfigVersionInfo,
    current_version: ConfigVersionInfo,
}

fn redact_virtual_key_tokens(keys: &mut [VirtualKeyConfig]) {
    for key in keys {
        key.token = "redacted".to_string();
    }
}

fn redact_diff_change_tokens(changes: &mut [ConfigVersionDiffEntry]) {
    for change in changes {
        change.before.token = "redacted".to_string();
        change.after.token = "redacted".to_string();
    }
}

fn virtual_key_equal(lhs: &VirtualKeyConfig, rhs: &VirtualKeyConfig) -> bool {
    serde_json::to_vec(lhs).ok() == serde_json::to_vec(rhs).ok()
}

fn router_config_equal(lhs: &RouterConfig, rhs: &RouterConfig) -> bool {
    serde_json::to_vec(lhs).ok() == serde_json::to_vec(rhs).ok()
}

fn validate_router_against_backends(
    router: &RouterConfig,
    backend_names: &std::collections::HashSet<String>,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let mut unknown_refs: Vec<String> = Vec::new();
    let mut invalid_fields: Vec<String> = Vec::new();

    for (idx, backend) in router.default_backends.iter().enumerate() {
        let name = backend.backend.trim();
        if name.is_empty() {
            invalid_fields.push(format!("router.default_backends[{idx}].backend"));
            continue;
        }
        if !backend_names.contains(name) {
            unknown_refs.push(name.to_string());
        }
    }

    for (rule_idx, rule) in router.rules.iter().enumerate() {
        let model_prefix = rule.model_prefix.trim();
        if model_prefix.is_empty() {
            invalid_fields.push(format!("router.rules[{rule_idx}].model_prefix"));
        }

        let mut has_backend = false;
        let legacy_backend = rule.backend.trim();
        if !legacy_backend.is_empty() {
            has_backend = true;
            if !backend_names.contains(legacy_backend) {
                unknown_refs.push(legacy_backend.to_string());
            }
        }

        for (backend_idx, backend) in rule.backends.iter().enumerate() {
            let name = backend.backend.trim();
            if name.is_empty() {
                invalid_fields.push(format!(
                    "router.rules[{rule_idx}].backends[{backend_idx}].backend"
                ));
                continue;
            }
            has_backend = true;
            if !backend_names.contains(name) {
                unknown_refs.push(name.to_string());
            }
        }

        if !has_backend {
            invalid_fields.push(format!(
                "router.rules[{rule_idx}] requires `backend` or non-empty `backends[]`"
            ));
        }
    }

    if !invalid_fields.is_empty() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            format!(
                "invalid router config fields: {}",
                invalid_fields.join(", ")
            ),
        ));
    }

    if !unknown_refs.is_empty() {
        unknown_refs.sort();
        unknown_refs.dedup();
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            format!(
                "router references unknown backends: {}",
                unknown_refs.join(", ")
            ),
        ));
    }

    Ok(())
}

fn push_validation_issue(
    issues: &mut Vec<ConfigValidationIssue>,
    code: &'static str,
    message: impl Into<String>,
    path: Option<String>,
) {
    issues.push(ConfigValidationIssue {
        code,
        message: message.into(),
        path,
    });
}

async fn get_config_version(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
) -> Result<Json<ConfigVersionInfo>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;
    if admin.tenant_id.is_some() {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "tenant-scoped admin tokens cannot access global config versions",
        ));
    }

    let history = state.config_versions.lock().await;
    let Some(current) = history.current_info() else {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "config version history is empty",
        ));
    };

    Ok(Json(current))
}

async fn get_config_version_by_id(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Path(version_id): Path<String>,
    Query(query): Query<GetConfigVersionQuery>,
) -> Result<Json<ConfigVersionDetailResponse>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;
    if admin.tenant_id.is_some() {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "tenant-scoped admin tokens cannot access global config versions",
        ));
    }

    let version_id = version_id.trim();
    if version_id.is_empty() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "version_id cannot be empty",
        ));
    }

    let Some(snapshot) = state.config_versions.lock().await.find_snapshot(version_id) else {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("config version not found: {version_id}"),
        ));
    };

    let mut virtual_keys = snapshot.virtual_keys;
    if !query.include_tokens {
        redact_virtual_key_tokens(&mut virtual_keys);
    }

    Ok(Json(ConfigVersionDetailResponse {
        info: snapshot.info,
        virtual_keys,
        router: snapshot.router,
    }))
}

async fn export_config(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Query(query): Query<ExportConfigQuery>,
) -> Result<Json<ConfigVersionDetailResponse>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;
    if admin.tenant_id.is_some() {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "tenant-scoped admin tokens cannot access global config versions",
        ));
    }

    let snapshot = {
        let history = state.config_versions.lock().await;
        if let Some(version_id) = query
            .version_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            history.find_snapshot(version_id).ok_or_else(|| {
                error_response(
                    StatusCode::NOT_FOUND,
                    "not_found",
                    format!("config version not found: {version_id}"),
                )
            })?
        } else {
            let current = history.current_info().ok_or_else(|| {
                error_response(
                    StatusCode::NOT_FOUND,
                    "not_found",
                    "config version history is empty",
                )
            })?;
            history
                .find_snapshot(current.version_id.as_str())
                .ok_or_else(|| {
                    error_response(
                        StatusCode::NOT_FOUND,
                        "not_found",
                        "config version history is empty",
                    )
                })?
        }
    };

    let mut virtual_keys = snapshot.virtual_keys;
    if !query.include_tokens {
        redact_virtual_key_tokens(&mut virtual_keys);
    }

    Ok(Json(ConfigVersionDetailResponse {
        info: snapshot.info,
        virtual_keys,
        router: snapshot.router,
    }))
}

async fn validate_config_payload(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Json(payload): Json<ConfigValidateRequest>,
) -> Result<Json<ConfigValidateResponse>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;
    if admin.tenant_id.is_some() {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "tenant-scoped admin tokens cannot validate global config payloads",
        ));
    }

    let computed_virtual_keys_sha256 = virtual_keys_sha256(&payload.virtual_keys);
    let mut issues = Vec::new();
    let mut seen_ids: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    let mut seen_tokens: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();

    for (idx, key) in payload.virtual_keys.iter().enumerate() {
        let id = key.id.trim();
        if id.is_empty() {
            push_validation_issue(
                &mut issues,
                "invalid_id",
                "virtual key id cannot be empty",
                Some(format!("/virtual_keys/{idx}/id")),
            );
        } else if let Some(first_idx) = seen_ids.insert(id, idx) {
            push_validation_issue(
                &mut issues,
                "duplicate_id",
                format!("duplicate virtual key id `{id}` (first at index {first_idx})"),
                Some(format!("/virtual_keys/{idx}/id")),
            );
        }

        let token = key.token.trim();
        if token.is_empty() {
            push_validation_issue(
                &mut issues,
                "invalid_token",
                "virtual key token cannot be empty",
                Some(format!("/virtual_keys/{idx}/token")),
            );
        } else if let Some(first_idx) = seen_tokens.insert(token, idx) {
            push_validation_issue(
                &mut issues,
                "duplicate_token",
                format!("duplicate virtual key token at index {idx} (first at index {first_idx})"),
                Some(format!("/virtual_keys/{idx}/token")),
            );
        }
    }

    if let Some(expected) = payload
        .expected_virtual_keys_sha256
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if expected != computed_virtual_keys_sha256 {
            push_validation_issue(
                &mut issues,
                "hash_mismatch",
                format!(
                    "virtual_keys_sha256 mismatch: expected={expected}, got={computed_virtual_keys_sha256}"
                ),
                None,
            );
        }
    }

    let mut router_default_backend_count = None;
    let mut router_rule_count = None;
    let mut computed_router_sha256 = None;
    if let Some(router) = payload.router.as_ref() {
        let backend_names = state
            .backend_names_snapshot()
            .into_iter()
            .collect::<std::collections::HashSet<_>>();
        if let Err((_, Json(err))) = validate_router_against_backends(router, &backend_names) {
            push_validation_issue(
                &mut issues,
                "invalid_router",
                err.error.message,
                Some("/router".to_string()),
            );
        }

        let computed_router = router_sha256(router);
        if let Some(expected) = payload
            .expected_router_sha256
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if expected != computed_router {
                push_validation_issue(
                    &mut issues,
                    "router_hash_mismatch",
                    format!("router_sha256 mismatch: expected={expected}, got={computed_router}"),
                    None,
                );
            }
        }
        router_default_backend_count = Some(router.default_backends.len());
        router_rule_count = Some(router.rules.len());
        computed_router_sha256 = Some(computed_router);
    }

    Ok(Json(ConfigValidateResponse {
        valid: issues.is_empty(),
        virtual_key_count: payload.virtual_keys.len(),
        computed_virtual_keys_sha256,
        router_default_backend_count,
        router_rule_count,
        computed_router_sha256,
        issues,
    }))
}

async fn upsert_config_router(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Json(payload): Json<ConfigRouterUpsertRequest>,
) -> Result<Json<ConfigRouterUpsertResponse>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_write(&state, &headers)?;
    if admin.tenant_id.is_some() {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "tenant-scoped admin tokens cannot modify global config router",
        ));
    }

    let current_version = {
        let history = state.config_versions.lock().await;
        history.current_info()
    };
    let Some(current_version) = current_version else {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "config version history is empty",
        ));
    };

    let backend_names = state
        .backend_names_snapshot()
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let current_router = state.router_config_snapshot();
    let current_keys = state.list_virtual_keys_snapshot();
    validate_router_against_backends(&payload.router, &backend_names)?;
    let router_changed = !router_config_equal(&current_router, &payload.router);
    let target_router_sha256 = router_sha256(&payload.router);

    if payload.dry_run || !router_changed {
        return Ok(Json(ConfigRouterUpsertResponse {
            dry_run: payload.dry_run,
            noop: true,
            router_changed,
            target_router_sha256,
            previous_version: current_version.clone(),
            current_version,
        }));
    }

    state.gateway.replace_router_config(payload.router.clone());
    state.sync_control_plane_from_gateway();

    let reason = "admin.config.router.upsert";
    let next_version = persist_virtual_keys(&state, &current_keys, reason).await?;

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    append_admin_audit_log(
        &state,
        "admin.config.router.upsert",
        serde_json::json!({
            "previous_version_id": current_version.version_id,
            "result_version_id": &next_version.version_id,
            "router_rule_count": next_version.router_rule_count,
            "router_sha256": &next_version.router_sha256,
        }),
    )
    .await;

    Ok(Json(ConfigRouterUpsertResponse {
        dry_run: false,
        noop: false,
        router_changed: true,
        target_router_sha256,
        previous_version: current_version,
        current_version: next_version,
    }))
}

async fn diff_config_versions(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Query(query): Query<ConfigDiffQuery>,
) -> Result<Json<ConfigVersionDiffResponse>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;
    if admin.tenant_id.is_some() {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "tenant-scoped admin tokens cannot access global config versions",
        ));
    }

    let from_version_id = query.from_version_id.trim();
    if from_version_id.is_empty() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "from_version_id cannot be empty",
        ));
    }

    let to_version_id = query.to_version_id.trim();
    if to_version_id.is_empty() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "to_version_id cannot be empty",
        ));
    }

    let (from, to) = {
        let history = state.config_versions.lock().await;
        (
            history.find_snapshot(from_version_id),
            history.find_snapshot(to_version_id),
        )
    };
    let Some(from) = from else {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("config version not found: {from_version_id}"),
        ));
    };
    let Some(to) = to else {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("config version not found: {to_version_id}"),
        ));
    };

    let mut from_by_id: BTreeMap<String, VirtualKeyConfig> = BTreeMap::new();
    for key in from.virtual_keys {
        from_by_id.insert(key.id.clone(), key);
    }

    let mut to_by_id: BTreeMap<String, VirtualKeyConfig> = BTreeMap::new();
    for key in to.virtual_keys {
        to_by_id.insert(key.id.clone(), key);
    }

    let mut key_ids = std::collections::BTreeSet::new();
    key_ids.extend(from_by_id.keys().cloned());
    key_ids.extend(to_by_id.keys().cloned());

    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut changed = Vec::new();
    let mut unchanged = 0usize;

    for id in key_ids {
        match (from_by_id.get(&id), to_by_id.get(&id)) {
            (Some(before), Some(after)) => {
                if virtual_key_equal(before, after) {
                    unchanged = unchanged.saturating_add(1);
                } else {
                    changed.push(ConfigVersionDiffEntry {
                        id,
                        before: before.clone(),
                        after: after.clone(),
                    });
                }
            }
            (Some(before), None) => removed.push(before.clone()),
            (None, Some(after)) => added.push(after.clone()),
            (None, None) => {}
        }
    }

    if !query.include_tokens {
        redact_virtual_key_tokens(&mut added);
        redact_virtual_key_tokens(&mut removed);
        redact_diff_change_tokens(&mut changed);
    }

    let router_changed = !router_config_equal(&from.router, &to.router);
    let router_before = router_changed.then_some(from.router.clone());
    let router_after = router_changed.then_some(to.router.clone());

    let summary = ConfigVersionDiffSummary {
        from_virtual_key_count: from.info.virtual_key_count,
        to_virtual_key_count: to.info.virtual_key_count,
        added: added.len(),
        removed: removed.len(),
        changed: changed.len(),
        unchanged,
        router_changed,
    };

    Ok(Json(ConfigVersionDiffResponse {
        from_version: from.info,
        to_version: to.info,
        summary,
        added,
        removed,
        changed,
        router_before,
        router_after,
    }))
}

async fn list_config_versions(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Query(query): Query<ListConfigVersionsQuery>,
) -> Result<Json<Vec<ConfigVersionInfo>>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;
    if admin.tenant_id.is_some() {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "tenant-scoped admin tokens cannot access global config versions",
        ));
    }

    let mut versions = { state.config_versions.lock().await.list_infos_desc() };

    if query.offset > 0 {
        if query.offset >= versions.len() {
            versions.clear();
        } else {
            versions.drain(0..query.offset);
        }
    }

    if let Some(limit) = query
        .limit
        .map(|value| value.min(MAX_CONFIG_VERSIONS_LIMIT))
    {
        if versions.len() > limit {
            versions.truncate(limit);
        }
    }

    Ok(Json(versions))
}

async fn rollback_config_version(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Json(payload): Json<ConfigRollbackRequest>,
) -> Result<Json<ConfigRollbackResponse>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_write(&state, &headers)?;
    if admin.tenant_id.is_some() {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "tenant-scoped admin tokens cannot rollback global config",
        ));
    }

    let version_id = payload.version_id.trim();
    if version_id.is_empty() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "version_id cannot be empty",
        ));
    }

    let (current, target) = {
        let history = state.config_versions.lock().await;
        (history.current_info(), history.find_snapshot(version_id))
    };
    let Some(current) = current else {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "config version history is empty",
        ));
    };
    let Some(target) = target else {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("config version not found: {version_id}"),
        ));
    };

    if current.version_id == target.info.version_id {
        return Ok(Json(ConfigRollbackResponse {
            dry_run: payload.dry_run,
            noop: true,
            rolled_back_to_version_id: target.info.version_id.clone(),
            target_version: target.info,
            current_version: current,
        }));
    }

    if payload.dry_run {
        return Ok(Json(ConfigRollbackResponse {
            dry_run: true,
            noop: true,
            rolled_back_to_version_id: version_id.to_string(),
            target_version: target.info,
            current_version: current,
        }));
    }

    let restored_keys = target.virtual_keys.clone();
    let restored_router = target.router.clone();
    state.gateway.mutate_control_plane(|gateway| {
        gateway.replace_virtual_keys(restored_keys.clone());
        gateway.replace_router_config(restored_router);
    });
    state.sync_control_plane_from_gateway();

    let reason = format!("admin.config.rollback:{version_id}");
    let current_version = persist_virtual_keys(&state, &restored_keys, reason.as_str()).await?;

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    append_admin_audit_log(
        &state,
        "admin.config.rollback",
        serde_json::json!({
            "target_version_id": version_id,
            "result_version_id": &current_version.version_id,
            "virtual_key_count": current_version.virtual_key_count,
            "router_rule_count": current_version.router_rule_count,
            "router_sha256": &current_version.router_sha256,
        }),
    )
    .await;

    Ok(Json(ConfigRollbackResponse {
        dry_run: false,
        noop: false,
        rolled_back_to_version_id: version_id.to_string(),
        target_version: target.info,
        current_version,
    }))
}
// end inline: admin/config_versions.rs
// inlined from admin/keys.rs
async fn list_keys(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Query(query): Query<ListKeysQuery>,
) -> Result<Json<Vec<VirtualKeyConfig>>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;
    let mut keys = state.list_virtual_keys_snapshot();

    if let Some(enabled) = query.enabled {
        keys.retain(|key| key.enabled == enabled);
    }

    if let Some(prefix) = query
        .id_prefix
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        keys.retain(|key| key.id.starts_with(prefix));
    }

    let tenant_filter = if let Some(admin_tenant) = admin.tenant_id.as_deref() {
        if let Some(query_tenant) = query
            .tenant_id
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            if query_tenant != admin_tenant {
                return Err(error_response(
                    StatusCode::FORBIDDEN,
                    "forbidden",
                    "cross-tenant admin access is not allowed",
                ));
            }
        }
        Some(admin_tenant)
    } else {
        query
            .tenant_id
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
    };

    if let Some(tenant_id) = tenant_filter {
        keys.retain(|key| key.tenant_id.as_deref() == Some(tenant_id));
    }

    if let Some(project_id) = query
        .project_id
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        keys.retain(|key| key.project_id.as_deref() == Some(project_id));
    }

    if let Some(user_id) = query
        .user_id
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        keys.retain(|key| key.user_id.as_deref() == Some(user_id));
    }

    keys.sort_by(|a, b| a.id.cmp(&b.id));

    if query.offset > 0 {
        if query.offset >= keys.len() {
            keys.clear();
        } else {
            keys.drain(0..query.offset);
        }
    }

    if let Some(limit) = query.limit.map(|limit| limit.min(MAX_ADMIN_LIST_LIMIT)) {
        if keys.len() > limit {
            keys.truncate(limit);
        }
    }

    if !query.include_tokens {
        for key in &mut keys {
            key.token = "redacted".to_string();
        }
    }
    Ok(Json(keys))
}

async fn upsert_key(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Json(key): Json<VirtualKeyConfig>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_write(&state, &headers)?;
    let mut key = key;
    if let Some(admin_tenant) = admin.tenant_id.as_deref() {
        if let Some(tenant_id) = key.tenant_id.as_deref() {
            if tenant_id != admin_tenant {
                return Err(error_response(
                    StatusCode::FORBIDDEN,
                    "forbidden",
                    "cannot upsert keys for a different tenant",
                ));
            }
        } else {
            key.tenant_id = Some(admin_tenant.to_string());
        }
    }
    if let Err(err) = key.guardrails.validate() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            format!("invalid guardrails config: {err}"),
        ));
    }
    let (inserted, persisted_keys) = state.gateway.mutate_control_plane(|gateway| {
        let inserted = gateway.upsert_virtual_key(key.clone());
        let persisted_keys = gateway.list_virtual_keys();
        (inserted, persisted_keys)
    });
    state.sync_control_plane_from_gateway();
    let _ = persist_virtual_keys(&state, &persisted_keys, "admin.key.upsert").await?;

    #[cfg(feature = "sdk")]
    emit_devtools_log(
        &state,
        "admin.key.upsert",
        serde_json::json!({
            "key_id": &key.id,
            "enabled": key.enabled,
            "inserted": inserted,
        }),
    );

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    append_admin_audit_log(
        &state,
        "admin.key.upsert",
        serde_json::json!({
            "key_id": &key.id,
            "enabled": key.enabled,
            "inserted": inserted,
            "tenant_id": key.tenant_id.as_deref(),
            "project_id": key.project_id.as_deref(),
            "user_id": key.user_id.as_deref(),
        }),
    )
    .await;

    let status = if inserted {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(key)))
}

async fn upsert_key_with_id(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(mut key): Json<VirtualKeyConfig>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_write(&state, &headers)?;
    key.id = id;
    if let Some(admin_tenant) = admin.tenant_id.as_deref() {
        if let Some(tenant_id) = key.tenant_id.as_deref() {
            if tenant_id != admin_tenant {
                return Err(error_response(
                    StatusCode::FORBIDDEN,
                    "forbidden",
                    "cannot upsert keys for a different tenant",
                ));
            }
        } else {
            key.tenant_id = Some(admin_tenant.to_string());
        }
    }
    if let Err(err) = key.guardrails.validate() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            format!("invalid guardrails config: {err}"),
        ));
    }
    let (inserted, persisted_keys) = state.gateway.mutate_control_plane(|gateway| {
        let inserted = gateway.upsert_virtual_key(key.clone());
        let persisted_keys = gateway.list_virtual_keys();
        (inserted, persisted_keys)
    });
    state.sync_control_plane_from_gateway();
    let _ = persist_virtual_keys(&state, &persisted_keys, "admin.key.upsert").await?;

    #[cfg(feature = "sdk")]
    emit_devtools_log(
        &state,
        "admin.key.upsert",
        serde_json::json!({
            "key_id": &key.id,
            "enabled": key.enabled,
            "inserted": inserted,
        }),
    );

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    append_admin_audit_log(
        &state,
        "admin.key.upsert",
        serde_json::json!({
            "key_id": &key.id,
            "enabled": key.enabled,
            "inserted": inserted,
            "tenant_id": key.tenant_id.as_deref(),
            "project_id": key.project_id.as_deref(),
            "user_id": key.user_id.as_deref(),
        }),
    )
    .await;

    let status = if inserted {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(key)))
}

async fn delete_key(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_write(&state, &headers)?;
    let (removed, persisted_keys) = state.gateway.mutate_control_plane(|gateway| {
        if let Some(admin_tenant) = admin.tenant_id.as_deref() {
            let existing = gateway.list_virtual_keys();
            let Some(existing_key) = existing.iter().find(|key| key.id == id) else {
                return Err(error_response(
                    StatusCode::NOT_FOUND,
                    "not_found",
                    "virtual key not found",
                ));
            };
            if existing_key.tenant_id.as_deref() != Some(admin_tenant) {
                return Err(error_response(
                    StatusCode::FORBIDDEN,
                    "forbidden",
                    "cannot delete keys for a different tenant",
                ));
            }
        }
        let removed = gateway.remove_virtual_key(&id).is_some();
        let persisted_keys = gateway.list_virtual_keys();
        Ok((removed, persisted_keys))
    })?;
    state.sync_control_plane_from_gateway();
    if removed {
        let _ = persist_virtual_keys(&state, &persisted_keys, "admin.key.delete").await?;

        #[cfg(feature = "sdk")]
        emit_devtools_log(
            &state,
            "admin.key.delete",
            serde_json::json!({
                "key_id": &id,
            }),
        );

        #[cfg(any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        ))]
        append_admin_audit_log(
            &state,
            "admin.key.delete",
            serde_json::json!({
                "key_id": &id,
                "tenant_id": admin.tenant_id.as_deref(),
            }),
        )
        .await;

        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "virtual key not found",
        ))
    }
}

const MAX_ADMIN_LIST_LIMIT: usize = 10_000;

#[derive(Debug, Deserialize)]
struct ListKeysQuery {
    #[serde(default)]
    include_tokens: bool,
    #[serde(default)]
    tenant_id: Option<String>,
    #[serde(default)]
    project_id: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    id_prefix: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: usize,
}
// end inline: admin/keys.rs
// inlined from admin/maintenance.rs
#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
#[derive(Debug, Deserialize)]
struct ReapReservationsRequest {
    #[serde(default = "default_reap_reservations_older_than_secs")]
    older_than_secs: u64,
    #[serde(default = "default_reap_reservations_limit")]
    limit: usize,
    #[serde(default)]
    dry_run: bool,
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
fn default_reap_reservations_older_than_secs() -> u64 {
    24 * 60 * 60
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
fn default_reap_reservations_limit() -> usize {
    1000
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
#[derive(Debug, Serialize)]
struct ReapReservationsCounts {
    scanned: u64,
    reaped: u64,
    released: u64,
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
#[derive(Debug, Serialize)]
struct ReapReservationsResponse {
    store: &'static str,
    dry_run: bool,
    cutoff_ts_ms: u64,
    budget: ReapReservationsCounts,
    cost: ReapReservationsCounts,
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
fn now_millis_u64() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
async fn reap_reservations(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Json(payload): Json<ReapReservationsRequest>,
) -> Result<Json<ReapReservationsResponse>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_write(&state, &headers)?;
    if admin.tenant_id.is_some() {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "tenant-scoped admin tokens cannot reap reservations",
        ));
    }

    let now_ts_ms = now_millis_u64();
    let cutoff_ts_ms = now_ts_ms.saturating_sub(payload.older_than_secs.saturating_mul(1000));
    let limit = payload.limit.clamp(1, 100_000);
    let dry_run = payload.dry_run;

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.stores.sqlite.as_ref() {
        let (budget_scanned, budget_reaped, budget_released) = store
            .reap_stale_budget_reservations(cutoff_ts_ms, limit, dry_run)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;
        let (cost_scanned, cost_reaped, cost_released) = store
            .reap_stale_cost_reservations(cutoff_ts_ms, limit, dry_run)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;

        return Ok(Json(ReapReservationsResponse {
            store: "sqlite",
            dry_run,
            cutoff_ts_ms,
            budget: ReapReservationsCounts {
                scanned: budget_scanned,
                reaped: budget_reaped,
                released: budget_released,
            },
            cost: ReapReservationsCounts {
                scanned: cost_scanned,
                reaped: cost_reaped,
                released: cost_released,
            },
        }));
    }

    #[cfg(feature = "gateway-store-postgres")]
    if let Some(store) = state.stores.postgres.as_ref() {
        let (budget_scanned, budget_reaped, budget_released) = store
            .reap_stale_budget_reservations(cutoff_ts_ms, limit, dry_run)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;
        let (cost_scanned, cost_reaped, cost_released) = store
            .reap_stale_cost_reservations(cutoff_ts_ms, limit, dry_run)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;

        return Ok(Json(ReapReservationsResponse {
            store: "postgres",
            dry_run,
            cutoff_ts_ms,
            budget: ReapReservationsCounts {
                scanned: budget_scanned,
                reaped: budget_reaped,
                released: budget_released,
            },
            cost: ReapReservationsCounts {
                scanned: cost_scanned,
                reaped: cost_reaped,
                released: cost_released,
            },
        }));
    }

    #[cfg(feature = "gateway-store-mysql")]
    if let Some(store) = state.stores.mysql.as_ref() {
        let (budget_scanned, budget_reaped, budget_released) = store
            .reap_stale_budget_reservations(cutoff_ts_ms, limit, dry_run)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;
        let (cost_scanned, cost_reaped, cost_released) = store
            .reap_stale_cost_reservations(cutoff_ts_ms, limit, dry_run)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;

        return Ok(Json(ReapReservationsResponse {
            store: "mysql",
            dry_run,
            cutoff_ts_ms,
            budget: ReapReservationsCounts {
                scanned: budget_scanned,
                reaped: budget_reaped,
                released: budget_released,
            },
            cost: ReapReservationsCounts {
                scanned: cost_scanned,
                reaped: cost_reaped,
                released: cost_released,
            },
        }));
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.stores.redis.as_ref() {
        let (budget_scanned, budget_reaped, budget_released) = store
            .reap_stale_budget_reservations(cutoff_ts_ms, limit, dry_run)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;
        let (cost_scanned, cost_reaped, cost_released) = store
            .reap_stale_cost_reservations(cutoff_ts_ms, limit, dry_run)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;

        return Ok(Json(ReapReservationsResponse {
            store: "redis",
            dry_run,
            cutoff_ts_ms,
            budget: ReapReservationsCounts {
                scanned: budget_scanned,
                reaped: budget_reaped,
                released: budget_released,
            },
            cost: ReapReservationsCounts {
                scanned: cost_scanned,
                reaped: cost_reaped,
                released: cost_released,
            },
        }));
    }

    Err(error_response(
        StatusCode::BAD_REQUEST,
        "not_configured",
        "store not configured",
    ))
}
// end inline: admin/maintenance.rs
// inlined from admin/ledger_grouping_tests.rs
#[cfg(all(
    test,
    any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    )
))]
mod ledger_grouping_tests {
    use super::*;

    #[test]
    fn budget_ledgers_group_by_project_and_user() {
        let mut key_1 = VirtualKeyConfig::new("key-1", "vk-1");
        key_1.tenant_id = Some("tenant-a".to_string());
        key_1.project_id = Some("proj-a".to_string());
        key_1.user_id = Some("user-a".to_string());

        let mut key_2 = VirtualKeyConfig::new("key-2", "vk-2");
        key_2.tenant_id = Some("tenant-a".to_string());
        key_2.project_id = Some("proj-a".to_string());
        key_2.user_id = Some("user-b".to_string());

        let ledgers = vec![
            BudgetLedgerRecord {
                key_id: "key-1".to_string(),
                spent_tokens: 10,
                reserved_tokens: 3,
                updated_at_ms: 100,
            },
            BudgetLedgerRecord {
                key_id: "key-2".to_string(),
                spent_tokens: 7,
                reserved_tokens: 0,
                updated_at_ms: 200,
            },
            BudgetLedgerRecord {
                key_id: "key-unknown".to_string(),
                spent_tokens: 1,
                reserved_tokens: 2,
                updated_at_ms: 50,
            },
        ];

        let keys = vec![key_1, key_2];

        let projects = group_budget_ledgers_by_project(&ledgers, &keys);
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[0].project_id, None);
        assert_eq!(projects[0].spent_tokens, 1);
        assert_eq!(projects[0].reserved_tokens, 2);
        assert_eq!(projects[0].key_count, 1);
        assert_eq!(projects[0].updated_at_ms, 50);
        assert_eq!(projects[1].project_id.as_deref(), Some("proj-a"));
        assert_eq!(projects[1].spent_tokens, 17);
        assert_eq!(projects[1].reserved_tokens, 3);
        assert_eq!(projects[1].key_count, 2);
        assert_eq!(projects[1].updated_at_ms, 200);

        let users = group_budget_ledgers_by_user(&ledgers, &keys);
        assert_eq!(users.len(), 3);
        assert_eq!(users[0].user_id, None);
        assert_eq!(users[0].spent_tokens, 1);
        assert_eq!(users[0].reserved_tokens, 2);
        assert_eq!(users[0].key_count, 1);
        assert_eq!(users[0].updated_at_ms, 50);
        assert_eq!(users[1].user_id.as_deref(), Some("user-a"));
        assert_eq!(users[1].spent_tokens, 10);
        assert_eq!(users[1].reserved_tokens, 3);
        assert_eq!(users[1].key_count, 1);
        assert_eq!(users[1].updated_at_ms, 100);
        assert_eq!(users[2].user_id.as_deref(), Some("user-b"));
        assert_eq!(users[2].spent_tokens, 7);
        assert_eq!(users[2].reserved_tokens, 0);
        assert_eq!(users[2].key_count, 1);
        assert_eq!(users[2].updated_at_ms, 200);

        let tenants = group_budget_ledgers_by_tenant(&ledgers, &keys);
        assert_eq!(tenants.len(), 2);
        assert_eq!(tenants[0].tenant_id, None);
        assert_eq!(tenants[0].spent_tokens, 1);
        assert_eq!(tenants[0].reserved_tokens, 2);
        assert_eq!(tenants[0].key_count, 1);
        assert_eq!(tenants[0].updated_at_ms, 50);
        assert_eq!(tenants[1].tenant_id.as_deref(), Some("tenant-a"));
        assert_eq!(tenants[1].spent_tokens, 17);
        assert_eq!(tenants[1].reserved_tokens, 3);
        assert_eq!(tenants[1].key_count, 2);
        assert_eq!(tenants[1].updated_at_ms, 200);
    }

    #[cfg(feature = "gateway-costing")]
    #[test]
    fn cost_ledgers_group_by_project_and_user() {
        let mut key_1 = VirtualKeyConfig::new("key-1", "vk-1");
        key_1.tenant_id = Some("tenant-a".to_string());
        key_1.project_id = Some("proj-a".to_string());
        key_1.user_id = Some("user-a".to_string());

        let ledgers = vec![
            CostLedgerRecord {
                key_id: "key-1".to_string(),
                spent_usd_micros: 10,
                reserved_usd_micros: 3,
                updated_at_ms: 100,
            },
            CostLedgerRecord {
                key_id: "key-unknown".to_string(),
                spent_usd_micros: 1,
                reserved_usd_micros: 2,
                updated_at_ms: 50,
            },
        ];

        let keys = vec![key_1];

        let projects = group_cost_ledgers_by_project(&ledgers, &keys);
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[0].project_id, None);
        assert_eq!(projects[0].spent_usd_micros, 1);
        assert_eq!(projects[0].reserved_usd_micros, 2);
        assert_eq!(projects[0].key_count, 1);
        assert_eq!(projects[0].updated_at_ms, 50);
        assert_eq!(projects[1].project_id.as_deref(), Some("proj-a"));
        assert_eq!(projects[1].spent_usd_micros, 10);
        assert_eq!(projects[1].reserved_usd_micros, 3);
        assert_eq!(projects[1].key_count, 1);
        assert_eq!(projects[1].updated_at_ms, 100);

        let users = group_cost_ledgers_by_user(&ledgers, &keys);
        assert_eq!(users.len(), 2);
        assert_eq!(users[0].user_id, None);
        assert_eq!(users[0].spent_usd_micros, 1);
        assert_eq!(users[0].reserved_usd_micros, 2);
        assert_eq!(users[0].key_count, 1);
        assert_eq!(users[0].updated_at_ms, 50);
        assert_eq!(users[1].user_id.as_deref(), Some("user-a"));
        assert_eq!(users[1].spent_usd_micros, 10);
        assert_eq!(users[1].reserved_usd_micros, 3);
        assert_eq!(users[1].key_count, 1);
        assert_eq!(users[1].updated_at_ms, 100);

        let tenants = group_cost_ledgers_by_tenant(&ledgers, &keys);
        assert_eq!(tenants.len(), 2);
        assert_eq!(tenants[0].tenant_id, None);
        assert_eq!(tenants[0].spent_usd_micros, 1);
        assert_eq!(tenants[0].reserved_usd_micros, 2);
        assert_eq!(tenants[0].key_count, 1);
        assert_eq!(tenants[0].updated_at_ms, 50);
        assert_eq!(tenants[1].tenant_id.as_deref(), Some("tenant-a"));
        assert_eq!(tenants[1].spent_usd_micros, 10);
        assert_eq!(tenants[1].reserved_usd_micros, 3);
        assert_eq!(tenants[1].key_count, 1);
        assert_eq!(tenants[1].updated_at_ms, 100);
    }
}
// end inline: admin/ledger_grouping_tests.rs
// inlined from admin/auth.rs
#[derive(Clone, Copy, Debug)]
enum AdminPermission {
    Read,
    Write,
}

#[derive(Clone, Debug)]
struct AdminContext {
    tenant_id: Option<String>,
}

fn ensure_admin_read(
    state: &GatewayHttpState,
    headers: &HeaderMap,
) -> Result<AdminContext, (StatusCode, Json<ErrorResponse>)> {
    ensure_admin(state, headers, AdminPermission::Read)
}

fn ensure_admin_write(
    state: &GatewayHttpState,
    headers: &HeaderMap,
) -> Result<AdminContext, (StatusCode, Json<ErrorResponse>)> {
    ensure_admin(state, headers, AdminPermission::Write)
}

fn ensure_admin(
    state: &GatewayHttpState,
    headers: &HeaderMap,
    permission: AdminPermission,
) -> Result<AdminContext, (StatusCode, Json<ErrorResponse>)> {
    let write_token = state.admin.admin_token.as_deref();
    let read_token = state.admin.admin_read_token.as_deref();
    let has_tenant_tokens = !state.admin.admin_tenant_tokens.is_empty();

    if write_token.is_none() && read_token.is_none() && !has_tenant_tokens {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            "not_configured",
            "admin auth not configured",
        ));
    }

    let provided = extract_bearer(headers)
        .or_else(|| extract_header(headers, "x-admin-token"))
        .unwrap_or_default();

    if write_token.is_some_and(|expected| provided == expected) {
        return Ok(AdminContext { tenant_id: None });
    }

    if let AdminPermission::Read = permission {
        if read_token.is_some_and(|expected| provided == expected) {
            return Ok(AdminContext { tenant_id: None });
        }
    }

    if has_tenant_tokens {
        for binding in &state.admin.admin_tenant_tokens {
            if provided != binding.token {
                continue;
            }
            if let AdminPermission::Write = permission {
                if binding.read_only {
                    break;
                }
            }
            return Ok(AdminContext {
                tenant_id: Some(binding.tenant_id.clone()),
            });
        }
    }

    Err(error_response(
        StatusCode::UNAUTHORIZED,
        "unauthorized",
        "invalid admin token",
    ))
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
async fn append_admin_audit_log(state: &GatewayHttpState, kind: &str, payload: serde_json::Value) {
    let Some(payload) = state.prepare_observability_event(
        crate::gateway::observability::GatewayObservabilitySink::Audit,
        payload,
    ) else {
        return;
    };

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.stores.sqlite.as_ref() {
        let _ = store.append_audit_log(kind, payload.clone()).await;
    }

    #[cfg(feature = "gateway-store-postgres")]
    if let Some(store) = state.stores.postgres.as_ref() {
        let _ = store.append_audit_log(kind, payload.clone()).await;
    }

    #[cfg(feature = "gateway-store-mysql")]
    if let Some(store) = state.stores.mysql.as_ref() {
        let _ = store.append_audit_log(kind, payload.clone()).await;
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.stores.redis.as_ref() {
        let _ = store.append_audit_log(kind, payload).await;
    }
}

fn extract_header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn extract_query_param(uri: &axum::http::Uri, name: &str) -> Option<String> {
    let query = uri.query()?;
    extract_query_param_str(query, name)
}

fn extract_query_param_str(query: &str, name: &str) -> Option<String> {
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if key != name {
            continue;
        }
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        return percent_decode_www_form(value);
    }
    None
}

fn percent_decode_www_form(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' => {
                if i + 2 >= bytes.len() {
                    return None;
                }
                let hi = from_hex(bytes[i + 1])?;
                let lo = from_hex(bytes[i + 2])?;
                out.push((hi << 4) | lo);
                i += 3;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn from_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn extract_bearer(headers: &HeaderMap) -> Option<String> {
    let auth = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())?
        .trim()
        .to_string();
    let rest = auth
        .strip_prefix("Bearer ")
        .or_else(|| auth.strip_prefix("bearer "))?;
    let token = rest.trim();
    (!token.is_empty()).then(|| token.to_string())
}

fn extract_litellm_api_key(headers: &HeaderMap) -> Option<String> {
    let raw = extract_header(headers, "x-litellm-api-key")?;
    let token = raw
        .strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
        .unwrap_or(raw.as_str())
        .trim();
    (!token.is_empty()).then(|| token.to_string())
}

fn extract_virtual_key(headers: &HeaderMap) -> Option<String> {
    extract_litellm_api_key(headers)
        .or_else(|| extract_bearer(headers))
        .or_else(|| extract_header(headers, "x-ditto-virtual-key"))
        .or_else(|| extract_header(headers, "x-api-key"))
}

fn map_gateway_error(err: GatewayError) -> (StatusCode, Json<ErrorResponse>) {
    match err {
        GatewayError::Unauthorized => error_response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "unauthorized virtual key",
        ),
        GatewayError::RateLimited { limit } => error_response(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            format!("rate limit exceeded: {limit}"),
        ),
        GatewayError::GuardrailRejected { reason } => error_response(
            StatusCode::FORBIDDEN,
            "guardrail_rejected",
            format!("guardrail rejected: {reason}"),
        ),
        GatewayError::BudgetExceeded { limit, attempted } => error_response(
            StatusCode::PAYMENT_REQUIRED,
            "budget_exceeded",
            format!("budget exceeded: limit={limit} attempted={attempted}"),
        ),
        GatewayError::CostBudgetExceeded {
            limit_usd_micros,
            attempted_usd_micros,
        } => error_response(
            StatusCode::PAYMENT_REQUIRED,
            "cost_budget_exceeded",
            format!(
                "cost budget exceeded: limit_usd_micros={limit_usd_micros} attempted_usd_micros={attempted_usd_micros}"
            ),
        ),
        GatewayError::BackendNotFound { name } => error_response(
            StatusCode::BAD_GATEWAY,
            "backend_not_found",
            format!("backend not found: {name}"),
        ),
        GatewayError::Backend { message } => {
            error_response(StatusCode::BAD_GATEWAY, "backend_error", message)
        }
        GatewayError::BackendTimeout { message } => {
            error_response(StatusCode::GATEWAY_TIMEOUT, "backend_timeout", message)
        }
        GatewayError::InvalidRequest { reason } => {
            error_response(StatusCode::BAD_REQUEST, "invalid_request", reason)
        }
    }
}

fn error_response(
    status: StatusCode,
    code: &'static str,
    message: impl Into<String>,
) -> (StatusCode, Json<ErrorResponse>) {
    (
        status,
        Json(ErrorResponse {
            error: ErrorDetail {
                code,
                message: message.into(),
            },
        }),
    )
}

fn persist_state_file(
    path: &StdPath,
    keys: &[VirtualKeyConfig],
    router: &RouterConfig,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    GatewayStateFile {
        virtual_keys: keys.to_vec(),
        router: Some(router.clone()),
    }
    .save(path)
    .map_err(|err| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            err.to_string(),
        )
    })
}

async fn persist_virtual_keys(
    state: &GatewayHttpState,
    keys: &[VirtualKeyConfig],
    reason: &str,
) -> Result<ConfigVersionInfo, (StatusCode, Json<ErrorResponse>)> {
    let router = state.router_config_snapshot();

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.stores.sqlite.as_ref() {
        store.replace_virtual_keys(keys).await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        store.replace_router_config(&router).await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
    }

    #[cfg(feature = "gateway-store-postgres")]
    if let Some(store) = state.stores.postgres.as_ref() {
        store.replace_virtual_keys(keys).await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        store.replace_router_config(&router).await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
    }

    #[cfg(feature = "gateway-store-mysql")]
    if let Some(store) = state.stores.mysql.as_ref() {
        store.replace_virtual_keys(keys).await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        store.replace_router_config(&router).await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.stores.redis.as_ref() {
        store.replace_virtual_keys(keys).await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        store.replace_router_config(&router).await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
    }

    if let Some(path) = state.admin.state_file.as_ref() {
        persist_state_file(path.as_path(), keys, &router)?;
    }

    let version = state
        .config_versions
        .lock()
        .await
        .push_snapshot(keys.to_vec(), router, reason);
    Ok(version)
}

#[cfg(test)]
mod admin_auth_tests {
    use super::*;

    fn test_state() -> GatewayHttpState {
        let config = crate::gateway::GatewayConfig {
            backends: Vec::new(),
            virtual_keys: Vec::new(),
            router: crate::gateway::RouterConfig {
                default_backends: Vec::new(),
                rules: Vec::new(),
            },
            a2a_agents: Vec::new(),
            mcp_servers: Vec::new(),
            observability: Default::default(),
        };
        GatewayHttpState::new(crate::gateway::Gateway::new(config))
    }

    #[test]
    fn ensure_admin_read_rejects_when_not_configured() {
        let state = test_state();
        let headers = HeaderMap::new();
        let (status, Json(body)) = ensure_admin_read(&state, &headers).unwrap_err();
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body.error.code, "not_configured");
    }

    #[test]
    fn ensure_admin_write_rejects_when_not_configured() {
        let state = test_state();
        let headers = HeaderMap::new();
        let (status, Json(body)) = ensure_admin_write(&state, &headers).unwrap_err();
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body.error.code, "not_configured");
    }
}
// end inline: admin/auth.rs
// end inline: ../../http/admin.rs
// inlined from ../../http/translation_backend.rs
// This file is intentionally split to keep each staged Rust file under the pre-commit size limit.
// inlined from translation_backend/attempt.rs
#[cfg(feature = "gateway-translation")]
async fn attempt_translation_backend(
    params: ProxyAttemptParams<'_>,
    backend_name: &str,
    translation_backend: super::TranslationBackend,
    attempted_backends: &[String],
) -> Result<BackendAttemptOutcome, (StatusCode, Json<OpenAiErrorResponse>)> {
    let state = params.state;
    let parts = params.parts;
    let body = params.body;
    let parsed_json = params.parsed_json;
    let model = params.model;
    #[cfg(feature = "gateway-costing")]
    let service_tier = params.service_tier;
    #[cfg(not(feature = "gateway-costing"))]
    let _service_tier = params.service_tier;
    let request_id = params.request_id;
    let path_and_query = params.path_and_query;
    let _now_epoch_seconds = params.now_epoch_seconds;
    let charge_tokens = params.charge_tokens;
    let _stream_requested = params.stream_requested;
    let use_persistent_budget = params.use_persistent_budget;
    #[cfg(not(feature = "gateway-costing"))]
    let _ = use_persistent_budget;
    let virtual_key_id = params.virtual_key_id;
    let budget = params.budget;
    let project_budget_scope = params.project_budget_scope;
    let user_budget_scope = params.user_budget_scope;
    let charge_cost_usd_micros = params.charge_cost_usd_micros;

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    let token_budget_reservation_ids = params.token_budget_reservation_ids;

    let _cost_budget_reserved = params.cost_budget_reserved;
    #[cfg(all(
        feature = "gateway-costing",
        any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        ),
    ))]
    let cost_budget_reservation_ids = params.cost_budget_reservation_ids;

    #[cfg(feature = "gateway-metrics-prometheus")]
    let metrics_path = params.metrics_path;
    #[cfg(feature = "gateway-metrics-prometheus")]
    let metrics_timer_start = params.metrics_timer_start;

    let batch_cancel_id = translation::batches_cancel_id(path_and_query);
    let batch_retrieve_id = translation::batches_retrieve_id(path_and_query);
    let batches_root = translation::is_batches_path(path_and_query);
    let models_root = translation::is_models_path(path_and_query);
    let models_retrieve_id = translation::models_retrieve_id(path_and_query);
    let files_root = translation::is_files_path(path_and_query);
    let files_retrieve_id = translation::files_retrieve_id(path_and_query);
    let files_content_id = translation::files_content_id(path_and_query);
    let responses_retrieve_id = translation::responses_retrieve_id(path_and_query);
    let responses_input_items_id = translation::responses_input_items_id(path_and_query);
    let responses_input_tokens = translation::is_responses_input_tokens_path(path_and_query);
    let videos_root = translation::is_videos_path(path_and_query);
    let videos_retrieve_id = translation::videos_retrieve_id(path_and_query);
    let videos_content_id = translation::videos_content_id(path_and_query);
    let videos_remix_id = translation::videos_remix_id(path_and_query);

    let Some(endpoint_descriptor) =
        translation::translation_endpoint_descriptor(&parts.method, path_and_query)
    else {
        return Ok(BackendAttemptOutcome::Continue(Some(openai_error(
            StatusCode::NOT_IMPLEMENTED,
            "invalid_request_error",
            Some("unsupported_endpoint"),
            format!(
                "translation backend does not support {} {}",
                parts.method, path_and_query
            ),
        ))));
    };

    let mapped_request_model = model
        .as_deref()
        .map(|requested_model| translation_backend.map_model(requested_model))
        .filter(|requested_model| !requested_model.trim().is_empty());
    if !translation_backend.supports_endpoint(&endpoint_descriptor, mapped_request_model.as_deref())
    {
        return Ok(BackendAttemptOutcome::Continue(Some(openai_error(
            StatusCode::NOT_IMPLEMENTED,
            "invalid_request_error",
            Some("unsupported_endpoint"),
            format!(
                "translation backend does not support {} {}",
                parts.method, path_and_query
            ),
        ))));
    }

    let mut proxy_permits = match try_acquire_proxy_permits(state, backend_name)? {
        ProxyPermitOutcome::Acquired(permits) => permits,
        ProxyPermitOutcome::BackendRateLimited(err) => {
            return Ok(BackendAttemptOutcome::Continue(Some(err)));
        }
    };

    state.record_backend_call();

    #[cfg(feature = "gateway-metrics-prometheus")]
    let backend_timer_start = std::time::Instant::now();

    #[cfg(feature = "gateway-metrics-prometheus")]
    if let Some(metrics) = state.proxy.metrics.as_ref() {
        let mut metrics = metrics.lock().await;
        metrics.record_proxy_backend_attempt(backend_name);
        metrics.record_proxy_backend_in_flight_inc(backend_name);
    }

    let default_spend = ProxySpend {
        tokens: u64::from(charge_tokens),
        cost_usd_micros: charge_cost_usd_micros,
    };

    let result: Result<
        (axum::response::Response, ProxySpend),
        (StatusCode, Json<OpenAiErrorResponse>),
    > = 'translation_backend_attempt: {
        #[allow(clippy::collapsible_else_if)]
        if models_root && parts.method == axum::http::Method::GET {
            let models = translation::collect_models_from_translation_backends(
                state.backends.translation_backends.as_ref(),
            );
            let value = translation::models_list_to_openai(&models, _now_epoch_seconds);
            let bytes = serde_json::to_vec(&value)
                .map(Bytes::from)
                .unwrap_or_else(|_| Bytes::from(value.to_string()));

            let mut headers = HeaderMap::new();
            headers.insert(
                axum::http::header::CONTENT_TYPE,
                axum::http::HeaderValue::from_static("application/json"),
            );
            headers.insert(
                "x-ditto-translation",
                axum::http::HeaderValue::from_static("multi"),
            );
            apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

            let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
            let mut response = axum::response::Response::new(body);
            *response.status_mut() = StatusCode::OK;
            *response.headers_mut() = headers;
            Ok((response, default_spend))
        } else if let Some(model_id) = models_retrieve_id.as_deref()
            && parts.method == axum::http::Method::GET
        {
            let models = translation::collect_models_from_translation_backends(
                state.backends.translation_backends.as_ref(),
            );
            let Some(owned_by) = models.get(model_id) else {
                break 'translation_backend_attempt Err(openai_error(
                    StatusCode::NOT_FOUND,
                    "invalid_request_error",
                    Some("model_not_found"),
                    format!("model {model_id} not found"),
                ));
            };

            let value = translation::model_to_openai(model_id, owned_by, _now_epoch_seconds);
            let bytes = serde_json::to_vec(&value)
                .map(Bytes::from)
                .unwrap_or_else(|_| Bytes::from(value.to_string()));

            let mut headers = HeaderMap::new();
            headers.insert(
                axum::http::header::CONTENT_TYPE,
                axum::http::HeaderValue::from_static("application/json"),
            );
            headers.insert(
                "x-ditto-translation",
                axum::http::HeaderValue::from_str(owned_by)
                    .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
            );
            apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

            let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
            let mut response = axum::response::Response::new(body);
            *response.status_mut() = StatusCode::OK;
            *response.headers_mut() = headers;
            Ok((response, default_spend))
        } else if batches_root && parts.method == axum::http::Method::GET {
            let mut limit: Option<u32> = None;
            let mut after: Option<String> = None;
            let query = parts.uri.query().unwrap_or_default();
            for pair in query.split('&') {
                let Some((key, value)) = pair.split_once('=') else {
                    continue;
                };
                if key == "limit" {
                    limit = value.parse::<u32>().ok();
                } else if key == "after" {
                    let value = value.trim();
                    if !value.is_empty() {
                        after = Some(value.to_string());
                    }
                }
            }

            let listed = match translation_backend.list_batches(limit, after).await {
                Ok(listed) => listed,
                Err(err) => {
                    let (status, kind, code, message) =
                        translation::map_provider_error_to_openai(err);
                    break 'translation_backend_attempt Err(openai_error(
                        status, kind, code, message,
                    ));
                }
            };

            let value = translation::batch_list_response_to_openai(&listed);
            let bytes = serde_json::to_vec(&value)
                .map(Bytes::from)
                .unwrap_or_else(|_| Bytes::from(value.to_string()));

            let mut headers = HeaderMap::new();
            headers.insert(
                axum::http::header::CONTENT_TYPE,
                axum::http::HeaderValue::from_static("application/json"),
            );
            headers.insert(
                "x-ditto-translation",
                axum::http::HeaderValue::from_str(&translation_backend.provider)
                    .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
            );
            apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

            let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
            let mut response = axum::response::Response::new(body);
            *response.status_mut() = StatusCode::OK;
            *response.headers_mut() = headers;
            Ok((response, default_spend))
        } else if batches_root && parts.method == axum::http::Method::POST {
            let Some(parsed_json) = parsed_json.as_ref() else {
                break 'translation_backend_attempt Err(openai_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request_error",
                    Some("invalid_request"),
                    "request body must be application/json",
                ));
            };

            if _stream_requested {
                break 'translation_backend_attempt Err(openai_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request_error",
                    Some("invalid_request"),
                    "batches endpoint does not support stream=true",
                ));
            }

            let request = match translation::batches_create_request_to_request(parsed_json) {
                Ok(request) => request,
                Err(err) => {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        err,
                    ));
                }
            };

            let created = match translation_backend.create_batch(request).await {
                Ok(created) => created,
                Err(err) => {
                    let (status, kind, code, message) =
                        translation::map_provider_error_to_openai(err);
                    break 'translation_backend_attempt Err(openai_error(
                        status, kind, code, message,
                    ));
                }
            };

            let value = translation::batch_to_openai(&created.batch);
            let bytes = serde_json::to_vec(&value)
                .map(Bytes::from)
                .unwrap_or_else(|_| Bytes::from(value.to_string()));

            let mut headers = HeaderMap::new();
            headers.insert(
                axum::http::header::CONTENT_TYPE,
                axum::http::HeaderValue::from_static("application/json"),
            );
            headers.insert(
                "x-ditto-translation",
                axum::http::HeaderValue::from_str(&translation_backend.provider)
                    .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
            );
            apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

            let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
            let mut response = axum::response::Response::new(body);
            *response.status_mut() = StatusCode::OK;
            *response.headers_mut() = headers;
            Ok((response, default_spend))
        } else if let Some(batch_id) = batch_retrieve_id.as_deref() {
            let retrieved = match translation_backend.retrieve_batch(batch_id).await {
                Ok(retrieved) => retrieved,
                Err(err) => {
                    let (status, kind, code, message) =
                        translation::map_provider_error_to_openai(err);
                    break 'translation_backend_attempt Err(openai_error(
                        status, kind, code, message,
                    ));
                }
            };

            let value = translation::batch_to_openai(&retrieved.batch);
            let bytes = serde_json::to_vec(&value)
                .map(Bytes::from)
                .unwrap_or_else(|_| Bytes::from(value.to_string()));

            let mut headers = HeaderMap::new();
            headers.insert(
                axum::http::header::CONTENT_TYPE,
                axum::http::HeaderValue::from_static("application/json"),
            );
            headers.insert(
                "x-ditto-translation",
                axum::http::HeaderValue::from_str(&translation_backend.provider)
                    .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
            );
            apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

            let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
            let mut response = axum::response::Response::new(body);
            *response.status_mut() = StatusCode::OK;
            *response.headers_mut() = headers;
            Ok((response, default_spend))
        } else if let Some(batch_id) = batch_cancel_id.as_deref() {
            if _stream_requested {
                break 'translation_backend_attempt Err(openai_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request_error",
                    Some("invalid_request"),
                    "batches endpoint does not support stream=true",
                ));
            }

            let cancelled = match translation_backend.cancel_batch(batch_id).await {
                Ok(cancelled) => cancelled,
                Err(err) => {
                    let (status, kind, code, message) =
                        translation::map_provider_error_to_openai(err);
                    break 'translation_backend_attempt Err(openai_error(
                        status, kind, code, message,
                    ));
                }
            };

            let value = translation::batch_to_openai(&cancelled.batch);
            let bytes = serde_json::to_vec(&value)
                .map(Bytes::from)
                .unwrap_or_else(|_| Bytes::from(value.to_string()));

            let mut headers = HeaderMap::new();
            headers.insert(
                axum::http::header::CONTENT_TYPE,
                axum::http::HeaderValue::from_static("application/json"),
            );
            headers.insert(
                "x-ditto-translation",
                axum::http::HeaderValue::from_str(&translation_backend.provider)
                    .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
            );
            apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

            let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
            let mut response = axum::response::Response::new(body);
            *response.status_mut() = StatusCode::OK;
            *response.headers_mut() = headers;
            Ok((response, default_spend))
        } else if translation::is_rerank_path(path_and_query) {
            let Some(parsed_json) = parsed_json.as_ref() else {
                break 'translation_backend_attempt Err(openai_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request_error",
                    Some("invalid_request"),
                    "request body must be application/json",
                ));
            };

            if _stream_requested {
                break 'translation_backend_attempt Err(openai_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request_error",
                    Some("invalid_request"),
                    "rerank endpoint does not support stream=true",
                ));
            }

            let mut request = match translation::rerank_request_to_request(parsed_json) {
                Ok(request) => request,
                Err(err) => {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        err,
                    ));
                }
            };

            let Some(original_model) = request.model.clone() else {
                break 'translation_backend_attempt Err(openai_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request_error",
                    Some("invalid_request"),
                    "missing model",
                ));
            };

            let mapped_model = translation_backend.map_model(&original_model);
            if mapped_model.trim().is_empty() {
                break 'translation_backend_attempt Err(openai_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request_error",
                    Some("invalid_request"),
                    "missing model",
                ));
            }
            request.model = Some(mapped_model.clone());

            let reranked = match translation_backend.rerank(&mapped_model, request).await {
                Ok(reranked) => reranked,
                Err(err) => {
                    let (status, kind, code, message) =
                        translation::map_provider_error_to_openai(err);
                    break 'translation_backend_attempt Err(openai_error(
                        status, kind, code, message,
                    ));
                }
            };

            let value = translation::rerank_response_to_openai(&reranked);
            let bytes = serde_json::to_vec(&value)
                .map(Bytes::from)
                .unwrap_or_else(|_| Bytes::from(value.to_string()));

            let mut headers = HeaderMap::new();
            headers.insert(
                axum::http::header::CONTENT_TYPE,
                axum::http::HeaderValue::from_static("application/json"),
            );
            headers.insert(
                "x-ditto-translation",
                axum::http::HeaderValue::from_str(&translation_backend.provider)
                    .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
            );
            apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

            let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
            let mut response = axum::response::Response::new(body);
            *response.status_mut() = StatusCode::OK;
            *response.headers_mut() = headers;
            Ok((response, default_spend))
        } else if translation::is_audio_transcriptions_path(path_and_query)
            || translation::is_audio_translations_path(path_and_query)
        {
            let endpoint = if translation::is_audio_translations_path(path_and_query) {
                "audio/translations"
            } else {
                "audio/transcriptions"
            };

            let Some(content_type) = parts
                .headers
                .get("content-type")
                .and_then(|value| value.to_str().ok())
            else {
                break 'translation_backend_attempt Err(openai_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request_error",
                    Some("invalid_request"),
                    format!("{endpoint} request missing content-type"),
                ));
            };

            if !content_type
                .to_ascii_lowercase()
                .starts_with("multipart/form-data")
            {
                break 'translation_backend_attempt Err(openai_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request_error",
                    Some("invalid_request"),
                    format!("{endpoint} request must be multipart/form-data"),
                ));
            }

            let request =
                match translation::audio_transcriptions_request_to_request(content_type, body) {
                    Ok(request) => request,
                    Err(err) => {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            err,
                        ));
                    }
                };

            let Some(original_model) = request.model.clone() else {
                break 'translation_backend_attempt Err(openai_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request_error",
                    Some("invalid_request"),
                    "missing model",
                ));
            };

            let mapped_model = translation_backend.map_model(&original_model);
            if mapped_model.trim().is_empty() {
                break 'translation_backend_attempt Err(openai_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request_error",
                    Some("invalid_request"),
                    "missing model",
                ));
            }
            if !translation_backend
                .supports_endpoint(&endpoint_descriptor, Some(mapped_model.as_str()))
            {
                break 'translation_backend_attempt Err(openai_error(
                    StatusCode::NOT_IMPLEMENTED,
                    "invalid_request_error",
                    Some("unsupported_endpoint"),
                    format!(
                        "translation backend does not support {} {}",
                        parts.method, path_and_query
                    ),
                ));
            }

            let request_format = request.response_format;
            let mut request = request;
            request.model = Some(mapped_model.clone());

            let transcribed = match translation_backend
                .transcribe_audio(&mapped_model, request)
                .await
            {
                Ok(transcribed) => transcribed,
                Err(err) => {
                    let (status, kind, code, message) =
                        translation::map_provider_error_to_openai(err);
                    break 'translation_backend_attempt Err(openai_error(
                        status, kind, code, message,
                    ));
                }
            };

            let (content_type, is_json) =
                translation::transcription_format_to_content_type(request_format);
            let bytes = if is_json {
                let value = serde_json::json!({ "text": transcribed.text });
                serde_json::to_vec(&value)
                    .map(Bytes::from)
                    .unwrap_or_else(|_| Bytes::from(value.to_string()))
            } else {
                Bytes::from(transcribed.text)
            };

            let mut headers = HeaderMap::new();
            headers.insert(
                axum::http::header::CONTENT_TYPE,
                axum::http::HeaderValue::from_str(content_type).unwrap_or_else(|_| {
                    axum::http::HeaderValue::from_static("application/octet-stream")
                }),
            );
            headers.insert(
                "x-ditto-translation",
                axum::http::HeaderValue::from_str(&translation_backend.provider)
                    .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
            );
            apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

            let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
            let mut response = axum::response::Response::new(body);
            *response.status_mut() = StatusCode::OK;
            *response.headers_mut() = headers;
            Ok((response, default_spend))
        } else if translation::is_audio_speech_path(path_and_query) {
            let Some(parsed_json) = parsed_json.as_ref() else {
                break 'translation_backend_attempt Err(openai_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request_error",
                    Some("invalid_request"),
                    "request body must be application/json",
                ));
            };

            if _stream_requested {
                break 'translation_backend_attempt Err(openai_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request_error",
                    Some("invalid_request"),
                    "audio/speech endpoint does not support stream=true",
                ));
            }

            let request = match translation::audio_speech_request_to_request(parsed_json) {
                Ok(request) => request,
                Err(err) => {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        err,
                    ));
                }
            };

            let Some(original_model) = request.model.clone() else {
                break 'translation_backend_attempt Err(openai_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request_error",
                    Some("invalid_request"),
                    "missing model",
                ));
            };

            let mapped_model = translation_backend.map_model(&original_model);
            if mapped_model.trim().is_empty() {
                break 'translation_backend_attempt Err(openai_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request_error",
                    Some("invalid_request"),
                    "missing model",
                ));
            }

            let request_format = request.response_format;
            let mut request = request;
            request.model = Some(mapped_model.clone());

            let spoken = match translation_backend
                .speak_audio(&mapped_model, request)
                .await
            {
                Ok(spoken) => spoken,
                Err(err) => {
                    let (status, kind, code, message) =
                        translation::map_provider_error_to_openai(err);
                    break 'translation_backend_attempt Err(openai_error(
                        status, kind, code, message,
                    ));
                }
            };

            let content_type = spoken.media_type.clone().unwrap_or_else(|| {
                translation::speech_response_format_to_content_type(request_format).to_string()
            });

            let mut headers = HeaderMap::new();
            headers.insert(
                axum::http::header::CONTENT_TYPE,
                axum::http::HeaderValue::from_str(&content_type).unwrap_or_else(|_| {
                    axum::http::HeaderValue::from_static("application/octet-stream")
                }),
            );
            headers.insert(
                "x-ditto-translation",
                axum::http::HeaderValue::from_str(&translation_backend.provider)
                    .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
            );
            apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

            let body =
                proxy_body_from_bytes_with_permit(Bytes::from(spoken.audio), proxy_permits.take());
            let mut response = axum::response::Response::new(body);
            *response.status_mut() = StatusCode::OK;
            *response.headers_mut() = headers;
            Ok((response, default_spend))
        } else if translation::is_embeddings_path(path_and_query) {
            let Some(parsed_json) = parsed_json.as_ref() else {
                break 'translation_backend_attempt Err(openai_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request_error",
                    Some("invalid_request"),
                    "request body must be application/json",
                ));
            };

            let original_model = model.clone().unwrap_or_default();
            let mapped_model = translation_backend.map_model(&original_model);

            if mapped_model.trim().is_empty() {
                break 'translation_backend_attempt Err(openai_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request_error",
                    Some("invalid_request"),
                    "missing model",
                ));
            }
            if _stream_requested {
                break 'translation_backend_attempt Err(openai_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request_error",
                    Some("invalid_request"),
                    "embeddings endpoint does not support stream=true",
                ));
            }

            let texts = match translation::embeddings_request_to_texts(parsed_json) {
                Ok(texts) => texts,
                Err(err) => {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        err,
                    ));
                }
            };

            let embeddings = match translation_backend.embed(&mapped_model, texts).await {
                Ok(embeddings) => embeddings,
                Err(err) => {
                    let (status, kind, code, message) =
                        translation::map_provider_error_to_openai(err);
                    break 'translation_backend_attempt Err(openai_error(
                        status, kind, code, message,
                    ));
                }
            };

            let value = translation::embeddings_to_openai_response(embeddings, &original_model);
            let bytes = serde_json::to_vec(&value)
                .map(Bytes::from)
                .unwrap_or_else(|_| Bytes::from(value.to_string()));

            let mut headers = HeaderMap::new();
            headers.insert(
                axum::http::header::CONTENT_TYPE,
                axum::http::HeaderValue::from_static("application/json"),
            );
            headers.insert(
                "x-ditto-translation",
                axum::http::HeaderValue::from_str(&translation_backend.provider)
                    .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
            );
            apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

            let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
            let mut response = axum::response::Response::new(body);
            *response.status_mut() = StatusCode::OK;
            *response.headers_mut() = headers;
            Ok((response, default_spend))
        } else {
            // inlined from rest.rs
            if translation::is_moderations_path(path_and_query) {
                let Some(request_json) = parsed_json.as_ref() else {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "request body must be application/json",
                    ));
                };

                let original_model = model.clone().unwrap_or_default();
                let mapped_model = translation_backend.map_model(&original_model);

                if _stream_requested {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "moderations endpoint does not support stream=true",
                    ));
                }

                let mut request = match translation::moderations_request_to_request(request_json) {
                    Ok(request) => request,
                    Err(err) => {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            err,
                        ));
                    }
                };

                if !mapped_model.trim().is_empty() {
                    request.model = Some(mapped_model);
                }

                let moderated = match translation_backend.moderate(request).await {
                    Ok(moderated) => moderated,
                    Err(err) => {
                        let (status, kind, code, message) =
                            translation::map_provider_error_to_openai(err);
                        break 'translation_backend_attempt Err(openai_error(
                            status, kind, code, message,
                        ));
                    }
                };

                let fallback_id = format!("modr_{request_id}");
                let value = translation::moderation_response_to_openai(&moderated, &fallback_id);

                let bytes = serde_json::to_vec(&value)
                    .map(Bytes::from)
                    .unwrap_or_else(|_| Bytes::from(value.to_string()));

                let mut headers = HeaderMap::new();
                headers.insert(
                    axum::http::header::CONTENT_TYPE,
                    axum::http::HeaderValue::from_static("application/json"),
                );
                headers.insert(
                    "x-ditto-translation",
                    axum::http::HeaderValue::from_str(&translation_backend.provider)
                        .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
                );
                apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

                let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
                let mut response = axum::response::Response::new(body);
                *response.status_mut() = StatusCode::OK;
                *response.headers_mut() = headers;
                Ok((response, default_spend))
            } else if files_root && parts.method == axum::http::Method::POST {
                if _stream_requested {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "files endpoint does not support stream=true",
                    ));
                }

                let Some(content_type) = parts
                    .headers
                    .get("content-type")
                    .and_then(|value| value.to_str().ok())
                else {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "files request missing content-type",
                    ));
                };

                if !content_type
                    .to_ascii_lowercase()
                    .starts_with("multipart/form-data")
                {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "files request must be multipart/form-data",
                    ));
                }

                let request = match translation::files_upload_request_to_request(content_type, body)
                {
                    Ok(request) => request,
                    Err(err) => {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            err,
                        ));
                    }
                };

                let bytes_len = request.bytes.len();
                let filename = request.filename.clone();
                let purpose = request.purpose.clone();
                let file_id = match translation_backend.upload_file(request).await {
                    Ok(file_id) => file_id,
                    Err(err) => {
                        let (status, kind, code, message) =
                            translation::map_provider_error_to_openai(err);
                        break 'translation_backend_attempt Err(openai_error(
                            status, kind, code, message,
                        ));
                    }
                };

                let value = translation::file_upload_response_to_openai(
                    &file_id,
                    filename,
                    purpose,
                    bytes_len,
                    _now_epoch_seconds,
                );
                let bytes = serde_json::to_vec(&value)
                    .map(Bytes::from)
                    .unwrap_or_else(|_| Bytes::from(value.to_string()));

                let mut headers = HeaderMap::new();
                headers.insert(
                    axum::http::header::CONTENT_TYPE,
                    axum::http::HeaderValue::from_static("application/json"),
                );
                headers.insert(
                    "x-ditto-translation",
                    axum::http::HeaderValue::from_str(&translation_backend.provider)
                        .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
                );
                apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

                let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
                let mut response = axum::response::Response::new(body);
                *response.status_mut() = StatusCode::OK;
                *response.headers_mut() = headers;
                Ok((response, default_spend))
            } else if files_root && parts.method == axum::http::Method::GET {
                if _stream_requested {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "files endpoint does not support stream=true",
                    ));
                }

                let files = match translation_backend.list_files().await {
                    Ok(files) => files,
                    Err(err) => {
                        let (status, kind, code, message) =
                            translation::map_provider_error_to_openai(err);
                        break 'translation_backend_attempt Err(openai_error(
                            status, kind, code, message,
                        ));
                    }
                };

                let value = translation::file_list_response_to_openai(&files);
                let bytes = serde_json::to_vec(&value)
                    .map(Bytes::from)
                    .unwrap_or_else(|_| Bytes::from(value.to_string()));

                let mut headers = HeaderMap::new();
                headers.insert(
                    axum::http::header::CONTENT_TYPE,
                    axum::http::HeaderValue::from_static("application/json"),
                );
                headers.insert(
                    "x-ditto-translation",
                    axum::http::HeaderValue::from_str(&translation_backend.provider)
                        .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
                );
                apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

                let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
                let mut response = axum::response::Response::new(body);
                *response.status_mut() = StatusCode::OK;
                *response.headers_mut() = headers;
                Ok((response, default_spend))
            } else if let Some(file_id) = files_content_id.as_deref()
                && parts.method == axum::http::Method::GET
            {
                if _stream_requested {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "files endpoint does not support stream=true",
                    ));
                }

                let content = match translation_backend.download_file_content(file_id).await {
                    Ok(content) => content,
                    Err(err) => {
                        let (status, kind, code, message) =
                            translation::map_provider_error_to_openai(err);
                        break 'translation_backend_attempt Err(openai_error(
                            status, kind, code, message,
                        ));
                    }
                };

                let content_type = content
                    .media_type
                    .unwrap_or_else(|| "application/octet-stream".to_string());

                let mut headers = HeaderMap::new();
                headers.insert(
                    axum::http::header::CONTENT_TYPE,
                    axum::http::HeaderValue::from_str(&content_type).unwrap_or_else(|_| {
                        axum::http::HeaderValue::from_static("application/octet-stream")
                    }),
                );
                headers.insert(
                    "x-ditto-translation",
                    axum::http::HeaderValue::from_str(&translation_backend.provider)
                        .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
                );
                apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

                let body = proxy_body_from_bytes_with_permit(
                    Bytes::from(content.bytes),
                    proxy_permits.take(),
                );
                let mut response = axum::response::Response::new(body);
                *response.status_mut() = StatusCode::OK;
                *response.headers_mut() = headers;
                Ok((response, default_spend))
            } else if let Some(file_id) = files_retrieve_id.as_deref() {
                if _stream_requested {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "files endpoint does not support stream=true",
                    ));
                }

                let value = if parts.method == axum::http::Method::GET {
                    let file = match translation_backend.retrieve_file(file_id).await {
                        Ok(file) => file,
                        Err(err) => {
                            let (status, kind, code, message) =
                                translation::map_provider_error_to_openai(err);
                            break 'translation_backend_attempt Err(openai_error(
                                status, kind, code, message,
                            ));
                        }
                    };

                    translation::file_to_openai(&file)
                } else if parts.method == axum::http::Method::DELETE {
                    let deleted = match translation_backend.delete_file(file_id).await {
                        Ok(deleted) => deleted,
                        Err(err) => {
                            let (status, kind, code, message) =
                                translation::map_provider_error_to_openai(err);
                            break 'translation_backend_attempt Err(openai_error(
                                status, kind, code, message,
                            ));
                        }
                    };

                    translation::file_delete_response_to_openai(&deleted)
                } else {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::NOT_IMPLEMENTED,
                        "invalid_request_error",
                        Some("unsupported_endpoint"),
                        format!(
                            "translation backend does not support {} {}",
                            parts.method, path_and_query
                        ),
                    ));
                };

                let bytes = serde_json::to_vec(&value)
                    .map(Bytes::from)
                    .unwrap_or_else(|_| Bytes::from(value.to_string()));

                let mut headers = HeaderMap::new();
                headers.insert(
                    axum::http::header::CONTENT_TYPE,
                    axum::http::HeaderValue::from_static("application/json"),
                );
                headers.insert(
                    "x-ditto-translation",
                    axum::http::HeaderValue::from_str(&translation_backend.provider)
                        .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
                );
                apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

                let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
                let mut response = axum::response::Response::new(body);
                *response.status_mut() = StatusCode::OK;
                *response.headers_mut() = headers;
                Ok((response, default_spend))
            } else if videos_root && parts.method == axum::http::Method::POST {
                let Some(content_type) = parts
                    .headers
                    .get("content-type")
                    .and_then(|value| value.to_str().ok())
                else {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "videos request missing content-type",
                    ));
                };

                let multipart_stream_requested = if content_type
                    .to_ascii_lowercase()
                    .starts_with("multipart/form-data")
                {
                    match translation::multipart_extract_text_field(content_type, body, "stream") {
                        Ok(Some(value)) => matches!(value.trim(), "true" | "1"),
                        Ok(None) => false,
                        Err(err) => {
                            break 'translation_backend_attempt Err(openai_error(
                                StatusCode::BAD_REQUEST,
                                "invalid_request_error",
                                Some("invalid_request"),
                                err,
                            ));
                        }
                    }
                } else {
                    false
                };

                if _stream_requested || multipart_stream_requested {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "videos endpoint does not support stream=true",
                    ));
                }

                let mut request = if content_type
                    .to_ascii_lowercase()
                    .starts_with("multipart/form-data")
                {
                    match translation::videos_create_multipart_request_to_request(
                        content_type,
                        body,
                    ) {
                        Ok(request) => request,
                        Err(err) => {
                            break 'translation_backend_attempt Err(openai_error(
                                StatusCode::BAD_REQUEST,
                                "invalid_request_error",
                                Some("invalid_request"),
                                err,
                            ));
                        }
                    }
                } else {
                    let Some(parsed_json) = parsed_json.as_ref() else {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            "request body must be application/json or multipart/form-data",
                        ));
                    };
                    match translation::videos_create_request_to_request(parsed_json) {
                        Ok(request) => request,
                        Err(err) => {
                            break 'translation_backend_attempt Err(openai_error(
                                StatusCode::BAD_REQUEST,
                                "invalid_request_error",
                                Some("invalid_request"),
                                err,
                            ));
                        }
                    }
                };

                let original_model = request.model.clone().unwrap_or_default();
                let mapped_model = translation_backend.map_model(&original_model);
                if !mapped_model.trim().is_empty() {
                    request.model = Some(mapped_model);
                }

                let generated = match translation_backend.create_video(request).await {
                    Ok(generated) => generated,
                    Err(err) => {
                        let (status, kind, code, message) =
                            translation::map_provider_error_to_openai(err);
                        break 'translation_backend_attempt Err(openai_error(
                            status, kind, code, message,
                        ));
                    }
                };

                let value = translation::video_generation_response_to_openai(&generated);
                let bytes = serde_json::to_vec(&value)
                    .map(Bytes::from)
                    .unwrap_or_else(|_| Bytes::from(value.to_string()));

                let mut headers = HeaderMap::new();
                headers.insert(
                    axum::http::header::CONTENT_TYPE,
                    axum::http::HeaderValue::from_static("application/json"),
                );
                headers.insert(
                    "x-ditto-translation",
                    axum::http::HeaderValue::from_str(&translation_backend.provider)
                        .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
                );
                apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

                let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
                let mut response = axum::response::Response::new(body);
                *response.status_mut() = StatusCode::OK;
                *response.headers_mut() = headers;
                Ok((response, default_spend))
            } else if videos_root && parts.method == axum::http::Method::GET {
                if _stream_requested {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "videos endpoint does not support stream=true",
                    ));
                }

                let request = match translation::videos_list_request_from_path(path_and_query) {
                    Ok(request) => request,
                    Err(err) => {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            err,
                        ));
                    }
                };

                let videos = match translation_backend.list_videos(request).await {
                    Ok(videos) => videos,
                    Err(err) => {
                        let (status, kind, code, message) =
                            translation::map_provider_error_to_openai(err);
                        break 'translation_backend_attempt Err(openai_error(
                            status, kind, code, message,
                        ));
                    }
                };

                let value = translation::video_list_response_to_openai(&videos);
                let bytes = serde_json::to_vec(&value)
                    .map(Bytes::from)
                    .unwrap_or_else(|_| Bytes::from(value.to_string()));

                let mut headers = HeaderMap::new();
                headers.insert(
                    axum::http::header::CONTENT_TYPE,
                    axum::http::HeaderValue::from_static("application/json"),
                );
                headers.insert(
                    "x-ditto-translation",
                    axum::http::HeaderValue::from_str(&translation_backend.provider)
                        .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
                );
                apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

                let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
                let mut response = axum::response::Response::new(body);
                *response.status_mut() = StatusCode::OK;
                *response.headers_mut() = headers;
                Ok((response, default_spend))
            } else if let Some(video_id) = videos_content_id.as_deref()
                && parts.method == axum::http::Method::GET
            {
                if _stream_requested {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "videos endpoint does not support stream=true",
                    ));
                }

                let variant = match translation::videos_content_variant_from_path(path_and_query) {
                    Ok(variant) => variant,
                    Err(err) => {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            err,
                        ));
                    }
                };

                let content = match translation_backend
                    .download_video_content(video_id, variant)
                    .await
                {
                    Ok(content) => content,
                    Err(err) => {
                        let (status, kind, code, message) =
                            translation::map_provider_error_to_openai(err);
                        break 'translation_backend_attempt Err(openai_error(
                            status, kind, code, message,
                        ));
                    }
                };

                let content_type = content
                    .media_type
                    .unwrap_or_else(|| "application/octet-stream".to_string());

                let mut headers = HeaderMap::new();
                headers.insert(
                    axum::http::header::CONTENT_TYPE,
                    axum::http::HeaderValue::from_str(&content_type).unwrap_or_else(|_| {
                        axum::http::HeaderValue::from_static("application/octet-stream")
                    }),
                );
                headers.insert(
                    "x-ditto-translation",
                    axum::http::HeaderValue::from_str(&translation_backend.provider)
                        .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
                );
                apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

                let body = proxy_body_from_bytes_with_permit(
                    Bytes::from(content.bytes),
                    proxy_permits.take(),
                );
                let mut response = axum::response::Response::new(body);
                *response.status_mut() = StatusCode::OK;
                *response.headers_mut() = headers;
                Ok((response, default_spend))
            } else if let Some(video_id) = videos_remix_id.as_deref()
                && parts.method == axum::http::Method::POST
            {
                let Some(parsed_json) = parsed_json.as_ref() else {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "request body must be application/json",
                    ));
                };

                if _stream_requested {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "videos endpoint does not support stream=true",
                    ));
                }

                let request = match translation::videos_remix_request_to_request(parsed_json) {
                    Ok(request) => request,
                    Err(err) => {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            err,
                        ));
                    }
                };

                let remixed = match translation_backend.remix_video(video_id, request).await {
                    Ok(remixed) => remixed,
                    Err(err) => {
                        let (status, kind, code, message) =
                            translation::map_provider_error_to_openai(err);
                        break 'translation_backend_attempt Err(openai_error(
                            status, kind, code, message,
                        ));
                    }
                };

                let value = translation::video_generation_response_to_openai(&remixed);
                let bytes = serde_json::to_vec(&value)
                    .map(Bytes::from)
                    .unwrap_or_else(|_| Bytes::from(value.to_string()));

                let mut headers = HeaderMap::new();
                headers.insert(
                    axum::http::header::CONTENT_TYPE,
                    axum::http::HeaderValue::from_static("application/json"),
                );
                headers.insert(
                    "x-ditto-translation",
                    axum::http::HeaderValue::from_str(&translation_backend.provider)
                        .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
                );
                apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

                let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
                let mut response = axum::response::Response::new(body);
                *response.status_mut() = StatusCode::OK;
                *response.headers_mut() = headers;
                Ok((response, default_spend))
            } else if let Some(video_id) = videos_retrieve_id.as_deref() {
                if _stream_requested {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "videos endpoint does not support stream=true",
                    ));
                }

                let value = if parts.method == axum::http::Method::GET {
                    let video = match translation_backend.retrieve_video(video_id).await {
                        Ok(video) => video,
                        Err(err) => {
                            let (status, kind, code, message) =
                                translation::map_provider_error_to_openai(err);
                            break 'translation_backend_attempt Err(openai_error(
                                status, kind, code, message,
                            ));
                        }
                    };

                    translation::video_generation_response_to_openai(&video)
                } else if parts.method == axum::http::Method::DELETE {
                    let deleted = match translation_backend.delete_video(video_id).await {
                        Ok(deleted) => deleted,
                        Err(err) => {
                            let (status, kind, code, message) =
                                translation::map_provider_error_to_openai(err);
                            break 'translation_backend_attempt Err(openai_error(
                                status, kind, code, message,
                            ));
                        }
                    };

                    translation::video_delete_response_to_openai(&deleted)
                } else {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::NOT_IMPLEMENTED,
                        "invalid_request_error",
                        Some("unsupported_endpoint"),
                        format!(
                            "translation backend does not support {} {}",
                            parts.method, path_and_query
                        ),
                    ));
                };

                let bytes = serde_json::to_vec(&value)
                    .map(Bytes::from)
                    .unwrap_or_else(|_| Bytes::from(value.to_string()));

                let mut headers = HeaderMap::new();
                headers.insert(
                    axum::http::header::CONTENT_TYPE,
                    axum::http::HeaderValue::from_static("application/json"),
                );
                headers.insert(
                    "x-ditto-translation",
                    axum::http::HeaderValue::from_str(&translation_backend.provider)
                        .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
                );
                apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

                let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
                let mut response = axum::response::Response::new(body);
                *response.status_mut() = StatusCode::OK;
                *response.headers_mut() = headers;
                Ok((response, default_spend))
            } else if responses_input_tokens && parts.method == axum::http::Method::POST {
                if _stream_requested {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "responses input_tokens endpoint does not support stream=true",
                    ));
                }

                let Some(request_json) = parsed_json.as_ref() else {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "request body must be application/json",
                    ));
                };

                let original_model = model.clone().unwrap_or_default();
                if original_model.trim().is_empty() {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "responses input_tokens endpoint requires model",
                    ));
                }

                let mapped_model = translation_backend.map_model(&original_model);

                #[cfg(feature = "gateway-tokenizer")]
                let input_tokens = {
                    let tokenizer_model = mapped_model
                        .trim()
                        .split_once('/')
                        .map(|(_, model)| model)
                        .unwrap_or_else(|| mapped_model.trim());
                    let tokenizer_model = if tokenizer_model.is_empty() {
                        original_model.trim()
                    } else {
                        tokenizer_model
                    };
                    token_count::estimate_input_tokens(
                        "/v1/responses",
                        tokenizer_model,
                        request_json,
                    )
                    .unwrap_or_else(|| estimate_tokens_from_bytes(body))
                };

                #[cfg(not(feature = "gateway-tokenizer"))]
                {
                    let _ = (&mapped_model, &original_model, request_json);
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::NOT_IMPLEMENTED,
                        "invalid_request_error",
                        Some("unsupported_endpoint"),
                        "responses input_tokens endpoint requires gateway-tokenizer feature",
                    ));
                }

                #[cfg(feature = "gateway-tokenizer")]
                {
                    let value = translation::responses_input_tokens_to_openai(input_tokens);
                    let bytes = serde_json::to_vec(&value)
                        .map(Bytes::from)
                        .unwrap_or_else(|_| Bytes::from(value.to_string()));

                    let mut headers = HeaderMap::new();
                    headers.insert(
                        axum::http::header::CONTENT_TYPE,
                        axum::http::HeaderValue::from_static("application/json"),
                    );
                    headers.insert(
                        "x-ditto-translation",
                        axum::http::HeaderValue::from_str(&translation_backend.provider)
                            .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
                    );
                    apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

                    let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
                    let mut response = axum::response::Response::new(body);
                    *response.status_mut() = StatusCode::OK;
                    *response.headers_mut() = headers;
                    Ok((
                        response,
                        ProxySpend {
                            tokens: 0,
                            cost_usd_micros: None,
                        },
                    ))
                }
            } else if let Some(response_id) = responses_input_items_id.as_deref()
                && parts.method == axum::http::Method::GET
            {
                if _stream_requested {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "responses endpoint does not support stream=true",
                    ));
                }

                let Some((stored_backend_name, stored_provider, stored_response)) =
                    translation::find_stored_response_from_translation_backends(
                        state.backends.translation_backends.as_ref(),
                        response_id,
                    )
                    .await
                else {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::NOT_FOUND,
                        "invalid_request_error",
                        Some("response_not_found"),
                        format!("response {response_id} not found"),
                    ));
                };

                let value =
                    translation::responses_input_items_to_openai(&stored_response.input_items);
                let bytes = serde_json::to_vec(&value)
                    .map(Bytes::from)
                    .unwrap_or_else(|_| Bytes::from(value.to_string()));

                let mut headers = HeaderMap::new();
                headers.insert(
                    axum::http::header::CONTENT_TYPE,
                    axum::http::HeaderValue::from_static("application/json"),
                );
                headers.insert(
                    "x-ditto-translation",
                    axum::http::HeaderValue::from_str(&stored_provider)
                        .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
                );
                apply_proxy_response_headers(&mut headers, &stored_backend_name, request_id, false);

                let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
                let mut response = axum::response::Response::new(body);
                *response.status_mut() = StatusCode::OK;
                *response.headers_mut() = headers;
                Ok((response, default_spend))
            } else if let Some(response_id) = responses_retrieve_id.as_deref() {
                if _stream_requested {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "responses endpoint does not support stream=true",
                    ));
                }

                if parts.method == axum::http::Method::GET {
                    let Some((stored_backend_name, stored_provider, stored_response)) =
                        translation::find_stored_response_from_translation_backends(
                            state.backends.translation_backends.as_ref(),
                            response_id,
                        )
                        .await
                    else {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::NOT_FOUND,
                            "invalid_request_error",
                            Some("response_not_found"),
                            format!("response {response_id} not found"),
                        ));
                    };

                    let bytes = serde_json::to_vec(&stored_response.response)
                        .map(Bytes::from)
                        .unwrap_or_else(|_| Bytes::from(stored_response.response.to_string()));

                    let mut headers = HeaderMap::new();
                    headers.insert(
                        axum::http::header::CONTENT_TYPE,
                        axum::http::HeaderValue::from_static("application/json"),
                    );
                    headers.insert(
                        "x-ditto-translation",
                        axum::http::HeaderValue::from_str(&stored_provider)
                            .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
                    );
                    apply_proxy_response_headers(
                        &mut headers,
                        &stored_backend_name,
                        request_id,
                        false,
                    );

                    let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
                    let mut response = axum::response::Response::new(body);
                    *response.status_mut() = StatusCode::OK;
                    *response.headers_mut() = headers;
                    Ok((response, default_spend))
                } else if parts.method == axum::http::Method::DELETE {
                    let Some((stored_backend_name, stored_provider)) =
                        translation::delete_stored_response_from_translation_backends(
                            state.backends.translation_backends.as_ref(),
                            response_id,
                        )
                        .await
                    else {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::NOT_FOUND,
                            "invalid_request_error",
                            Some("response_not_found"),
                            format!("response {response_id} not found"),
                        ));
                    };

                    let value = translation::response_delete_to_openai(response_id);
                    let bytes = serde_json::to_vec(&value)
                        .map(Bytes::from)
                        .unwrap_or_else(|_| Bytes::from(value.to_string()));

                    let mut headers = HeaderMap::new();
                    headers.insert(
                        axum::http::header::CONTENT_TYPE,
                        axum::http::HeaderValue::from_static("application/json"),
                    );
                    headers.insert(
                        "x-ditto-translation",
                        axum::http::HeaderValue::from_str(&stored_provider)
                            .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
                    );
                    apply_proxy_response_headers(
                        &mut headers,
                        &stored_backend_name,
                        request_id,
                        false,
                    );

                    let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
                    let mut response = axum::response::Response::new(body);
                    *response.status_mut() = StatusCode::OK;
                    *response.headers_mut() = headers;
                    Ok((response, default_spend))
                } else {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::NOT_IMPLEMENTED,
                        "invalid_request_error",
                        Some("unsupported_endpoint"),
                        format!(
                            "translation backend does not support {} {}",
                            parts.method, path_and_query
                        ),
                    ));
                }
            } else if translation::is_responses_compact_path(path_and_query) {
                let Some(parsed_json) = parsed_json.as_ref() else {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "request body must be application/json",
                    ));
                };

                let original_model = model.clone().unwrap_or_default();
                let mapped_model = translation_backend.map_model(&original_model);

                if mapped_model.trim().is_empty() {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "missing model",
                    ));
                }

                if _stream_requested {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "responses/compact endpoint does not support stream=true",
                    ));
                }

                let instructions = parsed_json
                    .get("instructions")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();

                let Some(input) = parsed_json.get("input") else {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "missing input",
                    ));
                };

                let input_items = match translation::responses_input_items_from_value(input) {
                    Ok(items) => items,
                    Err(err) => {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            err,
                        ));
                    }
                };

                let (output, usage) = match translation_backend
                    .compact_responses_history(&mapped_model, instructions, &input_items)
                    .await
                {
                    Ok(compacted) => compacted,
                    Err(err) => {
                        let (status, kind, code, message) =
                            translation::map_provider_error_to_openai(err);
                        break 'translation_backend_attempt Err(openai_error(
                            status, kind, code, message,
                        ));
                    }
                };

                let value = serde_json::json!({ "output": output });
                let bytes = serde_json::to_vec(&value)
                    .map(Bytes::from)
                    .unwrap_or_else(|_| Bytes::from(value.to_string()));

                let mut headers = HeaderMap::new();
                headers.insert(
                    axum::http::header::CONTENT_TYPE,
                    axum::http::HeaderValue::from_static("application/json"),
                );
                headers.insert(
                    "x-ditto-translation",
                    axum::http::HeaderValue::from_str(&translation_backend.provider)
                        .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
                );
                apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

                let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
                let mut response = axum::response::Response::new(body);
                *response.status_mut() = StatusCode::OK;
                *response.headers_mut() = headers;

                let tokens = usage
                    .total_tokens
                    .unwrap_or_else(|| u64::from(charge_tokens));
                #[cfg(feature = "gateway-costing")]
                let cost_usd_micros = model.as_deref().and_then(|model| {
                    state.proxy.pricing.as_ref().and_then(|pricing| {
                        let (Some(input), Some(output)) = (usage.input_tokens, usage.output_tokens)
                        else {
                            return None;
                        };
                        pricing.estimate_cost_usd_micros_with_cache_for_service_tier(
                            model,
                            clamp_u64_to_u32(input),
                            usage.cache_input_tokens.map(clamp_u64_to_u32),
                            usage.cache_creation_input_tokens.map(clamp_u64_to_u32),
                            clamp_u64_to_u32(output),
                            service_tier.as_deref(),
                        )
                    })
                });
                #[cfg(not(feature = "gateway-costing"))]
                let cost_usd_micros: Option<u64> = None;

                Ok((
                    response,
                    ProxySpend {
                        tokens,
                        cost_usd_micros: cost_usd_micros.or(charge_cost_usd_micros),
                    },
                ))
            } else if translation::is_images_edits_path(path_and_query) {
                let Some(content_type) = parts
                    .headers
                    .get("content-type")
                    .and_then(|value| value.to_str().ok())
                else {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "images/edits request missing content-type",
                    ));
                };

                if !content_type
                    .to_ascii_lowercase()
                    .starts_with("multipart/form-data")
                {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "images/edits request must be multipart/form-data",
                    ));
                }

                let multipart_stream_requested =
                    match translation::multipart_extract_text_field(content_type, body, "stream") {
                        Ok(Some(value)) => matches!(value.trim(), "true" | "1"),
                        Ok(None) => false,
                        Err(err) => {
                            break 'translation_backend_attempt Err(openai_error(
                                StatusCode::BAD_REQUEST,
                                "invalid_request_error",
                                Some("invalid_request"),
                                err,
                            ));
                        }
                    };

                if _stream_requested || multipart_stream_requested {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "images endpoint does not support stream=true",
                    ));
                }

                let mut request =
                    match translation::images_edits_request_to_request(content_type, body) {
                        Ok(request) => request,
                        Err(err) => {
                            break 'translation_backend_attempt Err(openai_error(
                                StatusCode::BAD_REQUEST,
                                "invalid_request_error",
                                Some("invalid_request"),
                                err,
                            ));
                        }
                    };

                let original_model = request.model.clone().unwrap_or_default();
                let mapped_model = translation_backend.map_model(&original_model);
                if !mapped_model.trim().is_empty() {
                    request.model = Some(mapped_model);
                }

                let edited = match translation_backend.edit_image(request).await {
                    Ok(edited) => edited,
                    Err(err) => {
                        let (status, kind, code, message) =
                            translation::map_provider_error_to_openai(err);
                        break 'translation_backend_attempt Err(openai_error(
                            status, kind, code, message,
                        ));
                    }
                };

                let value =
                    translation::image_generation_response_to_openai(&edited, _now_epoch_seconds);
                let bytes = serde_json::to_vec(&value)
                    .map(Bytes::from)
                    .unwrap_or_else(|_| Bytes::from(value.to_string()));

                let mut headers = HeaderMap::new();
                headers.insert(
                    axum::http::header::CONTENT_TYPE,
                    axum::http::HeaderValue::from_static("application/json"),
                );
                headers.insert(
                    "x-ditto-translation",
                    axum::http::HeaderValue::from_str(&translation_backend.provider)
                        .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
                );
                apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

                let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
                let mut response = axum::response::Response::new(body);
                *response.status_mut() = StatusCode::OK;
                *response.headers_mut() = headers;
                Ok((response, default_spend))
            } else if translation::is_images_generations_path(path_and_query) {
                let Some(parsed_json) = parsed_json.as_ref() else {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "request body must be application/json",
                    ));
                };

                let original_model = model.clone().unwrap_or_default();
                let mapped_model = translation_backend.map_model(&original_model);

                if _stream_requested {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "images endpoint does not support stream=true",
                    ));
                }

                let mut request =
                    match translation::images_generation_request_to_request(parsed_json) {
                        Ok(request) => request,
                        Err(err) => {
                            break 'translation_backend_attempt Err(openai_error(
                                StatusCode::BAD_REQUEST,
                                "invalid_request_error",
                                Some("invalid_request"),
                                err,
                            ));
                        }
                    };

                if !mapped_model.trim().is_empty() {
                    request.model = Some(mapped_model);
                }

                let generated = match translation_backend.generate_image(request).await {
                    Ok(generated) => generated,
                    Err(err) => {
                        let (status, kind, code, message) =
                            translation::map_provider_error_to_openai(err);
                        break 'translation_backend_attempt Err(openai_error(
                            status, kind, code, message,
                        ));
                    }
                };

                let value = translation::image_generation_response_to_openai(
                    &generated,
                    _now_epoch_seconds,
                );
                let bytes = serde_json::to_vec(&value)
                    .map(Bytes::from)
                    .unwrap_or_else(|_| Bytes::from(value.to_string()));

                let mut headers = HeaderMap::new();
                headers.insert(
                    axum::http::header::CONTENT_TYPE,
                    axum::http::HeaderValue::from_static("application/json"),
                );
                headers.insert(
                    "x-ditto-translation",
                    axum::http::HeaderValue::from_str(&translation_backend.provider)
                        .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
                );
                apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

                let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
                let mut response = axum::response::Response::new(body);
                *response.status_mut() = StatusCode::OK;
                *response.headers_mut() = headers;
                Ok((response, default_spend))
            } else {
                let Some(parsed_json) = parsed_json.as_ref() else {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "request body must be application/json",
                    ));
                };

                let original_model = model.clone().unwrap_or_default();
                let mapped_model = translation_backend.map_model(&original_model);

                if mapped_model.trim().is_empty() {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "missing model",
                    ));
                }

                let responses_input_items = if translation::is_responses_create_path(path_and_query)
                {
                    let Some(input) = parsed_json.get("input") else {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            "missing input",
                        ));
                    };
                    Some(match translation::responses_input_items_from_value(input) {
                        Ok(items) => items,
                        Err(err) => {
                            break 'translation_backend_attempt Err(openai_error(
                                StatusCode::BAD_REQUEST,
                                "invalid_request_error",
                                Some("invalid_request"),
                                err,
                            ));
                        }
                    })
                } else {
                    None
                };

                let generate_request = if translation::is_chat_completions_path(path_and_query) {
                    translation::chat_completions_request_to_generate_request(parsed_json)
                } else if translation::is_completions_path(path_and_query) {
                    translation::completions_request_to_generate_request(parsed_json)
                } else {
                    translation::responses_request_to_generate_request(parsed_json)
                };

                let generate_request = match generate_request {
                    Ok(mut request) => {
                        request.model = Some(mapped_model);
                        request
                    }
                    Err(err) => {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            err,
                        ));
                    }
                };

                let fallback_response_id = if translation::is_chat_completions_path(path_and_query)
                {
                    format!("chatcmpl_{request_id}")
                } else if translation::is_completions_path(path_and_query) {
                    format!("cmpl_{request_id}")
                } else {
                    format!("resp_{request_id}")
                };

                let include_usage = _stream_requested
                    && translation::is_chat_completions_path(path_and_query)
                    && parsed_json
                        .get("stream_options")
                        .and_then(|value| value.get("include_usage"))
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false);

                if _stream_requested {
                    let stream = match translation_backend.model.stream(generate_request).await {
                        Ok(stream) => stream,
                        Err(err) => {
                            let (status, kind, code, message) =
                                translation::map_provider_error_to_openai(err);
                            break 'translation_backend_attempt Err(openai_error(
                                status, kind, code, message,
                            ));
                        }
                    };

                    let stream = if translation::is_chat_completions_path(path_and_query) {
                        translation::stream_to_chat_completions_sse(
                            stream,
                            fallback_response_id.clone(),
                            original_model.clone(),
                            _now_epoch_seconds,
                            include_usage,
                        )
                    } else if translation::is_completions_path(path_and_query) {
                        translation::stream_to_completions_sse(
                            stream,
                            fallback_response_id.clone(),
                            original_model.clone(),
                            _now_epoch_seconds,
                        )
                    } else {
                        translation::stream_to_responses_sse(stream, fallback_response_id)
                    };

                    let mut headers = HeaderMap::new();
                    headers.insert(
                        axum::http::header::CONTENT_TYPE,
                        axum::http::HeaderValue::from_static("text/event-stream"),
                    );
                    headers.insert(
                        "x-ditto-translation",
                        axum::http::HeaderValue::from_str(&translation_backend.provider)
                            .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
                    );
                    headers.remove("content-length");
                    apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

                    let stream = ProxyBodyStreamWithPermit {
                        inner: stream.boxed(),
                        _permits: proxy_permits.take(),
                    };
                    let mut response = axum::response::Response::new(Body::from_stream(stream));
                    *response.status_mut() = StatusCode::OK;
                    *response.headers_mut() = headers;
                    Ok((response, default_spend))
                } else {
                    let generated = match translation_backend.model.generate(generate_request).await
                    {
                        Ok(generated) => generated,
                        Err(err) => {
                            let (status, kind, code, message) =
                                translation::map_provider_error_to_openai(err);
                            break 'translation_backend_attempt Err(openai_error(
                                status, kind, code, message,
                            ));
                        }
                    };

                    let response_id =
                        translation::provider_response_id(&generated, &fallback_response_id);
                    let value = if translation::is_chat_completions_path(path_and_query) {
                        translation::generate_response_to_chat_completions(
                            &generated,
                            &response_id,
                            &original_model,
                            _now_epoch_seconds,
                        )
                    } else if translation::is_completions_path(path_and_query) {
                        translation::generate_response_to_completions(
                            &generated,
                            &response_id,
                            &original_model,
                            _now_epoch_seconds,
                        )
                    } else {
                        translation::generate_response_to_responses(
                            &generated,
                            &response_id,
                            &original_model,
                            _now_epoch_seconds,
                        )
                    };

                    if let Some(input_items) = responses_input_items {
                        translation_backend
                            .store_response_record(&response_id, value.clone(), input_items)
                            .await;
                    }

                    let bytes = serde_json::to_vec(&value)
                        .map(Bytes::from)
                        .unwrap_or_else(|_| Bytes::from(value.to_string()));

                    let mut headers = HeaderMap::new();
                    headers.insert(
                        axum::http::header::CONTENT_TYPE,
                        axum::http::HeaderValue::from_static("application/json"),
                    );
                    headers.insert(
                        "x-ditto-translation",
                        axum::http::HeaderValue::from_str(&translation_backend.provider)
                            .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
                    );
                    apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

                    let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
                    let mut response = axum::response::Response::new(body);
                    *response.status_mut() = StatusCode::OK;
                    *response.headers_mut() = headers;
                    let mut usage = generated.usage;
                    usage.merge_total();
                    let tokens = usage
                        .total_tokens
                        .unwrap_or_else(|| u64::from(charge_tokens));
                    #[cfg(feature = "gateway-costing")]
                    let cost_usd_micros = model.as_deref().and_then(|model| {
                        state.proxy.pricing.as_ref().and_then(|pricing| {
                            let (Some(input), Some(output)) =
                                (usage.input_tokens, usage.output_tokens)
                            else {
                                return None;
                            };
                            pricing.estimate_cost_usd_micros_with_cache_for_service_tier(
                                model,
                                clamp_u64_to_u32(input),
                                usage.cache_input_tokens.map(clamp_u64_to_u32),
                                usage.cache_creation_input_tokens.map(clamp_u64_to_u32),
                                clamp_u64_to_u32(output),
                                service_tier.as_deref(),
                            )
                        })
                    });
                    #[cfg(not(feature = "gateway-costing"))]
                    let cost_usd_micros: Option<u64> = None;
                    Ok((
                        response,
                        ProxySpend {
                            tokens,
                            cost_usd_micros: cost_usd_micros.or(charge_cost_usd_micros),
                        },
                    ))
                }
            }
            // end inline: rest.rs
        }
    };
    // inlined from post.rs
    {
        #[cfg(feature = "gateway-metrics-prometheus")]
        if let Some(metrics) = state.proxy.metrics.as_ref() {
            let mut metrics = metrics.lock().await;
            metrics.record_proxy_backend_in_flight_dec(backend_name);
            metrics.observe_proxy_backend_request_duration(
                backend_name,
                backend_timer_start.elapsed(),
            );
        }

        let (response, spend) = match result {
            Ok((response, spend)) => (response, spend),
            Err(err) => {
                return Ok(BackendAttemptOutcome::Continue(Some(err)));
            }
        };

        let status = StatusCode::OK;
        let spend_tokens = true;
        let spent_tokens = spend.tokens;
        let spent_cost_usd_micros = spend.cost_usd_micros;
        #[cfg(not(any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-redis",
            feature = "gateway-costing",
        )))]
        let _ = spent_cost_usd_micros;

        #[cfg(feature = "gateway-metrics-prometheus")]
        if let Some(metrics) = state.proxy.metrics.as_ref() {
            let duration = metrics_timer_start.elapsed();
            let mut metrics = metrics.lock().await;
            if spend_tokens {
                metrics.record_proxy_backend_success(backend_name);
            } else {
                metrics.record_proxy_backend_failure(backend_name);
            }
            metrics.record_proxy_response_status_by_path(metrics_path, status.as_u16());
            metrics.record_proxy_response_status_by_backend(backend_name, status.as_u16());
            if let Some(model) = model.as_deref() {
                metrics.record_proxy_response_status_by_model(model, status.as_u16());
                metrics.observe_proxy_request_duration_by_model(model, duration);
            }
            metrics.observe_proxy_request_duration(metrics_path, duration);
        }

        #[cfg(any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        ))]
        if !token_budget_reservation_ids.is_empty() {
            settle_proxy_token_budget_reservations(
                state,
                token_budget_reservation_ids,
                spend_tokens,
                spent_tokens,
            )
            .await;
        } else if let (Some(virtual_key_id), Some(budget)) =
            (virtual_key_id.clone(), budget.clone())
        {
            if spend_tokens {
                state.spend_budget_tokens(&virtual_key_id, &budget, spent_tokens);
                if let Some((scope, budget)) = project_budget_scope.as_ref() {
                    state.spend_budget_tokens(scope, budget, spent_tokens);
                }
                if let Some((scope, budget)) = user_budget_scope.as_ref() {
                    state.spend_budget_tokens(scope, budget, spent_tokens);
                }

                #[cfg(feature = "gateway-costing")]
                if !use_persistent_budget {
                    if let Some(spent_cost_usd_micros) = spent_cost_usd_micros {
                        state.spend_budget_cost(&virtual_key_id, &budget, spent_cost_usd_micros);
                        if let Some((scope, budget)) = project_budget_scope.as_ref() {
                            state.spend_budget_cost(scope, budget, spent_cost_usd_micros);
                        }
                        if let Some((scope, budget)) = user_budget_scope.as_ref() {
                            state.spend_budget_cost(scope, budget, spent_cost_usd_micros);
                        }
                    }
                }
            }
        }
        #[cfg(not(any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        )))]
        if let (Some(virtual_key_id), Some(budget)) = (virtual_key_id.clone(), budget.clone()) {
            if spend_tokens {
                state.spend_budget_tokens(&virtual_key_id, &budget, spent_tokens);
                if let Some((scope, budget)) = project_budget_scope.as_ref() {
                    state.spend_budget_tokens(scope, budget, spent_tokens);
                }
                if let Some((scope, budget)) = user_budget_scope.as_ref() {
                    state.spend_budget_tokens(scope, budget, spent_tokens);
                }

                #[cfg(feature = "gateway-costing")]
                if let Some(spent_cost_usd_micros) = spent_cost_usd_micros {
                    state.spend_budget_cost(&virtual_key_id, &budget, spent_cost_usd_micros);
                    if let Some((scope, budget)) = project_budget_scope.as_ref() {
                        state.spend_budget_cost(scope, budget, spent_cost_usd_micros);
                    }
                    if let Some((scope, budget)) = user_budget_scope.as_ref() {
                        state.spend_budget_cost(scope, budget, spent_cost_usd_micros);
                    }
                }
            }
        }

        #[cfg(all(
            feature = "gateway-costing",
            any(
                feature = "gateway-store-sqlite",
                feature = "gateway-store-postgres",
                feature = "gateway-store-mysql",
                feature = "gateway-store-redis"
            ),
        ))]
        if !cost_budget_reservation_ids.is_empty() {
            settle_proxy_cost_budget_reservations(
                state,
                cost_budget_reservation_ids,
                spend_tokens,
                spent_cost_usd_micros.unwrap_or_default(),
            )
            .await;
        }

        #[cfg(all(
            feature = "gateway-costing",
            any(
                feature = "gateway-store-sqlite",
                feature = "gateway-store-postgres",
                feature = "gateway-store-mysql",
                feature = "gateway-store-redis"
            ),
        ))]
        if !_cost_budget_reserved && use_persistent_budget && spend_tokens {
            if let (Some(virtual_key_id), Some(spent_cost_usd_micros)) =
                (virtual_key_id.as_deref(), spent_cost_usd_micros)
            {
                #[cfg(feature = "gateway-store-sqlite")]
                if let Some(store) = state.stores.sqlite.as_ref() {
                    let _ = store
                        .record_spent_cost_usd_micros(virtual_key_id, spent_cost_usd_micros)
                        .await;
                }
                #[cfg(feature = "gateway-store-postgres")]
                if let Some(store) = state.stores.postgres.as_ref() {
                    let _ = store
                        .record_spent_cost_usd_micros(virtual_key_id, spent_cost_usd_micros)
                        .await;
                }
                #[cfg(feature = "gateway-store-mysql")]
                if let Some(store) = state.stores.mysql.as_ref() {
                    let _ = store
                        .record_spent_cost_usd_micros(virtual_key_id, spent_cost_usd_micros)
                        .await;
                }
                #[cfg(feature = "gateway-store-redis")]
                if let Some(store) = state.stores.redis.as_ref() {
                    let _ = store
                        .record_spent_cost_usd_micros(virtual_key_id, spent_cost_usd_micros)
                        .await;
                }
            }
        }

        #[cfg(any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        ))]
        {
            let payload = serde_json::json!({
                "request_id": &request_id,
                "virtual_key_id": virtual_key_id.as_deref(),
                "backend": &backend_name,
                "attempted_backends": &attempted_backends,
                "method": parts.method.as_str(),
                "path": path_and_query,
                "model": &model,
                "status": status.as_u16(),
                "charge_tokens": charge_tokens,
                "spent_tokens": spent_tokens,
                "charge_cost_usd_micros": charge_cost_usd_micros,
                "spent_cost_usd_micros": spent_cost_usd_micros,
                "body_len": body.len(),
                "mode": "translation",
            });
            append_audit_log(state, "proxy", payload).await;
        }

        emit_json_log(
            state,
            "proxy.response",
            serde_json::json!({
                "request_id": &request_id,
                "backend": &backend_name,
                "status": status.as_u16(),
                "attempted_backends": &attempted_backends,
                "mode": "translation",
            }),
        );

        #[cfg(feature = "sdk")]
        emit_devtools_log(
            state,
            "proxy.response",
            serde_json::json!({
                "request_id": &request_id,
                "status": status.as_u16(),
                "path": path_and_query,
                "backend": &backend_name,
                "mode": "translation",
            }),
        );

        Ok(BackendAttemptOutcome::Response(response))
    }
    // end inline: post.rs
}
// end inline: translation_backend/attempt.rs
// end inline: ../../http/translation_backend.rs
// inlined from ../../http/proxy_backend.rs
async fn attempt_proxy_backend(
    params: ProxyAttemptParams<'_>,
    backend_name: &str,
    idx: usize,
    attempted_backends: &[String],
) -> Result<BackendAttemptOutcome, (StatusCode, Json<OpenAiErrorResponse>)> {
    let backend_name = backend_name.to_string();
    let state = params.state;
    let parts = params.parts;
    let body = params.body;
    let parsed_json = params.parsed_json;
    let model = params.model;
    let protocol =
        extract_header(&parts.headers, "x-ditto-protocol").unwrap_or_else(|| "openai".to_string());
    let service_tier = params.service_tier;
    let request_id = params.request_id.to_string();
    let path_and_query = params.path_and_query;
    let _now_epoch_seconds = params.now_epoch_seconds;
    let charge_tokens = params.charge_tokens;
    let _stream_requested = params.stream_requested;
    let strip_authorization = params.strip_authorization;
    let use_persistent_budget = params.use_persistent_budget;
    let virtual_key_id = params.virtual_key_id;
    let budget = params.budget;
    let tenant_budget_scope = params.tenant_budget_scope;
    let project_budget_scope = params.project_budget_scope;
    let user_budget_scope = params.user_budget_scope;
    let charge_cost_usd_micros = params.charge_cost_usd_micros;
    let _cost_budget_reserved = params.cost_budget_reserved;

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    let token_budget_reservation_ids = params.token_budget_reservation_ids;

    #[cfg(all(
        feature = "gateway-costing",
        any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        )
    ))]
    let cost_budget_reservation_ids = params.cost_budget_reservation_ids;

    let max_attempts = params.max_attempts;
    #[cfg(feature = "gateway-routing-advanced")]
    let retry_config = params.retry_config;

    #[cfg(feature = "gateway-proxy-cache")]
    let proxy_cache_key = params.proxy_cache_key;
    #[cfg(feature = "gateway-proxy-cache")]
    let proxy_cache_metadata = params.proxy_cache_metadata;

    #[cfg(feature = "gateway-metrics-prometheus")]
    let metrics_path = params.metrics_path;
    #[cfg(feature = "gateway-metrics-prometheus")]
    let metrics_timer_start = params.metrics_timer_start;

    #[cfg(not(all(
        feature = "gateway-costing",
        any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        )
    )))]
    let _ = use_persistent_budget;
    #[cfg(not(feature = "gateway-routing-advanced"))]
    let _ = idx;
    #[cfg(not(feature = "gateway-routing-advanced"))]
    let _ = max_attempts;

    let backend = match state.backends.proxy_backends.get(&backend_name) {
        Some(backend) => backend.clone(),
        None => {
            return Ok(BackendAttemptOutcome::Continue(Some(openai_error(
                StatusCode::BAD_GATEWAY,
                "api_error",
                Some("backend_not_found"),
                format!("backend not found: {backend_name}"),
            ))));
        }
    };

    let mut proxy_permits = match try_acquire_proxy_permits(state, &backend_name)? {
        ProxyPermitOutcome::Acquired(permits) => permits,
        ProxyPermitOutcome::BackendRateLimited(err) => {
            return Ok(BackendAttemptOutcome::Continue(Some(err)));
        }
    };

    state.record_backend_call();
    let backend_model_map: BTreeMap<String, String> = state.backend_model_map(&backend_name);

    #[cfg(feature = "gateway-metrics-prometheus")]
    let backend_timer_start = Instant::now();

    #[cfg(feature = "gateway-metrics-prometheus")]
    if let Some(metrics) = state.proxy.metrics.as_ref() {
        let mut metrics = metrics.lock().await;
        metrics.record_proxy_backend_attempt(&backend_name);
        metrics.record_proxy_backend_in_flight_inc(&backend_name);
    }

    let mut outgoing_headers = parts.headers.clone();
    sanitize_proxy_headers(&mut outgoing_headers, strip_authorization);
    apply_backend_headers(&mut outgoing_headers, backend.headers());
    insert_request_id(&mut outgoing_headers, &request_id);

    let (outgoing_body, upstream_model) = if let (Some(request_model), Some(parsed_json)) =
        (model.as_deref(), parsed_json.as_ref())
    {
        let mapped_model = backend_model_map
            .get(request_model)
            .or_else(|| backend_model_map.get("*"))
            .cloned();

        match mapped_model {
            Some(mapped_model) => {
                let mut value = parsed_json.clone();
                if let Some(obj) = value.as_object_mut() {
                    obj.insert(
                        "model".to_string(),
                        serde_json::Value::String(mapped_model.clone()),
                    );
                    match serde_json::to_vec(&value) {
                        Ok(bytes) => (Bytes::from(bytes), Some(mapped_model)),
                        Err(_) => (body.clone(), Some(request_model.to_string())),
                    }
                } else {
                    (body.clone(), Some(request_model.to_string()))
                }
            }
            None => (body.clone(), Some(request_model.to_string())),
        }
    } else {
        (body.clone(), model.clone())
    };

    #[cfg(feature = "sdk")]
    emit_devtools_log(
        state,
        "proxy.request",
        serde_json::json!({
            "request_id": &request_id,
            "method": parts.method.as_str(),
            "path": path_and_query,
            "backend": &backend_name,
            "provider": &protocol,
            "model": &model,
            "upstream_model": upstream_model.as_deref(),
            "virtual_key_id": virtual_key_id.as_deref(),
            "body_len": body.len(),
        }),
    );

    let upstream_response = match backend
        .request(
            parts.method.clone(),
            path_and_query,
            outgoing_headers,
            Some(outgoing_body),
        )
        .await
    {
        Ok(response) => response,
        Err(err) => {
            #[cfg(feature = "gateway-metrics-prometheus")]
            if let Some(metrics) = state.proxy.metrics.as_ref() {
                let mut metrics = metrics.lock().await;
                metrics.record_proxy_backend_in_flight_dec(&backend_name);
                metrics.observe_proxy_backend_request_duration(
                    &backend_name,
                    backend_timer_start.elapsed(),
                );
            }
            #[cfg(feature = "gateway-metrics-prometheus")]
            if let Some(metrics) = state.proxy.metrics.as_ref() {
                metrics
                    .lock()
                    .await
                    .record_proxy_backend_failure(&backend_name);
            }
            #[cfg(feature = "gateway-routing-advanced")]
            let failure_kind = classify_proxy_backend_transport_failure(&err);
            #[cfg(feature = "gateway-routing-advanced")]
            let failure_message = err.to_string();
            let mapped = map_openai_gateway_error(err);
            #[cfg(feature = "gateway-routing-advanced")]
            {
                let decision = retry_config.decision_for_failure(failure_kind);
                record_proxy_backend_failure(
                    state,
                    &backend_name,
                    _now_epoch_seconds,
                    failure_kind,
                    failure_message,
                )
                .await;
                emit_proxy_backend_decision_logs(
                    state,
                    decision,
                    ProxyDecisionLogContext {
                        request_id: &request_id,
                        backend_name: &backend_name,
                        path_and_query,
                        attempted_backends,
                        idx,
                        max_attempts,
                        status_code: None,
                    },
                )
                .await;
                return Ok(if decision.should_attempt_next_backend(idx, max_attempts) {
                    BackendAttemptOutcome::Continue(Some(mapped))
                } else {
                    BackendAttemptOutcome::Stop(mapped)
                });
            }
            #[cfg(not(feature = "gateway-routing-advanced"))]
            return Ok(BackendAttemptOutcome::Continue(Some(mapped)));
        }
    };

    #[cfg(feature = "gateway-metrics-prometheus")]
    if let Some(metrics) = state.proxy.metrics.as_ref() {
        let mut metrics = metrics.lock().await;
        metrics.record_proxy_backend_in_flight_dec(&backend_name);
        metrics
            .observe_proxy_backend_request_duration(&backend_name, backend_timer_start.elapsed());
    }

    let status = upstream_response.status();

    if responses_shim::should_attempt_responses_shim(&parts.method, path_and_query, status) {
        if let Some(parsed_json) = parsed_json.as_ref() {
            let _ = proxy_permits.take();
            let Some(mut chat_body) =
                responses_shim::responses_request_to_chat_completions(parsed_json)
            else {
                return Ok(BackendAttemptOutcome::Continue(Some(openai_error(
                    StatusCode::BAD_GATEWAY,
                    "api_error",
                    Some("invalid_responses_request"),
                    "responses request cannot be mapped to chat/completions",
                ))));
            };

            if let Some(mapped_model) = chat_body
                .get("model")
                .and_then(|value| value.as_str())
                .and_then(|model| {
                    backend_model_map
                        .get(model)
                        .or_else(|| backend_model_map.get("*"))
                })
                .cloned()
            {
                if let Some(obj) = chat_body.as_object_mut() {
                    obj.insert("model".to_string(), serde_json::Value::String(mapped_model));
                }
            }

            emit_json_log(
                state,
                "proxy.responses_shim",
                serde_json::json!({
                    "request_id": &request_id,
                    "backend": &backend_name,
                    "path": path_and_query,
                    "shim": "responses_via_chat_completions",
                }),
            );

            #[cfg(feature = "sdk")]
            emit_devtools_log(
                state,
                "proxy.responses_shim",
                serde_json::json!({
                    "request_id": &request_id,
                    "backend": &backend_name,
                    "path": path_and_query,
                }),
            );

            let chat_body_bytes = match serde_json::to_vec(&chat_body) {
                Ok(bytes) => Bytes::from(bytes),
                Err(err) => {
                    return Ok(BackendAttemptOutcome::Continue(Some(openai_error(
                        StatusCode::BAD_GATEWAY,
                        "api_error",
                        Some("invalid_responses_request"),
                        format!("failed to serialize shim chat/completions request: {err}"),
                    ))));
                }
            };

            let mut shim_headers = parts.headers.clone();
            sanitize_proxy_headers(&mut shim_headers, strip_authorization);
            apply_backend_headers(&mut shim_headers, backend.headers());
            insert_request_id(&mut shim_headers, &request_id);
            if _stream_requested {
                shim_headers.insert(
                    axum::http::header::ACCEPT,
                    axum::http::HeaderValue::from_static("text/event-stream"),
                );
            }

            let shim_permits = match try_acquire_proxy_permits(state, &backend_name)? {
                ProxyPermitOutcome::Acquired(permits) => permits,
                ProxyPermitOutcome::BackendRateLimited(err) => {
                    return Ok(BackendAttemptOutcome::Continue(Some(err)));
                }
            };
            #[cfg(feature = "gateway-metrics-prometheus")]
            let shim_timer_start = Instant::now();

            #[cfg(feature = "gateway-metrics-prometheus")]
            if let Some(metrics) = state.proxy.metrics.as_ref() {
                let mut metrics = metrics.lock().await;
                metrics.record_proxy_backend_attempt(&backend_name);
                metrics.record_proxy_backend_in_flight_inc(&backend_name);
            }

            let shim_response = match backend
                .request(
                    parts.method.clone(),
                    "/v1/chat/completions",
                    shim_headers,
                    Some(chat_body_bytes),
                )
                .await
            {
                Ok(response) => response,
                Err(err) => {
                    #[cfg(feature = "gateway-metrics-prometheus")]
                    if let Some(metrics) = state.proxy.metrics.as_ref() {
                        let mut metrics = metrics.lock().await;
                        metrics.record_proxy_backend_in_flight_dec(&backend_name);
                        metrics.observe_proxy_backend_request_duration(
                            &backend_name,
                            shim_timer_start.elapsed(),
                        );
                        metrics.record_proxy_backend_failure(&backend_name);
                    }
                    #[cfg(feature = "gateway-routing-advanced")]
                    let failure_kind = classify_proxy_backend_transport_failure(&err);
                    #[cfg(feature = "gateway-routing-advanced")]
                    let failure_message = err.to_string();
                    let mapped = map_openai_gateway_error(err);
                    #[cfg(feature = "gateway-routing-advanced")]
                    {
                        let decision = retry_config.decision_for_failure(failure_kind);
                        record_proxy_backend_failure(
                            state,
                            &backend_name,
                            _now_epoch_seconds,
                            failure_kind,
                            failure_message,
                        )
                        .await;
                        emit_proxy_backend_decision_logs(
                            state,
                            decision,
                            ProxyDecisionLogContext {
                                request_id: &request_id,
                                backend_name: &backend_name,
                                path_and_query,
                                attempted_backends,
                                idx,
                                max_attempts,
                                status_code: None,
                            },
                        )
                        .await;
                        return Ok(if decision.should_attempt_next_backend(idx, max_attempts) {
                            BackendAttemptOutcome::Continue(Some(mapped))
                        } else {
                            BackendAttemptOutcome::Stop(mapped)
                        });
                    }
                    #[cfg(not(feature = "gateway-routing-advanced"))]
                    return Ok(BackendAttemptOutcome::Continue(Some(mapped)));
                }
            };

            #[cfg(feature = "gateway-metrics-prometheus")]
            if let Some(metrics) = state.proxy.metrics.as_ref() {
                let mut metrics = metrics.lock().await;
                metrics.record_proxy_backend_in_flight_dec(&backend_name);
                metrics.observe_proxy_backend_request_duration(
                    &backend_name,
                    shim_timer_start.elapsed(),
                );
            }

            let status = shim_response.status();

            #[cfg(feature = "gateway-routing-advanced")]
            let status_code = status.as_u16();
            #[cfg(feature = "gateway-routing-advanced")]
            let failure_kind = FailureKind::Status(status_code);
            #[cfg(feature = "gateway-routing-advanced")]
            let decision = retry_config.decision_for_failure(failure_kind);
            #[cfg(feature = "gateway-routing-advanced")]
            let should_record_status_failure =
                should_record_proxy_status_failure(state, retry_config, failure_kind, status);

            #[cfg(feature = "gateway-routing-advanced")]
            if should_record_status_failure {
                record_proxy_backend_failure(
                    state,
                    &backend_name,
                    _now_epoch_seconds,
                    failure_kind,
                    format!("status {}", status_code),
                )
                .await;
                emit_proxy_backend_decision_logs(
                    state,
                    decision,
                    ProxyDecisionLogContext {
                        request_id: &request_id,
                        backend_name: &backend_name,
                        path_and_query,
                        attempted_backends,
                        idx,
                        max_attempts,
                        status_code: Some(status_code),
                    },
                )
                .await;
            } else {
                record_proxy_backend_success(state, &backend_name).await;
            }

            #[cfg(feature = "gateway-routing-advanced")]
            if decision.should_attempt_next_backend(idx, max_attempts) {
                return Ok(BackendAttemptOutcome::Continue(Some(
                    openai_status_routing_error(status, decision),
                )));
            }

            let spend_tokens = status.is_success();

            #[cfg(feature = "gateway-metrics-prometheus")]
            if let Some(metrics) = state.proxy.metrics.as_ref() {
                let is_failure_status = {
                    #[cfg(feature = "gateway-routing-advanced")]
                    {
                        should_record_status_failure
                    }
                    #[cfg(not(feature = "gateway-routing-advanced"))]
                    {
                        status.is_server_error()
                    }
                };
                let duration = metrics_timer_start.elapsed();
                let mut metrics = metrics.lock().await;
                if is_failure_status {
                    metrics.record_proxy_backend_failure(&backend_name);
                } else {
                    metrics.record_proxy_backend_success(&backend_name);
                }
                metrics.record_proxy_response_status_by_path(metrics_path, status.as_u16());
                metrics.record_proxy_response_status_by_backend(&backend_name, status.as_u16());
                if let Some(model) = model.as_deref() {
                    metrics.record_proxy_response_status_by_model(model, status.as_u16());
                    metrics.observe_proxy_request_duration_by_model(model, duration);
                }
                metrics.observe_proxy_request_duration(metrics_path, duration);
            }

            #[cfg(any(
                feature = "gateway-store-sqlite",
                feature = "gateway-store-postgres",
                feature = "gateway-store-mysql",
                feature = "gateway-store-redis"
            ))]
            if !token_budget_reservation_ids.is_empty() {
                settle_proxy_token_budget_reservations(
                    state,
                    token_budget_reservation_ids,
                    spend_tokens,
                    u64::MAX,
                )
                .await;
            } else if let (Some(virtual_key_id), Some(budget)) =
                (virtual_key_id.clone(), budget.clone())
            {
                if spend_tokens {
                    state.spend_budget_tokens(&virtual_key_id, &budget, u64::from(charge_tokens));
                    if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                        state.spend_budget_tokens(scope, budget, u64::from(charge_tokens));
                    }
                    if let Some((scope, budget)) = project_budget_scope.as_ref() {
                        state.spend_budget_tokens(scope, budget, u64::from(charge_tokens));
                    }
                    if let Some((scope, budget)) = user_budget_scope.as_ref() {
                        state.spend_budget_tokens(scope, budget, u64::from(charge_tokens));
                    }

                    #[cfg(feature = "gateway-costing")]
                    if !use_persistent_budget {
                        if let Some(charge_cost_usd_micros) = charge_cost_usd_micros {
                            state.spend_budget_cost(
                                &virtual_key_id,
                                &budget,
                                charge_cost_usd_micros,
                            );
                            if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                                state.spend_budget_cost(scope, budget, charge_cost_usd_micros);
                            }
                            if let Some((scope, budget)) = project_budget_scope.as_ref() {
                                state.spend_budget_cost(scope, budget, charge_cost_usd_micros);
                            }
                            if let Some((scope, budget)) = user_budget_scope.as_ref() {
                                state.spend_budget_cost(scope, budget, charge_cost_usd_micros);
                            }
                        }
                    }
                }
            }
            #[cfg(not(any(
                feature = "gateway-store-sqlite",
                feature = "gateway-store-postgres",
                feature = "gateway-store-mysql",
                feature = "gateway-store-redis"
            )))]
            if let (Some(virtual_key_id), Some(budget)) = (virtual_key_id.clone(), budget.clone()) {
                if spend_tokens {
                    state.spend_budget_tokens(&virtual_key_id, &budget, u64::from(charge_tokens));
                    if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                        state.spend_budget_tokens(scope, budget, u64::from(charge_tokens));
                    }
                    if let Some((scope, budget)) = project_budget_scope.as_ref() {
                        state.spend_budget_tokens(scope, budget, u64::from(charge_tokens));
                    }
                    if let Some((scope, budget)) = user_budget_scope.as_ref() {
                        state.spend_budget_tokens(scope, budget, u64::from(charge_tokens));
                    }

                    #[cfg(feature = "gateway-costing")]
                    if let Some(charge_cost_usd_micros) = charge_cost_usd_micros {
                        state.spend_budget_cost(&virtual_key_id, &budget, charge_cost_usd_micros);
                        if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                            state.spend_budget_cost(scope, budget, charge_cost_usd_micros);
                        }
                        if let Some((scope, budget)) = project_budget_scope.as_ref() {
                            state.spend_budget_cost(scope, budget, charge_cost_usd_micros);
                        }
                        if let Some((scope, budget)) = user_budget_scope.as_ref() {
                            state.spend_budget_cost(scope, budget, charge_cost_usd_micros);
                        }
                    }
                }
            }

            #[cfg(all(
                feature = "gateway-costing",
                any(
                    feature = "gateway-store-sqlite",
                    feature = "gateway-store-postgres",
                    feature = "gateway-store-mysql",
                    feature = "gateway-store-redis"
                )
            ))]
            if !cost_budget_reservation_ids.is_empty() {
                settle_proxy_cost_budget_reservations(
                    state,
                    cost_budget_reservation_ids,
                    spend_tokens,
                    u64::MAX,
                )
                .await;
            }

            #[cfg(all(
                feature = "gateway-costing",
                any(
                    feature = "gateway-store-sqlite",
                    feature = "gateway-store-postgres",
                    feature = "gateway-store-mysql",
                    feature = "gateway-store-redis"
                )
            ))]
            if !_cost_budget_reserved && use_persistent_budget && spend_tokens {
                if let (Some(virtual_key_id), Some(charge_cost_usd_micros)) =
                    (virtual_key_id.as_deref(), charge_cost_usd_micros)
                {
                    #[cfg(feature = "gateway-store-sqlite")]
                    if let Some(store) = state.stores.sqlite.as_ref() {
                        let _ = store
                            .record_spent_cost_usd_micros(virtual_key_id, charge_cost_usd_micros)
                            .await;
                    }
                    #[cfg(feature = "gateway-store-postgres")]
                    if let Some(store) = state.stores.postgres.as_ref() {
                        let _ = store
                            .record_spent_cost_usd_micros(virtual_key_id, charge_cost_usd_micros)
                            .await;
                    }
                    #[cfg(feature = "gateway-store-mysql")]
                    if let Some(store) = state.stores.mysql.as_ref() {
                        let _ = store
                            .record_spent_cost_usd_micros(virtual_key_id, charge_cost_usd_micros)
                            .await;
                    }
                    #[cfg(feature = "gateway-store-redis")]
                    if let Some(store) = state.stores.redis.as_ref() {
                        let _ = store
                            .record_spent_cost_usd_micros(virtual_key_id, charge_cost_usd_micros)
                            .await;
                    }
                }
            }

            #[cfg(any(
                feature = "gateway-store-sqlite",
                feature = "gateway-store-postgres",
                feature = "gateway-store-mysql",
                feature = "gateway-store-redis"
            ))]
            {
                let payload = serde_json::json!({
                    "request_id": &request_id,
                    "provider": &protocol,
                    "virtual_key_id": virtual_key_id.as_deref(),
                    "backend": &backend_name,
                    "attempted_backends": &attempted_backends,
                    "method": parts.method.as_str(),
                    "path": path_and_query,
                    "model": &model,
                    "upstream_model": upstream_model.as_deref(),
                    "status": status.as_u16(),
                    "charge_tokens": charge_tokens,
                    "charge_cost_usd_micros": charge_cost_usd_micros,
                    "body_len": body.len(),
                    "shim": "responses_via_chat_completions",
                });

                append_audit_log(state, "proxy", payload).await;
            }

            emit_json_log(
                state,
                "proxy.response",
                serde_json::json!({
                    "request_id": &request_id,
                    "provider": &protocol,
                    "backend": &backend_name,
                    "status": status.as_u16(),
                    "attempted_backends": &attempted_backends,
                    "model": &model,
                    "upstream_model": upstream_model.as_deref(),
                }),
            );

            #[cfg(feature = "sdk")]
            emit_devtools_log(
                state,
                "proxy.response",
                serde_json::json!({
                    "request_id": &request_id,
                    "status": status.as_u16(),
                    "path": path_and_query,
                    "backend": &backend_name,
                }),
            );

            #[cfg(feature = "gateway-otel")]
            {
                tracing::Span::current().record("cache", tracing::field::display("miss"));
                tracing::Span::current().record("backend", tracing::field::display(&backend_name));
                tracing::Span::current().record("status", tracing::field::display(status.as_u16()));
            }

            if status.is_success() {
                match responses_shim_response(
                    ProxyResponseContext {
                        state,
                        backend: &backend_name,
                        request_id: &request_id,
                        #[cfg(feature = "gateway-metrics-prometheus")]
                        metrics_path,
                        cache_key: {
                            #[cfg(feature = "gateway-proxy-cache")]
                            {
                                proxy_cache_key.as_deref()
                            }
                            #[cfg(not(feature = "gateway-proxy-cache"))]
                            {
                                None
                            }
                        },
                        #[cfg(feature = "gateway-proxy-cache")]
                        cache_metadata: proxy_cache_metadata.as_ref(),
                    },
                    shim_response,
                    shim_permits,
                )
                .await
                {
                    Ok(response) => return Ok(BackendAttemptOutcome::Response(response)),
                    Err(err) => {
                        return Ok(BackendAttemptOutcome::Continue(Some(err)));
                    }
                }
            } else {
                return Ok(BackendAttemptOutcome::Response(
                    proxy_response(
                        ProxyResponseContext {
                            state,
                            backend: &backend_name,
                            request_id: &request_id,
                            #[cfg(feature = "gateway-metrics-prometheus")]
                            metrics_path,
                            cache_key: {
                                #[cfg(feature = "gateway-proxy-cache")]
                                {
                                    proxy_cache_key.as_deref()
                                }
                                #[cfg(not(feature = "gateway-proxy-cache"))]
                                {
                                    None
                                }
                            },
                            #[cfg(feature = "gateway-proxy-cache")]
                            cache_metadata: proxy_cache_metadata.as_ref(),
                        },
                        shim_response,
                        shim_permits,
                    )
                    .await,
                ));
            }
        }
    }

    #[cfg(feature = "gateway-routing-advanced")]
    let status_code = status.as_u16();
    #[cfg(feature = "gateway-routing-advanced")]
    let failure_kind = FailureKind::Status(status_code);
    #[cfg(feature = "gateway-routing-advanced")]
    let decision = retry_config.decision_for_failure(failure_kind);
    #[cfg(feature = "gateway-routing-advanced")]
    let should_record_status_failure =
        should_record_proxy_status_failure(state, retry_config, failure_kind, status);

    #[cfg(feature = "gateway-routing-advanced")]
    if should_record_status_failure {
        record_proxy_backend_failure(
            state,
            &backend_name,
            _now_epoch_seconds,
            failure_kind,
            format!("status {}", status_code),
        )
        .await;
        emit_proxy_backend_decision_logs(
            state,
            decision,
            ProxyDecisionLogContext {
                request_id: &request_id,
                backend_name: &backend_name,
                path_and_query,
                attempted_backends,
                idx,
                max_attempts,
                status_code: Some(status_code),
            },
        )
        .await;
    } else {
        record_proxy_backend_success(state, &backend_name).await;
    }

    #[cfg(feature = "gateway-routing-advanced")]
    if decision.should_attempt_next_backend(idx, max_attempts) {
        return Ok(BackendAttemptOutcome::Continue(Some(
            openai_status_routing_error(status, decision),
        )));
    }

    let spend_tokens = status.is_success();

    #[cfg(feature = "gateway-metrics-prometheus")]
    if let Some(metrics) = state.proxy.metrics.as_ref() {
        let is_failure_status = {
            #[cfg(feature = "gateway-routing-advanced")]
            {
                should_record_status_failure
            }
            #[cfg(not(feature = "gateway-routing-advanced"))]
            {
                status.is_server_error()
            }
        };
        let duration = metrics_timer_start.elapsed();
        let mut metrics = metrics.lock().await;
        if is_failure_status {
            metrics.record_proxy_backend_failure(&backend_name);
        } else {
            metrics.record_proxy_backend_success(&backend_name);
        }
        metrics.record_proxy_response_status_by_path(metrics_path, status.as_u16());
        metrics.record_proxy_response_status_by_backend(&backend_name, status.as_u16());
        if let Some(model) = model.as_deref() {
            metrics.record_proxy_response_status_by_model(model, status.as_u16());
            metrics.observe_proxy_request_duration_by_model(model, duration);
        }
        metrics.observe_proxy_request_duration(metrics_path, duration);
    }

    let upstream_headers = upstream_response.headers().clone();
    let content_type = upstream_headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let is_event_stream = content_type.starts_with("text/event-stream");

    if is_event_stream {
        // inlined from proxy_backend/stream.rs
        {
            const SSE_USAGE_TRACKER_MAX_BUFFER_BYTES: usize = 512 * 1024;
            const SSE_USAGE_TRACKER_TAIL_BYTES: usize = 128 * 1024;
            const PROXY_SSE_ABORT_FINALIZER_WORKERS: usize = 2;
            const PROXY_SSE_ABORT_FINALIZER_QUEUE_CAPACITY: usize = 1024;

            #[derive(Default)]
            struct SseUsageTracker {
                buffer: bytes::BytesMut,
                observed_usage: Option<ObservedUsage>,
            }

            impl SseUsageTracker {
                fn ingest(&mut self, chunk: &Bytes) {
                    self.buffer.extend_from_slice(chunk.as_ref());

                    loop {
                        let Some((pos, delimiter_len)) = find_sse_delimiter(self.buffer.as_ref())
                        else {
                            break;
                        };

                        let event_bytes = self.buffer.split_to(pos);
                        let _ = self.buffer.split_to(delimiter_len);

                        let Some(data) = extract_sse_data(event_bytes.as_ref()) else {
                            continue;
                        };
                        let trimmed = trim_ascii_whitespace(&data);
                        if trimmed == b"[DONE]" {
                            continue;
                        }

                        if trimmed.starts_with(b"{") {
                            if let Some(usage) = extract_openai_usage_from_slice(trimmed) {
                                self.observed_usage = Some(usage);
                            }
                        }
                    }

                    if self.buffer.len() > SSE_USAGE_TRACKER_MAX_BUFFER_BYTES {
                        let keep_from = self
                            .buffer
                            .len()
                            .saturating_sub(SSE_USAGE_TRACKER_TAIL_BYTES);
                        self.buffer = self.buffer.split_off(keep_from);
                    }
                }

                fn observed_usage(&self) -> Option<ObservedUsage> {
                    self.observed_usage
                }
            }

            fn find_sse_delimiter(buf: &[u8]) -> Option<(usize, usize)> {
                if buf.len() < 2 {
                    return None;
                }

                // Use a single forward scan so mixed newline styles still split at the earliest
                // event boundary instead of whichever delimiter pattern we searched first.
                let mut idx = 0usize;
                while idx + 1 < buf.len() {
                    if buf[idx] == b'\n' && buf[idx + 1] == b'\n' {
                        return Some((idx, 2));
                    }
                    if idx + 3 < buf.len()
                        && buf[idx] == b'\r'
                        && buf[idx + 1] == b'\n'
                        && buf[idx + 2] == b'\r'
                        && buf[idx + 3] == b'\n'
                    {
                        return Some((idx, 4));
                    }
                    idx += 1;
                }

                None
            }

            fn extract_sse_data(event: &[u8]) -> Option<Vec<u8>> {
                let mut out = Vec::<u8>::new();
                for line in event.split(|b| *b == b'\n') {
                    let line = line.strip_suffix(b"\r").unwrap_or(line);
                    let Some(rest) = line.strip_prefix(b"data:") else {
                        continue;
                    };
                    let rest = trim_ascii_whitespace(rest);
                    if rest.is_empty() {
                        continue;
                    }
                    if !out.is_empty() {
                        out.push(b'\n');
                    }
                    out.extend_from_slice(rest);
                }
                (!out.is_empty()).then_some(out)
            }

            fn trim_ascii_whitespace(bytes: &[u8]) -> &[u8] {
                let start = bytes
                    .iter()
                    .position(|b| !b.is_ascii_whitespace())
                    .unwrap_or(bytes.len());
                let end = bytes
                    .iter()
                    .rposition(|b| !b.is_ascii_whitespace())
                    .map(|pos| pos + 1)
                    .unwrap_or(start);
                &bytes[start..end]
            }

            #[derive(Clone, Copy, Debug)]
            enum StreamEnd {
                Completed,
                Error,
                Aborted,
            }

            struct ProxySseFinalizer {
                state: GatewayHttpState,
                backend_name: String,
                attempted_backends: Vec<String>,
                request_id: String,
                provider: String,
                #[cfg(any(
                    feature = "gateway-store-sqlite",
                    feature = "gateway-store-postgres",
                    feature = "gateway-store-mysql",
                    feature = "gateway-store-redis",
                    feature = "sdk"
                ))]
                method: String,
                #[cfg(any(
                    feature = "gateway-store-sqlite",
                    feature = "gateway-store-postgres",
                    feature = "gateway-store-mysql",
                    feature = "gateway-store-redis",
                    feature = "sdk"
                ))]
                path_and_query: String,
                #[cfg(feature = "gateway-metrics-prometheus")]
                metrics_path: String,
                model: Option<String>,
                upstream_model: Option<String>,
                service_tier: Option<String>,
                backend_model_map: BTreeMap<String, String>,
                status: u16,
                charge_tokens: u32,
                charge_cost_usd_micros: Option<u64>,
                spend_tokens: bool,
                #[cfg(any(
                    feature = "gateway-store-sqlite",
                    feature = "gateway-store-postgres",
                    feature = "gateway-store-mysql",
                    feature = "gateway-store-redis"
                ))]
                use_persistent_budget: bool,
                virtual_key_id: Option<String>,
                budget: Option<super::BudgetConfig>,
                tenant_budget_scope: Option<(String, super::BudgetConfig)>,
                project_budget_scope: Option<(String, super::BudgetConfig)>,
                user_budget_scope: Option<(String, super::BudgetConfig)>,
                #[cfg(any(
                    feature = "gateway-store-sqlite",
                    feature = "gateway-store-postgres",
                    feature = "gateway-store-mysql",
                    feature = "gateway-store-redis"
                ))]
                token_budget_reservation_ids: Vec<String>,
                #[cfg(all(
                    feature = "gateway-costing",
                    any(
                        feature = "gateway-store-sqlite",
                        feature = "gateway-store-postgres",
                        feature = "gateway-store-mysql",
                        feature = "gateway-store-redis"
                    )
                ))]
                cost_budget_reserved: bool,
                #[cfg(all(
                    feature = "gateway-costing",
                    any(
                        feature = "gateway-store-sqlite",
                        feature = "gateway-store-postgres",
                        feature = "gateway-store-mysql",
                        feature = "gateway-store-redis"
                    )
                ))]
                cost_budget_reservation_ids: Vec<String>,
                #[cfg(any(
                    feature = "gateway-store-sqlite",
                    feature = "gateway-store-postgres",
                    feature = "gateway-store-mysql",
                    feature = "gateway-store-redis",
                    feature = "sdk"
                ))]
                request_body_len: usize,
            }

            impl ProxySseFinalizer {
                async fn finalize(
                    self,
                    observed_usage: Option<ObservedUsage>,
                    end: StreamEnd,
                    stream_bytes: u64,
                ) {
                    #[cfg(not(feature = "gateway-metrics-prometheus"))]
                    let _ = (&end, stream_bytes);

                    #[cfg(feature = "gateway-metrics-prometheus")]
                    if let Some(metrics) = self.state.proxy.metrics.as_ref() {
                        let mut metrics = metrics.lock().await;
                        metrics.record_proxy_stream_close(&self.backend_name, &self.metrics_path);
                        metrics.record_proxy_stream_bytes(
                            &self.backend_name,
                            &self.metrics_path,
                            stream_bytes,
                        );
                        match end {
                            StreamEnd::Completed => {
                                metrics.record_proxy_stream_completed(
                                    &self.backend_name,
                                    &self.metrics_path,
                                );
                            }
                            StreamEnd::Error => {
                                metrics.record_proxy_stream_error(
                                    &self.backend_name,
                                    &self.metrics_path,
                                );
                            }
                            StreamEnd::Aborted => {
                                metrics.record_proxy_stream_aborted(
                                    &self.backend_name,
                                    &self.metrics_path,
                                );
                            }
                        }
                    }

                    let spent_tokens = if self.spend_tokens {
                        observed_usage
                            .and_then(|usage| usage.total_tokens)
                            .unwrap_or_else(|| u64::from(self.charge_tokens))
                    } else {
                        0
                    };

                    #[cfg(feature = "gateway-costing")]
                    let spent_cost_usd_micros = if self.spend_tokens {
                        self.model
                            .as_deref()
                            .map(|request_model| {
                                self.backend_model_map
                                    .get(request_model)
                                    .map(|model| model.as_str())
                                    .unwrap_or(request_model)
                            })
                            .and_then(|cost_model| {
                                self.state.proxy.pricing.as_ref().and_then(|pricing| {
                                    let usage = observed_usage?;
                                    let input = usage.input_tokens?;
                                    let output = usage.output_tokens?;
                                    pricing.estimate_cost_usd_micros_with_cache_for_service_tier(
                                        cost_model,
                                        clamp_u64_to_u32(input),
                                        usage.cache_input_tokens.map(clamp_u64_to_u32),
                                        usage.cache_creation_input_tokens.map(clamp_u64_to_u32),
                                        clamp_u64_to_u32(output),
                                        self.service_tier.as_deref(),
                                    )
                                })
                            })
                            .or(self.charge_cost_usd_micros)
                    } else {
                        None
                    };
                    #[cfg(not(feature = "gateway-costing"))]
                    let spent_cost_usd_micros: Option<u64> = None;

                    #[cfg(not(feature = "gateway-costing"))]
                    let _ = (
                        spent_cost_usd_micros,
                        &self.model,
                        &self.service_tier,
                        &self.backend_model_map,
                        self.charge_cost_usd_micros,
                    );

                    #[cfg(any(
                        feature = "gateway-store-sqlite",
                        feature = "gateway-store-postgres",
                        feature = "gateway-store-mysql",
                        feature = "gateway-store-redis",
                        feature = "sdk",
                    ))]
                    let _ = (&self.method, &self.path_and_query, self.request_body_len);

                    #[cfg(any(
                        feature = "gateway-store-sqlite",
                        feature = "gateway-store-postgres",
                        feature = "gateway-store-mysql",
                        feature = "gateway-store-redis"
                    ))]
                    let _ = (
                        self.use_persistent_budget,
                        &self.token_budget_reservation_ids,
                    );

                    #[cfg(all(
                        feature = "gateway-costing",
                        any(
                            feature = "gateway-store-sqlite",
                            feature = "gateway-store-postgres",
                            feature = "gateway-store-mysql",
                            feature = "gateway-store-redis"
                        )
                    ))]
                    let _ = (self.cost_budget_reserved, &self.cost_budget_reservation_ids);

                    #[cfg(any(
                        feature = "gateway-store-sqlite",
                        feature = "gateway-store-postgres",
                        feature = "gateway-store-mysql",
                        feature = "gateway-store-redis"
                    ))]
                    if !self.token_budget_reservation_ids.is_empty() {
                        settle_proxy_token_budget_reservations(
                            &self.state,
                            &self.token_budget_reservation_ids,
                            self.spend_tokens,
                            spent_tokens,
                        )
                        .await;
                    } else if let (Some(virtual_key_id), Some(budget)) =
                        (self.virtual_key_id.clone(), self.budget.clone())
                    {
                        if self.spend_tokens {
                            self.state
                                .spend_budget_tokens(&virtual_key_id, &budget, spent_tokens);
                            if let Some((scope, budget)) = self.tenant_budget_scope.as_ref() {
                                self.state.spend_budget_tokens(scope, budget, spent_tokens);
                            }
                            if let Some((scope, budget)) = self.project_budget_scope.as_ref() {
                                self.state.spend_budget_tokens(scope, budget, spent_tokens);
                            }
                            if let Some((scope, budget)) = self.user_budget_scope.as_ref() {
                                self.state.spend_budget_tokens(scope, budget, spent_tokens);
                            }

                            #[cfg(feature = "gateway-costing")]
                            if !self.use_persistent_budget {
                                if let Some(spent_cost_usd_micros) = spent_cost_usd_micros {
                                    self.state.spend_budget_cost(
                                        &virtual_key_id,
                                        &budget,
                                        spent_cost_usd_micros,
                                    );
                                    if let Some((scope, budget)) = self.tenant_budget_scope.as_ref()
                                    {
                                        self.state.spend_budget_cost(
                                            scope,
                                            budget,
                                            spent_cost_usd_micros,
                                        );
                                    }
                                    if let Some((scope, budget)) =
                                        self.project_budget_scope.as_ref()
                                    {
                                        self.state.spend_budget_cost(
                                            scope,
                                            budget,
                                            spent_cost_usd_micros,
                                        );
                                    }
                                    if let Some((scope, budget)) = self.user_budget_scope.as_ref() {
                                        self.state.spend_budget_cost(
                                            scope,
                                            budget,
                                            spent_cost_usd_micros,
                                        );
                                    }
                                }
                            }
                        }
                    }

                    #[cfg(not(any(
                        feature = "gateway-store-sqlite",
                        feature = "gateway-store-postgres",
                        feature = "gateway-store-mysql",
                        feature = "gateway-store-redis"
                    )))]
                    if let (Some(virtual_key_id), Some(budget)) =
                        (self.virtual_key_id.clone(), self.budget.clone())
                    {
                        if self.spend_tokens {
                            self.state
                                .spend_budget_tokens(&virtual_key_id, &budget, spent_tokens);
                            if let Some((scope, budget)) = self.tenant_budget_scope.as_ref() {
                                self.state.spend_budget_tokens(scope, budget, spent_tokens);
                            }
                            if let Some((scope, budget)) = self.project_budget_scope.as_ref() {
                                self.state.spend_budget_tokens(scope, budget, spent_tokens);
                            }
                            if let Some((scope, budget)) = self.user_budget_scope.as_ref() {
                                self.state.spend_budget_tokens(scope, budget, spent_tokens);
                            }

                            #[cfg(feature = "gateway-costing")]
                            if let Some(spent_cost_usd_micros) = spent_cost_usd_micros {
                                self.state.spend_budget_cost(
                                    &virtual_key_id,
                                    &budget,
                                    spent_cost_usd_micros,
                                );
                                if let Some((scope, budget)) = self.tenant_budget_scope.as_ref() {
                                    self.state.spend_budget_cost(
                                        scope,
                                        budget,
                                        spent_cost_usd_micros,
                                    );
                                }
                                if let Some((scope, budget)) = self.project_budget_scope.as_ref() {
                                    self.state.spend_budget_cost(
                                        scope,
                                        budget,
                                        spent_cost_usd_micros,
                                    );
                                }
                                if let Some((scope, budget)) = self.user_budget_scope.as_ref() {
                                    self.state.spend_budget_cost(
                                        scope,
                                        budget,
                                        spent_cost_usd_micros,
                                    );
                                }
                            }
                        }
                    }

                    #[cfg(all(
                        feature = "gateway-costing",
                        any(
                            feature = "gateway-store-sqlite",
                            feature = "gateway-store-postgres",
                            feature = "gateway-store-mysql",
                            feature = "gateway-store-redis"
                        ),
                    ))]
                    if !self.cost_budget_reservation_ids.is_empty() {
                        settle_proxy_cost_budget_reservations(
                            &self.state,
                            &self.cost_budget_reservation_ids,
                            self.spend_tokens,
                            spent_cost_usd_micros.unwrap_or_default(),
                        )
                        .await;
                    }

                    #[cfg(all(
                        feature = "gateway-costing",
                        any(
                            feature = "gateway-store-sqlite",
                            feature = "gateway-store-postgres",
                            feature = "gateway-store-mysql",
                            feature = "gateway-store-redis"
                        ),
                    ))]
                    if !self.cost_budget_reserved && self.use_persistent_budget && self.spend_tokens
                    {
                        if let (Some(virtual_key_id), Some(spent_cost_usd_micros)) =
                            (self.virtual_key_id.as_deref(), spent_cost_usd_micros)
                        {
                            #[cfg(feature = "gateway-store-sqlite")]
                            if let Some(store) = self.state.stores.sqlite.as_ref() {
                                let _ = store
                                    .record_spent_cost_usd_micros(
                                        virtual_key_id,
                                        spent_cost_usd_micros,
                                    )
                                    .await;
                            }
                            #[cfg(feature = "gateway-store-postgres")]
                            if let Some(store) = self.state.stores.postgres.as_ref() {
                                let _ = store
                                    .record_spent_cost_usd_micros(
                                        virtual_key_id,
                                        spent_cost_usd_micros,
                                    )
                                    .await;
                            }
                            #[cfg(feature = "gateway-store-mysql")]
                            if let Some(store) = self.state.stores.mysql.as_ref() {
                                let _ = store
                                    .record_spent_cost_usd_micros(
                                        virtual_key_id,
                                        spent_cost_usd_micros,
                                    )
                                    .await;
                            }
                            #[cfg(feature = "gateway-store-redis")]
                            if let Some(store) = self.state.stores.redis.as_ref() {
                                let _ = store
                                    .record_spent_cost_usd_micros(
                                        virtual_key_id,
                                        spent_cost_usd_micros,
                                    )
                                    .await;
                            }
                        }
                    }

                    #[cfg(any(
                        feature = "gateway-store-sqlite",
                        feature = "gateway-store-postgres",
                        feature = "gateway-store-mysql",
                        feature = "gateway-store-redis"
                    ))]
                    {
                        let payload = serde_json::json!({
                            "request_id": &self.request_id,
                            "provider": &self.provider,
                            "virtual_key_id": self.virtual_key_id.as_deref(),
                            "backend": &self.backend_name,
                            "attempted_backends": &self.attempted_backends,
                            "method": &self.method,
                            "path": &self.path_and_query,
                            "model": &self.model,
                            "upstream_model": self.upstream_model.as_deref(),
                            "status": self.status,
                            "charge_tokens": self.charge_tokens,
                            "input_tokens": observed_usage.and_then(|usage| usage.input_tokens),
                            "cache_input_tokens": observed_usage.and_then(|usage| usage.cache_input_tokens),
                            "cache_creation_input_tokens": observed_usage.and_then(|usage| usage.cache_creation_input_tokens),
                            "output_tokens": observed_usage.and_then(|usage| usage.output_tokens),
                            "reasoning_tokens": observed_usage.and_then(|usage| usage.reasoning_tokens),
                            "total_tokens": observed_usage.and_then(|usage| usage.total_tokens),
                            "spent_tokens": spent_tokens,
                            "charge_cost_usd_micros": self.charge_cost_usd_micros,
                            "spent_cost_usd_micros": spent_cost_usd_micros,
                            "body_len": self.request_body_len,
                            "stream": true,
                        });
                        append_audit_log(&self.state, "proxy", payload).await;
                    }

                    emit_json_log(
                        &self.state,
                        "proxy.response",
                        serde_json::json!({
                            "request_id": &self.request_id,
                            "provider": &self.provider,
                            "backend": &self.backend_name,
                            "status": self.status,
                            "attempted_backends": &self.attempted_backends,
                            "model": &self.model,
                            "upstream_model": self.upstream_model.as_deref(),
                            "input_tokens": observed_usage.and_then(|usage| usage.input_tokens),
                            "cache_input_tokens": observed_usage.and_then(|usage| usage.cache_input_tokens),
                            "cache_creation_input_tokens": observed_usage.and_then(|usage| usage.cache_creation_input_tokens),
                            "output_tokens": observed_usage.and_then(|usage| usage.output_tokens),
                            "reasoning_tokens": observed_usage.and_then(|usage| usage.reasoning_tokens),
                            "total_tokens": observed_usage.and_then(|usage| usage.total_tokens),
                            "spent_tokens": spent_tokens,
                        }),
                    );

                    #[cfg(feature = "sdk")]
                    emit_devtools_log(
                        &self.state,
                        "proxy.response",
                        serde_json::json!({
                            "request_id": &self.request_id,
                            "status": self.status,
                            "path": &self.path_and_query,
                            "backend": &self.backend_name,
                            "spent_tokens": spent_tokens,
                        }),
                    );
                }
            }

            struct ProxySseAbortFinalizeJob {
                finalizer: ProxySseFinalizer,
                observed: Option<ObservedUsage>,
                bytes_sent: u64,
            }

            struct ProxySseAbortFinalizerPool {
                senders: Vec<std::sync::mpsc::SyncSender<ProxySseAbortFinalizeJob>>,
                next_sender: std::sync::atomic::AtomicUsize,
            }

            fn proxy_sse_abort_finalizer_pool() -> &'static ProxySseAbortFinalizerPool {
                static POOL: std::sync::OnceLock<ProxySseAbortFinalizerPool> =
                    std::sync::OnceLock::new();
                POOL.get_or_init(|| {
                    let workers = PROXY_SSE_ABORT_FINALIZER_WORKERS.max(1);
                    let capacity = PROXY_SSE_ABORT_FINALIZER_QUEUE_CAPACITY.max(1);
                    let mut senders = Vec::with_capacity(workers);

                    for worker in 0..workers {
                        let (tx, rx) =
                            std::sync::mpsc::sync_channel::<ProxySseAbortFinalizeJob>(capacity);
                        let thread_name = format!("ditto-proxy-sse-finalizer-{worker}");
                        let spawn_result =
                            std::thread::Builder::new()
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
                                                .finalize(
                                                    job.observed,
                                                    StreamEnd::Aborted,
                                                    job.bytes_sent,
                                                )
                                                .await;
                                        });
                                    }
                                });

                        if spawn_result.is_ok() {
                            senders.push(tx);
                        }
                    }

                    ProxySseAbortFinalizerPool {
                        senders,
                        next_sender: std::sync::atomic::AtomicUsize::new(0),
                    }
                })
            }

            fn enqueue_proxy_sse_abort_finalize(
                finalizer: ProxySseFinalizer,
                observed: Option<ObservedUsage>,
                bytes_sent: u64,
            ) {
                fn spawn_proxy_sse_abort_finalize(job: ProxySseAbortFinalizeJob) {
                    match tokio::runtime::Handle::try_current() {
                        Ok(handle) => {
                            handle.spawn(async move {
                                job.finalizer
                                    .finalize(job.observed, StreamEnd::Aborted, job.bytes_sent)
                                    .await;
                            });
                        }
                        Err(_) => {
                            let _ = std::thread::Builder::new()
                                .name("ditto-proxy-sse-finalizer-fallback".to_string())
                                .spawn(move || {
                                    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                                        .enable_all()
                                        .build()
                                    else {
                                        return;
                                    };
                                    runtime.block_on(async move {
                                        job.finalizer
                                            .finalize(
                                                job.observed,
                                                StreamEnd::Aborted,
                                                job.bytes_sent,
                                            )
                                            .await;
                                    });
                                });
                        }
                    }
                }

                let job = ProxySseAbortFinalizeJob {
                    finalizer,
                    observed,
                    bytes_sent,
                };

                let pool = proxy_sse_abort_finalizer_pool();
                if pool.senders.is_empty() {
                    spawn_proxy_sse_abort_finalize(job);
                    return;
                }

                let idx = pool.next_sender.fetch_add(1, Ordering::Relaxed) % pool.senders.len();
                if let Err(err) = pool.senders[idx].try_send(job) {
                    let job = match err {
                        std::sync::mpsc::TrySendError::Full(job) => job,
                        std::sync::mpsc::TrySendError::Disconnected(job) => job,
                    };
                    spawn_proxy_sse_abort_finalize(job);
                }
            }

            struct ProxySseStreamState {
                upstream: ProxyBodyStream,
                tracker: SseUsageTracker,
                bytes_sent: u64,
                finalizer: Option<ProxySseFinalizer>,
                #[cfg(feature = "gateway-proxy-cache")]
                cache_completion: Option<ProxyCompletedStreamCacheWrite>,
                _permits: ProxyPermits,
            }

            impl Drop for ProxySseStreamState {
                fn drop(&mut self) {
                    let Some(finalizer) = self.finalizer.take() else {
                        return;
                    };
                    let observed = self.tracker.observed_usage();
                    let bytes_sent = self.bytes_sent;
                    enqueue_proxy_sse_abort_finalize(finalizer, observed, bytes_sent);
                }
            }

            impl ProxySseStreamState {
                async fn finalize(&mut self, end: StreamEnd) {
                    #[cfg(feature = "gateway-proxy-cache")]
                    if matches!(end, StreamEnd::Completed) {
                        if let Some(cache_completion) = self.cache_completion.take() {
                            cache_completion.finish().await;
                        }
                    }

                    let Some(finalizer) = self.finalizer.take() else {
                        return;
                    };
                    let observed = self.tracker.observed_usage();
                    let bytes_sent = self.bytes_sent;
                    finalizer.finalize(observed, end, bytes_sent).await;
                }
            }

            let mut headers = upstream_headers;
            apply_proxy_response_headers(&mut headers, &backend_name, &request_id, false);
            #[cfg(feature = "gateway-proxy-cache")]
            if let Some(cache_key) = proxy_cache_key.as_ref() {
                if let Ok(value) = axum::http::HeaderValue::from_str(cache_key) {
                    headers.insert("x-ditto-cache-key", value);
                }
            }

            #[cfg(feature = "gateway-otel")]
            {
                tracing::Span::current().record("cache", tracing::field::display("miss"));
                tracing::Span::current().record("backend", tracing::field::display(&backend_name));
                tracing::Span::current().record("status", tracing::field::display(status.as_u16()));
            }

            let upstream_stream: ProxyBodyStream = upstream_response
                .bytes_stream()
                .map(|chunk| chunk.map_err(std::io::Error::other))
                .boxed();

            #[cfg(any(
                feature = "gateway-store-sqlite",
                feature = "gateway-store-postgres",
                feature = "gateway-store-mysql",
                feature = "gateway-store-redis"
            ))]
            let token_budget_reservation_ids = token_budget_reservation_ids.to_vec();

            #[cfg(all(
                feature = "gateway-costing",
                any(
                    feature = "gateway-store-sqlite",
                    feature = "gateway-store-postgres",
                    feature = "gateway-store-mysql",
                    feature = "gateway-store-redis"
                ),
            ))]
            let cost_budget_reservation_ids = cost_budget_reservation_ids.to_vec();

            let finalizer = ProxySseFinalizer {
                state: state.to_owned(),
                backend_name: backend_name.clone(),
                attempted_backends: attempted_backends.to_vec(),
                request_id: request_id.clone(),
                provider: protocol.clone(),
                #[cfg(any(
                    feature = "gateway-store-sqlite",
                    feature = "gateway-store-postgres",
                    feature = "gateway-store-mysql",
                    feature = "gateway-store-redis",
                    feature = "sdk"
                ))]
                method: parts.method.as_str().to_string(),
                #[cfg(any(
                    feature = "gateway-store-sqlite",
                    feature = "gateway-store-postgres",
                    feature = "gateway-store-mysql",
                    feature = "gateway-store-redis",
                    feature = "sdk"
                ))]
                path_and_query: path_and_query.to_string(),
                #[cfg(feature = "gateway-metrics-prometheus")]
                metrics_path: metrics_path.to_string(),
                model: model.to_owned(),
                upstream_model: upstream_model.clone(),
                service_tier: service_tier.to_owned(),
                backend_model_map: backend_model_map.clone(),
                status: status.as_u16(),
                charge_tokens,
                charge_cost_usd_micros,
                spend_tokens,
                #[cfg(any(
                    feature = "gateway-store-sqlite",
                    feature = "gateway-store-postgres",
                    feature = "gateway-store-mysql",
                    feature = "gateway-store-redis"
                ))]
                use_persistent_budget,
                virtual_key_id: virtual_key_id.to_owned(),
                budget: budget.to_owned(),
                tenant_budget_scope: tenant_budget_scope.to_owned(),
                project_budget_scope: project_budget_scope.to_owned(),
                user_budget_scope: user_budget_scope.to_owned(),
                #[cfg(any(
                    feature = "gateway-store-sqlite",
                    feature = "gateway-store-postgres",
                    feature = "gateway-store-mysql",
                    feature = "gateway-store-redis"
                ))]
                token_budget_reservation_ids,
                #[cfg(all(
                    feature = "gateway-costing",
                    any(
                        feature = "gateway-store-sqlite",
                        feature = "gateway-store-postgres",
                        feature = "gateway-store-mysql",
                        feature = "gateway-store-redis"
                    )
                ))]
                cost_budget_reserved: _cost_budget_reserved,
                #[cfg(all(
                    feature = "gateway-costing",
                    any(
                        feature = "gateway-store-sqlite",
                        feature = "gateway-store-postgres",
                        feature = "gateway-store-mysql",
                        feature = "gateway-store-redis"
                    )
                ))]
                cost_budget_reservation_ids,
                #[cfg(any(
                    feature = "gateway-store-sqlite",
                    feature = "gateway-store-postgres",
                    feature = "gateway-store-mysql",
                    feature = "gateway-store-redis",
                    feature = "sdk"
                ))]
                request_body_len: body.len(),
            };

            #[cfg(feature = "gateway-metrics-prometheus")]
            if let Some(metrics) = state.proxy.metrics.as_ref() {
                metrics
                    .lock()
                    .await
                    .record_proxy_stream_open(&backend_name, metrics_path);
            }

            let state = ProxySseStreamState {
                upstream: upstream_stream,
                tracker: SseUsageTracker::default(),
                bytes_sent: 0,
                finalizer: Some(finalizer),
                #[cfg(feature = "gateway-proxy-cache")]
                cache_completion: ProxyCompletedStreamCacheWrite::new(
                    state,
                    &backend_name,
                    status,
                    &headers,
                    proxy_cache_key.as_deref(),
                    proxy_cache_metadata.as_ref(),
                ),
                _permits: proxy_permits.take(),
            };

            let stream = futures_util::stream::try_unfold(state, |mut state| async move {
                match state.upstream.next().await {
                    Some(Ok(chunk)) => {
                        state.bytes_sent = state.bytes_sent.saturating_add(chunk.len() as u64);
                        state.tracker.ingest(&chunk);
                        #[cfg(feature = "gateway-proxy-cache")]
                        if let Some(cache_completion) = state.cache_completion.as_mut() {
                            cache_completion.ingest(&chunk);
                        }
                        Ok(Some((chunk, state)))
                    }
                    Some(Err(err)) => {
                        state.finalize(StreamEnd::Error).await;
                        Err(err)
                    }
                    None => {
                        state.finalize(StreamEnd::Completed).await;
                        Ok(None)
                    }
                }
            });

            let mut response = axum::response::Response::new(Body::from_stream(stream));
            *response.status_mut() = status;
            *response.headers_mut() = headers;
            return Ok(BackendAttemptOutcome::Response(response));
        }
        // end inline: proxy_backend/stream.rs
    }

    // inlined from proxy_backend/nonstream.rs
    {
        let content_length = upstream_headers
            .get("content-length")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<usize>().ok());

        #[cfg(feature = "gateway-proxy-cache")]
        let should_attempt_buffer_for_cache =
            status.is_success() && proxy_cache_key.is_some() && state.proxy.cache_config.is_some();

        let should_attempt_buffer_for_usage =
            content_type.starts_with("application/json") && state.proxy.usage_max_body_bytes > 0;

        let cache_max_buffer_bytes = {
            #[cfg(feature = "gateway-proxy-cache")]
            {
                if should_attempt_buffer_for_cache {
                    state
                        .proxy
                        .cache_config
                        .as_ref()
                        .map(|config| config.max_body_bytes)
                        .unwrap_or(1024 * 1024)
                } else {
                    0
                }
            }
            #[cfg(not(feature = "gateway-proxy-cache"))]
            {
                0
            }
        };

        let usage_max_buffer_bytes = if should_attempt_buffer_for_usage {
            state.proxy.usage_max_body_bytes
        } else {
            0
        };

        let max_buffer_bytes = cache_max_buffer_bytes.max(usage_max_buffer_bytes);
        let should_try_buffer =
            max_buffer_bytes > 0 && content_length.is_none_or(|len| len <= max_buffer_bytes);

        enum ProxyResponseBody {
            Bytes(Bytes),
            Stream(ProxyBodyStream),
        }

        let response_body = if should_try_buffer {
            let mut upstream_stream = upstream_response.bytes_stream();
            let initial_capacity = content_length
                .map(|len| len.min(max_buffer_bytes))
                .unwrap_or(0);
            let mut buffered = bytes::BytesMut::with_capacity(initial_capacity);
            let mut first_unbuffered: Option<Bytes> = None;
            let mut stream_error: Option<std::io::Error> = None;

            while let Some(next) = upstream_stream.next().await {
                match next {
                    Ok(chunk) => {
                        if buffered.len().saturating_add(chunk.len()) <= max_buffer_bytes {
                            buffered.extend_from_slice(chunk.as_ref());
                        } else {
                            first_unbuffered = Some(chunk);
                            break;
                        }
                    }
                    Err(err) => {
                        stream_error = Some(std::io::Error::other(err));
                        break;
                    }
                }
            }

            match (first_unbuffered, stream_error) {
                (None, None) => ProxyResponseBody::Bytes(buffered.freeze()),
                (Some(chunk), _) => {
                    let prefix_bytes = buffered.freeze();
                    let prefix: ProxyBodyStream = if prefix_bytes.is_empty() {
                        futures_util::stream::empty().boxed()
                    } else {
                        futures_util::stream::once(async move {
                            Ok::<Bytes, std::io::Error>(prefix_bytes)
                        })
                        .boxed()
                    };
                    let first =
                        futures_util::stream::once(
                            async move { Ok::<Bytes, std::io::Error>(chunk) },
                        );
                    let rest = upstream_stream.map(|chunk| chunk.map_err(std::io::Error::other));
                    let stream = prefix.chain(first).chain(rest).boxed();
                    ProxyResponseBody::Stream(stream)
                }
                (None, Some(err)) => {
                    let prefix_bytes = buffered.freeze();
                    let prefix: ProxyBodyStream = if prefix_bytes.is_empty() {
                        futures_util::stream::empty().boxed()
                    } else {
                        futures_util::stream::once(async move {
                            Ok::<Bytes, std::io::Error>(prefix_bytes)
                        })
                        .boxed()
                    };
                    let err_stream =
                        futures_util::stream::once(
                            async move { Err::<Bytes, std::io::Error>(err) },
                        );
                    let stream = prefix.chain(err_stream).boxed();
                    ProxyResponseBody::Stream(stream)
                }
            }
        } else {
            let stream = upstream_response
                .bytes_stream()
                .map(|chunk| chunk.map_err(std::io::Error::other))
                .boxed();
            ProxyResponseBody::Stream(stream)
        };

        let observed_usage = if should_attempt_buffer_for_usage {
            match &response_body {
                ProxyResponseBody::Bytes(bytes) => extract_openai_usage_from_bytes(bytes),
                ProxyResponseBody::Stream(_) => None,
            }
        } else {
            None
        };

        let spent_tokens = if spend_tokens {
            observed_usage
                .and_then(|usage| usage.total_tokens)
                .unwrap_or_else(|| u64::from(charge_tokens))
        } else {
            0
        };

        #[cfg(feature = "gateway-costing")]
        let spent_cost_usd_micros = if spend_tokens {
            model
                .as_deref()
                .map(|request_model| {
                    backend_model_map
                        .get(request_model)
                        .map(|model| model.as_str())
                        .unwrap_or(request_model)
                })
                .and_then(|cost_model| {
                    state.proxy.pricing.as_ref().and_then(|pricing| {
                        let usage = observed_usage?;
                        let input = usage.input_tokens?;
                        let output = usage.output_tokens?;
                        pricing.estimate_cost_usd_micros_with_cache_for_service_tier(
                            cost_model,
                            clamp_u64_to_u32(input),
                            usage.cache_input_tokens.map(clamp_u64_to_u32),
                            usage.cache_creation_input_tokens.map(clamp_u64_to_u32),
                            clamp_u64_to_u32(output),
                            service_tier.as_deref(),
                        )
                    })
                })
                .or(charge_cost_usd_micros)
        } else {
            None
        };
        #[cfg(not(feature = "gateway-costing"))]
        let spent_cost_usd_micros: Option<u64> = None;

        #[cfg(not(any(
            feature = "gateway-costing",
            feature = "gateway-store-sqlite",
            feature = "gateway-store-redis"
        )))]
        let _ = spent_cost_usd_micros;

        #[cfg(any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        ))]
        if !token_budget_reservation_ids.is_empty() {
            settle_proxy_token_budget_reservations(
                state,
                token_budget_reservation_ids,
                spend_tokens,
                spent_tokens,
            )
            .await;
        } else if let (Some(virtual_key_id), Some(budget)) =
            (virtual_key_id.clone(), budget.clone())
        {
            if spend_tokens {
                state.spend_budget_tokens(&virtual_key_id, &budget, spent_tokens);
                if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                    state.spend_budget_tokens(scope, budget, spent_tokens);
                }
                if let Some((scope, budget)) = project_budget_scope.as_ref() {
                    state.spend_budget_tokens(scope, budget, spent_tokens);
                }
                if let Some((scope, budget)) = user_budget_scope.as_ref() {
                    state.spend_budget_tokens(scope, budget, spent_tokens);
                }

                #[cfg(feature = "gateway-costing")]
                if !use_persistent_budget {
                    if let Some(spent_cost_usd_micros) = spent_cost_usd_micros {
                        state.spend_budget_cost(&virtual_key_id, &budget, spent_cost_usd_micros);
                        if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                            state.spend_budget_cost(scope, budget, spent_cost_usd_micros);
                        }
                        if let Some((scope, budget)) = project_budget_scope.as_ref() {
                            state.spend_budget_cost(scope, budget, spent_cost_usd_micros);
                        }
                        if let Some((scope, budget)) = user_budget_scope.as_ref() {
                            state.spend_budget_cost(scope, budget, spent_cost_usd_micros);
                        }
                    }
                }
            }
        }
        #[cfg(not(any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        )))]
        if let (Some(virtual_key_id), Some(budget)) = (virtual_key_id.clone(), budget.clone()) {
            if spend_tokens {
                state.spend_budget_tokens(&virtual_key_id, &budget, spent_tokens);
                if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                    state.spend_budget_tokens(scope, budget, spent_tokens);
                }
                if let Some((scope, budget)) = project_budget_scope.as_ref() {
                    state.spend_budget_tokens(scope, budget, spent_tokens);
                }
                if let Some((scope, budget)) = user_budget_scope.as_ref() {
                    state.spend_budget_tokens(scope, budget, spent_tokens);
                }

                #[cfg(feature = "gateway-costing")]
                if let Some(spent_cost_usd_micros) = spent_cost_usd_micros {
                    state.spend_budget_cost(&virtual_key_id, &budget, spent_cost_usd_micros);
                    if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                        state.spend_budget_cost(scope, budget, spent_cost_usd_micros);
                    }
                    if let Some((scope, budget)) = project_budget_scope.as_ref() {
                        state.spend_budget_cost(scope, budget, spent_cost_usd_micros);
                    }
                    if let Some((scope, budget)) = user_budget_scope.as_ref() {
                        state.spend_budget_cost(scope, budget, spent_cost_usd_micros);
                    }
                }
            }
        }

        #[cfg(all(
            feature = "gateway-costing",
            any(
                feature = "gateway-store-sqlite",
                feature = "gateway-store-postgres",
                feature = "gateway-store-mysql",
                feature = "gateway-store-redis"
            ),
        ))]
        if !cost_budget_reservation_ids.is_empty() {
            settle_proxy_cost_budget_reservations(
                state,
                cost_budget_reservation_ids,
                spend_tokens,
                spent_cost_usd_micros.unwrap_or_default(),
            )
            .await;
        }

        #[cfg(all(
            feature = "gateway-costing",
            any(
                feature = "gateway-store-sqlite",
                feature = "gateway-store-postgres",
                feature = "gateway-store-mysql",
                feature = "gateway-store-redis"
            ),
        ))]
        if !_cost_budget_reserved && use_persistent_budget && spend_tokens {
            if let (Some(virtual_key_id), Some(spent_cost_usd_micros)) =
                (virtual_key_id.as_deref(), spent_cost_usd_micros)
            {
                #[cfg(feature = "gateway-store-sqlite")]
                if let Some(store) = state.stores.sqlite.as_ref() {
                    let _ = store
                        .record_spent_cost_usd_micros(virtual_key_id, spent_cost_usd_micros)
                        .await;
                }
                #[cfg(feature = "gateway-store-postgres")]
                if let Some(store) = state.stores.postgres.as_ref() {
                    let _ = store
                        .record_spent_cost_usd_micros(virtual_key_id, spent_cost_usd_micros)
                        .await;
                }
                #[cfg(feature = "gateway-store-mysql")]
                if let Some(store) = state.stores.mysql.as_ref() {
                    let _ = store
                        .record_spent_cost_usd_micros(virtual_key_id, spent_cost_usd_micros)
                        .await;
                }
                #[cfg(feature = "gateway-store-redis")]
                if let Some(store) = state.stores.redis.as_ref() {
                    let _ = store
                        .record_spent_cost_usd_micros(virtual_key_id, spent_cost_usd_micros)
                        .await;
                }
            }
        }

        #[cfg(any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        ))]
        {
            let payload = serde_json::json!({
                "request_id": &request_id,
                "provider": &protocol,
                "virtual_key_id": virtual_key_id.as_deref(),
                "backend": &backend_name,
                "attempted_backends": &attempted_backends,
                "method": parts.method.as_str(),
                "path": path_and_query,
                "model": &model,
                "upstream_model": upstream_model.as_deref(),
                "service_tier": service_tier.as_deref(),
                "status": status.as_u16(),
                "charge_tokens": charge_tokens,
                "input_tokens": observed_usage.and_then(|usage| usage.input_tokens),
                "cache_input_tokens": observed_usage.and_then(|usage| usage.cache_input_tokens),
                "cache_creation_input_tokens": observed_usage.and_then(|usage| usage.cache_creation_input_tokens),
                "output_tokens": observed_usage.and_then(|usage| usage.output_tokens),
                "reasoning_tokens": observed_usage.and_then(|usage| usage.reasoning_tokens),
                "total_tokens": observed_usage.and_then(|usage| usage.total_tokens),
                "spent_tokens": spent_tokens,
                "charge_cost_usd_micros": charge_cost_usd_micros,
                "spent_cost_usd_micros": spent_cost_usd_micros,
                "body_len": body.len(),
            });
            append_audit_log(state, "proxy", payload).await;
        }

        emit_json_log(
            state,
            "proxy.response",
            serde_json::json!({
                "request_id": &request_id,
                "provider": &protocol,
                "backend": &backend_name,
                "status": status.as_u16(),
                "attempted_backends": &attempted_backends,
                "model": &model,
                "upstream_model": upstream_model.as_deref(),
                "input_tokens": observed_usage.and_then(|usage| usage.input_tokens),
                "cache_input_tokens": observed_usage.and_then(|usage| usage.cache_input_tokens),
                "cache_creation_input_tokens": observed_usage.and_then(|usage| usage.cache_creation_input_tokens),
                "output_tokens": observed_usage.and_then(|usage| usage.output_tokens),
                "reasoning_tokens": observed_usage.and_then(|usage| usage.reasoning_tokens),
                "total_tokens": observed_usage.and_then(|usage| usage.total_tokens),
                "spent_tokens": spent_tokens,
            }),
        );

        #[cfg(feature = "sdk")]
        emit_devtools_log(
            state,
            "proxy.response",
            serde_json::json!({
                "request_id": &request_id,
                "status": status.as_u16(),
                "path": path_and_query,
                "backend": &backend_name,
            }),
        );

        #[cfg(feature = "gateway-otel")]
        {
            tracing::Span::current().record("cache", tracing::field::display("miss"));
            tracing::Span::current().record("backend", tracing::field::display(&backend_name));
            tracing::Span::current().record("status", tracing::field::display(status.as_u16()));
        }

        #[cfg(feature = "gateway-proxy-cache")]
        if should_attempt_buffer_for_cache && status.is_success() {
            if let (Some(cache_key), Some(cache_metadata)) =
                (proxy_cache_key.as_deref(), proxy_cache_metadata.as_ref())
            {
                if let ProxyResponseBody::Bytes(bytes) = &response_body {
                    let cached = CachedProxyResponse {
                        status: status.as_u16(),
                        headers: upstream_headers.clone(),
                        body: bytes.clone(),
                        backend: backend_name.clone(),
                    };
                    store_proxy_cache_response(
                        state,
                        cache_key,
                        cached,
                        cache_metadata,
                        now_epoch_seconds(),
                    )
                    .await;
                }
            }
        }

        let mut headers = upstream_headers;
        apply_proxy_response_headers(&mut headers, &backend_name, &request_id, false);
        #[cfg(feature = "gateway-proxy-cache")]
        if let Some(cache_key) = proxy_cache_key.as_deref() {
            if let Ok(value) = axum::http::HeaderValue::from_str(cache_key) {
                headers.insert("x-ditto-cache-key", value);
            }
        }
        match response_body {
            ProxyResponseBody::Bytes(bytes) => {
                let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
                let mut response = axum::response::Response::new(body);
                *response.status_mut() = status;
                *response.headers_mut() = headers;
                Ok(BackendAttemptOutcome::Response(response))
            }
            ProxyResponseBody::Stream(stream) => {
                headers.remove("content-length");
                let stream = ProxyBodyStreamWithPermit {
                    inner: stream,
                    _permits: proxy_permits.take(),
                };
                let mut response = axum::response::Response::new(Body::from_stream(stream));
                *response.status_mut() = status;
                *response.headers_mut() = headers;
                Ok(BackendAttemptOutcome::Response(response))
            }
        }
    }
    // end inline: proxy_backend/nonstream.rs
}

#[cfg(feature = "gateway-routing-advanced")]
fn classify_proxy_backend_transport_failure(err: &GatewayError) -> FailureKind {
    match err {
        GatewayError::BackendTimeout { .. } => FailureKind::Timeout,
        _ => FailureKind::Network,
    }
}

#[cfg(feature = "gateway-routing-advanced")]
fn should_record_proxy_status_failure(
    state: &GatewayHttpState,
    retry_config: &crate::gateway::ProxyRetryConfig,
    kind: FailureKind,
    status: StatusCode,
) -> bool {
    status.is_server_error()
        || retry_config.action_for_failure(kind)
            != crate::gateway::proxy_routing::ProxyFailureAction::None
        || state
            .proxy
            .routing
            .as_ref()
            .map(|config| config.circuit_breaker.should_count_failure(kind))
            .unwrap_or(false)
}

#[cfg(feature = "gateway-routing-advanced")]
struct ProxyDecisionLogContext<'a> {
    request_id: &'a str,
    backend_name: &'a str,
    path_and_query: &'a str,
    attempted_backends: &'a [String],
    idx: usize,
    max_attempts: usize,
    status_code: Option<u16>,
}

#[cfg(feature = "gateway-routing-advanced")]
async fn emit_proxy_backend_decision_logs(
    state: &GatewayHttpState,
    decision: crate::gateway::proxy_routing::ProxyFailureDecision,
    ctx: ProxyDecisionLogContext<'_>,
) {
    let will_attempt_next_backend = decision.should_attempt_next_backend(ctx.idx, ctx.max_attempts);
    emit_json_log(
        state,
        decision.event_name(),
        serde_json::json!({
            "request_id": ctx.request_id,
            "backend": ctx.backend_name,
            "action": decision.action.as_str(),
            "failure_kind": decision.kind.as_str(),
            "reason": decision.reason_code(),
            "status": ctx.status_code.or_else(|| decision.kind.status_code()),
            "path": ctx.path_and_query,
            "will_attempt_next_backend": will_attempt_next_backend,
            "attempted_backends": ctx.attempted_backends,
        }),
    );

    #[cfg(feature = "sdk")]
    emit_devtools_log(
        state,
        decision.event_name(),
        serde_json::json!({
            "request_id": ctx.request_id,
            "backend": ctx.backend_name,
            "action": decision.action.as_str(),
            "failure_kind": decision.kind.as_str(),
            "reason": decision.reason_code(),
            "status": ctx.status_code.or_else(|| decision.kind.status_code()),
            "will_attempt_next_backend": will_attempt_next_backend,
            "path": ctx.path_and_query,
        }),
    );
}

#[cfg(feature = "gateway-routing-advanced")]
fn openai_status_routing_error(
    status: StatusCode,
    decision: crate::gateway::proxy_routing::ProxyFailureDecision,
) -> (StatusCode, Json<OpenAiErrorResponse>) {
    let message = match decision.action {
        crate::gateway::proxy_routing::ProxyFailureAction::Retry => {
            format!("retryable upstream status {}", status.as_u16())
        }
        crate::gateway::proxy_routing::ProxyFailureAction::Fallback => {
            format!("fallbackable upstream status {}", status.as_u16())
        }
        crate::gateway::proxy_routing::ProxyFailureAction::None => {
            format!("upstream status {}", status.as_u16())
        }
    };

    openai_error(status, "api_error", Some("backend_error"), message)
}
// end inline: ../../http/proxy_backend.rs
// inlined from ../../http/litellm_keys.rs
static LITELLM_KEY_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Deserialize)]
struct LitellmKeyGenerateRequest {
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    key_alias: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    team_id: Option<String>,
    #[serde(default)]
    organization_id: Option<String>,
    #[serde(default)]
    models: Vec<String>,
    #[serde(default)]
    max_budget: Option<f64>,
    #[serde(default)]
    rpm_limit: Option<u32>,
    #[serde(default)]
    tpm_limit: Option<u32>,
}

#[derive(Debug, Serialize)]
struct LitellmKeyGenerateResponse {
    key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    key_alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    key_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    team_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    organization_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_budget: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rpm_limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tpm_limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    models: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct LitellmKeyDeleteRequest {
    #[serde(default)]
    keys: Option<Vec<String>>,
    #[serde(default)]
    key_aliases: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
struct LitellmKeyDeleteResponse {
    deleted_keys: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct LitellmKeyInfoQuery {
    #[serde(default)]
    key: Option<String>,
}

#[derive(Debug, Serialize)]
struct LitellmKeyInfoResponse {
    key: String,
    info: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct LitellmKeyListQuery {
    #[serde(default)]
    page: Option<usize>,
    #[serde(default)]
    size: Option<usize>,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    team_id: Option<String>,
    #[serde(default)]
    organization_id: Option<String>,
    #[serde(default)]
    key_alias: Option<String>,
    #[serde(default)]
    return_full_object: Option<bool>,
}

#[derive(Debug, Serialize)]
struct LitellmKeyListResponse {
    keys: Vec<serde_json::Value>,
    total_count: usize,
    current_page: usize,
    total_pages: usize,
}

fn litellm_key_info_value(key: &VirtualKeyConfig) -> serde_json::Value {
    serde_json::json!({
        "key_alias": key.id,
        "key_name": key.id,
        "user_id": key.user_id,
        "team_id": key.tenant_id,
        "enabled": key.enabled,
        "blocked": !key.enabled,
        "tpm_limit": key.limits.tpm,
        "rpm_limit": key.limits.rpm,
        "max_budget": key.budget.total_usd_micros.map(|v| (v as f64) / 1_000_000.0),
        "models": key.guardrails.allow_models,
    })
}

fn litellm_key_full_value(key: &VirtualKeyConfig) -> serde_json::Value {
    let mut value = litellm_key_info_value(key);
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "token".to_string(),
            serde_json::Value::String(key.token.clone()),
        );
    }
    value
}

fn litellm_key_router() -> Router<GatewayHttpState> {
    Router::new()
        .route("/key/generate", post(litellm_key_generate))
        .route("/key/update", post(litellm_key_update))
        .route("/key/delete", post(litellm_key_delete))
        .route("/key/info", get(litellm_key_info))
        .route("/key/list", get(litellm_key_list))
        .route("/key/regenerate", post(litellm_key_regenerate))
        .route("/key/:key/regenerate", post(litellm_key_regenerate_path))
}

async fn litellm_key_generate(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Json(payload): Json<LitellmKeyGenerateRequest>,
) -> Result<Json<LitellmKeyGenerateResponse>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_write(&state, &headers)?;

    let key_alias = payload
        .key_alias
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .unwrap_or_else(generate_key_id);

    let key = payload
        .key
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .unwrap_or_else(generate_key_token);

    let tenant_id = payload
        .organization_id
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .or_else(|| {
            payload
                .team_id
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(|v| v.to_string())
        });

    let mut virtual_key = VirtualKeyConfig::new(key_alias.clone(), key.clone());
    virtual_key.enabled = true;
    virtual_key.tenant_id = tenant_id.clone();
    virtual_key.user_id = payload.user_id.clone();
    virtual_key.limits.rpm = payload.rpm_limit;
    virtual_key.limits.tpm = payload.tpm_limit;

    if !payload.models.is_empty() {
        virtual_key.guardrails.allow_models = payload.models.clone();
    }

    if let Some(max_budget) = payload.max_budget {
        if !max_budget.is_finite() || max_budget < 0.0 {
            return Err(error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "max_budget must be a non-negative finite number",
            ));
        }
        let micros = (max_budget * 1_000_000.0).round();
        if micros > 0.0 {
            virtual_key.budget.total_usd_micros = Some(micros as u64);
        }
    }

    if let Some(admin_tenant) = admin.tenant_id.as_deref() {
        if tenant_id.as_deref().is_some_and(|t| t != admin_tenant) {
            return Err(error_response(
                StatusCode::FORBIDDEN,
                "forbidden",
                "cannot generate keys for a different tenant",
            ));
        }
        if virtual_key.tenant_id.is_none() {
            virtual_key.tenant_id = Some(admin_tenant.to_string());
        }
    }

    let persisted_keys = state.gateway.mutate_control_plane(
        |gateway| -> Result<_, (StatusCode, Json<ErrorResponse>)> {
            if gateway
                .list_virtual_keys()
                .iter()
                .any(|existing| existing.id == virtual_key.id)
            {
                return Err(error_response(
                    StatusCode::CONFLICT,
                    "conflict",
                    "key_alias already exists",
                ));
            }
            gateway.upsert_virtual_key(virtual_key.clone());
            Ok(gateway.list_virtual_keys())
        },
    )?;
    state.sync_control_plane_from_gateway();
    let _ = persist_virtual_keys(&state, &persisted_keys, "litellm.key.generate").await?;

    #[cfg(feature = "sdk")]
    emit_devtools_log(
        &state,
        "litellm.key.generate",
        serde_json::json!({
            "key_id": &virtual_key.id,
            "tenant_id": virtual_key.tenant_id.as_deref(),
        }),
    );

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    append_admin_audit_log(
        &state,
        "litellm.key.generate",
        serde_json::json!({
            "key_id": &virtual_key.id,
            "tenant_id": virtual_key.tenant_id.as_deref(),
            "user_id": virtual_key.user_id.as_deref(),
        }),
    )
    .await;

    Ok(Json(LitellmKeyGenerateResponse {
        key: key.clone(),
        token: Some(key),
        key_alias: Some(key_alias),
        key_name: Some(virtual_key.id),
        user_id: payload.user_id,
        team_id: payload.team_id,
        organization_id: payload.organization_id,
        max_budget: payload.max_budget,
        rpm_limit: payload.rpm_limit,
        tpm_limit: payload.tpm_limit,
        models: payload.models,
    }))
}

#[derive(Debug, Deserialize)]
struct LitellmKeyUpdateRequest {
    key: String,
    #[serde(default)]
    key_alias: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    team_id: Option<String>,
    #[serde(default)]
    organization_id: Option<String>,
    #[serde(default)]
    models: Option<Vec<String>>,
    #[serde(default)]
    max_budget: Option<f64>,
    #[serde(default)]
    rpm_limit: Option<u32>,
    #[serde(default)]
    tpm_limit: Option<u32>,
    #[serde(default)]
    blocked: Option<bool>,
}

fn litellm_generate_response_from_virtual_key(
    key: &VirtualKeyConfig,
) -> LitellmKeyGenerateResponse {
    LitellmKeyGenerateResponse {
        key: key.token.clone(),
        token: Some(key.token.clone()),
        key_alias: Some(key.id.clone()),
        key_name: Some(key.id.clone()),
        user_id: key.user_id.clone(),
        team_id: key.tenant_id.clone(),
        organization_id: key.tenant_id.clone(),
        max_budget: key
            .budget
            .total_usd_micros
            .map(|v| (v as f64) / 1_000_000.0),
        rpm_limit: key.limits.rpm,
        tpm_limit: key.limits.tpm,
        models: key.guardrails.allow_models.clone(),
    }
}

async fn litellm_key_update(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Json(payload): Json<LitellmKeyUpdateRequest>,
) -> Result<Json<LitellmKeyGenerateResponse>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_write(&state, &headers)?;

    let key_token = payload.key.trim();
    if key_token.is_empty() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "key is required",
        ));
    }

    let new_alias = payload
        .key_alias
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string());

    let tenant_id = payload
        .organization_id
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .or_else(|| {
            payload
                .team_id
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(|v| v.to_string())
        });

    let (key, persisted_keys) = state.gateway.mutate_control_plane(
        |gateway| -> Result<_, (StatusCode, Json<ErrorResponse>)> {
            let keys = gateway.list_virtual_keys();

            let Some(existing) = keys.iter().find(|key| key.token == key_token).cloned() else {
                return Err(error_response(
                    StatusCode::NOT_FOUND,
                    "not_found",
                    "virtual key not found",
                ));
            };

            if let Some(admin_tenant) = admin.tenant_id.as_deref() {
                if existing.tenant_id.as_deref() != Some(admin_tenant) {
                    return Err(error_response(
                        StatusCode::FORBIDDEN,
                        "forbidden",
                        "cannot update keys for a different tenant",
                    ));
                }
            }

            let mut key = existing.clone();

            if let Some(user_id) = payload
                .user_id
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
            {
                key.user_id = Some(user_id.to_string());
            }

            if let Some(tenant_id) = tenant_id.as_deref() {
                if let Some(admin_tenant) = admin.tenant_id.as_deref() {
                    if tenant_id != admin_tenant {
                        return Err(error_response(
                            StatusCode::FORBIDDEN,
                            "forbidden",
                            "cannot update keys for a different tenant",
                        ));
                    }
                }
                key.tenant_id = Some(tenant_id.to_string());
            }

            if let Some(models) = payload.models.as_ref() {
                key.guardrails.allow_models = models.clone();
            }

            if let Some(max_budget) = payload.max_budget {
                if !max_budget.is_finite() || max_budget < 0.0 {
                    return Err(error_response(
                        StatusCode::BAD_REQUEST,
                        "invalid_request",
                        "max_budget must be a non-negative finite number",
                    ));
                }
                let micros = (max_budget * 1_000_000.0).round();
                if micros <= 0.0 {
                    key.budget.total_usd_micros = None;
                } else {
                    key.budget.total_usd_micros = Some(micros as u64);
                }
            }

            if let Some(rpm) = payload.rpm_limit {
                key.limits.rpm = Some(rpm);
            }
            if let Some(tpm) = payload.tpm_limit {
                key.limits.tpm = Some(tpm);
            }

            if let Some(blocked) = payload.blocked {
                key.enabled = !blocked;
            }

            let old_id = &existing.id;
            if let Some(new_id) = new_alias.as_deref() {
                if new_id != old_id {
                    if keys.iter().any(|candidate| candidate.id == new_id) {
                        return Err(error_response(
                            StatusCode::CONFLICT,
                            "conflict",
                            "key_alias already exists",
                        ));
                    }
                    gateway.remove_virtual_key(old_id);
                    key.id = new_id.to_string();
                }
            }

            gateway.upsert_virtual_key(key.clone());
            Ok((key, gateway.list_virtual_keys()))
        },
    )?;
    state.sync_control_plane_from_gateway();

    let _ = persist_virtual_keys(&state, &persisted_keys, "litellm.key.update").await?;

    #[cfg(feature = "sdk")]
    emit_devtools_log(
        &state,
        "litellm.key.update",
        serde_json::json!({
            "key_id": &key.id,
            "tenant_id": key.tenant_id.as_deref(),
        }),
    );

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    append_admin_audit_log(
        &state,
        "litellm.key.update",
        serde_json::json!({
            "key_id": &key.id,
            "tenant_id": key.tenant_id.as_deref(),
            "user_id": key.user_id.as_deref(),
        }),
    )
    .await;

    Ok(Json(litellm_generate_response_from_virtual_key(&key)))
}

async fn litellm_key_delete(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Json(payload): Json<LitellmKeyDeleteRequest>,
) -> Result<Json<LitellmKeyDeleteResponse>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_write(&state, &headers)?;

    let mut keys = payload.keys.unwrap_or_default();
    let mut aliases = payload.key_aliases.unwrap_or_default();
    keys.retain(|value| !value.trim().is_empty());
    aliases.retain(|value| !value.trim().is_empty());

    if keys.is_empty() && aliases.is_empty() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "expected keys or key_aliases",
        ));
    }

    let mut deleted_keys: Vec<String> = Vec::new();
    let mut deleted_key_ids: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    let persisted_keys = state.gateway.mutate_control_plane(
        |gateway| -> Result<_, (StatusCode, Json<ErrorResponse>)> {
            let mut current = gateway.list_virtual_keys();

            for alias in aliases {
                let alias = alias.trim().to_string();
                if alias.is_empty() {
                    continue;
                }
                let Some((found_id, found_tenant)) = current
                    .iter()
                    .find(|key| key.id == alias)
                    .map(|key| (key.id.clone(), key.tenant_id.clone()))
                else {
                    missing.push(alias);
                    continue;
                };
                if let Some(admin_tenant) = admin.tenant_id.as_deref() {
                    if found_tenant.as_deref() != Some(admin_tenant) {
                        return Err(error_response(
                            StatusCode::FORBIDDEN,
                            "forbidden",
                            "cannot delete keys for a different tenant",
                        ));
                    }
                }
                if gateway.remove_virtual_key(&found_id).is_some() {
                    deleted_keys.push(alias);
                    deleted_key_ids.push(found_id.clone());
                    current.retain(|key| key.id != found_id);
                } else {
                    missing.push(alias);
                }
            }

            for token in keys {
                let token = token.trim().to_string();
                if token.is_empty() {
                    continue;
                }
                let Some((found_id, found_tenant)) = current
                    .iter()
                    .find(|key| key.token == token)
                    .map(|key| (key.id.clone(), key.tenant_id.clone()))
                else {
                    missing.push(token);
                    continue;
                };
                if let Some(admin_tenant) = admin.tenant_id.as_deref() {
                    if found_tenant.as_deref() != Some(admin_tenant) {
                        return Err(error_response(
                            StatusCode::FORBIDDEN,
                            "forbidden",
                            "cannot delete keys for a different tenant",
                        ));
                    }
                }
                if gateway.remove_virtual_key(&found_id).is_some() {
                    deleted_keys.push(token);
                    deleted_key_ids.push(found_id.clone());
                    current.retain(|key| key.id != found_id);
                } else {
                    missing.push(token);
                }
            }

            Ok(gateway.list_virtual_keys())
        },
    )?;
    state.sync_control_plane_from_gateway();

    if !missing.is_empty() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "not all keys passed in were deleted",
        ));
    }

    let _ = persist_virtual_keys(&state, &persisted_keys, "litellm.key.delete").await?;

    #[cfg(feature = "sdk")]
    emit_devtools_log(
        &state,
        "litellm.key.delete",
        serde_json::json!({
            "deleted": deleted_keys.len(),
            "tenant_id": admin.tenant_id.as_deref(),
        }),
    );

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    append_admin_audit_log(
        &state,
        "litellm.key.delete",
        serde_json::json!({
            "deleted": deleted_key_ids.len(),
            "deleted_key_ids": &deleted_key_ids,
            "tenant_id": admin.tenant_id.as_deref(),
        }),
    )
    .await;

    Ok(Json(LitellmKeyDeleteResponse { deleted_keys }))
}

async fn litellm_key_info(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Query(query): Query<LitellmKeyInfoQuery>,
) -> Result<Json<LitellmKeyInfoResponse>, (StatusCode, Json<ErrorResponse>)> {
    let requested_token = query
        .key
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());

    let bearer_token = extract_bearer(&headers);

    let (admin, token) = if let Some(token) = requested_token {
        if bearer_token.as_deref() == Some(token) {
            (None, token.to_string())
        } else {
            let admin = ensure_admin_read(&state, &headers)?;
            (Some(admin), token.to_string())
        }
    } else {
        let token = bearer_token.ok_or_else(|| {
            error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "key is required",
            )
        })?;
        (None, token)
    };

    let key = state
        .list_virtual_keys_snapshot()
        .into_iter()
        .find(|key| key.token == token)
        .ok_or_else(|| {
            error_response(StatusCode::NOT_FOUND, "not_found", "virtual key not found")
        })?;

    if let Some(admin) = admin {
        if let Some(admin_tenant) = admin.tenant_id.as_deref() {
            if key.tenant_id.as_deref() != Some(admin_tenant) {
                return Err(error_response(
                    StatusCode::FORBIDDEN,
                    "forbidden",
                    "cannot access keys for a different tenant",
                ));
            }
        }
    }

    Ok(Json(LitellmKeyInfoResponse {
        key: token,
        info: litellm_key_info_value(&key),
    }))
}

async fn litellm_key_list(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Query(query): Query<LitellmKeyListQuery>,
) -> Result<Json<LitellmKeyListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;

    let return_full_object = query.return_full_object.unwrap_or(false);
    let page = query.page.unwrap_or(1).max(1);
    let size = query.size.unwrap_or(10).clamp(1, 100);
    let offset = (page - 1).saturating_mul(size);

    let tenant_filter = query
        .organization_id
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .or_else(|| {
            query
                .team_id
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(|v| v.to_string())
        });

    let user_filter = query
        .user_id
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string());

    let alias_filter = query
        .key_alias
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string());

    let mut keys = state.list_virtual_keys_snapshot();

    if let Some(admin_tenant) = admin.tenant_id.as_deref() {
        keys.retain(|key| key.tenant_id.as_deref() == Some(admin_tenant));
    }

    if let Some(tenant_id) = tenant_filter.as_deref() {
        keys.retain(|key| key.tenant_id.as_deref() == Some(tenant_id));
    }
    if let Some(user_id) = user_filter.as_deref() {
        keys.retain(|key| key.user_id.as_deref() == Some(user_id));
    }
    if let Some(alias) = alias_filter.as_deref() {
        keys.retain(|key| key.id == alias);
    }

    keys.sort_by(|a, b| a.id.cmp(&b.id));

    let total_count = keys.len();
    let total_pages = total_count.div_ceil(size);

    if offset >= keys.len() {
        keys.clear();
    } else {
        keys.drain(0..offset);
    }
    if keys.len() > size {
        keys.truncate(size);
    }

    let mut out = Vec::<serde_json::Value>::with_capacity(keys.len());
    for key in keys {
        if return_full_object {
            out.push(litellm_key_full_value(&key));
        } else {
            out.push(serde_json::Value::String(key.token));
        }
    }

    Ok(Json(LitellmKeyListResponse {
        keys: out,
        total_count,
        current_page: page,
        total_pages,
    }))
}

// inlined from litellm_keys/regenerate.rs
#[derive(Debug, Deserialize)]
struct LitellmKeyRegenerateRequest {
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    new_key: Option<String>,
    #[serde(default)]
    key_alias: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    team_id: Option<String>,
    #[serde(default)]
    organization_id: Option<String>,
    #[serde(default)]
    models: Option<Vec<String>>,
    #[serde(default)]
    max_budget: Option<f64>,
    #[serde(default)]
    rpm_limit: Option<u32>,
    #[serde(default)]
    tpm_limit: Option<u32>,
    #[serde(default)]
    blocked: Option<bool>,
}

async fn litellm_key_regenerate(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    payload: Option<Json<LitellmKeyRegenerateRequest>>,
) -> Result<Json<LitellmKeyGenerateResponse>, (StatusCode, Json<ErrorResponse>)> {
    litellm_key_regenerate_inner(state, headers, None, payload.map(|Json(v)| v)).await
}

async fn litellm_key_regenerate_path(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Path(path_key): Path<String>,
    payload: Option<Json<LitellmKeyRegenerateRequest>>,
) -> Result<Json<LitellmKeyGenerateResponse>, (StatusCode, Json<ErrorResponse>)> {
    litellm_key_regenerate_inner(state, headers, Some(path_key), payload.map(|Json(v)| v)).await
}

async fn litellm_key_regenerate_inner(
    state: GatewayHttpState,
    headers: HeaderMap,
    path_key: Option<String>,
    payload: Option<LitellmKeyRegenerateRequest>,
) -> Result<Json<LitellmKeyGenerateResponse>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_write(&state, &headers)?;

    let payload = payload.unwrap_or(LitellmKeyRegenerateRequest {
        key: None,
        new_key: None,
        key_alias: None,
        user_id: None,
        team_id: None,
        organization_id: None,
        models: None,
        max_budget: None,
        rpm_limit: None,
        tpm_limit: None,
        blocked: None,
    });

    let token = path_key
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .or_else(|| {
            payload
                .key
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(|v| v.to_string())
        })
        .ok_or_else(|| {
            error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "key is required",
            )
        })?;

    let new_token = if let Some(new_key) = payload
        .new_key
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        if !new_key.starts_with("sk-") {
            return Err(error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "new_key must start with 'sk-'",
            ));
        }
        new_key.to_string()
    } else {
        generate_key_token()
    };

    let new_alias = payload
        .key_alias
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string());

    let tenant_id = payload
        .organization_id
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .or_else(|| {
            payload
                .team_id
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(|v| v.to_string())
        });

    let (key, persisted_keys) = state.gateway.mutate_control_plane(
        |gateway| -> Result<_, (StatusCode, Json<ErrorResponse>)> {
            let keys = gateway.list_virtual_keys();

            let Some(existing) = keys.iter().find(|key| key.token == token).cloned() else {
                return Err(error_response(
                    StatusCode::NOT_FOUND,
                    "not_found",
                    "virtual key not found",
                ));
            };

            if let Some(admin_tenant) = admin.tenant_id.as_deref() {
                if existing.tenant_id.as_deref() != Some(admin_tenant) {
                    return Err(error_response(
                        StatusCode::FORBIDDEN,
                        "forbidden",
                        "cannot regenerate keys for a different tenant",
                    ));
                }
            }

            if keys
                .iter()
                .any(|candidate| candidate.token == new_token && candidate.id != existing.id)
            {
                return Err(error_response(
                    StatusCode::CONFLICT,
                    "conflict",
                    "new_key already exists",
                ));
            }

            let mut key = existing.clone();
            key.token = new_token.clone();

            if let Some(user_id) = payload
                .user_id
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
            {
                key.user_id = Some(user_id.to_string());
            }

            if let Some(tenant_id) = tenant_id.as_deref() {
                if let Some(admin_tenant) = admin.tenant_id.as_deref() {
                    if tenant_id != admin_tenant {
                        return Err(error_response(
                            StatusCode::FORBIDDEN,
                            "forbidden",
                            "cannot regenerate keys for a different tenant",
                        ));
                    }
                }
                key.tenant_id = Some(tenant_id.to_string());
            }

            if let Some(models) = payload.models.as_ref() {
                key.guardrails.allow_models = models.clone();
            }

            if let Some(max_budget) = payload.max_budget {
                if !max_budget.is_finite() || max_budget < 0.0 {
                    return Err(error_response(
                        StatusCode::BAD_REQUEST,
                        "invalid_request",
                        "max_budget must be a non-negative finite number",
                    ));
                }
                let micros = (max_budget * 1_000_000.0).round();
                if micros <= 0.0 {
                    key.budget.total_usd_micros = None;
                } else {
                    key.budget.total_usd_micros = Some(micros as u64);
                }
            }

            if let Some(rpm) = payload.rpm_limit {
                key.limits.rpm = Some(rpm);
            }
            if let Some(tpm) = payload.tpm_limit {
                key.limits.tpm = Some(tpm);
            }

            if let Some(blocked) = payload.blocked {
                key.enabled = !blocked;
            }

            let old_id = &existing.id;
            if let Some(new_id) = new_alias.as_deref() {
                if new_id != old_id {
                    if keys.iter().any(|candidate| candidate.id == new_id) {
                        return Err(error_response(
                            StatusCode::CONFLICT,
                            "conflict",
                            "key_alias already exists",
                        ));
                    }
                    gateway.remove_virtual_key(old_id);
                    key.id = new_id.to_string();
                }
            }

            gateway.upsert_virtual_key(key.clone());
            Ok((key, gateway.list_virtual_keys()))
        },
    )?;
    state.sync_control_plane_from_gateway();

    let _ = persist_virtual_keys(&state, &persisted_keys, "litellm.key.regenerate").await?;

    #[cfg(feature = "sdk")]
    emit_devtools_log(
        &state,
        "litellm.key.regenerate",
        serde_json::json!({
            "key_id": &key.id,
            "tenant_id": key.tenant_id.as_deref(),
        }),
    );

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    append_admin_audit_log(
        &state,
        "litellm.key.regenerate",
        serde_json::json!({
            "key_id": &key.id,
            "tenant_id": key.tenant_id.as_deref(),
            "user_id": key.user_id.as_deref(),
        }),
    )
    .await;

    Ok(Json(litellm_generate_response_from_virtual_key(&key)))
}
// end inline: litellm_keys/regenerate.rs

fn generate_key_id() -> String {
    let ts_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let seq = LITELLM_KEY_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("sk_{ts_ms}_{seq}")
}

fn generate_key_token() -> String {
    let mut bytes = [0u8; 32];
    if getrandom::fill(&mut bytes).is_err() {
        let ts_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0);
        let seq = LITELLM_KEY_SEQ.fetch_add(1, Ordering::Relaxed);
        return format!("sk_fallback_{ts_ms}_{seq}");
    }
    format!("sk-{}", hex_encode(&bytes))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}
// end inline: ../../http/litellm_keys.rs
// inlined from ../../http/router.rs
use axum::Router;
use axum::routing::{any, get, post, put};

fn base_http_router() -> Router<GatewayHttpState> {
    Router::new()
        .route("/health", get(health))
        .route("/metrics", get(metrics))
        .route("/v1/gateway", post(handle_gateway))
        .route(
            "/a2a/:agent_id/.well-known/agent-card.json",
            get(handle_a2a_agent_card),
        )
        .route("/a2a/:agent_id", post(handle_a2a_invoke))
        .route("/a2a/:agent_id/message/send", post(handle_a2a_invoke))
        .route("/a2a/:agent_id/message/stream", post(handle_a2a_invoke))
        .route("/v1/a2a/:agent_id/message/send", post(handle_a2a_invoke))
        .route("/v1/a2a/:agent_id/message/stream", post(handle_a2a_invoke))
        .route("/mcp/tools/list", any(handle_mcp_tools_list))
        .route("/mcp/tools/call", any(handle_mcp_tools_call))
        .route("/mcp", any(handle_mcp_root))
        .route("/mcp/", any(handle_mcp_root))
        .route("/mcp/*subpath", any(handle_mcp_subpath))
        .route("/:mcp_servers/mcp", any(handle_mcp_namespaced_root))
        .route(
            "/:mcp_servers/mcp/*path",
            any(handle_mcp_namespaced_subpath),
        )
        .route("/chat/completions", any(handle_openai_compat_proxy_root))
        .route("/completions", any(handle_openai_compat_proxy_root))
        .route("/embeddings", any(handle_openai_compat_proxy_root))
        .route("/moderations", any(handle_openai_compat_proxy_root))
        .route("/images/generations", any(handle_openai_compat_proxy_root))
        .route(
            "/audio/transcriptions",
            any(handle_openai_compat_proxy_root),
        )
        .route("/audio/translations", any(handle_openai_compat_proxy_root))
        .route("/audio/speech", any(handle_openai_compat_proxy_root))
        .route("/files", any(handle_openai_compat_proxy_root))
        .route("/files/*path", any(handle_openai_compat_proxy))
        .route("/rerank", any(handle_openai_compat_proxy_root))
        .route("/batches", any(handle_openai_compat_proxy_root))
        .route("/batches/*path", any(handle_openai_compat_proxy))
        .route("/models", get(handle_openai_models_list))
        .route("/models/*path", any(handle_openai_compat_proxy))
        .route("/v1/models", get(handle_openai_models_list))
        .route("/responses", any(handle_openai_compat_proxy_root))
        .route("/responses/compact", any(handle_openai_compat_proxy_root))
        .route("/responses/*path", any(handle_openai_compat_proxy))
        .route("/messages", post(handle_anthropic_messages))
        .route(
            "/messages/count_tokens",
            post(handle_anthropic_count_tokens),
        )
        .route("/v1/messages", post(handle_anthropic_messages))
        .route(
            "/v1/messages/count_tokens",
            post(handle_anthropic_count_tokens),
        )
        .route("/v1beta/models/*path", post(handle_google_genai))
        .route("/v1/*path", any(handle_openai_compat_proxy))
        .fallback(handle_fallback)
}

#[cfg(feature = "gateway-metrics-prometheus")]
fn attach_prometheus_http_routes(router: Router<GatewayHttpState>) -> Router<GatewayHttpState> {
    router.route("/metrics/prometheus", get(metrics_prometheus))
}

#[cfg(not(feature = "gateway-metrics-prometheus"))]
fn attach_prometheus_http_routes(router: Router<GatewayHttpState>) -> Router<GatewayHttpState> {
    router
}

fn attach_admin_http_routes(
    mut router: Router<GatewayHttpState>,
    state: &GatewayHttpState,
) -> Router<GatewayHttpState> {
    if state.admin.admin_token.is_some() || state.admin.admin_read_token.is_some() {
        router = router
            .route("/admin/config/version", get(get_config_version))
            .route("/admin/config/versions", get(list_config_versions))
            .route("/admin/config/export", get(export_config))
            .route("/admin/config/validate", post(validate_config_payload))
            .route("/admin/config/diff", get(diff_config_versions))
            .route(
                "/admin/config/versions/:version_id",
                get(get_config_version_by_id),
            );
    }
    if state.admin.admin_token.is_some() {
        router = router
            .route("/admin/config/router", put(upsert_config_router))
            .route("/admin/config/rollback", post(rollback_config_version));
    }

    let mut keys_router = get(list_keys);
    if state.has_admin_write_tokens() {
        keys_router = keys_router.post(upsert_key);
    }
    router = router.route("/admin/keys", keys_router);

    if state.has_admin_write_tokens() {
        router = router.route(
            "/admin/keys/:id",
            put(upsert_key_with_id).delete(delete_key),
        );
    }

    #[cfg(feature = "gateway-proxy-cache")]
    if state.proxy.cache.is_some() && state.admin.admin_token.is_some() {
        router = router.route("/admin/proxy_cache/purge", post(purge_proxy_cache));
    }

    #[cfg(feature = "gateway-routing-advanced")]
    {
        router = router.route("/admin/backends", get(list_backends));
        if state.admin.admin_token.is_some() {
            router = router.route("/admin/backends/:name/reset", post(reset_backend));
        }
    }

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    {
        router = router
            .route("/admin/audit", get(list_audit_logs))
            .route("/admin/audit/export", get(export_audit_logs));
    }

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    {
        router = router
            .route("/admin/budgets", get(list_budget_ledgers))
            .route("/admin/budgets/tenants", get(list_tenant_budget_ledgers))
            .route("/admin/budgets/projects", get(list_project_budget_ledgers))
            .route("/admin/budgets/users", get(list_user_budget_ledgers));

        #[cfg(feature = "gateway-costing")]
        {
            router = router
                .route("/admin/costs", get(list_cost_ledgers))
                .route("/admin/costs/tenants", get(list_tenant_cost_ledgers))
                .route("/admin/costs/projects", get(list_project_cost_ledgers))
                .route("/admin/costs/users", get(list_user_cost_ledgers));
        }

        if state.admin.admin_token.is_some() {
            router = router.route("/admin/reservations/reap", post(reap_reservations));
        }
    }

    router.merge(litellm_key_router())
}

#[cfg(feature = "gateway-routing-advanced")]
fn start_gateway_background_tasks(state: &mut GatewayHttpState) {
    state.proxy.health_check_task = start_proxy_health_checks(state);
}

#[cfg(not(feature = "gateway-routing-advanced"))]
fn start_gateway_background_tasks(_state: &mut GatewayHttpState) {}

pub fn router(state: GatewayHttpState) -> Router {
    let mut state = state;
    let mut router = attach_prometheus_http_routes(base_http_router());

    if state.has_any_admin_tokens() {
        router = attach_admin_http_routes(router, &state);
    }

    start_gateway_background_tasks(&mut state);
    router.with_state(state)
}
// end inline: ../../http/router.rs
// inlined from ../../http/a2a.rs
use serde_json::Map;

#[derive(Clone)]
pub struct A2aAgentState {
    agent_id: String,
    agent_name: Option<String>,
    agent_card_params: Value,
    backend: ProxyBackend,
}

impl A2aAgentState {
    pub fn new(agent_id: String, agent_card_params: Value, backend: ProxyBackend) -> Self {
        let agent_name = agent_card_params
            .as_object()
            .and_then(|obj| obj.get("name"))
            .and_then(|value| value.as_str())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        Self {
            agent_id,
            agent_name,
            agent_card_params,
            backend,
        }
    }

    fn matches(&self, requested: &str) -> bool {
        if self.agent_id == requested {
            return true;
        }
        if let Some(name) = self.agent_name.as_deref() {
            return name == requested;
        }
        false
    }
}

fn forwarded_header(headers: &HeaderMap, name: &str) -> Option<String> {
    let value = headers
        .get(name)
        .and_then(|value| value.to_str().ok())?
        .trim();
    let first = value.split(',').next().unwrap_or("").trim();
    (!first.is_empty()).then(|| first.to_string())
}

fn external_base_url(headers: &HeaderMap) -> Option<String> {
    let host = forwarded_header(headers, "x-forwarded-host")
        .or_else(|| forwarded_header(headers, "host"))?;
    let scheme = forwarded_header(headers, "x-forwarded-proto")
        .or_else(|| forwarded_header(headers, "x-forwarded-scheme"))
        .unwrap_or_else(|| "http".to_string());
    Some(format!("{scheme}://{host}"))
}

fn ensure_a2a_card_defaults(card: &mut Map<String, Value>, requested_agent_id: &str) {
    let name = card
        .get("name")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(requested_agent_id)
        .to_string();

    card.entry("protocolVersion".to_string())
        .or_insert_with(|| Value::String("1.0".to_string()));
    card.entry("name".to_string())
        .or_insert_with(|| Value::String(name.clone()));
    card.entry("description".to_string())
        .or_insert_with(|| Value::String(format!("A2A agent: {name}")));
    card.entry("version".to_string())
        .or_insert_with(|| Value::String("1.0.0".to_string()));
    card.entry("defaultInputModes".to_string())
        .or_insert_with(|| Value::Array(vec![Value::String("text".to_string())]));
    card.entry("defaultOutputModes".to_string())
        .or_insert_with(|| Value::Array(vec![Value::String("text".to_string())]));
    card.entry("capabilities".to_string())
        .or_insert_with(|| serde_json::json!({ "streaming": true }));
    card.entry("skills".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
}

async fn require_virtual_key_if_configured(
    state: &GatewayHttpState,
    headers: &HeaderMap,
) -> Result<bool, axum::response::Response> {
    if !state.uses_virtual_keys() {
        return Ok(false);
    }

    let token = extract_virtual_key(headers).ok_or_else(|| {
        error_response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "missing virtual key",
        )
        .into_response()
    })?;

    let key_enabled = state
        .virtual_key_by_token(&token)
        .ok_or_else(|| {
            error_response(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "unauthorized virtual key",
            )
            .into_response()
        })?
        .enabled;

    if !key_enabled {
        return Err(error_response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "virtual key disabled",
        )
        .into_response());
    }

    Ok(true)
}

fn jsonrpc_error(
    id: Option<Value>,
    code: i64,
    message: impl Into<String>,
) -> axum::response::Response {
    let payload = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(Value::Null),
        "error": { "code": code, "message": message.into() },
    });
    (StatusCode::BAD_REQUEST, Json(payload)).into_response()
}

async fn proxy_a2a_request(
    agent: &A2aAgentState,
    outgoing_headers: HeaderMap,
    body: Bytes,
    method: &str,
) -> Result<reqwest::Response, GatewayError> {
    let retry_path = match method {
        "message/send" => Some("/message/send"),
        "message/stream" => Some("/message/stream"),
        _ => None,
    };

    let (first_headers, first_body, retry_payload) = if retry_path.is_some() {
        (
            outgoing_headers.clone(),
            body.clone(),
            Some((outgoing_headers, body)),
        )
    } else {
        (outgoing_headers, body, None)
    };
    let mut response = agent
        .backend
        .request(reqwest::Method::POST, "", first_headers, Some(first_body))
        .await;

    if let (Ok(resp), Some(path), Some((retry_headers, retry_body))) =
        (&response, retry_path, retry_payload)
    {
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND
            || status == reqwest::StatusCode::METHOD_NOT_ALLOWED
        {
            response = agent
                .backend
                .request(reqwest::Method::POST, path, retry_headers, Some(retry_body))
                .await;
        }
    }

    response
}

async fn handle_a2a_agent_card(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
) -> axum::response::Response {
    let _strip_authorization = match require_virtual_key_if_configured(&state, &headers).await {
        Ok(strip) => strip,
        Err(resp) => return resp,
    };

    let agent = state
        .backends
        .a2a_agents
        .values()
        .find(|agent| agent.matches(&agent_id))
        .cloned();
    let Some(agent) = agent else {
        return error_response(StatusCode::NOT_FOUND, "not_found", "agent not found")
            .into_response();
    };

    let base_url = external_base_url(&headers);
    let url = if let Some(base) = base_url.as_deref() {
        format!("{}/a2a/{}", base.trim_end_matches('/'), agent_id)
    } else {
        format!("/a2a/{agent_id}")
    };

    let mut obj = match agent.agent_card_params {
        Value::Object(obj) => obj,
        _ => Map::new(),
    };

    ensure_a2a_card_defaults(&mut obj, &agent_id);
    obj.insert("url".to_string(), Value::String(url));

    (StatusCode::OK, Json(Value::Object(obj))).into_response()
}

async fn handle_a2a_invoke(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
    body: Body,
) -> axum::response::Response {
    let strip_authorization = match require_virtual_key_if_configured(&state, &headers).await {
        Ok(strip) => strip,
        Err(resp) => return resp,
    };

    let agent = state
        .backends
        .a2a_agents
        .values()
        .find(|agent| agent.matches(&agent_id))
        .cloned();
    let Some(agent) = agent else {
        return jsonrpc_error(None, -32000, format!("Agent '{agent_id}' not found"));
    };

    let body = match to_bytes(body, state.proxy.max_body_bytes).await {
        Ok(bytes) => bytes,
        Err(err) => {
            return jsonrpc_error(None, -32603, format!("Failed to read request body: {err}"));
        }
    };

    let parsed: Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(err) => return jsonrpc_error(None, -32700, format!("Parse error: {err}")),
    };
    let request_id = parsed.get("id").cloned();

    let obj = match parsed.as_object() {
        Some(obj) => obj,
        None => return jsonrpc_error(request_id, -32600, "Invalid Request: expected JSON object"),
    };

    match obj.get("jsonrpc").and_then(|value| value.as_str()) {
        Some("2.0") => {}
        _ => return jsonrpc_error(request_id, -32600, "Invalid Request: jsonrpc must be '2.0'"),
    }

    let method = match obj.get("method").and_then(|value| value.as_str()) {
        Some(method) if !method.trim().is_empty() => method.trim(),
        _ => return jsonrpc_error(request_id, -32600, "Invalid Request: missing method"),
    };

    match method {
        "message/send" | "message/stream" => {}
        _ => return jsonrpc_error(request_id, -32601, format!("Method '{method}' not found")),
    }

    let mut outgoing_headers = headers.clone();
    sanitize_proxy_headers(&mut outgoing_headers, strip_authorization);
    apply_backend_headers(&mut outgoing_headers, agent.backend.headers());

    let upstream = match proxy_a2a_request(&agent, outgoing_headers, body.clone(), method).await {
        Ok(resp) => resp,
        Err(err) => return jsonrpc_error(request_id, -32603, format!("Backend error: {err}")),
    };

    let status = upstream.status();
    let upstream_headers = upstream.headers().clone();
    let content_type = upstream_headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if content_type.starts_with("application/x-ndjson")
        || content_type.starts_with("text/event-stream")
    {
        let mut headers = upstream_headers;
        headers.remove("content-length");
        let stream = upstream
            .bytes_stream()
            .map(|chunk| chunk.map_err(std::io::Error::other))
            .boxed();
        let mut response = axum::response::Response::new(Body::from_stream(stream));
        *response.status_mut() = status;
        *response.headers_mut() = headers;
        return response;
    }

    let bytes = match read_reqwest_body_bytes_bounded_with_content_length(
        upstream,
        &upstream_headers,
        state.proxy.max_body_bytes,
    )
    .await
    {
        Ok(bytes) => bytes,
        Err(err) => {
            return jsonrpc_error(request_id, -32603, format!("Backend response error: {err}"));
        }
    };

    let mut headers = upstream_headers;
    headers.remove("content-length");
    let mut response = axum::response::Response::new(Body::from(bytes));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
}
// end inline: ../../http/a2a.rs
// inlined from ../../http/mcp.rs
const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
const MCP_MAX_REQUEST_BYTES: usize = 2 * 1024 * 1024;
const MCP_MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MCP_MAX_ERROR_RESPONSE_BYTES: usize = 64 * 1024;
const MCP_MAX_ERROR_BODY_SNIPPET_BYTES: usize = 8 * 1024;
const MCP_TOOLS_LIST_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(60);
const MCP_TOOLS_LIST_MAX_PAGES: usize = 8;
const MCP_TOOLS_LIST_MAX_CURSOR_BYTES: usize = 1024;

#[derive(Debug, Clone)]
struct McpToolsListResult {
    tools: Vec<Value>,
    next_cursor: Option<String>,
}

#[derive(Debug, Clone)]
struct McpToolsListCacheEntry {
    expires_at: std::time::Instant,
    tools: Vec<Value>,
    next_cursor: Option<String>,
}

#[derive(Debug, Default)]
struct McpToolsListCache {
    entries: std::collections::HashMap<Option<String>, McpToolsListCacheEntry>,
}

#[derive(Clone)]
pub struct McpServerState {
    server_id: String,
    url: String,
    client: reqwest::Client,
    headers: HeaderMap,
    query_params: BTreeMap<String, String>,
    request_timeout: Option<std::time::Duration>,
    tools_list_cache: std::sync::Arc<tokio::sync::Mutex<McpToolsListCache>>,
}

impl McpServerState {
    pub fn new(server_id: String, url: String) -> Result<Self, GatewayError> {
        let parsed =
            reqwest::Url::parse(url.trim()).map_err(|err| GatewayError::InvalidRequest {
                reason: format!("invalid MCP server url: {err}"),
            })?;
        match parsed.scheme() {
            "http" | "https" => {}
            other => {
                return Err(GatewayError::InvalidRequest {
                    reason: format!("unsupported MCP server url scheme: {other}"),
                });
            }
        }

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .map_err(|err| GatewayError::Backend {
                message: format!("mcp http client error: {err}"),
            })?;

        Ok(Self {
            server_id,
            url: parsed.to_string(),
            client,
            headers: HeaderMap::new(),
            query_params: BTreeMap::new(),
            request_timeout: None,
            tools_list_cache: std::sync::Arc::new(tokio::sync::Mutex::new(
                McpToolsListCache::default(),
            )),
        })
    }

    pub fn server_id(&self) -> &str {
        &self.server_id
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn with_request_timeout_seconds(mut self, timeout_seconds: Option<u64>) -> Self {
        self.request_timeout = timeout_seconds
            .filter(|seconds| *seconds > 0)
            .map(std::time::Duration::from_secs);
        self
    }

    pub fn with_headers(mut self, headers: BTreeMap<String, String>) -> Result<Self, GatewayError> {
        self.headers = parse_headers(&headers)?;
        Ok(self)
    }

    pub fn with_query_params(mut self, params: BTreeMap<String, String>) -> Self {
        self.query_params = normalize_query_params(&params);
        self
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    fn parse_tools_list_result(&self, result: Value) -> Result<McpToolsListResult, GatewayError> {
        match result {
            Value::Array(tools) => Ok(McpToolsListResult {
                tools,
                next_cursor: None,
            }),
            Value::Object(mut obj) => {
                let tools = obj
                    .remove("tools")
                    .and_then(|value| match value {
                        Value::Array(tools) => Some(tools),
                        _ => None,
                    })
                    .ok_or_else(|| GatewayError::Backend {
                        message: format!(
                            "mcp tools/list invalid result for server {}",
                            self.server_id
                        ),
                    })?;
                let next_cursor = obj
                    .remove("nextCursor")
                    .and_then(|value| match value {
                        Value::String(value) => Some(value),
                        _ => None,
                    })
                    .or_else(|| {
                        obj.remove("next_cursor").and_then(|value| match value {
                            Value::String(value) => Some(value),
                            _ => None,
                        })
                    })
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty());

                Ok(McpToolsListResult { tools, next_cursor })
            }
            _ => Err(GatewayError::Backend {
                message: format!(
                    "mcp tools/list invalid result for server {}",
                    self.server_id
                ),
            }),
        }
    }

    async fn list_tools_page_uncached(
        &self,
        cursor: Option<&str>,
    ) -> Result<McpToolsListResult, GatewayError> {
        let mut params = serde_json::json!({});
        if let Some(cursor) = cursor {
            if let Some(obj) = params.as_object_mut() {
                obj.insert("cursor".to_string(), Value::String(cursor.to_string()));
            }
        }
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": params,
        });
        let resp = self.jsonrpc(req).await?;

        if let Some(err) = resp.get("error") {
            return Err(GatewayError::Backend {
                message: format!("mcp tools/list failed for server {}: {err}", self.server_id),
            });
        }

        let result = resp.get("result").cloned().unwrap_or(Value::Null);
        self.parse_tools_list_result(result)
    }

    async fn list_tools_all_uncached(&self) -> Result<Vec<Value>, GatewayError> {
        let mut tools_out = Vec::<Value>::new();
        let mut cursor: Option<String> = None;
        let mut seen_cursors = std::collections::HashSet::<String>::new();

        for _ in 0..MCP_TOOLS_LIST_MAX_PAGES {
            let page = self.list_tools_page_uncached(cursor.as_deref()).await?;
            tools_out.extend(page.tools);
            let Some(next_cursor) = page.next_cursor else {
                return Ok(tools_out);
            };
            if !seen_cursors.insert(next_cursor.clone()) {
                return Err(GatewayError::Backend {
                    message: format!(
                        "mcp tools/list cursor loop detected for server {}",
                        self.server_id
                    ),
                });
            }
            cursor = Some(next_cursor);
        }

        Err(GatewayError::Backend {
            message: format!(
                "mcp tools/list exceeded max pages ({MCP_TOOLS_LIST_MAX_PAGES}) for server {}",
                self.server_id
            ),
        })
    }

    pub async fn list_tools_cached(
        &self,
        cursor: Option<String>,
    ) -> Result<Vec<Value>, GatewayError> {
        Ok(self.list_tools_page_cached(cursor).await?.tools)
    }

    async fn list_tools_page_cached(
        &self,
        cursor: Option<String>,
    ) -> Result<McpToolsListResult, GatewayError> {
        let now = std::time::Instant::now();

        if cursor
            .as_ref()
            .is_some_and(|cursor| cursor.len() > MCP_TOOLS_LIST_MAX_CURSOR_BYTES)
        {
            return Err(GatewayError::InvalidRequest {
                reason: format!("cursor exceeded max bytes ({MCP_TOOLS_LIST_MAX_CURSOR_BYTES})"),
            });
        }

        if let Some(cursor) = cursor.as_deref() {
            return self.list_tools_page_uncached(Some(cursor)).await;
        }

        {
            let cache = self.tools_list_cache.lock().await;
            if let Some(entry) = cache.entries.get(&cursor) {
                if entry.expires_at > now {
                    return Ok(McpToolsListResult {
                        tools: entry.tools.clone(),
                        next_cursor: entry.next_cursor.clone(),
                    });
                }
            }
        }

        let result = McpToolsListResult {
            tools: self.list_tools_all_uncached().await?,
            next_cursor: None,
        };

        {
            let mut cache = self.tools_list_cache.lock().await;
            cache.entries.clear();
            cache.entries.insert(
                cursor,
                McpToolsListCacheEntry {
                    expires_at: now.checked_add(MCP_TOOLS_LIST_CACHE_TTL).unwrap_or(now),
                    tools: result.tools.clone(),
                    next_cursor: result.next_cursor.clone(),
                },
            );
        }

        Ok(result)
    }

    pub async fn jsonrpc(&self, req: Value) -> Result<Value, GatewayError> {
        let mut request = self
            .client
            .post(&self.url)
            .headers(self.headers.clone())
            .json(&req);
        if !self.query_params.is_empty() {
            request = request.query(&self.query_params);
        }
        if let Some(timeout) = self.request_timeout {
            request = request.timeout(timeout);
        }
        let response = request.send().await.map_err(|err| GatewayError::Backend {
            message: format!("mcp request failed: {err}"),
        })?;
        let status = response.status();
        let headers = response.headers().clone();
        let max_bytes = if status.is_success() {
            MCP_MAX_RESPONSE_BYTES
        } else {
            MCP_MAX_ERROR_RESPONSE_BYTES
        };
        let bytes =
            read_reqwest_body_bytes_bounded_with_content_length(response, &headers, max_bytes)
                .await
                .map_err(|err| GatewayError::Backend {
                    message: format!("mcp response read failed: {err}"),
                })?;
        if !status.is_success() {
            let body_slice = bytes.as_ref();
            let truncated = body_slice.len() > MCP_MAX_ERROR_BODY_SNIPPET_BYTES;
            let body_slice = &body_slice[..body_slice.len().min(MCP_MAX_ERROR_BODY_SNIPPET_BYTES)];
            let mut body = String::from_utf8_lossy(body_slice).to_string();
            if truncated {
                body.push('…');
            }

            return Err(GatewayError::Backend {
                message: format!(
                    "mcp server responded with status={} body={}",
                    status.as_u16(),
                    body
                ),
            });
        }
        serde_json::from_slice(&bytes).map_err(|err| GatewayError::Backend {
            message: format!("mcp response is not valid JSON: {err}"),
        })
    }
}

fn parse_headers(headers: &BTreeMap<String, String>) -> Result<HeaderMap, GatewayError> {
    let mut out = HeaderMap::new();
    for (name, value) in headers {
        let header_name =
            name.parse::<axum::http::HeaderName>()
                .map_err(|_| GatewayError::InvalidRequest {
                    reason: format!("invalid header name: {name}"),
                })?;
        let header_value =
            value
                .parse::<axum::http::HeaderValue>()
                .map_err(|_| GatewayError::InvalidRequest {
                    reason: format!("invalid header value for {name}"),
                })?;
        out.insert(header_name, header_value);
    }
    Ok(out)
}

fn normalize_query_params(params: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    params
        .iter()
        .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
        .filter(|(name, _)| !name.is_empty())
        .collect()
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[serde(default)]
    jsonrpc: Option<String>,
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct McpToolsListRequest {
    #[serde(default)]
    servers: Option<Vec<String>>,
    #[serde(default)]
    cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct McpToolsCallRequest {
    name: String,
    #[serde(default)]
    arguments: Value,
    #[serde(default)]
    server_id: Option<String>,
}

async fn handle_mcp_root(
    State(state): State<GatewayHttpState>,
    req: axum::http::Request<Body>,
) -> axum::response::Response {
    handle_mcp_impl(state, None, req).await
}

async fn handle_mcp_subpath(
    State(state): State<GatewayHttpState>,
    Path(subpath): Path<String>,
    req: axum::http::Request<Body>,
) -> axum::response::Response {
    handle_mcp_impl(state, Some(subpath), req).await
}

async fn handle_mcp_namespaced_root(
    State(state): State<GatewayHttpState>,
    Path(servers): Path<String>,
    req: axum::http::Request<Body>,
) -> axum::response::Response {
    handle_mcp_impl(state, Some(servers), req).await
}

async fn handle_mcp_namespaced_subpath(
    State(state): State<GatewayHttpState>,
    Path((servers, _path)): Path<(String, String)>,
    req: axum::http::Request<Body>,
) -> axum::response::Response {
    handle_mcp_impl(state, Some(servers), req).await
}

async fn handle_mcp_tools_list(
    State(state): State<GatewayHttpState>,
    req: axum::http::Request<Body>,
) -> axum::response::Response {
    let (parts, body) = req.into_parts();

    if let Err(resp) = enforce_mcp_auth(&state, &parts.headers).await {
        return resp;
    }

    if parts.method != axum::http::Method::POST && parts.method != axum::http::Method::GET {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }

    let payload: Option<McpToolsListRequest> = if parts.method == axum::http::Method::POST {
        match to_bytes(body, MCP_MAX_REQUEST_BYTES).await {
            Ok(bytes) if bytes.is_empty() => None,
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(parsed) => Some(parsed),
                Err(_) => {
                    return error_response(
                        StatusCode::BAD_REQUEST,
                        "invalid_json",
                        "invalid JSON body",
                    )
                    .into_response();
                }
            },
            Err(_) => {
                return error_response(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "request_too_large",
                    format!("request exceeded max bytes ({MCP_MAX_REQUEST_BYTES})"),
                )
                .into_response();
            }
        }
    } else {
        None
    };

    let cursor = payload.as_ref().and_then(|p| p.cursor.clone());
    let server_ids = match resolve_requested_mcp_servers(
        &state,
        payload.and_then(|p| p.servers),
        &parts.headers,
        None,
    ) {
        Ok(ids) => ids,
        Err(resp) => return *resp,
    };

    match mcp_list_tools(&state, &server_ids, cursor).await {
        Ok(result) => Json(result).into_response(),
        Err(err) => map_mcp_gateway_error(err).into_response(),
    }
}

async fn handle_mcp_tools_call(
    State(state): State<GatewayHttpState>,
    req: axum::http::Request<Body>,
) -> axum::response::Response {
    let (parts, body) = req.into_parts();

    if let Err(resp) = enforce_mcp_auth(&state, &parts.headers).await {
        return resp;
    }

    if parts.method != axum::http::Method::POST {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }

    let bytes = match to_bytes(body, MCP_MAX_REQUEST_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return error_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                "request_too_large",
                format!("request exceeded max bytes ({MCP_MAX_REQUEST_BYTES})"),
            )
            .into_response();
        }
    };
    let parsed: McpToolsCallRequest = match serde_json::from_slice(&bytes) {
        Ok(parsed) => parsed,
        Err(_) => {
            return error_response(StatusCode::BAD_REQUEST, "invalid_json", "invalid JSON body")
                .into_response();
        }
    };

    let server_ids = match resolve_requested_mcp_servers(
        &state,
        parsed.server_id.clone().map(|id| vec![id]),
        &parts.headers,
        None,
    ) {
        Ok(ids) => ids,
        Err(resp) => return *resp,
    };

    match mcp_call_tool(&state, &server_ids, &parsed.name, parsed.arguments).await {
        Ok(result) => Json(result).into_response(),
        Err(err) => map_mcp_gateway_error(err).into_response(),
    }
}

async fn handle_mcp_impl(
    state: GatewayHttpState,
    selector: Option<String>,
    req: axum::http::Request<Body>,
) -> axum::response::Response {
    let (parts, body) = req.into_parts();

    if let Err(resp) = enforce_mcp_auth(&state, &parts.headers).await {
        return resp;
    }

    if parts.method != axum::http::Method::POST {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }

    let bytes = match to_bytes(body, MCP_MAX_REQUEST_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => {
            let message = format!("request exceeded max bytes ({MCP_MAX_REQUEST_BYTES})");
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(mcp_jsonrpc_error(Value::Null, -32600, &message)),
            )
                .into_response();
        }
    };
    if bytes.is_empty() {
        return Json(mcp_jsonrpc_error(Value::Null, -32600, "Invalid Request")).into_response();
    }

    let value: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => {
            return Json(mcp_jsonrpc_error(Value::Null, -32700, "Parse error")).into_response();
        }
    };
    if value.is_array() {
        return Json(mcp_jsonrpc_error(
            Value::Null,
            -32600,
            "Batch requests are not supported",
        ))
        .into_response();
    }

    let rpc: JsonRpcRequest = match serde_json::from_value(value) {
        Ok(rpc) => rpc,
        Err(_) => {
            return Json(mcp_jsonrpc_error(Value::Null, -32600, "Invalid Request")).into_response();
        }
    };

    let Some(id) = rpc.id.clone() else {
        return StatusCode::NO_CONTENT.into_response();
    };

    if rpc.jsonrpc.as_deref() != Some("2.0") {
        return Json(mcp_jsonrpc_error(id, -32600, "Invalid Request")).into_response();
    }

    match rpc.method.as_str() {
        "initialize" => {
            let result = serde_json::json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "serverInfo": {
                    "name": "ditto-gateway",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "capabilities": {
                    "tools": {},
                },
            });
            Json(mcp_jsonrpc_result(id, result)).into_response()
        }
        "notifications/initialized" => StatusCode::NO_CONTENT.into_response(),
        "tools/list" => {
            let server_ids = match resolve_requested_mcp_servers_jsonrpc(
                &state,
                &parts.headers,
                selector.as_deref(),
            ) {
                Ok(ids) => ids,
                Err(reason) => {
                    return Json(mcp_jsonrpc_error(id, -32602, &reason)).into_response();
                }
            };
            let cursor = rpc
                .params
                .as_ref()
                .and_then(|params| params.get("cursor"))
                .and_then(|value| value.as_str())
                .map(|value| value.to_string());
            match mcp_list_tools(&state, &server_ids, cursor).await {
                Ok(result) => Json(mcp_jsonrpc_result(id, result)).into_response(),
                Err(GatewayError::InvalidRequest { reason }) => {
                    Json(mcp_jsonrpc_error(id, -32602, &reason)).into_response()
                }
                Err(err) => Json(mcp_jsonrpc_error(id, -32000, &err.to_string())).into_response(),
            }
        }
        "tools/call" => {
            let Some(params) = rpc.params.as_ref() else {
                return Json(mcp_jsonrpc_error(id, -32602, "Invalid params")).into_response();
            };
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            if name.trim().is_empty() {
                return Json(mcp_jsonrpc_error(id, -32602, "Invalid params")).into_response();
            }
            let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);
            let server_ids = match resolve_requested_mcp_servers_jsonrpc(
                &state,
                &parts.headers,
                selector.as_deref(),
            ) {
                Ok(ids) => ids,
                Err(reason) => {
                    return Json(mcp_jsonrpc_error(id, -32602, &reason)).into_response();
                }
            };
            match mcp_call_tool(&state, &server_ids, name, arguments).await {
                Ok(result) => Json(mcp_jsonrpc_result(id, result)).into_response(),
                Err(GatewayError::InvalidRequest { reason }) => {
                    Json(mcp_jsonrpc_error(id, -32602, &reason)).into_response()
                }
                Err(err) => Json(mcp_jsonrpc_error(id, -32000, &err.to_string())).into_response(),
            }
        }
        _ => Json(mcp_jsonrpc_error(id, -32601, "Method not found")).into_response(),
    }
}

async fn enforce_mcp_auth(
    state: &GatewayHttpState,
    headers: &HeaderMap,
) -> Result<(), axum::response::Response> {
    if !state.uses_virtual_keys() {
        return Ok(());
    }
    let token =
        extract_virtual_key(headers).ok_or_else(|| StatusCode::UNAUTHORIZED.into_response())?;
    let key = state
        .virtual_key_by_token(&token)
        .ok_or_else(|| StatusCode::UNAUTHORIZED.into_response())?;
    if !key.enabled {
        return Err(StatusCode::UNAUTHORIZED.into_response());
    }
    state.record_request();
    Ok(())
}

fn map_mcp_gateway_error(err: GatewayError) -> (StatusCode, Json<ErrorResponse>) {
    match err {
        GatewayError::Unauthorized => {
            error_response(StatusCode::UNAUTHORIZED, "unauthorized", err.to_string())
        }
        GatewayError::RateLimited { limit } => error_response(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            format!("rate limit exceeded: {limit}"),
        ),
        GatewayError::GuardrailRejected { reason } => error_response(
            StatusCode::FORBIDDEN,
            "guardrail_rejected",
            format!("guardrail rejected: {reason}"),
        ),
        GatewayError::BudgetExceeded { limit, attempted } => error_response(
            StatusCode::PAYMENT_REQUIRED,
            "budget_exceeded",
            format!("budget exceeded: limit={limit} attempted={attempted}"),
        ),
        GatewayError::CostBudgetExceeded {
            limit_usd_micros,
            attempted_usd_micros,
        } => error_response(
            StatusCode::PAYMENT_REQUIRED,
            "cost_budget_exceeded",
            format!(
                "cost budget exceeded: limit_usd_micros={limit_usd_micros} attempted_usd_micros={attempted_usd_micros}"
            ),
        ),
        GatewayError::BackendNotFound { name } => error_response(
            StatusCode::NOT_FOUND,
            "backend_not_found",
            format!("backend not found: {name}"),
        ),
        GatewayError::Backend { message } => {
            error_response(StatusCode::BAD_GATEWAY, "mcp_backend_error", message)
        }
        GatewayError::BackendTimeout { message } => {
            error_response(StatusCode::GATEWAY_TIMEOUT, "mcp_backend_timeout", message)
        }
        GatewayError::InvalidRequest { reason } => {
            error_response(StatusCode::BAD_REQUEST, "invalid_request", reason)
        }
    }
}

fn resolve_requested_mcp_servers(
    state: &GatewayHttpState,
    servers: Option<Vec<String>>,
    headers: &HeaderMap,
    selector: Option<String>,
) -> Result<Vec<String>, Box<axum::response::Response>> {
    let selector = selector
        .as_deref()
        .and_then(|value| value.split('/').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());

    let header_selector = extract_header(headers, "x-mcp-servers");

    let mut requested = if let Some(servers) = servers {
        servers
    } else if let Some(selector) = selector.or(header_selector) {
        selector
            .split(',')
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect()
    } else {
        let mut all: Vec<String> = state.backends.mcp_servers.keys().cloned().collect();
        all.sort();
        all
    };

    requested.sort();
    requested.dedup();
    if requested.is_empty() {
        let message = if state.backends.mcp_servers.is_empty() {
            "no MCP servers configured"
        } else {
            "no MCP servers selected"
        };
        return Err(Box::new(
            error_response(StatusCode::BAD_REQUEST, "invalid_request", message).into_response(),
        ));
    }
    for server_id in &requested {
        if !state.backends.mcp_servers.contains_key(server_id) {
            return Err(Box::new(
                error_response(
                    StatusCode::NOT_FOUND,
                    "not_found",
                    format!("mcp server not found: {server_id}"),
                )
                .into_response(),
            ));
        }
    }
    Ok(requested)
}

fn resolve_requested_mcp_servers_jsonrpc(
    state: &GatewayHttpState,
    headers: &HeaderMap,
    selector: Option<&str>,
) -> Result<Vec<String>, String> {
    let selector = selector
        .and_then(|value| value.split('/').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());

    let header_selector = extract_header(headers, "x-mcp-servers");

    let mut requested = if let Some(selector) = selector.or(header_selector) {
        selector
            .split(',')
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect()
    } else {
        let mut all: Vec<String> = state.backends.mcp_servers.keys().cloned().collect();
        all.sort();
        all
    };

    requested.sort();
    requested.dedup();
    if requested.is_empty() {
        let message = if state.backends.mcp_servers.is_empty() {
            "no MCP servers configured"
        } else {
            "no MCP servers selected"
        };
        return Err(message.to_string());
    }
    for server_id in &requested {
        if !state.backends.mcp_servers.contains_key(server_id) {
            return Err(format!("mcp server not found: {server_id}"));
        }
    }
    Ok(requested)
}

async fn mcp_list_tools(
    state: &GatewayHttpState,
    server_ids: &[String],
    cursor: Option<String>,
) -> Result<Value, GatewayError> {
    if cursor.is_some() && server_ids.len() != 1 {
        return Err(GatewayError::InvalidRequest {
            reason: "cursor is only supported when selecting a single MCP server".to_string(),
        });
    }
    let prefix_names = server_ids.len() > 1;

    let mut futures = Vec::with_capacity(server_ids.len());
    for server_id in server_ids {
        let server = state.backends.mcp_servers.get(server_id).ok_or_else(|| {
            GatewayError::InvalidRequest {
                reason: format!("unknown MCP server: {server_id}"),
            }
        })?;
        let server = server.clone();
        let server_id = server_id.clone();
        let cursor = cursor.clone();
        futures.push(async move {
            let result = server.list_tools_page_cached(cursor).await?;
            Ok::<_, GatewayError>((server_id, result))
        });
    }

    let mut next_cursor_out: Option<String> = None;
    let mut tools_out: Vec<Value> = Vec::new();
    for (server_id, result) in futures_util::future::try_join_all(futures).await? {
        if server_ids.len() == 1 {
            next_cursor_out = result.next_cursor;
        }
        for tool in result.tools {
            let mut tool = tool;
            if prefix_names {
                if let Some(obj) = tool.as_object_mut() {
                    if let Some(Value::String(name)) = obj.get("name").cloned() {
                        obj.insert(
                            "name".to_string(),
                            Value::String(format!("{server_id}-{name}")),
                        );
                    }
                }
            }
            tools_out.push(tool);
        }
    }

    let mut out = serde_json::Map::new();
    out.insert("tools".to_string(), Value::Array(tools_out));
    if let Some(next_cursor) = next_cursor_out {
        out.insert("nextCursor".to_string(), Value::String(next_cursor));
    }

    Ok(Value::Object(out))
}

async fn mcp_call_tool(
    state: &GatewayHttpState,
    server_ids: &[String],
    name: &str,
    arguments: Value,
) -> Result<Value, GatewayError> {
    let (server_id, tool_name) = if server_ids.len() == 1 {
        (server_ids[0].as_str(), name)
    } else {
        let mut best_match: Option<(&str, &str, usize)> = None;
        for server_id in server_ids {
            let Some(tool_name) = name
                .strip_prefix(server_id.as_str())
                .and_then(|tail| tail.strip_prefix('-'))
            else {
                continue;
            };
            let candidate_len = server_id.len();
            match best_match {
                Some((_, _, best_len)) if candidate_len <= best_len => {}
                _ => best_match = Some((server_id.as_str(), tool_name, candidate_len)),
            }
        }

        let Some((server_id, tool_name, _)) = best_match else {
            return Err(GatewayError::InvalidRequest {
                reason: "ambiguous tool name; expected <server_id>-<tool_name>".to_string(),
            });
        };
        if tool_name.is_empty() {
            return Err(GatewayError::InvalidRequest {
                reason: "ambiguous tool name; expected <server_id>-<tool_name>".to_string(),
            });
        }
        (server_id, tool_name)
    };
    if tool_name.trim().is_empty() {
        return Err(GatewayError::InvalidRequest {
            reason: "invalid tool name; expected non-empty name".to_string(),
        });
    }

    let server =
        state
            .backends
            .mcp_servers
            .get(server_id)
            .ok_or_else(|| GatewayError::InvalidRequest {
                reason: format!("unknown MCP server: {server_id}"),
            })?;

    let req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": tool_name,
            "arguments": arguments,
        },
    });
    let resp = server.jsonrpc(req).await?;

    if let Some(err) = resp.get("error") {
        return Err(GatewayError::Backend {
            message: format!("mcp tool call failed: {err}"),
        });
    }

    Ok(resp.get("result").cloned().unwrap_or(Value::Null))
}

fn mcp_jsonrpc_result(id: Value, result: Value) -> Value {
    serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn mcp_jsonrpc_error(id: Value, code: i64, message: &str) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message,
        }
    })
}
// end inline: ../../http/mcp.rs
