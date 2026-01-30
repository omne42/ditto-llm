use std::collections::HashMap;
use std::path::{Path as StdPath, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::body::{Body, to_bytes};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{any, get, post, put};
use axum::{Json, Router};
use bytes::Bytes;
use futures_util::StreamExt;
use futures_util::stream::BoxStream;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};

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

use super::responses_shim;
#[cfg(feature = "gateway-translation")]
use super::translation;
use super::{
    Gateway, GatewayError, GatewayRequest, GatewayResponse, GatewayStateFile,
    ObservabilitySnapshot, ProxyBackend, VirtualKeyConfig,
};

static REQUEST_ID_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
pub struct GatewayHttpState {
    gateway: Arc<Mutex<Gateway>>,
    proxy_backends: Arc<HashMap<String, ProxyBackend>>,
    #[cfg(feature = "gateway-translation")]
    translation_backends: Arc<HashMap<String, super::TranslationBackend>>,
    admin_token: Option<String>,
    state_file: Option<PathBuf>,
    #[cfg(feature = "gateway-store-sqlite")]
    sqlite_store: Option<SqliteStore>,
    #[cfg(feature = "gateway-store-redis")]
    redis_store: Option<RedisStore>,
    #[cfg(feature = "gateway-costing")]
    pricing: Option<Arc<PricingTable>>,
    #[cfg(feature = "gateway-proxy-cache")]
    proxy_cache: Option<Arc<Mutex<ProxyResponseCache>>>,
    proxy_backpressure: Option<Arc<Semaphore>>,
    #[cfg(feature = "gateway-metrics-prometheus")]
    prometheus_metrics: Option<Arc<Mutex<PrometheusMetrics>>>,
    #[cfg(feature = "gateway-routing-advanced")]
    proxy_routing: Option<ProxyRoutingConfig>,
    #[cfg(feature = "gateway-routing-advanced")]
    proxy_backend_health: Option<Arc<Mutex<HashMap<String, BackendHealth>>>>,
    json_logs: bool,
    #[cfg(feature = "sdk")]
    devtools: Option<DevtoolsLogger>,
}

impl GatewayHttpState {
    pub fn new(gateway: Gateway) -> Self {
        Self {
            gateway: Arc::new(Mutex::new(gateway)),
            proxy_backends: Arc::new(HashMap::new()),
            #[cfg(feature = "gateway-translation")]
            translation_backends: Arc::new(HashMap::new()),
            admin_token: None,
            state_file: None,
            #[cfg(feature = "gateway-store-sqlite")]
            sqlite_store: None,
            #[cfg(feature = "gateway-store-redis")]
            redis_store: None,
            #[cfg(feature = "gateway-costing")]
            pricing: None,
            #[cfg(feature = "gateway-proxy-cache")]
            proxy_cache: None,
            proxy_backpressure: None,
            #[cfg(feature = "gateway-metrics-prometheus")]
            prometheus_metrics: None,
            #[cfg(feature = "gateway-routing-advanced")]
            proxy_routing: None,
            #[cfg(feature = "gateway-routing-advanced")]
            proxy_backend_health: None,
            json_logs: false,
            #[cfg(feature = "sdk")]
            devtools: None,
        }
    }

    pub fn with_admin_token(mut self, token: impl Into<String>) -> Self {
        self.admin_token = Some(token.into());
        self
    }

