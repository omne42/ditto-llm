use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path as StdPath, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

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
    #[cfg(feature = "gateway-proxy-cache")]
    proxy_cache_config: Option<ProxyCacheConfig>,
    proxy_max_body_bytes: usize,
    proxy_backpressure: Option<Arc<Semaphore>>,
    proxy_backend_backpressure: Arc<HashMap<String, Arc<Semaphore>>>,
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
            #[cfg(feature = "gateway-proxy-cache")]
            proxy_cache_config: None,
            proxy_max_body_bytes: 64 * 1024 * 1024,
            proxy_backpressure: None,
            proxy_backend_backpressure: Arc::new(proxy_backend_backpressure),
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

    pub fn with_proxy_max_body_bytes(mut self, max_body_bytes: usize) -> Self {
        self.proxy_max_body_bytes = max_body_bytes.max(1);
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

    if state.admin_token.is_some() {
        router = router
            .route("/admin/keys", get(list_keys).post(upsert_key))
            .route(
                "/admin/keys/:id",
                put(upsert_key_with_id).delete(delete_key),
            );

        #[cfg(feature = "gateway-proxy-cache")]
        if state.proxy_cache.is_some() {
            router = router.route("/admin/proxy_cache/purge", post(purge_proxy_cache));
        }

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
                .route("/admin/budgets", get(list_budget_ledgers))
                .route("/admin/budgets/projects", get(list_project_budget_ledgers))
                .route("/admin/budgets/users", get(list_user_budget_ledgers));

            #[cfg(feature = "gateway-costing")]
            {
                router = router
                    .route("/admin/costs", get(list_cost_ledgers))
                    .route("/admin/costs/projects", get(list_project_cost_ledgers))
                    .route("/admin/costs/users", get(list_user_cost_ledgers));
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

#[cfg(feature = "gateway-proxy-cache")]
#[derive(Debug, Deserialize)]
struct PurgeProxyCacheRequest {
    #[serde(default)]
    all: bool,
    #[serde(default)]
    cache_key: Option<String>,
}

#[cfg(feature = "gateway-proxy-cache")]
#[derive(Debug, Serialize)]
struct PurgeProxyCacheResponse {
    cleared_memory: bool,
    deleted_redis: Option<u64>,
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

    let use_virtual_keys = gateway_uses_virtual_keys(&state).await;

    let mut headers = HeaderMap::new();
    headers.insert("content-type", "application/json".parse().unwrap());
    if stream_requested {
        headers.insert("accept", "text/event-stream".parse().unwrap());
    }
    if let Some(value) = parts.headers.get("authorization") {
        headers.insert("authorization", value.clone());
    }
    if use_virtual_keys && !headers.contains_key("authorization") {
        let token = extract_header(&parts.headers, "x-ditto-virtual-key")
            .or_else(|| extract_header(&parts.headers, "x-api-key"))
            .or_else(|| extract_bearer(&parts.headers));
        if let Some(token) = token.and_then(|t| synthesize_bearer_header(&t)) {
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
            parts
                .headers
                .insert("content-type", "text/event-stream".parse().unwrap());
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
    response
        .headers_mut()
        .insert("content-type", "application/json".parse().unwrap());
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
    if gateway_uses_virtual_keys(&state).await {
        let token = extract_header(&parts.headers, "x-ditto-virtual-key")
            .or_else(|| extract_header(&parts.headers, "x-api-key"))
            .or_else(|| extract_bearer(&parts.headers))
            .ok_or_else(|| {
                anthropic_error(
                    StatusCode::UNAUTHORIZED,
                    "authentication_error",
                    "missing api key",
                )
            })?;
        let gateway = state.gateway.lock().await;
        let authorized = gateway
            .config
            .virtual_keys
            .iter()
            .any(|key| key.enabled && key.token == token);
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
