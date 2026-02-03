use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path as StdPath, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(feature = "gateway-metrics-prometheus")]
use std::time::Instant;

use axum::body::{Body, to_bytes};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{any, get, post, put};
use axum::{Json, Router};
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
use super::proxy_cache::{CachedProxyResponse, ProxyCacheConfig, ProxyResponseCache};

#[cfg(feature = "gateway-metrics-prometheus")]
use super::metrics_prometheus::{PrometheusMetrics, PrometheusMetricsConfig};

#[cfg(feature = "gateway-routing-advanced")]
use super::proxy_routing::{BackendHealth, BackendHealthSnapshot, FailureKind, ProxyRoutingConfig};

#[cfg(feature = "gateway-store-sqlite")]
use super::{SqliteStore, SqliteStoreError};

#[cfg(feature = "gateway-store-redis")]
use super::{RedisStore, RedisStoreError};

#[cfg(feature = "gateway-tokenizer")]
use super::token_count;

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
use super::{AuditLogRecord, BudgetLedgerRecord};

#[cfg(all(
    feature = "gateway-costing",
    any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
))]
use super::CostLedgerRecord;

use super::interop;
use super::responses_shim;
#[cfg(feature = "gateway-translation")]
use super::translation;
use super::{
    Gateway, GatewayError, GatewayRequest, GatewayResponse, GatewayStateFile,
    ObservabilitySnapshot, ProxyBackend, VirtualKeyConfig,
};

static REQUEST_ID_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
struct AdminTenantToken {
    tenant_id: String,
    token: String,
    read_only: bool,
}

#[derive(Clone)]
pub struct GatewayHttpState {
    gateway: Arc<Mutex<Gateway>>,
    proxy_backends: Arc<HashMap<String, ProxyBackend>>,
    a2a_agents: Arc<HashMap<String, A2aAgentState>>,
    mcp_servers: Arc<HashMap<String, McpServerState>>,
    #[cfg(feature = "gateway-translation")]
    translation_backends: Arc<HashMap<String, super::TranslationBackend>>,
    admin_token: Option<String>,
    admin_read_token: Option<String>,
    admin_tenant_tokens: Vec<AdminTenantToken>,
    state_file: Option<PathBuf>,
    #[cfg(feature = "gateway-store-sqlite")]
    sqlite_store: Option<SqliteStore>,
    #[cfg(feature = "gateway-store-redis")]
    redis_store: Option<RedisStore>,
    #[cfg(feature = "gateway-costing")]
    pricing: Option<Arc<PricingTable>>,
    #[cfg(feature = "gateway-proxy-cache")]
    proxy_cache: Option<Arc<Mutex<ProxyResponseCache>>>,
    #[cfg(feature = "gateway-proxy-cache")]
    proxy_cache_config: Option<ProxyCacheConfig>,
    proxy_max_body_bytes: usize,
    proxy_usage_max_body_bytes: usize,
    proxy_backpressure: Option<Arc<Semaphore>>,
    proxy_backend_backpressure: Arc<HashMap<String, Arc<Semaphore>>>,
    #[cfg(feature = "gateway-metrics-prometheus")]
    prometheus_metrics: Option<Arc<Mutex<PrometheusMetrics>>>,
    #[cfg(feature = "gateway-routing-advanced")]
    proxy_routing: Option<ProxyRoutingConfig>,
    #[cfg(feature = "gateway-routing-advanced")]
    proxy_backend_health: Option<Arc<Mutex<HashMap<String, BackendHealth>>>>,
    #[cfg(feature = "gateway-routing-advanced")]
    proxy_health_check_task: Option<Arc<AbortOnDrop>>,
    json_logs: bool,
    #[cfg(feature = "sdk")]
    devtools: Option<DevtoolsLogger>,
}

impl GatewayHttpState {
    pub fn new(gateway: Gateway) -> Self {
        let mut proxy_backend_backpressure: HashMap<String, Arc<Semaphore>> = HashMap::new();
        for backend in &gateway.config.backends {
            let Some(max_in_flight) = backend.max_in_flight else {
                continue;
            };
            proxy_backend_backpressure.insert(
                backend.name.clone(),
                Arc::new(Semaphore::new(max_in_flight.max(1))),
            );
        }

        Self {
            gateway: Arc::new(Mutex::new(gateway)),
            proxy_backends: Arc::new(HashMap::new()),
            a2a_agents: Arc::new(HashMap::new()),
            mcp_servers: Arc::new(HashMap::new()),
            #[cfg(feature = "gateway-translation")]
            translation_backends: Arc::new(HashMap::new()),
            admin_token: None,
            admin_read_token: None,
            admin_tenant_tokens: Vec::new(),
            state_file: None,
            #[cfg(feature = "gateway-store-sqlite")]
            sqlite_store: None,
            #[cfg(feature = "gateway-store-redis")]
            redis_store: None,
            #[cfg(feature = "gateway-costing")]
            pricing: None,
            #[cfg(feature = "gateway-proxy-cache")]
            proxy_cache: None,
            #[cfg(feature = "gateway-proxy-cache")]
            proxy_cache_config: None,
            proxy_max_body_bytes: 64 * 1024 * 1024,
            proxy_usage_max_body_bytes: 1024 * 1024,
            proxy_backpressure: None,
            proxy_backend_backpressure: Arc::new(proxy_backend_backpressure),
            #[cfg(feature = "gateway-metrics-prometheus")]
            prometheus_metrics: None,
            #[cfg(feature = "gateway-routing-advanced")]
            proxy_routing: None,
            #[cfg(feature = "gateway-routing-advanced")]
            proxy_backend_health: None,
            #[cfg(feature = "gateway-routing-advanced")]
            proxy_health_check_task: None,
            json_logs: false,
            #[cfg(feature = "sdk")]
            devtools: None,
        }
    }

