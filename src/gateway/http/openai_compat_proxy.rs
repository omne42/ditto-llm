// This file is intentionally split to keep each staged Rust file under the pre-commit size limit.
include!("openai_compat_proxy/preamble.rs");
include!("openai_compat_proxy/cost_budget.rs");
include!("openai_compat_proxy/costing.rs");
include!("openai_compat_proxy/rate_limit.rs");
include!("openai_compat_proxy/resolve_gateway_context.rs");
include!("openai_compat_proxy/streaming_multipart.rs");
include!("openai_compat_proxy/path_normalize.rs");
include!("openai_compat_proxy/mcp.rs");
include!("openai_compat_proxy/proxy_cache_hit.rs");
include!("openai_compat_proxy/proxy_failure.rs");

async fn handle_openai_compat_proxy(
    State(state): State<GatewayHttpState>,
    Path(_path): Path<String>,
    req: axum::http::Request<Body>,
) -> Result<axum::response::Response, (StatusCode, Json<OpenAiErrorResponse>)> {
    let max_body_bytes = state.proxy_max_body_bytes;
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
    let body = to_bytes(incoming_body, max_body_bytes)
        .await
        .map_err(|err| openai_error(StatusCode::BAD_REQUEST, "invalid_request_error", None, err))?;

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
    let charge_tokens = input_tokens_estimate.saturating_add(max_output_tokens);

    #[cfg(feature = "gateway-store-sqlite")]
    let use_sqlite_budget = state.sqlite_store.is_some();
    #[cfg(not(feature = "gateway-store-sqlite"))]
    let use_sqlite_budget = false;

    #[cfg(feature = "gateway-store-redis")]
    let use_redis_budget = state.redis_store.is_some();
    #[cfg(not(feature = "gateway-store-redis"))]
    let use_redis_budget = false;

    let use_persistent_budget = use_sqlite_budget || use_redis_budget;

    let _now_epoch_seconds = now_epoch_seconds();
    let minute = _now_epoch_seconds / 60;
    #[cfg(feature = "gateway-store-redis")]
    let rate_limit_route = normalize_rate_limit_route(path_and_query);


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
        strip_authorization,
        charge_cost_usd_micros,
    ) = resolve_openai_compat_proxy_gateway_context(
        &state,
        &parts,
        &body,
        &parsed_json,
        &request_id,
        path_and_query,
        &model,
        &service_tier,
        input_tokens_estimate,
        max_output_tokens,
        charge_tokens,
        minute,
        use_redis_budget,
        use_persistent_budget,
        #[cfg(feature = "gateway-metrics-prometheus")]
        metrics_path.clone(),
        #[cfg(feature = "gateway-metrics-prometheus")]
        metrics_timer_start,
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
    if let (Some(store), Some(virtual_key_id), Some(limits)) =
        (state.redis_store.as_ref(), virtual_key_id.as_deref(), limits.as_ref())
    {
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
                let mut gateway = state.gateway.lock().await;
                gateway.observability.record_rate_limited();
            }
            let mapped = map_openai_gateway_error(err);
            #[cfg(feature = "gateway-metrics-prometheus")]
            if is_rate_limited {
                if let Some(metrics) = state.prometheus_metrics.as_ref() {
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
        (state.redis_store.as_ref(), tenant_limits_scope.as_ref())
    {
        if let Err(err) = store
            .check_and_consume_rate_limits(scope, &rate_limit_route, limits, charge_tokens, _now_epoch_seconds)
            .await
        {
            let is_rate_limited = matches!(err, GatewayError::RateLimited { .. });
            if is_rate_limited {
                let mut gateway = state.gateway.lock().await;
                gateway.observability.record_rate_limited();
            }
            let mapped = map_openai_gateway_error(err);
            #[cfg(feature = "gateway-metrics-prometheus")]
            if is_rate_limited {
                if let Some(metrics) = state.prometheus_metrics.as_ref() {
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
        (state.redis_store.as_ref(), project_limits_scope.as_ref())
    {
        if let Err(err) = store
            .check_and_consume_rate_limits(scope, &rate_limit_route, limits, charge_tokens, _now_epoch_seconds)
            .await
        {
            let is_rate_limited = matches!(err, GatewayError::RateLimited { .. });
            if is_rate_limited {
                let mut gateway = state.gateway.lock().await;
                gateway.observability.record_rate_limited();
            }
            let mapped = map_openai_gateway_error(err);
            #[cfg(feature = "gateway-metrics-prometheus")]
            if is_rate_limited {
                if let Some(metrics) = state.prometheus_metrics.as_ref() {
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
        (state.redis_store.as_ref(), user_limits_scope.as_ref())
    {
        if let Err(err) = store
            .check_and_consume_rate_limits(scope, &rate_limit_route, limits, charge_tokens, _now_epoch_seconds)
            .await
        {
            let is_rate_limited = matches!(err, GatewayError::RateLimited { .. });
            if is_rate_limited {
                let mut gateway = state.gateway.lock().await;
                gateway.observability.record_rate_limited();
            }
            let mapped = map_openai_gateway_error(err);
            #[cfg(feature = "gateway-metrics-prometheus")]
            if is_rate_limited {
                if let Some(metrics) = state.prometheus_metrics.as_ref() {
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
    if let Some(metrics) = state.prometheus_metrics.as_ref() {
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


    #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
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

    #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
    let (_token_budget_reserved, token_budget_reservation_ids) =
        match reserve_proxy_token_budgets_for_request(budget_reservation_params).await {
            Ok(reserved) => reserved,
            Err(err) => {
                if err.0 == StatusCode::PAYMENT_REQUIRED {
                    let mut gateway = state.gateway.lock().await;
                    gateway.observability.record_budget_exceeded();
                }

                #[cfg(feature = "gateway-metrics-prometheus")]
                if let Some(metrics) = state.prometheus_metrics.as_ref() {
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

    #[cfg(not(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis")))]
    let _token_budget_reserved = false;

    #[cfg(all(
        feature = "gateway-costing",
        any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
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
                    let mut gateway = state.gateway.lock().await;
                    gateway.observability.record_budget_exceeded();
                }

                #[cfg(feature = "gateway-metrics-prometheus")]
                if let Some(metrics) = state.prometheus_metrics.as_ref() {
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
        #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
        token_budget_reservation_ids: &token_budget_reservation_ids,
        cost_budget_reserved: _cost_budget_reserved,
        #[cfg(all(
            feature = "gateway-costing",
            any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
        ))]
        cost_budget_reservation_ids: &cost_budget_reservation_ids,
        max_attempts,
        #[cfg(feature = "gateway-routing-advanced")]
        retry_config: &retry_config,
        #[cfg(feature = "gateway-proxy-cache")]
        proxy_cache_key: &proxy_cache_key,
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
        if let Some(translation_backend) = state.translation_backends.get(&backend_name).cloned() {
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
            }
        }

        match attempt_proxy_backend(attempt_params, &backend_name, idx, &attempted_backends).await? {
            BackendAttemptOutcome::Response(response) => return Ok(response),
            BackendAttemptOutcome::Continue(err) => {
                if let Some(err) = err {
                    last_err = Some(err);
                }
                continue;
            }
        }
    }

    #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
    rollback_proxy_token_budget_reservations(&state, &token_budget_reservation_ids).await;

    #[cfg(all(
        feature = "gateway-costing",
        any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
    ))]
    rollback_proxy_cost_budget_reservations(&state, &cost_budget_reservation_ids).await;

    #[cfg(feature = "gateway-metrics-prometheus")]
    let proxy_metrics = Some((metrics_path.as_str(), metrics_timer_start));
    #[cfg(not(feature = "gateway-metrics-prometheus"))]
    let proxy_metrics = None;

    Err(
        finalize_openai_compat_proxy_failure(
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
        .await,
    )

}