    pub fn with_proxy_backends(mut self, backends: HashMap<String, ProxyBackend>) -> Self {
        self.proxy_backends = Arc::new(backends);
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
        self.proxy_cache = Some(Arc::new(Mutex::new(ProxyResponseCache::new(config))));
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

#[derive(Debug, Deserialize)]
struct ListKeysQuery {
    #[serde(default)]
    include_tokens: bool,
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
    let mut router = Router::new()
        .route("/health", get(health))
        .route("/metrics", get(metrics))
        .route("/v1/gateway", post(handle_gateway))
        .route("/v1/*path", any(handle_openai_compat_proxy));

    #[cfg(feature = "gateway-metrics-prometheus")]
    {
        router = router.route("/metrics/prometheus", get(metrics_prometheus));
    }

    if state.admin_token.is_some() {
        router = router
            .route("/admin/keys", get(list_keys).post(upsert_key))
            .route(
                "/admin/keys/:id",
                put(upsert_key_with_id).delete(delete_key),
            );

        #[cfg(feature = "gateway-routing-advanced")]
        {
            router = router
                .route("/admin/backends", get(list_backends))
                .route("/admin/backends/:name/reset", post(reset_backend));
        }

        #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
        {
            router = router
                .route("/admin/audit", get(list_audit_logs))
                .route("/admin/budgets", get(list_budget_ledgers));

            #[cfg(feature = "gateway-costing")]
            {
                router = router.route("/admin/costs", get(list_cost_ledgers));
            }
        }
    }

    #[cfg(feature = "gateway-routing-advanced")]
    start_proxy_health_checks(&state);

    router.with_state(state)
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn metrics(State(state): State<GatewayHttpState>) -> Json<ObservabilitySnapshot> {
    let gateway = state.gateway.lock().await;
    Json(gateway.observability())
}

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

async fn handle_openai_compat_proxy(
    State(state): State<GatewayHttpState>,
    Path(_path): Path<String>,
    req: axum::http::Request<Body>,
) -> Result<axum::response::Response, (StatusCode, Json<OpenAiErrorResponse>)> {
    const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;

    let (parts, body) = req.into_parts();
    let body = to_bytes(body, MAX_BODY_BYTES)
        .await
        .map_err(|err| openai_error(StatusCode::BAD_REQUEST, "invalid_request_error", None, err))?;

    let request_id =
        extract_header(&parts.headers, "x-request-id").unwrap_or_else(generate_request_id);

    let path_and_query = parts
        .uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or_else(|| parts.uri.path());

    #[cfg(feature = "gateway-otel")]
    let proxy_span = tracing::info_span!(
        "ditto.gateway.proxy",
        request_id = %request_id,
        method = %parts.method,
        path = %path_and_query,
        model = tracing::field::Empty,
        virtual_key_id = tracing::field::Empty,
        backend = tracing::field::Empty,
        status = tracing::field::Empty,
        cache = tracing::field::Empty,
    );
    #[cfg(feature = "gateway-otel")]
    let _proxy_span_guard = proxy_span.enter();

    let parsed_json: Option<serde_json::Value> = parts
        .headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .filter(|ct| ct.to_ascii_lowercase().starts_with("application/json"))
        .and_then(|_| serde_json::from_slice(&body).ok());

    let model = parsed_json
        .as_ref()
        .and_then(|value| value.get("model"))
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
    let charge_tokens = input_tokens_estimate.saturating_add(max_output_tokens);

    #[cfg(feature = "gateway-costing")]
    let charge_cost_usd_micros: Option<u64> = model.as_deref().and_then(|model| {
        state.pricing.as_ref().and_then(|pricing| {
            pricing.estimate_cost_usd_micros(model, input_tokens_estimate, max_output_tokens)
        })
    });
    #[cfg(not(feature = "gateway-costing"))]
    let charge_cost_usd_micros: Option<u64> = None;

    #[cfg(feature = "gateway-store-sqlite")]
    let use_sqlite_budget = state.sqlite_store.is_some();
    #[cfg(not(feature = "gateway-store-sqlite"))]
    let use_sqlite_budget = false;

    #[cfg(feature = "gateway-store-redis")]
    let use_redis_budget = state.redis_store.is_some();
    #[cfg(not(feature = "gateway-store-redis"))]
    let use_redis_budget = false;

    let use_persistent_budget = use_sqlite_budget || use_redis_budget;

    let (virtual_key_id, budget, backend_candidates, strip_authorization) = {
        let mut gateway = state.gateway.lock().await;
        gateway.observability.record_request();

        let strip_authorization = !gateway.config.virtual_keys.is_empty();
        let key = if gateway.config.virtual_keys.is_empty() {
            None
        } else {
            let token = extract_bearer(&parts.headers)
                .or_else(|| extract_header(&parts.headers, "x-ditto-virtual-key"))
                .ok_or_else(|| {
                    openai_error(
                        StatusCode::UNAUTHORIZED,
                        "authentication_error",
                        Some("invalid_api_key"),
                        "missing virtual key",
                    )
                })?;
            let key = gateway
                .config
                .virtual_keys
                .iter()
                .find(|key| key.token == token)
                .cloned()
                .ok_or_else(|| {
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

        if let Some(key) = key.as_ref() {
            let virtual_key_id = Some(key.id.clone());

            let now = gateway.clock.now_epoch_seconds();
            let minute = now / 60;

            if let Err(err) =
                gateway
                    .limits
                    .check_and_consume(&key.id, &key.limits, charge_tokens, minute)
            {
                gateway.observability.record_rate_limited();
                return Err(map_openai_gateway_error(err));
            }

            if let Some(model) = model.as_deref() {
                if let Some(reason) = key.guardrails.check_model(model) {
                    gateway.observability.record_guardrail_blocked();
                    return Err(openai_error(
                        StatusCode::FORBIDDEN,
                        "policy_error",
                        Some("guardrail_rejected"),
                        reason,
                    ));
                }
            }

            if let Some(limit) = key.guardrails.max_input_tokens {
                if input_tokens_estimate > limit {
                    gateway.observability.record_guardrail_blocked();
                    return Err(openai_error(
                        StatusCode::FORBIDDEN,
                        "policy_error",
                        Some("guardrail_rejected"),
                        format!("input_tokens>{limit}"),
                    ));
                }
            }

            if key.guardrails.has_text_filters() {
                if let Ok(text) = std::str::from_utf8(&body) {
                    if let Some(reason) = key.guardrails.check_text(text) {
                        gateway.observability.record_guardrail_blocked();
                        return Err(openai_error(
                            StatusCode::FORBIDDEN,
                            "policy_error",
                            Some("guardrail_rejected"),
                            reason,
                        ));
                    }
                }
            }

            if !use_persistent_budget {
                if let Err(err) =
                    gateway
                        .budget
                        .can_spend(&key.id, &key.budget, u64::from(charge_tokens))
                {
                    gateway.observability.record_budget_exceeded();
                    return Err(map_openai_gateway_error(err));
                }

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

                    if let Err(err) = gateway.budget.can_spend_cost_usd_micros(
                        &key.id,
                        &key.budget,
                        charge_cost_usd_micros,
                    ) {
                        gateway.observability.record_budget_exceeded();
                        return Err(map_openai_gateway_error(err));
                    }
                }
            }

            let budget = Some(key.budget.clone());

            let backends = gateway
                .router
                .select_backends_for_model_seeded(
                    model.as_deref().unwrap_or_default(),
                    Some(key),
                    Some(&request_id),
                )
                .map_err(map_openai_gateway_error)?;

            (virtual_key_id, budget, backends, strip_authorization)
        } else {
            let backends = gateway
                .router
                .select_backends_for_model_seeded(
                    model.as_deref().unwrap_or_default(),
                    None,
                    Some(&request_id),
                )
                .map_err(map_openai_gateway_error)?;

            (None, None, backends, strip_authorization)
        }
    };

    #[cfg(feature = "gateway-otel")]
    if let Some(virtual_key_id) = virtual_key_id.as_deref() {
        proxy_span.record("virtual_key_id", tracing::field::display(virtual_key_id));
    }

    let _now_epoch_seconds = now_epoch_seconds();

    #[cfg(feature = "gateway-metrics-prometheus")]
    if let Some(metrics) = state.prometheus_metrics.as_ref() {
        metrics
            .lock()
            .await
            .record_proxy_request(virtual_key_id.as_deref(), model.as_deref());
    }

    #[cfg(feature = "gateway-routing-advanced")]
    let backend_candidates =
        filter_backend_candidates_by_health(&state, backend_candidates, _now_epoch_seconds).await;

    #[cfg(feature = "gateway-proxy-cache")]
    let proxy_cache_key = if state.proxy_cache.is_some()
        && proxy_cache_can_read(&parts.method)
        && !_stream_requested
        && !proxy_cache_bypass(&parts.headers)
        && (parts.method == axum::http::Method::GET || parsed_json.is_some())
    {
        let scope = proxy_cache_scope(virtual_key_id.as_deref(), &parts.headers);
        Some(proxy_cache_key(
            &parts.method,
            path_and_query,
            &body,
            &scope,
        ))
    } else {
        None
    };

    #[cfg(feature = "gateway-proxy-cache")]
    if let (Some(cache), Some(cache_key)) = (state.proxy_cache.as_ref(), proxy_cache_key.as_ref()) {
        let cached = { cache.lock().await.get(cache_key, _now_epoch_seconds) };
        if let Some(cached) = cached {
            {
                let mut gateway = state.gateway.lock().await;
                gateway.observability.record_cache_hit();
            }

            emit_json_log(
                &state,
                "proxy.cache_hit",
                serde_json::json!({
                    "request_id": &request_id,
                    "backend": &cached.backend,
                    "path": path_and_query,
                }),
            );

            #[cfg(feature = "gateway-otel")]
            {
                proxy_span.record("cache", tracing::field::display("hit"));
                proxy_span.record("backend", tracing::field::display(&cached.backend));
                proxy_span.record("status", tracing::field::display(cached.status));
            }

            #[cfg(feature = "gateway-metrics-prometheus")]
            if let Some(metrics) = state.prometheus_metrics.as_ref() {
                let mut metrics = metrics.lock().await;
                metrics.record_proxy_cache_hit();
                metrics.record_proxy_response_status(cached.status);
            }

            return Ok(cached_proxy_response(cached, request_id.clone()));
        }
    }

    #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
    let _token_budget_reserved = if use_persistent_budget {
        if let (Some(virtual_key_id), Some(budget)) = (virtual_key_id.as_deref(), budget.as_ref()) {
            if let Some(limit) = budget.total_tokens {
                #[cfg(feature = "gateway-store-sqlite")]
                {
                    if let Some(store) = state.sqlite_store.as_ref() {
                        match store
                            .reserve_budget_tokens(
                                &request_id,
                                virtual_key_id,
                                limit,
                                u64::from(charge_tokens),
                            )
                            .await
                        {
                            Ok(()) => true,
                            Err(SqliteStoreError::BudgetExceeded { limit, attempted }) => {
                                let _ = store
                                    .append_audit_log(
                                        "proxy.blocked",
                                        serde_json::json!({
                                            "request_id": &request_id,
                                            "virtual_key_id": virtual_key_id,
                                            "reason": "budget_exceeded",
                                            "limit": limit,
                                            "attempted": attempted,
                                            "charge_tokens": charge_tokens,
                                            "path": path_and_query,
                                            "model": &model,
                                        }),
                                    )
                                    .await;
                                emit_json_log(
                                    &state,
                                    "proxy.blocked",
                                    serde_json::json!({
                                        "request_id": &request_id,
                                        "virtual_key_id": virtual_key_id,
                                        "reason": "budget_exceeded",
                                        "limit": limit,
                                        "attempted": attempted,
                                    }),
                                );
                                return Err(map_openai_gateway_error(
                                    GatewayError::BudgetExceeded { limit, attempted },
                                ));
                            }
                            Err(err) => {
                                let _ = store
                                    .append_audit_log(
                                        "proxy.blocked",
                                        serde_json::json!({
                                            "request_id": &request_id,
                                            "virtual_key_id": virtual_key_id,
                                            "reason": "storage_error",
                                            "error": err.to_string(),
                                            "path": path_and_query,
                                            "model": &model,
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
                    } else {
                        #[cfg(feature = "gateway-store-redis")]
                        {
                            if let Some(store) = state.redis_store.as_ref() {
                                match store
                                    .reserve_budget_tokens(
                                        &request_id,
                                        virtual_key_id,
                                        limit,
                                        u64::from(charge_tokens),
                                    )
                                    .await
                                {
                                    Ok(()) => true,
                                    Err(RedisStoreError::BudgetExceeded { limit, attempted }) => {
                                        let _ = store
                                            .append_audit_log(
                                                "proxy.blocked",
                                                serde_json::json!({
                                                    "request_id": &request_id,
                                                    "virtual_key_id": virtual_key_id,
                                                    "reason": "budget_exceeded",
                                                    "limit": limit,
                                                    "attempted": attempted,
                                                    "charge_tokens": charge_tokens,
                                                    "path": path_and_query,
                                                    "model": &model,
                                                }),
                                            )
                                            .await;
                                        emit_json_log(
                                            &state,
                                            "proxy.blocked",
                                            serde_json::json!({
                                                "request_id": &request_id,
                                                "virtual_key_id": virtual_key_id,
                                                "reason": "budget_exceeded",
                                                "limit": limit,
                                                "attempted": attempted,
                                            }),
                                        );
                                        return Err(map_openai_gateway_error(
                                            GatewayError::BudgetExceeded { limit, attempted },
                                        ));
                                    }
                                    Err(err) => {
                                        let _ = store
                                            .append_audit_log(
                                                "proxy.blocked",
                                                serde_json::json!({
                                                    "request_id": &request_id,
                                                    "virtual_key_id": virtual_key_id,
                                                    "reason": "storage_error",
                                                    "error": err.to_string(),
                                                    "path": path_and_query,
                                                    "model": &model,
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
                            } else {
                                false
                            }
                        }
                        #[cfg(not(feature = "gateway-store-redis"))]
                        {
                            false
                        }
                    }
                }
                #[cfg(not(feature = "gateway-store-sqlite"))]
                {
                    #[cfg(feature = "gateway-store-redis")]
                    {
                        if let Some(store) = state.redis_store.as_ref() {
                            match store
                                .reserve_budget_tokens(
                                    &request_id,
                                    virtual_key_id,
                                    limit,
                                    u64::from(charge_tokens),
                                )
                                .await
                            {
                                Ok(()) => true,
                                Err(RedisStoreError::BudgetExceeded { limit, attempted }) => {
                                    let _ = store
                                        .append_audit_log(
                                            "proxy.blocked",
                                            serde_json::json!({
                                                "request_id": &request_id,
                                                "virtual_key_id": virtual_key_id,
                                                "reason": "budget_exceeded",
                                                "limit": limit,
                                                "attempted": attempted,
                                                "charge_tokens": charge_tokens,
                                                "path": path_and_query,
                                                "model": &model,
                                            }),
                                        )
                                        .await;
                                    emit_json_log(
                                        &state,
                                        "proxy.blocked",
                                        serde_json::json!({
                                            "request_id": &request_id,
                                            "virtual_key_id": virtual_key_id,
                                            "reason": "budget_exceeded",
                                            "limit": limit,
                                            "attempted": attempted,
                                        }),
                                    );
                                    return Err(map_openai_gateway_error(
                                        GatewayError::BudgetExceeded { limit, attempted },
                                    ));
                                }
                                Err(err) => {
                                    let _ = store
                                        .append_audit_log(
                                            "proxy.blocked",
                                            serde_json::json!({
                                                "request_id": &request_id,
                                                "virtual_key_id": virtual_key_id,
                                                "reason": "storage_error",
                                                "error": err.to_string(),
                                                "path": path_and_query,
                                                "model": &model,
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
                        } else {
                            false
                        }
                    }
                    #[cfg(not(feature = "gateway-store-redis"))]
                    {
                        false
                    }
                }
            } else {
                false
            }
        } else {
            false
        }
    } else {
        false
    };
    #[cfg(not(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis")))]
    let _token_budget_reserved = false;

    #[cfg(all(
        feature = "gateway-costing",
        any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
    ))]
    let _cost_budget_reserved = if use_persistent_budget {
        if let (Some(virtual_key_id), Some(budget)) = (virtual_key_id.as_deref(), budget.as_ref()) {
            if let Some(limit_usd_micros) = budget.total_usd_micros {
                let Some(charge_cost_usd_micros) = charge_cost_usd_micros else {
                    if _token_budget_reserved {
                        #[cfg(feature = "gateway-store-sqlite")]
                        if let Some(store) = state.sqlite_store.as_ref() {
                            let _ = store.rollback_budget_reservation(&request_id).await;
                        }
                        #[cfg(feature = "gateway-store-redis")]
                        if let Some(store) = state.redis_store.as_ref() {
                            let _ = store.rollback_budget_reservation(&request_id).await;
                        }
                    }
                    return Err(openai_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "api_error",
                        Some("pricing_not_configured"),
                        "pricing not configured for cost budgets",
                    ));
                };

                #[cfg(feature = "gateway-store-sqlite")]
                {
                    if let Some(store) = state.sqlite_store.as_ref() {
                        match store
                            .reserve_cost_usd_micros(
                                &request_id,
                                virtual_key_id,
                                limit_usd_micros,
                                charge_cost_usd_micros,
                            )
                            .await
                        {
                            Ok(()) => true,
                            Err(SqliteStoreError::CostBudgetExceeded {
                                limit_usd_micros,
                                attempted_usd_micros,
                            }) => {
                                if _token_budget_reserved {
                                    let _ = store.rollback_budget_reservation(&request_id).await;
                                }
                                let _ = store
                                    .append_audit_log(
                                        "proxy.blocked",
                                        serde_json::json!({
                                            "request_id": &request_id,
                                            "virtual_key_id": virtual_key_id,
                                            "reason": "cost_budget_exceeded",
                                            "limit_usd_micros": limit_usd_micros,
                                            "attempted_usd_micros": attempted_usd_micros,
                                            "charge_cost_usd_micros": charge_cost_usd_micros,
                                            "path": path_and_query,
                                            "model": &model,
                                        }),
                                    )
                                    .await;
                                emit_json_log(
                                    &state,
                                    "proxy.blocked",
                                    serde_json::json!({
                                        "request_id": &request_id,
                                        "virtual_key_id": virtual_key_id,
                                        "reason": "cost_budget_exceeded",
                                        "limit_usd_micros": limit_usd_micros,
                                        "attempted_usd_micros": attempted_usd_micros,
                                    }),
                                );
                                return Err(map_openai_gateway_error(
                                    GatewayError::CostBudgetExceeded {
                                        limit_usd_micros,
                                        attempted_usd_micros,
                                    },
                                ));
                            }
                            Err(err) => {
                                if _token_budget_reserved {
                                    let _ = store.rollback_budget_reservation(&request_id).await;
                                }
                                let _ = store
                                    .append_audit_log(
                                        "proxy.blocked",
                                        serde_json::json!({
                                            "request_id": &request_id,
                                            "virtual_key_id": virtual_key_id,
                                            "reason": "storage_error",
                                            "error": err.to_string(),
                                            "path": path_and_query,
                                            "model": &model,
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
                    } else {
                        #[cfg(feature = "gateway-store-redis")]
                        {
                            if let Some(store) = state.redis_store.as_ref() {
                                match store
                                    .reserve_cost_usd_micros(
                                        &request_id,
                                        virtual_key_id,
                                        limit_usd_micros,
                                        charge_cost_usd_micros,
                                    )
                                    .await
                                {
                                    Ok(()) => true,
                                    Err(RedisStoreError::CostBudgetExceeded {
                                        limit_usd_micros,
                                        attempted_usd_micros,
                                    }) => {
                                        if _token_budget_reserved {
                                            let _ = store
                                                .rollback_budget_reservation(&request_id)
                                                .await;
                                        }
                                        let _ = store
                                            .append_audit_log(
                                                "proxy.blocked",
                                                serde_json::json!({
                                                    "request_id": &request_id,
                                                    "virtual_key_id": virtual_key_id,
                                                    "reason": "cost_budget_exceeded",
                                                    "limit_usd_micros": limit_usd_micros,
                                                    "attempted_usd_micros": attempted_usd_micros,
                                                    "charge_cost_usd_micros": charge_cost_usd_micros,
                                                    "path": path_and_query,
                                                    "model": &model,
                                                }),
                                            )
                                            .await;
                                        emit_json_log(
                                            &state,
                                            "proxy.blocked",
                                            serde_json::json!({
                                                "request_id": &request_id,
                                                "virtual_key_id": virtual_key_id,
                                                "reason": "cost_budget_exceeded",
                                                "limit_usd_micros": limit_usd_micros,
                                                "attempted_usd_micros": attempted_usd_micros,
                                            }),
                                        );
                                        return Err(map_openai_gateway_error(
                                            GatewayError::CostBudgetExceeded {
                                                limit_usd_micros,
                                                attempted_usd_micros,
                                            },
                                        ));
                                    }
                                    Err(err) => {
                                        if _token_budget_reserved {
                                            let _ = store
                                                .rollback_budget_reservation(&request_id)
                                                .await;
                                        }
                                        let _ = store
                                            .append_audit_log(
                                                "proxy.blocked",
                                                serde_json::json!({
                                                    "request_id": &request_id,
                                                    "virtual_key_id": virtual_key_id,
                                                    "reason": "storage_error",
                                                    "error": err.to_string(),
                                                    "path": path_and_query,
                                                    "model": &model,
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
                            } else {
                                false
                            }
                        }
                        #[cfg(not(feature = "gateway-store-redis"))]
                        {
                            false
                        }
                    }
                }
                #[cfg(not(feature = "gateway-store-sqlite"))]
                {
                    #[cfg(feature = "gateway-store-redis")]
                    {
                        if let Some(store) = state.redis_store.as_ref() {
                            match store
                                .reserve_cost_usd_micros(
                                    &request_id,
                                    virtual_key_id,
                                    limit_usd_micros,
                                    charge_cost_usd_micros,
                                )
                                .await
                            {
                                Ok(()) => true,
                                Err(RedisStoreError::CostBudgetExceeded {
                                    limit_usd_micros,
                                    attempted_usd_micros,
                                }) => {
                                    if _token_budget_reserved {
                                        let _ =
                                            store.rollback_budget_reservation(&request_id).await;
                                    }
                                    let _ = store
                                        .append_audit_log(
                                            "proxy.blocked",
                                            serde_json::json!({
                                                "request_id": &request_id,
                                                "virtual_key_id": virtual_key_id,
                                                "reason": "cost_budget_exceeded",
                                                "limit_usd_micros": limit_usd_micros,
                                                "attempted_usd_micros": attempted_usd_micros,
                                                "charge_cost_usd_micros": charge_cost_usd_micros,
                                                "path": path_and_query,
                                                "model": &model,
                                            }),
                                        )
                                        .await;
                                    emit_json_log(
                                        &state,
                                        "proxy.blocked",
                                        serde_json::json!({
                                            "request_id": &request_id,
                                            "virtual_key_id": virtual_key_id,
                                            "reason": "cost_budget_exceeded",
                                            "limit_usd_micros": limit_usd_micros,
                                            "attempted_usd_micros": attempted_usd_micros,
                                        }),
                                    );
                                    return Err(map_openai_gateway_error(
                                        GatewayError::CostBudgetExceeded {
                                            limit_usd_micros,
                                            attempted_usd_micros,
                                        },
                                    ));
                                }
                                Err(err) => {
                                    if _token_budget_reserved {
                                        let _ =
                                            store.rollback_budget_reservation(&request_id).await;
                                    }
                                    let _ = store
                                        .append_audit_log(
                                            "proxy.blocked",
                                            serde_json::json!({
                                                "request_id": &request_id,
                                                "virtual_key_id": virtual_key_id,
                                                "reason": "storage_error",
                                                "error": err.to_string(),
                                                "path": path_and_query,
                                                "model": &model,
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
                        } else {
                            false
                        }
                    }
                    #[cfg(not(feature = "gateway-store-redis"))]
                    {
                        false
                    }
                }
            } else {
                false
            }
        } else {
            false
        }
    } else {
        false
    };
    #[cfg(not(all(
        feature = "gateway-costing",
        any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
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
        .proxy_routing
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
    for (idx, backend_name) in backend_candidates.into_iter().enumerate() {
        if idx >= max_attempts {
            break;
        }

        attempted_backends.push(backend_name.clone());

        #[cfg(feature = "gateway-translation")]
        if let Some(translation_backend) = state.translation_backends.get(&backend_name).cloned() {
            let batch_cancel_id = translation::batches_cancel_id(path_and_query);
            let batch_retrieve_id = translation::batches_retrieve_id(path_and_query);
            let batches_root = translation::is_batches_path(path_and_query);

            let supported_path = translation::is_chat_completions_path(path_and_query)
                || translation::is_responses_create_path(path_and_query)
                || translation::is_embeddings_path(path_and_query)
                || translation::is_moderations_path(path_and_query)
                || translation::is_images_generations_path(path_and_query)
                || translation::is_audio_transcriptions_path(path_and_query)
                || translation::is_audio_speech_path(path_and_query)
                || translation::is_rerank_path(path_and_query)
                || batches_root
                || batch_cancel_id.is_some()
                || batch_retrieve_id.is_some();

            let supported_method = if parts.method == axum::http::Method::POST {
                translation::is_chat_completions_path(path_and_query)
                    || translation::is_responses_create_path(path_and_query)
                    || translation::is_embeddings_path(path_and_query)
                    || translation::is_moderations_path(path_and_query)
                    || translation::is_images_generations_path(path_and_query)
                    || translation::is_audio_transcriptions_path(path_and_query)
                    || translation::is_audio_speech_path(path_and_query)
                    || translation::is_rerank_path(path_and_query)
                    || batches_root
                    || batch_cancel_id.is_some()
            } else if parts.method == axum::http::Method::GET {
                batches_root || batch_retrieve_id.is_some()
            } else {
                false
            };

            if !supported_path || !supported_method {
                last_err = Some(openai_error(
                    StatusCode::NOT_IMPLEMENTED,
                    "invalid_request_error",
                    Some("unsupported_endpoint"),
                    format!(
                        "translation backend does not support {} {}",
                        parts.method, path_and_query
                    ),
                ));
                continue;
            }

            let mut proxy_permit = try_acquire_proxy_permit(&state)?;

            {
                let mut gateway = state.gateway.lock().await;
                gateway.observability.record_backend_call();
            }

            let backend_timer_start = Instant::now();

            #[cfg(feature = "gateway-metrics-prometheus")]
            if let Some(metrics) = state.prometheus_metrics.as_ref() {
                let mut metrics = metrics.lock().await;
                metrics.record_proxy_backend_attempt(&backend_name);
                metrics.record_proxy_backend_in_flight_inc(&backend_name);
            }

            let default_spend = ProxySpend {
                tokens: u64::from(charge_tokens),
                cost_usd_micros: charge_cost_usd_micros,
            };

            let result: Result<
                (axum::response::Response, ProxySpend),
                (StatusCode, Json<OpenAiErrorResponse>),
            > = 'translation_backend_attempt: {
                if batches_root && parts.method == axum::http::Method::GET {
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
                    headers.insert("content-type", "application/json".parse().unwrap());
                    headers.insert(
                        "x-ditto-translation",
                        translation_backend
                            .provider
                            .parse()
                            .unwrap_or_else(|_| "enabled".parse().unwrap()),
                    );
                    apply_proxy_response_headers(&mut headers, &backend_name, &request_id, false);

                    let mut response = axum::response::Response::new(Body::from(bytes));
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

                    let request = match translation::batches_create_request_to_request(parsed_json)
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
                    headers.insert("content-type", "application/json".parse().unwrap());
                    headers.insert(
                        "x-ditto-translation",
                        translation_backend
                            .provider
                            .parse()
                            .unwrap_or_else(|_| "enabled".parse().unwrap()),
                    );
                    apply_proxy_response_headers(&mut headers, &backend_name, &request_id, false);

                    let mut response = axum::response::Response::new(Body::from(bytes));
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
                    headers.insert("content-type", "application/json".parse().unwrap());
                    headers.insert(
                        "x-ditto-translation",
                        translation_backend
                            .provider
                            .parse()
                            .unwrap_or_else(|_| "enabled".parse().unwrap()),
                    );
                    apply_proxy_response_headers(&mut headers, &backend_name, &request_id, false);

                    let mut response = axum::response::Response::new(Body::from(bytes));
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
                    headers.insert("content-type", "application/json".parse().unwrap());
                    headers.insert(
                        "x-ditto-translation",
                        translation_backend
                            .provider
                            .parse()
                            .unwrap_or_else(|_| "enabled".parse().unwrap()),
                    );
                    apply_proxy_response_headers(&mut headers, &backend_name, &request_id, false);

                    let mut response = axum::response::Response::new(Body::from(bytes));
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
                    headers.insert("content-type", "application/json".parse().unwrap());
                    headers.insert(
                        "x-ditto-translation",
                        translation_backend
                            .provider
                            .parse()
                            .unwrap_or_else(|_| "enabled".parse().unwrap()),
                    );
                    apply_proxy_response_headers(&mut headers, &backend_name, &request_id, false);

                    let mut response = axum::response::Response::new(Body::from(bytes));
                    *response.status_mut() = StatusCode::OK;
                    *response.headers_mut() = headers;
                    Ok((response, default_spend))
                } else if translation::is_audio_transcriptions_path(path_and_query) {
                    let Some(content_type) = parts
                        .headers
                        .get("content-type")
                        .and_then(|value| value.to_str().ok())
                    else {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            "audio/transcriptions request missing content-type",
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
                            "audio/transcriptions request must be multipart/form-data",
                        ));
                    }

                    let request = match translation::audio_transcriptions_request_to_request(
                        content_type,
                        &body,
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
                        "content-type",
                        content_type
                            .parse()
                            .unwrap_or_else(|_| "application/octet-stream".parse().unwrap()),
                    );
                    headers.insert(
                        "x-ditto-translation",
                        translation_backend
                            .provider
                            .parse()
                            .unwrap_or_else(|_| "enabled".parse().unwrap()),
                    );
                    apply_proxy_response_headers(&mut headers, &backend_name, &request_id, false);

                    let mut response = axum::response::Response::new(Body::from(bytes));
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
                        translation::speech_response_format_to_content_type(request_format)
                            .to_string()
                    });

                    let mut headers = HeaderMap::new();
                    headers.insert(
                        "content-type",
                        content_type
                            .parse()
                            .unwrap_or_else(|_| "application/octet-stream".parse().unwrap()),
                    );
                    headers.insert(
                        "x-ditto-translation",
                        translation_backend
                            .provider
                            .parse()
                            .unwrap_or_else(|_| "enabled".parse().unwrap()),
                    );
                    apply_proxy_response_headers(&mut headers, &backend_name, &request_id, false);

                    let mut response = axum::response::Response::new(Body::from(spoken.audio));
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

                    let value =
                        translation::embeddings_to_openai_response(embeddings, &original_model);
                    let bytes = serde_json::to_vec(&value)
                        .map(Bytes::from)
                        .unwrap_or_else(|_| Bytes::from(value.to_string()));

                    let mut headers = HeaderMap::new();
                    headers.insert("content-type", "application/json".parse().unwrap());
                    headers.insert(
                        "x-ditto-translation",
                        translation_backend
                            .provider
                            .parse()
                            .unwrap_or_else(|_| "enabled".parse().unwrap()),
                    );
                    apply_proxy_response_headers(&mut headers, &backend_name, &request_id, false);

                    let mut response = axum::response::Response::new(Body::from(bytes));
                    *response.status_mut() = StatusCode::OK;
                    *response.headers_mut() = headers;
                    Ok((response, default_spend))
                } else if translation::is_moderations_path(path_and_query) {
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
                            "moderations endpoint does not support stream=true",
                        ));
                    }

                    let mut request = match translation::moderations_request_to_request(parsed_json)
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
                    let value =
                        translation::moderation_response_to_openai(&moderated, &fallback_id);

                    let bytes = serde_json::to_vec(&value)
                        .map(Bytes::from)
                        .unwrap_or_else(|_| Bytes::from(value.to_string()));

                    let mut headers = HeaderMap::new();
                    headers.insert("content-type", "application/json".parse().unwrap());
                    headers.insert(
                        "x-ditto-translation",
                        translation_backend
                            .provider
                            .parse()
                            .unwrap_or_else(|_| "enabled".parse().unwrap()),
                    );
                    apply_proxy_response_headers(&mut headers, &backend_name, &request_id, false);

                    let mut response = axum::response::Response::new(Body::from(bytes));
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
                    headers.insert("content-type", "application/json".parse().unwrap());
                    headers.insert(
                        "x-ditto-translation",
                        translation_backend
                            .provider
                            .parse()
                            .unwrap_or_else(|_| "enabled".parse().unwrap()),
                    );
                    apply_proxy_response_headers(&mut headers, &backend_name, &request_id, false);

                    let mut response = axum::response::Response::new(Body::from(bytes));
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

                    let generate_request = if translation::is_chat_completions_path(path_and_query)
                    {
                        translation::chat_completions_request_to_generate_request(parsed_json)
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

                    let fallback_response_id =
                        if translation::is_chat_completions_path(path_and_query) {
                            format!("chatcmpl_{request_id}")
                        } else {
                            format!("resp_{request_id}")
                        };

                    if _stream_requested {
                        let stream = match translation_backend.model.stream(generate_request).await
                        {
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
                            )
                        } else {
                            translation::stream_to_responses_sse(stream, fallback_response_id)
                        };

                        let mut headers = HeaderMap::new();
                        headers.insert("content-type", "text/event-stream".parse().unwrap());
                        headers.insert(
                            "x-ditto-translation",
                            translation_backend
                                .provider
                                .parse()
                                .unwrap_or_else(|_| "enabled".parse().unwrap()),
                        );
                        headers.remove("content-length");
                        apply_proxy_response_headers(
                            &mut headers,
                            &backend_name,
                            &request_id,
                            false,
                        );

                        let stream = ProxyBodyStreamWithPermit {
                            inner: stream.boxed(),
                            _permit: proxy_permit.take(),
                        };
                        let mut response = axum::response::Response::new(Body::from_stream(stream));
                        *response.status_mut() = StatusCode::OK;
                        *response.headers_mut() = headers;
                        Ok((response, default_spend))
                    } else {
                        let generated =
                            match translation_backend.model.generate(generate_request).await {
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
                        } else {
                            translation::generate_response_to_responses(
                                &generated,
                                &response_id,
                                &original_model,
                                _now_epoch_seconds,
                            )
                        };

                        let bytes = serde_json::to_vec(&value)
                            .map(Bytes::from)
                            .unwrap_or_else(|_| Bytes::from(value.to_string()));

                        let mut headers = HeaderMap::new();
                        headers.insert("content-type", "application/json".parse().unwrap());
                        headers.insert(
                            "x-ditto-translation",
                            translation_backend
                                .provider
                                .parse()
                                .unwrap_or_else(|_| "enabled".parse().unwrap()),
                        );
                        apply_proxy_response_headers(
                            &mut headers,
                            &backend_name,
                            &request_id,
                            false,
                        );

                        let mut response = axum::response::Response::new(Body::from(bytes));
                        *response.status_mut() = StatusCode::OK;
                        *response.headers_mut() = headers;
                        let mut usage = generated.usage.clone();
                        usage.merge_total();
                        let tokens = usage.total_tokens.unwrap_or(u64::from(charge_tokens));
                        #[cfg(feature = "gateway-costing")]
                        let cost_usd_micros = model.as_deref().and_then(|model| {
                            state.pricing.as_ref().and_then(|pricing| {
                                let (Some(input), Some(output)) =
                                    (usage.input_tokens, usage.output_tokens)
                                else {
                                    return None;
                                };
                                pricing.estimate_cost_usd_micros_with_cache(
                                    model,
                                    clamp_u64_to_u32(input),
                                    usage.cache_input_tokens.map(clamp_u64_to_u32),
                                    clamp_u64_to_u32(output),
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
            };

            #[cfg(feature = "gateway-metrics-prometheus")]
            if let Some(metrics) = state.prometheus_metrics.as_ref() {
                let mut metrics = metrics.lock().await;
                metrics.record_proxy_backend_in_flight_dec(&backend_name);
                metrics.observe_proxy_backend_request_duration(
                    &backend_name,
                    backend_timer_start.elapsed(),
                );
            }

            let (response, spend) = match result {
                Ok((response, spend)) => (response, spend),
                Err(err) => {
                    last_err = Some(err);
                    continue;
                }
            };

            let status = StatusCode::OK;
            let spend_tokens = true;
            let spent_tokens = spend.tokens;
            let spent_cost_usd_micros = spend.cost_usd_micros;

            #[cfg(feature = "gateway-metrics-prometheus")]
            if let Some(metrics) = state.prometheus_metrics.as_ref() {
                let mut metrics = metrics.lock().await;
                if spend_tokens {
                    metrics.record_proxy_backend_success(&backend_name);
                } else {
                    metrics.record_proxy_backend_failure(&backend_name);
                }
                metrics.record_proxy_response_status(status.as_u16());
            }

            #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
            if _token_budget_reserved {
                #[cfg(feature = "gateway-store-sqlite")]
                if let Some(store) = state.sqlite_store.as_ref() {
                    if spend_tokens {
                        let _ = store
                            .commit_budget_reservation_with_tokens(&request_id, spent_tokens)
                            .await;
                    } else {
                        let _ = store.rollback_budget_reservation(&request_id).await;
                    }
                }
                #[cfg(feature = "gateway-store-redis")]
                if let Some(store) = state.redis_store.as_ref() {
                    if spend_tokens {
                        let _ = store
                            .commit_budget_reservation_with_tokens(&request_id, spent_tokens)
                            .await;
                    } else {
                        let _ = store.rollback_budget_reservation(&request_id).await;
                    }
                }
            } else if let (Some(virtual_key_id), Some(budget)) =
                (virtual_key_id.clone(), budget.clone())
            {
                if spend_tokens {
                    let mut gateway = state.gateway.lock().await;
                    gateway.budget.spend(&virtual_key_id, &budget, spent_tokens);

                    #[cfg(feature = "gateway-costing")]
                    if !use_persistent_budget {
                        if let Some(spent_cost_usd_micros) = spent_cost_usd_micros {
                            gateway.budget.spend_cost_usd_micros(
                                &virtual_key_id,
                                &budget,
                                spent_cost_usd_micros,
                            );
                        }
                    }
                }
            }
            #[cfg(not(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis")))]
            if let (Some(virtual_key_id), Some(budget)) = (virtual_key_id.clone(), budget.clone()) {
                if spend_tokens {
                    let mut gateway = state.gateway.lock().await;
                    gateway.budget.spend(&virtual_key_id, &budget, spent_tokens);

                    #[cfg(feature = "gateway-costing")]
                    if let Some(spent_cost_usd_micros) = spent_cost_usd_micros {
                        gateway.budget.spend_cost_usd_micros(
                            &virtual_key_id,
                            &budget,
                            spent_cost_usd_micros,
                        );
                    }
                }
            }

            #[cfg(all(
                feature = "gateway-costing",
                any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
            ))]
            if _cost_budget_reserved {
                #[cfg(feature = "gateway-store-sqlite")]
                if let Some(store) = state.sqlite_store.as_ref() {
                    if spend_tokens {
                        let _ = store
                            .commit_cost_reservation_with_usd_micros(
                                &request_id,
                                spent_cost_usd_micros.unwrap_or_default(),
                            )
                            .await;
                    } else {
                        let _ = store.rollback_cost_reservation(&request_id).await;
                    }
                }
                #[cfg(feature = "gateway-store-redis")]
                if let Some(store) = state.redis_store.as_ref() {
                    if spend_tokens {
                        let _ = store
                            .commit_cost_reservation_with_usd_micros(
                                &request_id,
                                spent_cost_usd_micros.unwrap_or_default(),
                            )
                            .await;
                    } else {
                        let _ = store.rollback_cost_reservation(&request_id).await;
                    }
                }
            }

            #[cfg(all(
                feature = "gateway-costing",
                any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
            ))]
            if !_cost_budget_reserved && use_persistent_budget && spend_tokens {
                if let (Some(virtual_key_id), Some(spent_cost_usd_micros)) =
                    (virtual_key_id.as_deref(), spent_cost_usd_micros)
                {
                    #[cfg(feature = "gateway-store-sqlite")]
                    if let Some(store) = state.sqlite_store.as_ref() {
                        let _ = store
                            .record_spent_cost_usd_micros(virtual_key_id, spent_cost_usd_micros)
                            .await;
                    }
                    #[cfg(feature = "gateway-store-redis")]
                    if let Some(store) = state.redis_store.as_ref() {
                        let _ = store
                            .record_spent_cost_usd_micros(virtual_key_id, spent_cost_usd_micros)
                            .await;
                    }
                }
            }

            #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
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

                #[cfg(feature = "gateway-store-sqlite")]
                if let Some(store) = state.sqlite_store.as_ref() {
                    let _ = store.append_audit_log("proxy", payload.clone()).await;
                }
                #[cfg(feature = "gateway-store-redis")]
                if let Some(store) = state.redis_store.as_ref() {
                    let _ = store.append_audit_log("proxy", payload.clone()).await;
                }
            }

            emit_json_log(
                &state,
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
            if let Some(logger) = state.devtools.as_ref() {
                let _ = logger.log_event(
                    "proxy.response",
                    serde_json::json!({
                        "request_id": &request_id,
                        "status": status.as_u16(),
                        "path": path_and_query,
                        "backend": &backend_name,
                        "mode": "translation",
                    }),
                );
            }

            return Ok(response);
        }

        let backend = match state.proxy_backends.get(&backend_name) {
            Some(backend) => backend.clone(),
            None => {
                last_err = Some(openai_error(
                    StatusCode::BAD_GATEWAY,
                    "api_error",
                    Some("backend_not_found"),
                    format!("backend not found: {backend_name}"),
                ));
                continue;
            }
        };

        let mut proxy_permit = try_acquire_proxy_permit(&state)?;

        {
            let mut gateway = state.gateway.lock().await;
            gateway.observability.record_backend_call();
        }

        let backend_timer_start = Instant::now();

        #[cfg(feature = "gateway-metrics-prometheus")]
        if let Some(metrics) = state.prometheus_metrics.as_ref() {
            let mut metrics = metrics.lock().await;
            metrics.record_proxy_backend_attempt(&backend_name);
            metrics.record_proxy_backend_in_flight_inc(&backend_name);
        }

        let mut outgoing_headers = parts.headers.clone();
        sanitize_proxy_headers(&mut outgoing_headers, strip_authorization);
        apply_backend_headers(&mut outgoing_headers, backend.headers());
        insert_request_id(&mut outgoing_headers, &request_id);

        #[cfg(feature = "sdk")]
        if let Some(logger) = state.devtools.as_ref() {
            let _ = logger.log_event(
                "proxy.request",
                serde_json::json!({
                    "request_id": &request_id,
                    "method": parts.method.as_str(),
                    "path": path_and_query,
                    "backend": &backend_name,
                    "model": &model,
                    "virtual_key_id": virtual_key_id.as_deref(),
                    "body_len": body.len(),
                }),
            );
        }

        let upstream_response = match backend
            .request(
                parts.method.clone(),
                path_and_query,
                outgoing_headers,
                Some(body.clone()),
            )
            .await
        {
            Ok(response) => response,
            Err(err) => {
                #[cfg(feature = "gateway-metrics-prometheus")]
                if let Some(metrics) = state.prometheus_metrics.as_ref() {
                    let mut metrics = metrics.lock().await;
                    metrics.record_proxy_backend_in_flight_dec(&backend_name);
                    metrics.observe_proxy_backend_request_duration(
                        &backend_name,
                        backend_timer_start.elapsed(),
                    );
                }
                #[cfg(feature = "gateway-metrics-prometheus")]
                if let Some(metrics) = state.prometheus_metrics.as_ref() {
                    metrics
                        .lock()
                        .await
                        .record_proxy_backend_failure(&backend_name);
                }
                #[cfg(feature = "gateway-routing-advanced")]
                record_proxy_backend_failure(
                    &state,
                    &backend_name,
                    _now_epoch_seconds,
                    FailureKind::Network,
                    err.to_string(),
                )
                .await;
                let mapped = map_openai_gateway_error(err);
                last_err = Some(mapped);
                continue;
            }
        };

        #[cfg(feature = "gateway-metrics-prometheus")]
        if let Some(metrics) = state.prometheus_metrics.as_ref() {
            let mut metrics = metrics.lock().await;
            metrics.record_proxy_backend_in_flight_dec(&backend_name);
            metrics.observe_proxy_backend_request_duration(
                &backend_name,
                backend_timer_start.elapsed(),
            );
        }

        let status = upstream_response.status();

        if responses_shim::should_attempt_responses_shim(&parts.method, path_and_query, status) {
            if let Some(parsed_json) = parsed_json.as_ref() {
                let _ = proxy_permit.take();
                let Some(chat_body) =
                    responses_shim::responses_request_to_chat_completions(parsed_json)
                else {
                    last_err = Some(openai_error(
                        StatusCode::BAD_GATEWAY,
                        "api_error",
                        Some("invalid_responses_request"),
                        "responses request cannot be mapped to chat/completions",
                    ));
                    continue;
                };

                emit_json_log(
                    &state,
                    "proxy.responses_shim",
                    serde_json::json!({
                        "request_id": &request_id,
                        "backend": &backend_name,
                        "path": path_and_query,
                        "shim": "responses_via_chat_completions",
                    }),
                );

                #[cfg(feature = "sdk")]
                if let Some(logger) = state.devtools.as_ref() {
                    let _ = logger.log_event(
                        "proxy.responses_shim",
                        serde_json::json!({
                            "request_id": &request_id,
                            "backend": &backend_name,
                            "path": path_and_query,
                        }),
                    );
                }

                let chat_body_bytes = match serde_json::to_vec(&chat_body) {
                    Ok(bytes) => Bytes::from(bytes),
                    Err(err) => {
                        last_err = Some(openai_error(
                            StatusCode::BAD_GATEWAY,
                            "api_error",
                            Some("invalid_responses_request"),
                            format!("failed to serialize shim chat/completions request: {err}"),
                        ));
                        continue;
                    }
                };

                let mut shim_headers = parts.headers.clone();
                sanitize_proxy_headers(&mut shim_headers, strip_authorization);
                apply_backend_headers(&mut shim_headers, backend.headers());
                insert_request_id(&mut shim_headers, &request_id);
                if _stream_requested {
                    shim_headers.insert(
                        "accept",
                        "text/event-stream"
                            .parse()
                            .unwrap_or_else(|_| "text/event-stream".parse().unwrap()),
                    );
                }

                let shim_permit = try_acquire_proxy_permit(&state)?;
                let shim_timer_start = Instant::now();

                #[cfg(feature = "gateway-metrics-prometheus")]
                if let Some(metrics) = state.prometheus_metrics.as_ref() {
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
                        if let Some(metrics) = state.prometheus_metrics.as_ref() {
                            let mut metrics = metrics.lock().await;
                            metrics.record_proxy_backend_in_flight_dec(&backend_name);
                            metrics.observe_proxy_backend_request_duration(
                                &backend_name,
                                shim_timer_start.elapsed(),
                            );
                            metrics.record_proxy_backend_failure(&backend_name);
                        }
                        #[cfg(feature = "gateway-routing-advanced")]
                        record_proxy_backend_failure(
                            &state,
                            &backend_name,
                            _now_epoch_seconds,
                            FailureKind::Network,
                            err.to_string(),
                        )
                        .await;
                        let mapped = map_openai_gateway_error(err);
                        last_err = Some(mapped);
                        continue;
                    }
                };

                #[cfg(feature = "gateway-metrics-prometheus")]
                if let Some(metrics) = state.prometheus_metrics.as_ref() {
                    let mut metrics = metrics.lock().await;
                    metrics.record_proxy_backend_in_flight_dec(&backend_name);
                    metrics.observe_proxy_backend_request_duration(
                        &backend_name,
                        shim_timer_start.elapsed(),
                    );
                }

                let status = shim_response.status();

                #[cfg(feature = "gateway-routing-advanced")]
                if retry_config.enabled
                    && retry_config.retry_status_codes.contains(&status.as_u16())
                    && idx + 1 < max_attempts
                {
                    #[cfg(feature = "gateway-metrics-prometheus")]
                    if let Some(metrics) = state.prometheus_metrics.as_ref() {
                        metrics
                            .lock()
                            .await
                            .record_proxy_backend_failure(&backend_name);
                    }
                    record_proxy_backend_failure(
                        &state,
                        &backend_name,
                        _now_epoch_seconds,
                        FailureKind::RetryableStatus(status.as_u16()),
                        format!("retryable status {}", status.as_u16()),
                    )
                    .await;

                    emit_json_log(
                        &state,
                        "proxy.retry",
                        serde_json::json!({
                            "request_id": &request_id,
                            "backend": &backend_name,
                            "status": status.as_u16(),
                            "attempted_backends": &attempted_backends,
                        }),
                    );

                    #[cfg(feature = "sdk")]
                    if let Some(logger) = state.devtools.as_ref() {
                        let _ = logger.log_event(
                            "proxy.retry",
                            serde_json::json!({
                                "request_id": &request_id,
                                "backend": &backend_name,
                                "status": status.as_u16(),
                                "path": path_and_query,
                            }),
                        );
                    }

                    last_err = Some(openai_error(
                        status,
                        "api_error",
                        Some("backend_error"),
                        format!("retryable status {}", status.as_u16()),
                    ));
                    continue;
                }

                #[cfg(feature = "gateway-routing-advanced")]
                if retry_config.retry_status_codes.contains(&status.as_u16()) {
                    record_proxy_backend_failure(
                        &state,
                        &backend_name,
                        _now_epoch_seconds,
                        FailureKind::RetryableStatus(status.as_u16()),
                        format!("status {}", status.as_u16()),
                    )
                    .await;
                } else {
                    record_proxy_backend_success(&state, &backend_name).await;
                }

                let spend_tokens = status.is_success();

                #[cfg(feature = "gateway-metrics-prometheus")]
                if let Some(metrics) = state.prometheus_metrics.as_ref() {
                    let is_failure_status = {
                        #[cfg(feature = "gateway-routing-advanced")]
                        {
                            retry_config.retry_status_codes.contains(&status.as_u16())
                        }
                        #[cfg(not(feature = "gateway-routing-advanced"))]
                        {
                            status.is_server_error()
                        }
                    };
                    let mut metrics = metrics.lock().await;
                    if is_failure_status {
                        metrics.record_proxy_backend_failure(&backend_name);
                    } else {
                        metrics.record_proxy_backend_success(&backend_name);
                    }
                    metrics.record_proxy_response_status(status.as_u16());
                }

                #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
                if _token_budget_reserved {
                    #[cfg(feature = "gateway-store-sqlite")]
                    if let Some(store) = state.sqlite_store.as_ref() {
                        if spend_tokens {
                            let _ = store.commit_budget_reservation(&request_id).await;
                        } else {
                            let _ = store.rollback_budget_reservation(&request_id).await;
                        }
                    }
                    #[cfg(feature = "gateway-store-redis")]
                    if let Some(store) = state.redis_store.as_ref() {
                        if spend_tokens {
                            let _ = store.commit_budget_reservation(&request_id).await;
                        } else {
                            let _ = store.rollback_budget_reservation(&request_id).await;
                        }
                    }
                } else if let (Some(virtual_key_id), Some(budget)) =
                    (virtual_key_id.clone(), budget.clone())
                {
                    if spend_tokens {
                        let mut gateway = state.gateway.lock().await;
                        gateway
                            .budget
                            .spend(&virtual_key_id, &budget, u64::from(charge_tokens));

                        #[cfg(feature = "gateway-costing")]
                        if !use_persistent_budget {
                            if let Some(charge_cost_usd_micros) = charge_cost_usd_micros {
                                gateway.budget.spend_cost_usd_micros(
                                    &virtual_key_id,
                                    &budget,
                                    charge_cost_usd_micros,
                                );
                            }
                        }
                    }
                }
                #[cfg(not(any(
                    feature = "gateway-store-sqlite",
                    feature = "gateway-store-redis"
                )))]
                if let (Some(virtual_key_id), Some(budget)) =
                    (virtual_key_id.clone(), budget.clone())
                {
                    if spend_tokens {
                        let mut gateway = state.gateway.lock().await;
                        gateway
                            .budget
                            .spend(&virtual_key_id, &budget, u64::from(charge_tokens));

                        #[cfg(feature = "gateway-costing")]
                        if let Some(charge_cost_usd_micros) = charge_cost_usd_micros {
                            gateway.budget.spend_cost_usd_micros(
                                &virtual_key_id,
                                &budget,
                                charge_cost_usd_micros,
                            );
                        }
                    }
                }

                #[cfg(all(
                    feature = "gateway-costing",
                    any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
                ))]
                if _cost_budget_reserved {
                    #[cfg(feature = "gateway-store-sqlite")]
                    if let Some(store) = state.sqlite_store.as_ref() {
                        if spend_tokens {
                            let _ = store.commit_cost_reservation(&request_id).await;
                        } else {
                            let _ = store.rollback_cost_reservation(&request_id).await;
                        }
                    }
                    #[cfg(feature = "gateway-store-redis")]
                    if let Some(store) = state.redis_store.as_ref() {
                        if spend_tokens {
                            let _ = store.commit_cost_reservation(&request_id).await;
                        } else {
                            let _ = store.rollback_cost_reservation(&request_id).await;
                        }
                    }
                }

                #[cfg(all(
                    feature = "gateway-costing",
                    any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
                ))]
                if !_cost_budget_reserved && use_persistent_budget && spend_tokens {
                    if let (Some(virtual_key_id), Some(charge_cost_usd_micros)) =
                        (virtual_key_id.as_deref(), charge_cost_usd_micros)
                    {
                        #[cfg(feature = "gateway-store-sqlite")]
                        if let Some(store) = state.sqlite_store.as_ref() {
                            let _ = store
                                .record_spent_cost_usd_micros(
                                    virtual_key_id,
                                    charge_cost_usd_micros,
                                )
                                .await;
                        }
                        #[cfg(feature = "gateway-store-redis")]
                        if let Some(store) = state.redis_store.as_ref() {
                            let _ = store
                                .record_spent_cost_usd_micros(
                                    virtual_key_id,
                                    charge_cost_usd_micros,
                                )
                                .await;
                        }
                    }
                }

                #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
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
                        "charge_cost_usd_micros": charge_cost_usd_micros,
                        "body_len": body.len(),
                        "shim": "responses_via_chat_completions",
                    });

                    #[cfg(feature = "gateway-store-sqlite")]
                    if let Some(store) = state.sqlite_store.as_ref() {
                        let _ = store.append_audit_log("proxy", payload.clone()).await;
                    }
                    #[cfg(feature = "gateway-store-redis")]
                    if let Some(store) = state.redis_store.as_ref() {
                        let _ = store.append_audit_log("proxy", payload.clone()).await;
                    }
                }

                emit_json_log(
                    &state,
                    "proxy.response",
                    serde_json::json!({
                        "request_id": &request_id,
                        "backend": &backend_name,
                        "status": status.as_u16(),
                        "attempted_backends": &attempted_backends,
                    }),
                );

                #[cfg(feature = "sdk")]
                if let Some(logger) = state.devtools.as_ref() {
                    let _ = logger.log_event(
                        "proxy.response",
                        serde_json::json!({
                            "request_id": &request_id,
                            "status": status.as_u16(),
                            "path": path_and_query,
                            "backend": &backend_name,
                        }),
                    );
                }

                #[cfg(feature = "gateway-otel")]
                {
                    proxy_span.record("cache", tracing::field::display("miss"));
                    proxy_span.record("backend", tracing::field::display(&backend_name));
                    proxy_span.record("status", tracing::field::display(status.as_u16()));
                }

                if status.is_success() {
                    match responses_shim_response(
                        &state,
                        shim_response,
                        backend_name.clone(),
                        request_id.clone(),
                        #[cfg(feature = "gateway-proxy-cache")]
                        proxy_cache_key.as_deref(),
                        #[cfg(not(feature = "gateway-proxy-cache"))]
                        None,
                        shim_permit,
                    )
                    .await
                    {
                        Ok(response) => return Ok(response),
                        Err(err) => {
                            last_err = Some(err);
                            continue;
                        }
                    }
                } else {
                    return Ok(proxy_response(
                        &state,
                        shim_response,
                        backend_name,
                        request_id.clone(),
                        #[cfg(feature = "gateway-proxy-cache")]
                        proxy_cache_key.as_deref(),
                        #[cfg(not(feature = "gateway-proxy-cache"))]
                        None,
                        shim_permit,
                    )
                    .await);
                }
            }
        }

        #[cfg(feature = "gateway-routing-advanced")]
        if retry_config.enabled
            && retry_config.retry_status_codes.contains(&status.as_u16())
            && idx + 1 < max_attempts
        {
            #[cfg(feature = "gateway-metrics-prometheus")]
            if let Some(metrics) = state.prometheus_metrics.as_ref() {
                metrics
                    .lock()
                    .await
                    .record_proxy_backend_failure(&backend_name);
            }
            record_proxy_backend_failure(
                &state,
                &backend_name,
                _now_epoch_seconds,
                FailureKind::RetryableStatus(status.as_u16()),
                format!("retryable status {}", status.as_u16()),
            )
            .await;

            emit_json_log(
                &state,
                "proxy.retry",
                serde_json::json!({
                    "request_id": &request_id,
                    "backend": &backend_name,
                    "status": status.as_u16(),
                    "attempted_backends": &attempted_backends,
                }),
            );

            #[cfg(feature = "sdk")]
            if let Some(logger) = state.devtools.as_ref() {
                let _ = logger.log_event(
                    "proxy.retry",
                    serde_json::json!({
                        "request_id": &request_id,
                        "backend": &backend_name,
                        "status": status.as_u16(),
                        "path": path_and_query,
                    }),
                );
            }

            continue;
        }

        #[cfg(feature = "gateway-routing-advanced")]
        if retry_config.retry_status_codes.contains(&status.as_u16()) {
            record_proxy_backend_failure(
                &state,
                &backend_name,
                _now_epoch_seconds,
                FailureKind::RetryableStatus(status.as_u16()),
                format!("status {}", status.as_u16()),
            )
            .await;
        } else {
            record_proxy_backend_success(&state, &backend_name).await;
        }

        let spend_tokens = status.is_success();

        #[cfg(feature = "gateway-metrics-prometheus")]
        if let Some(metrics) = state.prometheus_metrics.as_ref() {
            let is_failure_status = {
                #[cfg(feature = "gateway-routing-advanced")]
                {
                    retry_config.retry_status_codes.contains(&status.as_u16())
                }
                #[cfg(not(feature = "gateway-routing-advanced"))]
                {
                    status.is_server_error()
                }
            };
            let mut metrics = metrics.lock().await;
            if is_failure_status {
                metrics.record_proxy_backend_failure(&backend_name);
            } else {
                metrics.record_proxy_backend_success(&backend_name);
            }
            metrics.record_proxy_response_status(status.as_u16());
        }

        let upstream_headers = upstream_response.headers().clone();
        let content_type = upstream_headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let is_event_stream = content_type.starts_with("text/event-stream");

        if is_event_stream {
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

            #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
            if _token_budget_reserved {
                #[cfg(feature = "gateway-store-sqlite")]
                if let Some(store) = state.sqlite_store.as_ref() {
                    if spend_tokens {
                        let _ = store
                            .commit_budget_reservation_with_tokens(&request_id, spent_tokens)
                            .await;
                    } else {
                        let _ = store.rollback_budget_reservation(&request_id).await;
                    }
                }
                #[cfg(feature = "gateway-store-redis")]
                if let Some(store) = state.redis_store.as_ref() {
                    if spend_tokens {
                        let _ = store
                            .commit_budget_reservation_with_tokens(&request_id, spent_tokens)
                            .await;
                    } else {
                        let _ = store.rollback_budget_reservation(&request_id).await;
                    }
                }
            } else if let (Some(virtual_key_id), Some(budget)) =
                (virtual_key_id.clone(), budget.clone())
            {
                if spend_tokens {
                    let mut gateway = state.gateway.lock().await;
                    gateway.budget.spend(&virtual_key_id, &budget, spent_tokens);

                    #[cfg(feature = "gateway-costing")]
                    if !use_persistent_budget {
                        if let Some(spent_cost_usd_micros) = spent_cost_usd_micros {
                            gateway.budget.spend_cost_usd_micros(
                                &virtual_key_id,
                                &budget,
                                spent_cost_usd_micros,
                            );
                        }
                    }
                }
            }
            #[cfg(not(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis")))]
            if let (Some(virtual_key_id), Some(budget)) = (virtual_key_id.clone(), budget.clone()) {
                if spend_tokens {
                    let mut gateway = state.gateway.lock().await;
                    gateway.budget.spend(&virtual_key_id, &budget, spent_tokens);

                    #[cfg(feature = "gateway-costing")]
                    if let Some(spent_cost_usd_micros) = spent_cost_usd_micros {
                        gateway.budget.spend_cost_usd_micros(
                            &virtual_key_id,
                            &budget,
                            spent_cost_usd_micros,
                        );
                    }
                }
            }

            #[cfg(all(
                feature = "gateway-costing",
                any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
            ))]
            if _cost_budget_reserved {
                #[cfg(feature = "gateway-store-sqlite")]
                if let Some(store) = state.sqlite_store.as_ref() {
                    if spend_tokens {
                        let _ = store
                            .commit_cost_reservation_with_usd_micros(
                                &request_id,
                                spent_cost_usd_micros.unwrap_or_default(),
                            )
                            .await;
                    } else {
                        let _ = store.rollback_cost_reservation(&request_id).await;
                    }
                }
                #[cfg(feature = "gateway-store-redis")]
                if let Some(store) = state.redis_store.as_ref() {
                    if spend_tokens {
                        let _ = store
                            .commit_cost_reservation_with_usd_micros(
                                &request_id,
                                spent_cost_usd_micros.unwrap_or_default(),
                            )
                            .await;
                    } else {
                        let _ = store.rollback_cost_reservation(&request_id).await;
                    }
                }
            }

            #[cfg(all(
                feature = "gateway-costing",
                any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
            ))]
            if !_cost_budget_reserved && use_persistent_budget && spend_tokens {
                if let (Some(virtual_key_id), Some(spent_cost_usd_micros)) =
                    (virtual_key_id.as_deref(), spent_cost_usd_micros)
                {
                    #[cfg(feature = "gateway-store-sqlite")]
                    if let Some(store) = state.sqlite_store.as_ref() {
                        let _ = store
                            .record_spent_cost_usd_micros(virtual_key_id, spent_cost_usd_micros)
                            .await;
                    }
                    #[cfg(feature = "gateway-store-redis")]
                    if let Some(store) = state.redis_store.as_ref() {
                        let _ = store
                            .record_spent_cost_usd_micros(virtual_key_id, spent_cost_usd_micros)
                            .await;
                    }
                }
            }

            #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
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
                });

                #[cfg(feature = "gateway-store-sqlite")]
                if let Some(store) = state.sqlite_store.as_ref() {
                    let _ = store.append_audit_log("proxy", payload.clone()).await;
                }
                #[cfg(feature = "gateway-store-redis")]
                if let Some(store) = state.redis_store.as_ref() {
                    let _ = store.append_audit_log("proxy", payload.clone()).await;
                }
            }

            emit_json_log(
                &state,
                "proxy.response",
                serde_json::json!({
                    "request_id": &request_id,
                    "backend": &backend_name,
                    "status": status.as_u16(),
                    "attempted_backends": &attempted_backends,
                }),
            );

            #[cfg(feature = "sdk")]
            if let Some(logger) = state.devtools.as_ref() {
                let _ = logger.log_event(
                    "proxy.response",
                    serde_json::json!({
                        "request_id": &request_id,
                        "status": status.as_u16(),
                        "path": path_and_query,
                        "backend": &backend_name,
                    }),
                );
            }

            #[cfg(feature = "gateway-otel")]
            {
                proxy_span.record("cache", tracing::field::display("miss"));
                proxy_span.record("backend", tracing::field::display(&backend_name));
                proxy_span.record("status", tracing::field::display(status.as_u16()));
            }

            return Ok(proxy_response(
                &state,
                upstream_response,
                backend_name,
                request_id.clone(),
                #[cfg(feature = "gateway-proxy-cache")]
                proxy_cache_key.as_deref(),
                #[cfg(not(feature = "gateway-proxy-cache"))]
                None,
                proxy_permit,
            )
            .await);
        }

        let bytes = upstream_response.bytes().await.unwrap_or_default();
        let observed_usage = if spend_tokens && content_type.starts_with("application/json") {
            extract_openai_usage_from_bytes(&bytes)
        } else {
            None
        };

        let spent_tokens = if spend_tokens {
            observed_usage
                .and_then(|usage| usage.total_tokens)
                .unwrap_or(u64::from(charge_tokens))
        } else {
            0
        };

        #[cfg(feature = "gateway-costing")]
        let spent_cost_usd_micros = if spend_tokens {
            model
                .as_deref()
                .and_then(|model| {
                    state.pricing.as_ref().and_then(|pricing| {
                        let usage = observed_usage?;
                        let input = usage.input_tokens?;
                        let output = usage.output_tokens?;
                        pricing.estimate_cost_usd_micros_with_cache(
                            model,
                            clamp_u64_to_u32(input),
                            usage.cache_input_tokens.map(clamp_u64_to_u32),
                            clamp_u64_to_u32(output),
                        )
                    })
                })
                .or(charge_cost_usd_micros)
        } else {
            None
        };
        #[cfg(not(feature = "gateway-costing"))]
        let spent_cost_usd_micros: Option<u64> = None;

        #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
        if _token_budget_reserved {
            #[cfg(feature = "gateway-store-sqlite")]
            if let Some(store) = state.sqlite_store.as_ref() {
                if spend_tokens {
                    let _ = store
                        .commit_budget_reservation_with_tokens(&request_id, spent_tokens)
                        .await;
                } else {
                    let _ = store.rollback_budget_reservation(&request_id).await;
                }
            }
            #[cfg(feature = "gateway-store-redis")]
            if let Some(store) = state.redis_store.as_ref() {
                if spend_tokens {
                    let _ = store
                        .commit_budget_reservation_with_tokens(&request_id, spent_tokens)
                        .await;
                } else {
                    let _ = store.rollback_budget_reservation(&request_id).await;
                }
            }
        } else if let (Some(virtual_key_id), Some(budget)) =
            (virtual_key_id.clone(), budget.clone())
        {
            if spend_tokens {
                let mut gateway = state.gateway.lock().await;
                gateway.budget.spend(&virtual_key_id, &budget, spent_tokens);

                #[cfg(feature = "gateway-costing")]
                if !use_persistent_budget {
                    if let Some(spent_cost_usd_micros) = spent_cost_usd_micros {
                        gateway.budget.spend_cost_usd_micros(
                            &virtual_key_id,
                            &budget,
                            spent_cost_usd_micros,
                        );
                    }
                }
            }
        }
        #[cfg(not(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis")))]
        if let (Some(virtual_key_id), Some(budget)) = (virtual_key_id.clone(), budget.clone()) {
            if spend_tokens {
                let mut gateway = state.gateway.lock().await;
                gateway.budget.spend(&virtual_key_id, &budget, spent_tokens);

                #[cfg(feature = "gateway-costing")]
                if let Some(spent_cost_usd_micros) = spent_cost_usd_micros {
                    gateway.budget.spend_cost_usd_micros(
                        &virtual_key_id,
                        &budget,
                        spent_cost_usd_micros,
                    );
                }
            }
        }

        #[cfg(all(
            feature = "gateway-costing",
            any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
        ))]
        if _cost_budget_reserved {
            #[cfg(feature = "gateway-store-sqlite")]
            if let Some(store) = state.sqlite_store.as_ref() {
                if spend_tokens {
                    let _ = store
                        .commit_cost_reservation_with_usd_micros(
                            &request_id,
                            spent_cost_usd_micros.unwrap_or_default(),
                        )
                        .await;
                } else {
                    let _ = store.rollback_cost_reservation(&request_id).await;
                }
            }
            #[cfg(feature = "gateway-store-redis")]
            if let Some(store) = state.redis_store.as_ref() {
                if spend_tokens {
                    let _ = store
                        .commit_cost_reservation_with_usd_micros(
                            &request_id,
                            spent_cost_usd_micros.unwrap_or_default(),
                        )
                        .await;
                } else {
                    let _ = store.rollback_cost_reservation(&request_id).await;
                }
            }
        }

        #[cfg(all(
            feature = "gateway-costing",
            any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
        ))]
        if !_cost_budget_reserved && use_persistent_budget && spend_tokens {
            if let (Some(virtual_key_id), Some(spent_cost_usd_micros)) =
                (virtual_key_id.as_deref(), spent_cost_usd_micros)
            {
                #[cfg(feature = "gateway-store-sqlite")]
                if let Some(store) = state.sqlite_store.as_ref() {
                    let _ = store
                        .record_spent_cost_usd_micros(virtual_key_id, spent_cost_usd_micros)
                        .await;
                }
                #[cfg(feature = "gateway-store-redis")]
                if let Some(store) = state.redis_store.as_ref() {
                    let _ = store
                        .record_spent_cost_usd_micros(virtual_key_id, spent_cost_usd_micros)
                        .await;
                }
            }
        }

        #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
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
            });

            #[cfg(feature = "gateway-store-sqlite")]
            if let Some(store) = state.sqlite_store.as_ref() {
                let _ = store.append_audit_log("proxy", payload.clone()).await;
            }
            #[cfg(feature = "gateway-store-redis")]
            if let Some(store) = state.redis_store.as_ref() {
                let _ = store.append_audit_log("proxy", payload.clone()).await;
            }
        }

        emit_json_log(
            &state,
            "proxy.response",
            serde_json::json!({
                "request_id": &request_id,
                "backend": &backend_name,
                "status": status.as_u16(),
                "attempted_backends": &attempted_backends,
            }),
        );

        #[cfg(feature = "sdk")]
        if let Some(logger) = state.devtools.as_ref() {
            let _ = logger.log_event(
                "proxy.response",
                serde_json::json!({
                    "request_id": &request_id,
                    "status": status.as_u16(),
                    "path": path_and_query,
                    "backend": &backend_name,
                }),
            );
        }

        #[cfg(feature = "gateway-otel")]
        {
            proxy_span.record("cache", tracing::field::display("miss"));
            proxy_span.record("backend", tracing::field::display(&backend_name));
            proxy_span.record("status", tracing::field::display(status.as_u16()));
        }

        #[cfg(feature = "gateway-proxy-cache")]
        if status.is_success() {
            if let (Some(cache), Some(cache_key)) =
                (state.proxy_cache.as_ref(), proxy_cache_key.as_deref())
            {
                let now = now_epoch_seconds();
                let cached = CachedProxyResponse {
                    status: status.as_u16(),
                    headers: upstream_headers.clone(),
                    body: bytes.clone(),
                    backend: backend_name.clone(),
                };
                let mut cache = cache.lock().await;
                cache.insert(cache_key.to_string(), cached, now);
            }
        }

        let mut headers = upstream_headers;
        apply_proxy_response_headers(&mut headers, &backend_name, &request_id, false);
        let mut response = axum::response::Response::new(Body::from(bytes));
        *response.status_mut() = status;
        *response.headers_mut() = headers;
        return Ok(response);
    }

    #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
    if _token_budget_reserved {
        #[cfg(feature = "gateway-store-sqlite")]
        if let Some(store) = state.sqlite_store.as_ref() {
            let _ = store.rollback_budget_reservation(&request_id).await;
        }
        #[cfg(feature = "gateway-store-redis")]
        if let Some(store) = state.redis_store.as_ref() {
            let _ = store.rollback_budget_reservation(&request_id).await;
        }
    }

    #[cfg(all(
        feature = "gateway-costing",
        any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
    ))]
    if _cost_budget_reserved {
        #[cfg(feature = "gateway-store-sqlite")]
        if let Some(store) = state.sqlite_store.as_ref() {
            let _ = store.rollback_cost_reservation(&request_id).await;
        }
        #[cfg(feature = "gateway-store-redis")]
        if let Some(store) = state.redis_store.as_ref() {
            let _ = store.rollback_cost_reservation(&request_id).await;
        }
    }

    #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
    {
        let (status, err_kind, err_code, err_message) = match last_err.as_ref() {
            Some((status, body)) => (
                Some(status.as_u16()),
                Some(body.0.error.kind),
                body.0.error.code,
                Some(body.0.error.message.as_str()),
            ),
            None => (None, None, None, None),
        };
        let payload = serde_json::json!({
            "request_id": &request_id,
            "virtual_key_id": virtual_key_id.as_deref(),
            "attempted_backends": &attempted_backends,
            "method": parts.method.as_str(),
            "path": path_and_query,
            "model": &model,
            "charge_tokens": charge_tokens,
            "charge_cost_usd_micros": charge_cost_usd_micros,
            "body_len": body.len(),
            "status": status,
            "error_type": err_kind,
            "error_code": err_code,
            "error_message": err_message,
        });

        #[cfg(feature = "gateway-store-sqlite")]
        if let Some(store) = state.sqlite_store.as_ref() {
            let _ = store.append_audit_log("proxy.error", payload.clone()).await;
        }
        #[cfg(feature = "gateway-store-redis")]
        if let Some(store) = state.redis_store.as_ref() {
            let _ = store.append_audit_log("proxy.error", payload.clone()).await;
        }
    }

    emit_json_log(
        &state,
        "proxy.error",
        serde_json::json!({
            "request_id": &request_id,
            "attempted_backends": &attempted_backends,
            "status": last_err.as_ref().map(|(status, _)| status.as_u16()),
        }),
    );

    #[cfg(feature = "gateway-metrics-prometheus")]
    if let Some(metrics) = state.prometheus_metrics.as_ref() {
        let status = last_err
            .as_ref()
            .map(|(status, _)| status.as_u16())
            .unwrap_or(StatusCode::BAD_GATEWAY.as_u16());
        metrics.lock().await.record_proxy_response_status(status);
    }

    Err(last_err.unwrap_or_else(|| {
        openai_error(
            StatusCode::BAD_GATEWAY,
            "api_error",
            Some("backend_error"),
            "all backends failed",
        )
    }))
}

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

