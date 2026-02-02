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

    #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
    let token_budget_reservation_ids = params.token_budget_reservation_ids;

    #[cfg(all(feature = "gateway-costing", any(feature = "gateway-store-sqlite", feature = "gateway-store-redis")))]
    let cost_budget_reservation_ids = params.cost_budget_reservation_ids;

    let max_attempts = params.max_attempts;
    #[cfg(feature = "gateway-routing-advanced")]
    let retry_config = params.retry_config;

    #[cfg(feature = "gateway-proxy-cache")]
    let proxy_cache_key = params.proxy_cache_key;

    #[cfg(feature = "gateway-metrics-prometheus")]
    let metrics_path = params.metrics_path;
    #[cfg(feature = "gateway-metrics-prometheus")]
    let metrics_timer_start = params.metrics_timer_start;

    #[cfg(not(feature = "gateway-costing"))]
    let _ = use_persistent_budget;
    #[cfg(not(feature = "gateway-routing-advanced"))]
    let _ = idx;
    #[cfg(not(feature = "gateway-routing-advanced"))]
    let _ = max_attempts;

    let backend = match state.proxy_backends.get(&backend_name) {
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

    let backend_model_map: BTreeMap<String, String> = {
        let mut gateway = state.gateway.lock().await;
        gateway.observability.record_backend_call();
        gateway
            .config
            .backends
            .iter()
            .find(|backend| backend.name == backend_name)
            .map(|backend| backend.model_map.clone())
            .unwrap_or_default()
    };

        #[cfg(feature = "gateway-metrics-prometheus")]
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

        let outgoing_body = if let (Some(request_model), Some(parsed_json)) =
            (model.as_deref(), parsed_json.as_ref())
        {
            backend_model_map
                .get(request_model)
                .and_then(|mapped_model| {
                    let mut value = parsed_json.clone();
                    let obj = value.as_object_mut()?;
                    obj.insert(
                        "model".to_string(),
                        serde_json::Value::String(mapped_model.clone()),
                    );
                    serde_json::to_vec(&value).ok().map(Bytes::from)
                })
                .unwrap_or_else(|| body.clone())
        } else {
            body.clone()
        };

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
                Some(outgoing_body),
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
                    state,
                    &backend_name,
                    _now_epoch_seconds,
                    FailureKind::Network,
                    err.to_string(),
                )
                .await;
                let mapped = map_openai_gateway_error(err);
                return Ok(BackendAttemptOutcome::Continue(Some(mapped)));
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
                    .and_then(|model| backend_model_map.get(model))
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
                        "accept",
                        "text/event-stream"
                            .parse()
                            .unwrap_or_else(|_| "text/event-stream".parse().unwrap()),
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
                            state,
                            &backend_name,
                            _now_epoch_seconds,
                            FailureKind::Network,
                            err.to_string(),
                        )
                        .await;
                        let mapped = map_openai_gateway_error(err);
                        return Ok(BackendAttemptOutcome::Continue(Some(mapped)));
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
                        state,
                        &backend_name,
                        _now_epoch_seconds,
                        FailureKind::RetryableStatus(status.as_u16()),
                        format!("retryable status {}", status.as_u16()),
                    )
                    .await;

                    emit_json_log(
                        state,
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

                    return Ok(BackendAttemptOutcome::Continue(Some(openai_error(
                        status,
                        "api_error",
                        Some("backend_error"),
                        format!("retryable status {}", status.as_u16()),
                    ))));
                }

                #[cfg(feature = "gateway-routing-advanced")]
                if retry_config.retry_status_codes.contains(&status.as_u16()) {
                    record_proxy_backend_failure(
                        state,
                        &backend_name,
                        _now_epoch_seconds,
                        FailureKind::RetryableStatus(status.as_u16()),
                        format!("status {}", status.as_u16()),
                    )
                    .await;
                } else {
                    record_proxy_backend_success(state, &backend_name).await;
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
                    metrics.record_proxy_response_status_by_path(metrics_path, status.as_u16());
                    metrics.observe_proxy_request_duration(
                        metrics_path,
                        metrics_timer_start.elapsed(),
                    );
                }

                #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
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
                        let mut gateway = state.gateway.lock().await;
                        gateway
                            .budget
                            .spend(&virtual_key_id, &budget, u64::from(charge_tokens));
                        if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                            gateway
                                .budget
                                .spend(scope, budget, u64::from(charge_tokens));
                        }
                        if let Some((scope, budget)) = project_budget_scope.as_ref() {
                            gateway
                                .budget
                                .spend(scope, budget, u64::from(charge_tokens));
                        }
                        if let Some((scope, budget)) = user_budget_scope.as_ref() {
                            gateway
                                .budget
                                .spend(scope, budget, u64::from(charge_tokens));
                        }

                        #[cfg(feature = "gateway-costing")]
                        if !use_persistent_budget {
                            if let Some(charge_cost_usd_micros) = charge_cost_usd_micros {
                                gateway.budget.spend_cost_usd_micros(
                                    &virtual_key_id,
                                    &budget,
                                    charge_cost_usd_micros,
                                );
                                if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                                    gateway.budget.spend_cost_usd_micros(
                                        scope,
                                        budget,
                                        charge_cost_usd_micros,
                                    );
                                }
                                if let Some((scope, budget)) = project_budget_scope.as_ref() {
                                    gateway.budget.spend_cost_usd_micros(
                                        scope,
                                        budget,
                                        charge_cost_usd_micros,
                                    );
                                }
                                if let Some((scope, budget)) = user_budget_scope.as_ref() {
                                    gateway.budget.spend_cost_usd_micros(
                                        scope,
                                        budget,
                                        charge_cost_usd_micros,
                                    );
                                }
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
                        if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                            gateway
                                .budget
                                .spend(scope, budget, u64::from(charge_tokens));
                        }
                        if let Some((scope, budget)) = project_budget_scope.as_ref() {
                            gateway
                                .budget
                                .spend(scope, budget, u64::from(charge_tokens));
                        }
                        if let Some((scope, budget)) = user_budget_scope.as_ref() {
                            gateway
                                .budget
                                .spend(scope, budget, u64::from(charge_tokens));
                        }

                        #[cfg(feature = "gateway-costing")]
                        if let Some(charge_cost_usd_micros) = charge_cost_usd_micros {
                            gateway.budget.spend_cost_usd_micros(
                                &virtual_key_id,
                                &budget,
                                charge_cost_usd_micros,
                            );
                            if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                                gateway.budget.spend_cost_usd_micros(
                                    scope,
                                    budget,
                                    charge_cost_usd_micros,
                                );
                            }
                            if let Some((scope, budget)) = project_budget_scope.as_ref() {
                                gateway.budget.spend_cost_usd_micros(
                                    scope,
                                    budget,
                                    charge_cost_usd_micros,
                                );
                            }
                            if let Some((scope, budget)) = user_budget_scope.as_ref() {
                                gateway.budget.spend_cost_usd_micros(
                                    scope,
                                    budget,
                                    charge_cost_usd_micros,
                                );
                            }
                        }
                    }
                }

                #[cfg(all(feature = "gateway-costing", any(feature = "gateway-store-sqlite", feature = "gateway-store-redis")))]
                if !cost_budget_reservation_ids.is_empty() {
                    settle_proxy_cost_budget_reservations(
                        state,
                        cost_budget_reservation_ids,
                        spend_tokens,
                        u64::MAX,
                    )
                    .await;
                }

                #[cfg(all(feature = "gateway-costing", any(feature = "gateway-store-sqlite", feature = "gateway-store-redis")))]
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
                    state,
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
                    tracing::Span::current().record("cache", tracing::field::display("miss"));
                    tracing::Span::current().record("backend", tracing::field::display(&backend_name));
                    tracing::Span::current().record("status", tracing::field::display(status.as_u16()));
                }

                if status.is_success() {
                    match responses_shim_response(
                        state,
                        shim_response,
                        backend_name.clone(),
                        request_id.clone(),
                        #[cfg(feature = "gateway-proxy-cache")]
                        proxy_cache_key.as_deref(),
                        #[cfg(not(feature = "gateway-proxy-cache"))]
                        None,
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
                    return Ok(BackendAttemptOutcome::Response(proxy_response(
                        state,
                        shim_response,
                        backend_name,
                        request_id.clone(),
                        #[cfg(feature = "gateway-proxy-cache")]
                        proxy_cache_key.as_deref(),
                        #[cfg(not(feature = "gateway-proxy-cache"))]
                        None,
                        shim_permits,
                    )
                    .await));
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
                state,
                &backend_name,
                _now_epoch_seconds,
                FailureKind::RetryableStatus(status.as_u16()),
                format!("retryable status {}", status.as_u16()),
            )
            .await;

            emit_json_log(
                state,
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

            return Ok(BackendAttemptOutcome::Continue(None));
        }

        #[cfg(feature = "gateway-routing-advanced")]
        if retry_config.retry_status_codes.contains(&status.as_u16()) {
            record_proxy_backend_failure(
                state,
                &backend_name,
                _now_epoch_seconds,
                FailureKind::RetryableStatus(status.as_u16()),
                format!("status {}", status.as_u16()),
            )
            .await;
        } else {
            record_proxy_backend_success(state, &backend_name).await;
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
            metrics.record_proxy_response_status_by_path(metrics_path, status.as_u16());
            metrics.observe_proxy_request_duration(metrics_path, metrics_timer_start.elapsed());
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

            #[cfg(not(any(
                feature = "gateway-costing",
                feature = "gateway-store-sqlite",
                feature = "gateway-store-redis"
            )))]
            let _ = spent_cost_usd_micros;

            #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
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
                    let mut gateway = state.gateway.lock().await;
                    gateway.budget.spend(&virtual_key_id, &budget, spent_tokens);
                    if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                        gateway.budget.spend(scope, budget, spent_tokens);
                    }
                    if let Some((scope, budget)) = project_budget_scope.as_ref() {
                        gateway.budget.spend(scope, budget, spent_tokens);
                    }
                    if let Some((scope, budget)) = user_budget_scope.as_ref() {
                        gateway.budget.spend(scope, budget, spent_tokens);
                    }

                    #[cfg(feature = "gateway-costing")]
                    if !use_persistent_budget {
                        if let Some(spent_cost_usd_micros) = spent_cost_usd_micros {
                            gateway.budget.spend_cost_usd_micros(
                                &virtual_key_id,
                                &budget,
                                spent_cost_usd_micros,
                            );
                            if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                                gateway.budget.spend_cost_usd_micros(
                                    scope,
                                    budget,
                                    spent_cost_usd_micros,
                                );
                            }
                            if let Some((scope, budget)) = project_budget_scope.as_ref() {
                                gateway.budget.spend_cost_usd_micros(
                                    scope,
                                    budget,
                                    spent_cost_usd_micros,
                                );
                            }
                            if let Some((scope, budget)) = user_budget_scope.as_ref() {
                                gateway.budget.spend_cost_usd_micros(
                                    scope,
                                    budget,
                                    spent_cost_usd_micros,
                                );
                            }
                        }
                    }
                }
            }
            #[cfg(not(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis")))]
            if let (Some(virtual_key_id), Some(budget)) = (virtual_key_id.clone(), budget.clone()) {
                if spend_tokens {
                    let mut gateway = state.gateway.lock().await;
                    gateway.budget.spend(&virtual_key_id, &budget, spent_tokens);
                    if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                        gateway.budget.spend(scope, budget, spent_tokens);
                    }
                    if let Some((scope, budget)) = project_budget_scope.as_ref() {
                        gateway.budget.spend(scope, budget, spent_tokens);
                    }
                    if let Some((scope, budget)) = user_budget_scope.as_ref() {
                        gateway.budget.spend(scope, budget, spent_tokens);
                    }

                    #[cfg(feature = "gateway-costing")]
                    if let Some(spent_cost_usd_micros) = spent_cost_usd_micros {
                        gateway.budget.spend_cost_usd_micros(
                            &virtual_key_id,
                            &budget,
                            spent_cost_usd_micros,
                        );
                        if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                            gateway.budget.spend_cost_usd_micros(
                                scope,
                                budget,
                                spent_cost_usd_micros,
                            );
                        }
                        if let Some((scope, budget)) = project_budget_scope.as_ref() {
                            gateway.budget.spend_cost_usd_micros(
                                scope,
                                budget,
                                spent_cost_usd_micros,
                            );
                        }
                        if let Some((scope, budget)) = user_budget_scope.as_ref() {
                            gateway.budget.spend_cost_usd_micros(
                                scope,
                                budget,
                                spent_cost_usd_micros,
                            );
                        }
                    }
                }
            }

            #[cfg(all(feature = "gateway-costing", any(feature = "gateway-store-sqlite", feature = "gateway-store-redis")))]
            if !cost_budget_reservation_ids.is_empty() {
                settle_proxy_cost_budget_reservations(
                    state,
                    cost_budget_reservation_ids,
                    spend_tokens,
                    spent_cost_usd_micros.unwrap_or_default(),
                )
                .await;
            }

            #[cfg(all(feature = "gateway-costing", any(feature = "gateway-store-sqlite", feature = "gateway-store-redis")))]
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
                state,
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
                tracing::Span::current().record("cache", tracing::field::display("miss"));
                tracing::Span::current().record("backend", tracing::field::display(&backend_name));
                tracing::Span::current().record("status", tracing::field::display(status.as_u16()));
            }

            return Ok(BackendAttemptOutcome::Response(proxy_response(
                state,
                upstream_response,
                backend_name,
                request_id.clone(),
                #[cfg(feature = "gateway-proxy-cache")]
                proxy_cache_key.as_deref(),
                #[cfg(not(feature = "gateway-proxy-cache"))]
                None,
                proxy_permits,
            )
            .await));
        }

        include!("proxy_backend/nonstream.rs")
}
