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
    #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
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
        (state.prometheus_metrics.as_ref(), ctx.metrics)
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
