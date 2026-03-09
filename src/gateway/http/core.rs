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

include!("core/diagnostics.rs");

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