fn clamp_u64_to_u32(value: u64) -> u32 {
    if value > u64::from(u32::MAX) {
        u32::MAX
    } else {
        value as u32
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ObservedUsage {
    input_tokens: Option<u64>,
    cache_input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    total_tokens: Option<u64>,
}

fn extract_openai_usage_from_bytes(bytes: &Bytes) -> Option<ObservedUsage> {
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let usage = value.get("usage")?.as_object()?;
    let total_tokens = usage.get("total_tokens").and_then(|v| v.as_u64());
    let input_tokens = usage
        .get("input_tokens")
        .or_else(|| usage.get("prompt_tokens"))
        .and_then(|v| v.as_u64());
    let cache_input_tokens = usage
        .get("input_tokens_details")
        .or_else(|| usage.get("prompt_tokens_details"))
        .and_then(|details| details.get("cached_tokens"))
        .and_then(|v| v.as_u64());
    let output_tokens = usage
        .get("output_tokens")
        .or_else(|| usage.get("completion_tokens"))
        .and_then(|v| v.as_u64());
    let total_tokens = total_tokens.or_else(|| {
        input_tokens.and_then(|input| output_tokens.map(|output| input.saturating_add(output)))
    });
    Some(ObservedUsage {
        input_tokens,
        cache_input_tokens,
        output_tokens,
        total_tokens,
    })
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

fn sanitize_proxy_headers(headers: &mut HeaderMap, strip_authorization: bool) {
    if strip_authorization {
        headers.remove("authorization");
    }
    headers.remove("x-ditto-virtual-key");
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
    if !state.json_logs {
        return;
    }

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

type ProxyBodyStream = BoxStream<'static, Result<Bytes, std::io::Error>>;

struct ProxyBodyStreamWithPermit {
    inner: ProxyBodyStream,
    _permit: Option<OwnedSemaphorePermit>,
}

impl futures_util::Stream for ProxyBodyStreamWithPermit {
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        this.inner.as_mut().poll_next(cx)
    }
}

async fn proxy_response(
    _state: &GatewayHttpState,
    upstream: reqwest::Response,
    backend: String,
    request_id: String,
    _cache_key: Option<&str>,
    proxy_permit: Option<OwnedSemaphorePermit>,
) -> axum::response::Response {
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
        headers.remove("content-length");
        let stream = upstream
            .bytes_stream()
            .map(|chunk| chunk.map_err(std::io::Error::other))
            .boxed();
        let stream = ProxyBodyStreamWithPermit {
            inner: stream,
            _permit: proxy_permit,
        };
        let mut response = axum::response::Response::new(Body::from_stream(stream));
        *response.status_mut() = status;
        *response.headers_mut() = headers;
        response
    } else {
        let bytes = upstream.bytes().await.unwrap_or_default();
        #[cfg(feature = "gateway-proxy-cache")]
        if status.is_success() {
            if let (Some(cache), Some(cache_key)) = (_state.proxy_cache.as_ref(), _cache_key) {
                let now = now_epoch_seconds();
                let cached = CachedProxyResponse {
                    status: status.as_u16(),
                    headers: upstream_headers.clone(),
                    body: bytes.clone(),
                    backend: backend.clone(),
                };
                let mut cache = cache.lock().await;
                cache.insert(cache_key.to_string(), cached, now);
            }
        }

        let mut headers = upstream_headers;
        apply_proxy_response_headers(&mut headers, &backend, &request_id, false);
        let mut response = axum::response::Response::new(Body::from(bytes));
        *response.status_mut() = status;
        *response.headers_mut() = headers;
        response
    }
}

async fn responses_shim_response(
    _state: &GatewayHttpState,
    upstream: reqwest::Response,
    backend: String,
    request_id: String,
    _cache_key: Option<&str>,
    proxy_permit: Option<OwnedSemaphorePermit>,
) -> Result<axum::response::Response, (StatusCode, Json<OpenAiErrorResponse>)> {
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
        let stream = ProxyBodyStreamWithPermit {
            inner: stream.boxed(),
            _permit: proxy_permit,
        };
        let mut headers = upstream_headers;
        headers.insert(
            "x-ditto-shim",
            "responses_via_chat_completions".parse().unwrap(),
        );
        headers.insert("content-type", "text/event-stream".parse().unwrap());
        headers.remove("content-length");
        apply_proxy_response_headers(&mut headers, &backend, &request_id, false);
        let mut response = axum::response::Response::new(Body::from_stream(stream));
        *response.status_mut() = status;
        *response.headers_mut() = headers;
        Ok(response)
    } else {
        let bytes = upstream.bytes().await.unwrap_or_default();
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
            "responses_via_chat_completions".parse().unwrap(),
        );
        headers.insert("content-type", "application/json".parse().unwrap());
        headers.remove("content-length");

        #[cfg(feature = "gateway-proxy-cache")]
        if status.is_success() {
            if let (Some(cache), Some(cache_key)) = (_state.proxy_cache.as_ref(), _cache_key) {
                let now = now_epoch_seconds();
                let cached = CachedProxyResponse {
                    status: status.as_u16(),
                    headers: headers.clone(),
                    body: mapped_bytes.clone(),
                    backend: backend.clone(),
                };
                let mut cache = cache.lock().await;
                cache.insert(cache_key.to_string(), cached, now);
            }
        }

        apply_proxy_response_headers(&mut headers, &backend, &request_id, false);
        let mut response = axum::response::Response::new(Body::from(mapped_bytes));
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
        backend
            .parse()
            .unwrap_or_else(|_| "unknown".parse().unwrap()),
    );
    if cache_hit {
        headers.insert("x-ditto-cache", "hit".parse().unwrap());
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
    let Some(config) = state.proxy_routing.as_ref() else {
        return candidates;
    };
    if !config.circuit_breaker.enabled && !config.health_check.enabled {
        return candidates;
    }
    let Some(health) = state.proxy_backend_health.as_ref() else {
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
    let Some(config) = state.proxy_routing.as_ref() else {
        return;
    };
    let Some(health) = state.proxy_backend_health.as_ref() else {
        return;
    };

    let mut health = health.lock().await;
    let entry = health.entry(backend.to_string()).or_default();
    entry.record_failure(now_epoch_seconds, &config.circuit_breaker, kind, message);
}

#[cfg(feature = "gateway-routing-advanced")]
async fn record_proxy_backend_success(state: &GatewayHttpState, backend: &str) {
    let Some(health) = state.proxy_backend_health.as_ref() else {
        return;
    };

    let mut health = health.lock().await;
    health
        .entry(backend.to_string())
        .or_default()
        .record_success();
}

#[cfg(feature = "gateway-routing-advanced")]
fn start_proxy_health_checks(state: &GatewayHttpState) {
    let Some(config) = state.proxy_routing.as_ref() else {
        return;
    };
    if !config.health_check.enabled {
        return;
    }
    let Some(health) = state.proxy_backend_health.as_ref() else {
        return;
    };

    let backends = state.proxy_backends.clone();
    let health = health.clone();
    let path = config.health_check.path.clone();
    let interval = Duration::from_secs(config.health_check.interval_seconds.max(1));
    let timeout = Duration::from_secs(config.health_check.timeout_seconds.max(1));

    tokio::spawn(async move {
        loop {
            for (backend_name, backend) in backends.iter() {
                let mut headers = HeaderMap::new();
                apply_backend_headers(&mut headers, backend.headers());

                let result = backend
                    .request_with_timeout(reqwest::Method::GET, &path, headers, None, Some(timeout))
                    .await;

                let mut health = health.lock().await;
                let entry = health.entry(backend_name.clone()).or_default();
                match result {
                    Ok(response) => {
                        if response.status().is_success() {
                            entry.record_health_check_success();
                        } else {
                            entry.record_health_check_failure(format!(
                                "health check returned {}",
                                response.status()
                            ));
                        }
                    }
                    Err(err) => {
                        entry.record_health_check_failure(err.to_string());
                    }
                }
            }

            tokio::time::sleep(interval).await;
        }
    });
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

    "public".to_string()
}

#[cfg(feature = "gateway-proxy-cache")]
fn proxy_cache_key(method: &axum::http::Method, path: &str, body: &Bytes, scope: &str) -> String {
    let body_hash = hash64_fnv1a(body);
    let seed = format!("{}|{}|{}|{:016x}", method.as_str(), path, scope, body_hash);
    let hash = hash64_fnv1a(seed.as_bytes());
    format!("ditto-proxy-cache-v1-{hash:016x}")
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
        GatewayError::InvalidRequest { reason } => openai_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            Some("invalid_request"),
            reason,
        ),
    }
}

