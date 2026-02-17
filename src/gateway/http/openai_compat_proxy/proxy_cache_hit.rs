#[cfg(feature = "gateway-proxy-cache")]
async fn maybe_handle_proxy_cache_hit(
    state: &GatewayHttpState,
    cache_key: Option<&str>,
    request_id: &str,
    path_and_query: &str,
    now_epoch_seconds: u64,
    _metrics: Option<(&str, std::time::Instant)>,
) -> Option<axum::response::Response> {
    let (Some(cache), Some(cache_key)) = (state.proxy_cache.as_ref(), cache_key) else {
        return None;
    };

    #[cfg(feature = "gateway-metrics-prometheus")]
    if let (Some(metrics), Some((metrics_path, _))) = (state.prometheus_metrics.as_ref(), _metrics) {
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
        if let Some(store) = state.redis_store.as_ref() {
            if let Ok(redis_cached) = store.get_proxy_cache_response(cache_key).await {
                if redis_cached.is_some() {
                    cache_source = "redis";
                }
                cached = redis_cached;
            }
        }
    }

    let Some(cached) = cached else {
        #[cfg(feature = "gateway-metrics-prometheus")]
        if let (Some(metrics), Some((metrics_path, _))) = (state.prometheus_metrics.as_ref(), _metrics) {
            metrics.lock().await.record_proxy_cache_miss(metrics_path);
        }
        return None;
    };

    if cache_source == "redis" {
        let mut cache = cache.lock().await;
        cache.insert(cache_key.to_string(), cached.clone(), now_epoch_seconds);
    }

    {
        let mut gateway = state.gateway.lock().await;
        gateway.observability.record_cache_hit();
    }

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
        (state.prometheus_metrics.as_ref(), _metrics)
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
