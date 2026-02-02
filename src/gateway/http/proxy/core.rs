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
    let prompt_token_details = usage
        .get("input_tokens_details")
        .or_else(|| usage.get("prompt_tokens_details"));
    let cache_input_tokens = prompt_token_details
        .and_then(|details| details.get("cached_tokens"))
        .and_then(|v| v.as_u64());
    let cache_creation_input_tokens = usage
        .get("cache_creation_input_tokens")
        .and_then(|v| v.as_u64())
        .or_else(|| {
            prompt_token_details
                .and_then(|details| details.get("cache_creation_tokens"))
                .and_then(|v| v.as_u64())
        });
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
        cache_creation_input_tokens,
        output_tokens,
        total_tokens,
    })
}

fn sanitize_proxy_headers(headers: &mut HeaderMap, strip_authorization: bool) {
    if strip_authorization {
        headers.remove("authorization");
        headers.remove("x-api-key");
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

async fn proxy_response(
    _state: &GatewayHttpState,
    upstream: reqwest::Response,
    backend: String,
    request_id: String,
    _cache_key: Option<&str>,
    proxy_permits: ProxyPermits,
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
        if let Some(cache_key) = _cache_key {
            if let Ok(value) = axum::http::HeaderValue::from_str(cache_key) {
                headers.insert("x-ditto-cache-key", value);
            }
        }
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
    } else {
        let cacheable = status.is_success() && _cache_key.is_some();
        let content_length = upstream_headers
            .get("content-length")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<usize>().ok());
        let should_buffer = cacheable
            && {
                #[cfg(feature = "gateway-proxy-cache")]
                {
                    _state.proxy_cache_config.as_ref().is_some_and(|config| {
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
            let bytes = upstream.bytes().await.unwrap_or_default();

            #[cfg(feature = "gateway-proxy-cache")]
            if status.is_success() {
                if let Some(cache_key) = _cache_key {
                    let cached = CachedProxyResponse {
                        status: status.as_u16(),
                        headers: headers.clone(),
                        body: bytes.clone(),
                        backend: backend.clone(),
                    };
                    store_proxy_cache_response(_state, cache_key, cached, now_epoch_seconds()).await;
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
    _state: &GatewayHttpState,
    upstream: reqwest::Response,
    backend: String,
    request_id: String,
    _cache_key: Option<&str>,
    proxy_permits: ProxyPermits,
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
            _permits: proxy_permits,
        };
        let mut headers = upstream_headers;
        headers.insert(
            "x-ditto-shim",
            "responses_via_chat_completions".parse().unwrap(),
        );
        headers.insert("content-type", "text/event-stream".parse().unwrap());
        headers.remove("content-length");
        apply_proxy_response_headers(&mut headers, &backend, &request_id, false);
        if let Some(cache_key) = _cache_key {
            if let Ok(value) = axum::http::HeaderValue::from_str(cache_key) {
                headers.insert("x-ditto-cache-key", value);
            }
        }
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
            "responses_via_chat_completions".parse().unwrap(),
        );
        headers.insert("content-type", "application/json".parse().unwrap());
        headers.remove("content-length");

        #[cfg(feature = "gateway-proxy-cache")]
        if status.is_success() {
            if let Some(cache_key) = _cache_key {
                let cached = CachedProxyResponse {
                    status: status.as_u16(),
                    headers: headers.clone(),
                    body: mapped_bytes.clone(),
                    backend: backend.clone(),
                };
                store_proxy_cache_response(_state, cache_key, cached, now_epoch_seconds()).await;
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

#[cfg(feature = "gateway-proxy-cache")]
async fn store_proxy_cache_response(
    state: &GatewayHttpState,
    cache_key: &str,
    cached: CachedProxyResponse,
    now_epoch_seconds: u64,
) {
    if let Some(config) = state.proxy_cache_config.as_ref() {
        if cached.body.len() > config.max_body_bytes {
            return;
        }
    }

    #[cfg(feature = "gateway-metrics-prometheus")]
    let mut redis_store_error: Option<bool> = None;

    #[cfg(feature = "gateway-store-redis")]
    if let (Some(store), Some(config)) = (
        state.redis_store.as_ref(),
        state.proxy_cache_config.as_ref(),
    ) {
        #[cfg(feature = "gateway-metrics-prometheus")]
        {
            let result = store
                .set_proxy_cache_response(cache_key, &cached, config.ttl_seconds)
                .await;
            redis_store_error = Some(result.is_err());
        }

        #[cfg(not(feature = "gateway-metrics-prometheus"))]
        {
            let _ = store
                .set_proxy_cache_response(cache_key, &cached, config.ttl_seconds)
                .await;
        }
    }

    if let Some(cache) = state.proxy_cache.as_ref() {
        let mut cache = cache.lock().await;
        cache.insert(cache_key.to_string(), cached, now_epoch_seconds);
    }

    #[cfg(feature = "gateway-metrics-prometheus")]
    if let Some(metrics) = state.prometheus_metrics.as_ref() {
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
fn start_proxy_health_checks(state: &GatewayHttpState) -> Option<Arc<AbortOnDrop>> {
    let Some(config) = state.proxy_routing.as_ref() else {
        return None;
    };
    if !config.health_check.enabled {
        return None;
    }
    let Some(health) = state.proxy_backend_health.as_ref() else {
        return None;
    };

    let backends = state.proxy_backends.clone();
    let health = health.clone();
    let path = config.health_check.path.clone();
    let interval = Duration::from_secs(config.health_check.interval_seconds.max(1));
    let timeout = Duration::from_secs(config.health_check.timeout_seconds.max(1));

    let task = tokio::spawn(async move {
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