fn try_acquire_proxy_permit(
    state: &GatewayHttpState,
) -> Result<Option<OwnedSemaphorePermit>, (StatusCode, Json<OpenAiErrorResponse>)> {
    let Some(limit) = state.proxy_backpressure.as_ref() else {
        return Ok(None);
    };
    limit.clone().try_acquire_owned().map(Some).map_err(|_| {
        openai_error(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limit_error",
            Some("inflight_limit"),
            "too many in-flight proxy requests",
        )
    })
}

fn openai_error(
    status: StatusCode,
    kind: &'static str,
    code: Option<&'static str>,
    message: impl ToString,
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

async fn list_keys(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Query(query): Query<ListKeysQuery>,
) -> Result<Json<Vec<VirtualKeyConfig>>, (StatusCode, Json<ErrorResponse>)> {
    ensure_admin(&state, &headers)?;
    let gateway = state.gateway.lock().await;
    let mut keys = gateway.list_virtual_keys();
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
    ensure_admin(&state, &headers)?;
    if let Err(err) = key.guardrails.validate() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            format!("invalid guardrails config: {err}"),
        ));
    }
    let (inserted, persisted_keys) = {
        let mut gateway = state.gateway.lock().await;
        let inserted = gateway.upsert_virtual_key(key.clone());
        (inserted, gateway.list_virtual_keys())
    };
    persist_virtual_keys(&state, &persisted_keys).await?;

    #[cfg(feature = "sdk")]
    if let Some(logger) = state.devtools.as_ref() {
        let _ = logger.log_event(
            "admin.key.upsert",
            serde_json::json!({
                "key_id": &key.id,
                "enabled": key.enabled,
                "inserted": inserted,
            }),
        );
    }

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
    ensure_admin(&state, &headers)?;
    key.id = id;
    if let Err(err) = key.guardrails.validate() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            format!("invalid guardrails config: {err}"),
        ));
    }
    let (inserted, persisted_keys) = {
        let mut gateway = state.gateway.lock().await;
        let inserted = gateway.upsert_virtual_key(key.clone());
        (inserted, gateway.list_virtual_keys())
    };
    persist_virtual_keys(&state, &persisted_keys).await?;

    #[cfg(feature = "sdk")]
    if let Some(logger) = state.devtools.as_ref() {
        let _ = logger.log_event(
            "admin.key.upsert",
            serde_json::json!({
                "key_id": &key.id,
                "enabled": key.enabled,
                "inserted": inserted,
            }),
        );
    }

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
    ensure_admin(&state, &headers)?;
    let (removed, persisted_keys) = {
        let mut gateway = state.gateway.lock().await;
        let removed = gateway.remove_virtual_key(&id).is_some();
        (removed, gateway.list_virtual_keys())
    };
    if removed {
        persist_virtual_keys(&state, &persisted_keys).await?;

        #[cfg(feature = "sdk")]
        if let Some(logger) = state.devtools.as_ref() {
            let _ = logger.log_event(
                "admin.key.delete",
                serde_json::json!({
                    "key_id": &id,
                }),
            );
        }

        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "virtual key not found",
        ))
    }
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
#[derive(Debug, Deserialize)]
struct AuditQuery {
    #[serde(default = "default_audit_limit")]
    limit: usize,
    #[serde(default)]
    since_ts_ms: Option<u64>,
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
fn default_audit_limit() -> usize {
    100
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
async fn list_audit_logs(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Query(query): Query<AuditQuery>,
) -> Result<Json<Vec<AuditLogRecord>>, (StatusCode, Json<ErrorResponse>)> {
    ensure_admin(&state, &headers)?;

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.sqlite_store.as_ref() {
        let logs = store
            .list_audit_logs(query.limit.min(1000), query.since_ts_ms)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;
        return Ok(Json(logs));
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.redis_store.as_ref() {
        let logs = store
            .list_audit_logs(query.limit.min(1000), query.since_ts_ms)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;
        return Ok(Json(logs));
    }

    Err(error_response(
        StatusCode::BAD_REQUEST,
        "not_configured",
        "store not configured",
    ))
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
async fn list_budget_ledgers(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
) -> Result<Json<Vec<BudgetLedgerRecord>>, (StatusCode, Json<ErrorResponse>)> {
    ensure_admin(&state, &headers)?;

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.sqlite_store.as_ref() {
        let ledgers = store.list_budget_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(ledgers));
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.redis_store.as_ref() {
        let ledgers = store.list_budget_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
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
    any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
))]
async fn list_cost_ledgers(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
) -> Result<Json<Vec<CostLedgerRecord>>, (StatusCode, Json<ErrorResponse>)> {
    ensure_admin(&state, &headers)?;

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.sqlite_store.as_ref() {
        let ledgers = store.list_cost_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(ledgers));
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.redis_store.as_ref() {
        let ledgers = store.list_cost_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(ledgers));
    }

    Err(error_response(
        StatusCode::BAD_REQUEST,
        "not_configured",
        "store not configured",
    ))
}

#[cfg(feature = "gateway-routing-advanced")]
async fn list_backends(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
) -> Result<Json<Vec<BackendHealthSnapshot>>, (StatusCode, Json<ErrorResponse>)> {
    ensure_admin(&state, &headers)?;

    let Some(health) = state.proxy_backend_health.as_ref() else {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "not_configured",
            "proxy routing not enabled",
        ));
    };

