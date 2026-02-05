async fn proxy_response(
    _state: &GatewayHttpState,
    upstream: reqwest::Response,
    backend: String,
    request_id: String,
    #[cfg(feature = "gateway-metrics-prometheus")]
    metrics_path: &str,
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

        let upstream_stream: ProxyBodyStream = upstream
            .bytes_stream()
            .map(|chunk| chunk.map_err(std::io::Error::other))
            .boxed();

        #[cfg(feature = "gateway-metrics-prometheus")]
        {
            #[derive(Clone, Copy, Debug)]
            enum StreamEnd {
                Completed,
                Error,
                Aborted,
            }

            struct StreamFinalizer {
                metrics: Option<Arc<Mutex<super::metrics_prometheus::PrometheusMetrics>>>,
                backend: String,
                path: String,
            }

            impl StreamFinalizer {
                async fn finalize(self, end: StreamEnd, stream_bytes: u64) {
                    let Some(metrics) = self.metrics else {
                        return;
                    };
                    let mut metrics = metrics.lock().await;
                    metrics.record_proxy_stream_close(&self.backend, &self.path);
                    metrics.record_proxy_stream_bytes(&self.backend, &self.path, stream_bytes);
                    match end {
                        StreamEnd::Completed => {
                            metrics.record_proxy_stream_completed(&self.backend, &self.path);
                        }
                        StreamEnd::Error => {
                            metrics.record_proxy_stream_error(&self.backend, &self.path);
                        }
                        StreamEnd::Aborted => {
                            metrics.record_proxy_stream_aborted(&self.backend, &self.path);
                        }
                    }
                }
            }

            struct StreamState {
                upstream: ProxyBodyStream,
                bytes_sent: u64,
                finalizer: Option<StreamFinalizer>,
                _permits: ProxyPermits,
            }

            impl Drop for StreamState {
                fn drop(&mut self) {
                    let Some(finalizer) = self.finalizer.take() else {
                        return;
                    };
                    let bytes_sent = self.bytes_sent;
                    tokio::spawn(async move {
                        finalizer
                            .finalize(StreamEnd::Aborted, bytes_sent)
                            .await;
                    });
                }
            }

            impl StreamState {
                async fn finalize(&mut self, end: StreamEnd) {
                    let Some(finalizer) = self.finalizer.take() else {
                        return;
                    };
                    let bytes_sent = self.bytes_sent;
                    finalizer.finalize(end, bytes_sent).await;
                }
            }

            let metrics = _state.prometheus_metrics.clone();
            if let Some(metrics) = metrics.as_ref() {
                metrics.lock().await.record_proxy_stream_open(&backend, metrics_path);
            }

            let finalizer = StreamFinalizer {
                metrics,
                backend: backend.clone(),
                path: metrics_path.to_string(),
            };

            let state = StreamState {
                upstream: upstream_stream,
                bytes_sent: 0,
                finalizer: Some(finalizer),
                _permits: proxy_permits,
            };

            let stream = futures_util::stream::try_unfold(state, |mut state| async move {
                match state.upstream.next().await {
                    Some(Ok(chunk)) => {
                        state.bytes_sent = state.bytes_sent.saturating_add(chunk.len() as u64);
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
            response
        }

        #[cfg(not(feature = "gateway-metrics-prometheus"))]
        {
            let stream = ProxyBodyStreamWithPermit {
                inner: upstream_stream,
                _permits: proxy_permits,
            };
            let mut response = axum::response::Response::new(Body::from_stream(stream));
            *response.status_mut() = status;
            *response.headers_mut() = headers;
            response
        }
    } else {
        let cacheable = status.is_success() && _cache_key.is_some();
        let should_buffer = cacheable
            && {
                #[cfg(feature = "gateway-proxy-cache")]
                {
	                    let content_length = upstream_headers
	                        .get("content-length")
	                        .and_then(|value| value.to_str().ok())
	                        .and_then(|value| value.parse::<usize>().ok());
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
	            let max_body_bytes =
	                _state.proxy_cache_config.as_ref().map(|c| c.max_body_bytes).unwrap_or(1);
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
    #[cfg(feature = "gateway-metrics-prometheus")]
    metrics_path: &str,
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
        let upstream_stream: ProxyBodyStream = stream.boxed();
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

        #[cfg(feature = "gateway-metrics-prometheus")]
        {
            #[derive(Clone, Copy, Debug)]
            enum StreamEnd {
                Completed,
                Error,
                Aborted,
            }

            struct StreamFinalizer {
                metrics: Option<Arc<Mutex<super::metrics_prometheus::PrometheusMetrics>>>,
                backend: String,
                path: String,
            }

            impl StreamFinalizer {
                async fn finalize(self, end: StreamEnd, stream_bytes: u64) {
                    let Some(metrics) = self.metrics else {
                        return;
                    };
                    let mut metrics = metrics.lock().await;
                    metrics.record_proxy_stream_close(&self.backend, &self.path);
                    metrics.record_proxy_stream_bytes(&self.backend, &self.path, stream_bytes);
                    match end {
                        StreamEnd::Completed => {
                            metrics.record_proxy_stream_completed(&self.backend, &self.path);
                        }
                        StreamEnd::Error => {
                            metrics.record_proxy_stream_error(&self.backend, &self.path);
                        }
                        StreamEnd::Aborted => {
                            metrics.record_proxy_stream_aborted(&self.backend, &self.path);
                        }
                    }
                }
            }

            struct StreamState {
                upstream: ProxyBodyStream,
                bytes_sent: u64,
                finalizer: Option<StreamFinalizer>,
                _permits: ProxyPermits,
            }

            impl Drop for StreamState {
                fn drop(&mut self) {
                    let Some(finalizer) = self.finalizer.take() else {
                        return;
                    };
                    let bytes_sent = self.bytes_sent;
                    tokio::spawn(async move {
                        finalizer
                            .finalize(StreamEnd::Aborted, bytes_sent)
                            .await;
                    });
                }
            }

            impl StreamState {
                async fn finalize(&mut self, end: StreamEnd) {
                    let Some(finalizer) = self.finalizer.take() else {
                        return;
                    };
                    let bytes_sent = self.bytes_sent;
                    finalizer.finalize(end, bytes_sent).await;
                }
            }

            let metrics = _state.prometheus_metrics.clone();
            if let Some(metrics) = metrics.as_ref() {
                metrics.lock().await.record_proxy_stream_open(&backend, metrics_path);
            }

            let finalizer = StreamFinalizer {
                metrics,
                backend: backend.clone(),
                path: metrics_path.to_string(),
            };

            let state = StreamState {
                upstream: upstream_stream,
                bytes_sent: 0,
                finalizer: Some(finalizer),
                _permits: proxy_permits,
            };

            let stream = futures_util::stream::try_unfold(state, |mut state| async move {
                match state.upstream.next().await {
                    Some(Ok(chunk)) => {
                        state.bytes_sent = state.bytes_sent.saturating_add(chunk.len() as u64);
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
            Ok(response)
        }

        #[cfg(not(feature = "gateway-metrics-prometheus"))]
        {
            let stream = ProxyBodyStreamWithPermit {
                inner: upstream_stream,
                _permits: proxy_permits,
            };
            let mut response = axum::response::Response::new(Body::from_stream(stream));
            *response.status_mut() = status;
            *response.headers_mut() = headers;
            Ok(response)
        }
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
    let config = state.proxy_routing.as_ref()?;
    if !config.health_check.enabled {
        return None;
    }
    let health = state.proxy_backend_health.as_ref()?;

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