    fn has_any_admin_tokens(&self) -> bool {
        self.admin_token.is_some()
            || self.admin_read_token.is_some()
            || !self.admin_tenant_tokens.is_empty()
    }

    fn has_admin_write_tokens(&self) -> bool {
        self.admin_token.is_some()
            || self
                .admin_tenant_tokens
                .iter()
                .any(|binding| !binding.read_only)
    }

    pub fn with_admin_token(mut self, token: impl Into<String>) -> Self {
        self.admin_token = Some(token.into());
        self
    }

    pub fn with_admin_read_token(mut self, token: impl Into<String>) -> Self {
        self.admin_read_token = Some(token.into());
        self
    }

    pub fn with_admin_tenant_token(
        mut self,
        tenant_id: impl Into<String>,
        token: impl Into<String>,
    ) -> Self {
        self.admin_tenant_tokens.push(AdminTenantToken {
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
        self.admin_tenant_tokens.push(AdminTenantToken {
            tenant_id: tenant_id.into(),
            token: token.into(),
            read_only: true,
        });
        self
    }

    pub fn with_proxy_backends(mut self, backends: HashMap<String, ProxyBackend>) -> Self {
        self.proxy_backends = Arc::new(backends);
        self
    }

    pub fn with_a2a_agents(mut self, agents: HashMap<String, A2aAgentState>) -> Self {
        self.a2a_agents = Arc::new(agents);
        self
    }

    pub fn with_mcp_servers(mut self, servers: HashMap<String, McpServerState>) -> Self {
        self.mcp_servers = Arc::new(servers);
        self
    }

    pub fn with_proxy_max_body_bytes(mut self, max_body_bytes: usize) -> Self {
        self.proxy_max_body_bytes = max_body_bytes.max(1);
        self
    }

    pub fn with_proxy_usage_max_body_bytes(mut self, max_body_bytes: usize) -> Self {
        self.proxy_usage_max_body_bytes = max_body_bytes;
        self
    }

    #[cfg(feature = "gateway-translation")]
    pub fn with_translation_backends(
        mut self,
        backends: HashMap<String, super::TranslationBackend>,
    ) -> Self {
        self.translation_backends = Arc::new(backends);
        self
    }

    pub fn with_state_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.state_file = Some(path.into());
        self
    }

    #[cfg(feature = "gateway-store-sqlite")]
    pub fn with_sqlite_store(mut self, store: SqliteStore) -> Self {
        self.sqlite_store = Some(store);
        self
    }

    #[cfg(feature = "gateway-store-redis")]
    pub fn with_redis_store(mut self, store: RedisStore) -> Self {
        self.redis_store = Some(store);
        self
    }

    #[cfg(feature = "gateway-costing")]
    pub fn with_pricing_table(mut self, pricing: PricingTable) -> Self {
        self.pricing = Some(Arc::new(pricing));
        self
    }

    pub fn with_json_logs(mut self) -> Self {
        self.json_logs = true;
        self
    }

    #[cfg(feature = "gateway-proxy-cache")]
    pub fn with_proxy_cache(mut self, config: ProxyCacheConfig) -> Self {
        self.proxy_cache = Some(Arc::new(Mutex::new(ProxyResponseCache::new(
            config.clone(),
        ))));
        self.proxy_cache_config = Some(config);
        self
    }

    pub fn with_proxy_max_in_flight(mut self, max_in_flight: usize) -> Self {
        self.proxy_backpressure = Some(Arc::new(Semaphore::new(max_in_flight.max(1))));
        self
    }

    #[cfg(feature = "gateway-metrics-prometheus")]
    pub fn with_prometheus_metrics(mut self, config: PrometheusMetricsConfig) -> Self {
        self.prometheus_metrics = Some(Arc::new(Mutex::new(PrometheusMetrics::new(config))));
        self
    }

    #[cfg(feature = "gateway-routing-advanced")]
    pub fn with_proxy_routing(mut self, config: ProxyRoutingConfig) -> Self {
        self.proxy_routing = Some(config);
        self.proxy_backend_health = Some(Arc::new(Mutex::new(HashMap::new())));
        self
    }

    #[cfg(feature = "sdk")]
    pub fn with_devtools_logger(mut self, logger: DevtoolsLogger) -> Self {
        self.devtools = Some(logger);
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

pub fn router(state: GatewayHttpState) -> Router {
    #[cfg(feature = "gateway-routing-advanced")]
    let mut state = state;
    #[cfg(not(feature = "gateway-routing-advanced"))]
    let state = state;
    let mut router = Router::new()
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
        .route("/:mcp_servers/mcp/*path", any(handle_mcp_namespaced_subpath))
        .route("/chat/completions", any(handle_openai_compat_proxy_root))
        .route("/completions", any(handle_openai_compat_proxy_root))
        .route("/embeddings", any(handle_openai_compat_proxy_root))
        .route("/moderations", any(handle_openai_compat_proxy_root))
        .route("/images/generations", any(handle_openai_compat_proxy_root))
        .route("/audio/transcriptions", any(handle_openai_compat_proxy_root))
        .route("/audio/translations", any(handle_openai_compat_proxy_root))
        .route("/audio/speech", any(handle_openai_compat_proxy_root))
        .route("/files", any(handle_openai_compat_proxy_root))
        .route("/files/*path", any(handle_openai_compat_proxy))
        .route("/rerank", any(handle_openai_compat_proxy_root))
        .route("/batches", any(handle_openai_compat_proxy_root))
        .route("/batches/*path", any(handle_openai_compat_proxy))
        .route("/models", any(handle_openai_compat_proxy_root))
        .route("/models/*path", any(handle_openai_compat_proxy))
        .route("/responses", any(handle_openai_compat_proxy_root))
        .route("/responses/compact", any(handle_openai_compat_proxy_root))
        .route("/responses/*path", any(handle_openai_compat_proxy))
        .route("/v1/messages", post(handle_anthropic_messages))
        .route(
            "/v1/messages/count_tokens",
            post(handle_anthropic_count_tokens),
        )
        .route("/v1beta/models/*path", post(handle_google_genai))
        .route("/v1/*path", any(handle_openai_compat_proxy))
        .fallback(handle_fallback);

    #[cfg(feature = "gateway-metrics-prometheus")]
    {
        router = router.route("/metrics/prometheus", get(metrics_prometheus));
    }

    if state.has_any_admin_tokens() {
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
        if state.proxy_cache.is_some() && state.admin_token.is_some() {
            router = router.route("/admin/proxy_cache/purge", post(purge_proxy_cache));
        }

        #[cfg(feature = "gateway-routing-advanced")]
        {
            router = router.route("/admin/backends", get(list_backends));
            if state.admin_token.is_some() {
                router = router.route("/admin/backends/:name/reset", post(reset_backend));
            }
        }

        #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
        {
            router = router
                .route("/admin/audit", get(list_audit_logs))
                .route("/admin/audit/export", get(export_audit_logs))
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

            if state.admin_token.is_some() {
                router = router.route("/admin/reservations/reap", post(reap_reservations));
            }
        }

        router = router.merge(litellm_key_router());
    }

    #[cfg(feature = "gateway-routing-advanced")]
    {
        state.proxy_health_check_task = start_proxy_health_checks(&state);
    }

    router.with_state(state)
}

include!("core/diagnostics.rs");

#[cfg(feature = "gateway-metrics-prometheus")]
async fn metrics_prometheus(
    State(state): State<GatewayHttpState>,
) -> Result<(StatusCode, HeaderMap, String), (StatusCode, Json<ErrorResponse>)> {
    let Some(metrics) = state.prometheus_metrics.as_ref() else {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            "not_configured",
            "prometheus metrics not enabled",
        ));
    };

    let rendered = { metrics.lock().await.render() };
    let mut headers = HeaderMap::new();
    headers.insert(
        "content-type",
        "text/plain; version=0.0.4"
            .parse()
            .unwrap_or_else(|_| "text/plain".parse().unwrap()),
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
        .or_else(|| extract_bearer(&headers))
        .or_else(|| extract_header(&headers, "x-ditto-virtual-key"))
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

    let (_virtual_key_id, result) = {
        let mut gateway = state.gateway.lock().await;
        let virtual_key_id = gateway
            .config
            .virtual_key(&request.virtual_key)
            .map(|key| key.id.clone());
        (virtual_key_id, gateway.handle(request).await)
    };

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.sqlite_store.as_ref() {
        if let Some(virtual_key_id) = _virtual_key_id.as_deref() {
            let _ = store.record_spent_tokens(virtual_key_id, tokens).await;
            let _ = store
                .append_audit_log(
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
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.redis_store.as_ref() {
        if let Some(virtual_key_id) = _virtual_key_id.as_deref() {
            let _ = store.record_spent_tokens(virtual_key_id, tokens).await;
            let _ = store
                .append_audit_log(
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

async fn gateway_uses_virtual_keys(state: &GatewayHttpState) -> bool {
    let gateway = state.gateway.lock().await;
    !gateway.config.virtual_keys.is_empty()
}

fn synthesize_bearer_header(token: &str) -> Option<axum::http::HeaderValue> {
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    axum::http::HeaderValue::from_str(&format!("Bearer {token}")).ok()
}