    let mut names: Vec<String> = state.proxy_backends.keys().cloned().collect();
    names.sort();

    let mut out = Vec::with_capacity(names.len());
    let health = health.lock().await;
    for name in names {
        let snapshot = health
            .get(name.as_str())
            .map(|entry| entry.snapshot(&name))
            .unwrap_or_else(|| BackendHealth::default().snapshot(&name));
        out.push(snapshot);
    }

    Ok(Json(out))
}

#[cfg(feature = "gateway-routing-advanced")]
async fn reset_backend(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<Json<BackendHealthSnapshot>, (StatusCode, Json<ErrorResponse>)> {
    ensure_admin(&state, &headers)?;

    let Some(health) = state.proxy_backend_health.as_ref() else {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "not_configured",
            "proxy routing not enabled",
        ));
    };

    let mut health = health.lock().await;
    health.remove(name.as_str());
    Ok(Json(BackendHealth::default().snapshot(&name)))
}

fn ensure_admin(
    state: &GatewayHttpState,
    headers: &HeaderMap,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let Some(expected) = state.admin_token.as_deref() else {
        return Ok(());
    };
    let provided = extract_bearer(headers)
        .or_else(|| extract_header(headers, "x-admin-token"))
        .unwrap_or_default();
    if provided == expected {
        Ok(())
    } else {
        Err(error_response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "invalid admin token",
        ))
    }
}

fn extract_header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
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
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    GatewayStateFile {
        virtual_keys: keys.to_vec(),
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
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.sqlite_store.as_ref() {
        store.replace_virtual_keys(keys).await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(());
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.redis_store.as_ref() {
        store.replace_virtual_keys(keys).await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(());
    }

    if let Some(path) = state.state_file.as_ref() {
        persist_state_file(path.as_path(), keys)?;
    }

    Ok(())
}
