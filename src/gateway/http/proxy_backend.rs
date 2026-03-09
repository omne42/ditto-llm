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
    let protocol =
        extract_header(&parts.headers, "x-ditto-protocol").unwrap_or_else(|| "openai".to_string());
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

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    let token_budget_reservation_ids = params.token_budget_reservation_ids;

    #[cfg(all(
        feature = "gateway-costing",
        any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        )
    ))]
    let cost_budget_reservation_ids = params.cost_budget_reservation_ids;

    let max_attempts = params.max_attempts;
    #[cfg(feature = "gateway-routing-advanced")]
    let retry_config = params.retry_config;

    #[cfg(feature = "gateway-proxy-cache")]
    let proxy_cache_key = params.proxy_cache_key;
    #[cfg(feature = "gateway-proxy-cache")]
    let proxy_cache_metadata = params.proxy_cache_metadata;

    #[cfg(feature = "gateway-metrics-prometheus")]
    let metrics_path = params.metrics_path;
    #[cfg(feature = "gateway-metrics-prometheus")]
    let metrics_timer_start = params.metrics_timer_start;

    #[cfg(not(all(
        feature = "gateway-costing",
        any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        )
    )))]
    let _ = use_persistent_budget;
    #[cfg(not(feature = "gateway-routing-advanced"))]
    let _ = idx;
    #[cfg(not(feature = "gateway-routing-advanced"))]
    let _ = max_attempts;

    let backend = match state.backends.proxy_backends.get(&backend_name) {
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

    state.record_backend_call();
    let backend_model_map: BTreeMap<String, String> = state.backend_model_map(&backend_name);

    #[cfg(feature = "gateway-metrics-prometheus")]
    let backend_timer_start = Instant::now();

    #[cfg(feature = "gateway-metrics-prometheus")]
    if let Some(metrics) = state.proxy.metrics.as_ref() {
        let mut metrics = metrics.lock().await;
        metrics.record_proxy_backend_attempt(&backend_name);
        metrics.record_proxy_backend_in_flight_inc(&backend_name);
    }

    let mut outgoing_headers = parts.headers.clone();
    sanitize_proxy_headers(&mut outgoing_headers, strip_authorization);
    apply_backend_headers(&mut outgoing_headers, backend.headers());
    insert_request_id(&mut outgoing_headers, &request_id);

    let (outgoing_body, upstream_model) = if let (Some(request_model), Some(parsed_json)) =
        (model.as_deref(), parsed_json.as_ref())
    {
        let mapped_model = backend_model_map
            .get(request_model)
            .or_else(|| backend_model_map.get("*"))
            .cloned();

        match mapped_model {
            Some(mapped_model) => {
                let mut value = parsed_json.clone();
                if let Some(obj) = value.as_object_mut() {
                    obj.insert(
                        "model".to_string(),
                        serde_json::Value::String(mapped_model.clone()),
                    );
                    match serde_json::to_vec(&value) {
                        Ok(bytes) => (Bytes::from(bytes), Some(mapped_model)),
                        Err(_) => (body.clone(), Some(request_model.to_string())),
                    }
                } else {
                    (body.clone(), Some(request_model.to_string()))
                }
            }
            None => (body.clone(), Some(request_model.to_string())),
        }
    } else {
        (body.clone(), model.clone())
    };

    #[cfg(feature = "sdk")]
    emit_devtools_log(
        state,
        "proxy.request",
        serde_json::json!({
            "request_id": &request_id,
            "method": parts.method.as_str(),
            "path": path_and_query,
            "backend": &backend_name,
            "provider": &protocol,
            "model": &model,
            "upstream_model": upstream_model.as_deref(),
            "virtual_key_id": virtual_key_id.as_deref(),
            "body_len": body.len(),
        }),
    );

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
            if let Some(metrics) = state.proxy.metrics.as_ref() {
                let mut metrics = metrics.lock().await;
                metrics.record_proxy_backend_in_flight_dec(&backend_name);
                metrics.observe_proxy_backend_request_duration(
                    &backend_name,
                    backend_timer_start.elapsed(),
                );
            }
            #[cfg(feature = "gateway-metrics-prometheus")]
            if let Some(metrics) = state.proxy.metrics.as_ref() {
                metrics
                    .lock()
                    .await
                    .record_proxy_backend_failure(&backend_name);
            }
            #[cfg(feature = "gateway-routing-advanced")]
            let failure_kind = classify_proxy_backend_transport_failure(&err);
            #[cfg(feature = "gateway-routing-advanced")]
            let failure_message = err.to_string();
            let mapped = map_openai_gateway_error(err);
            #[cfg(feature = "gateway-routing-advanced")]
            {
                let decision = retry_config.decision_for_failure(failure_kind);
                record_proxy_backend_failure(
                    state,
                    &backend_name,
                    _now_epoch_seconds,
                    failure_kind,
                    failure_message,
                )
                .await;
                emit_proxy_backend_decision_logs(
                    state,
                    decision,
                    ProxyDecisionLogContext {
                        request_id: &request_id,
                        backend_name: &backend_name,
                        path_and_query,
                        attempted_backends,
                        idx,
                        max_attempts,
                        status_code: None,
                    },
                )
                .await;
                return Ok(if decision.should_attempt_next_backend(idx, max_attempts) {
                    BackendAttemptOutcome::Continue(Some(mapped))
                } else {
                    BackendAttemptOutcome::Stop(mapped)
                });
            }
            #[cfg(not(feature = "gateway-routing-advanced"))]
            return Ok(BackendAttemptOutcome::Continue(Some(mapped)));
        }
    };

    #[cfg(feature = "gateway-metrics-prometheus")]
    if let Some(metrics) = state.proxy.metrics.as_ref() {
        let mut metrics = metrics.lock().await;
        metrics.record_proxy_backend_in_flight_dec(&backend_name);
        metrics
            .observe_proxy_backend_request_duration(&backend_name, backend_timer_start.elapsed());
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
                .and_then(|model| {
                    backend_model_map
                        .get(model)
                        .or_else(|| backend_model_map.get("*"))
                })
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
            emit_devtools_log(
                state,
                "proxy.responses_shim",
                serde_json::json!({
                    "request_id": &request_id,
                    "backend": &backend_name,
                    "path": path_and_query,
                }),
            );

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
                    axum::http::header::ACCEPT,
                    axum::http::HeaderValue::from_static("text/event-stream"),
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
            if let Some(metrics) = state.proxy.metrics.as_ref() {
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
                    if let Some(metrics) = state.proxy.metrics.as_ref() {
                        let mut metrics = metrics.lock().await;
                        metrics.record_proxy_backend_in_flight_dec(&backend_name);
                        metrics.observe_proxy_backend_request_duration(
                            &backend_name,
                            shim_timer_start.elapsed(),
                        );
                        metrics.record_proxy_backend_failure(&backend_name);
                    }
                    #[cfg(feature = "gateway-routing-advanced")]
                    let failure_kind = classify_proxy_backend_transport_failure(&err);
                    #[cfg(feature = "gateway-routing-advanced")]
                    let failure_message = err.to_string();
                    let mapped = map_openai_gateway_error(err);
                    #[cfg(feature = "gateway-routing-advanced")]
                    {
                        let decision = retry_config.decision_for_failure(failure_kind);
                        record_proxy_backend_failure(
                            state,
                            &backend_name,
                            _now_epoch_seconds,
                            failure_kind,
                            failure_message,
                        )
                        .await;
                        emit_proxy_backend_decision_logs(
                            state,
                            decision,
                            ProxyDecisionLogContext {
                                request_id: &request_id,
                                backend_name: &backend_name,
                                path_and_query,
                                attempted_backends,
                                idx,
                                max_attempts,
                                status_code: None,
                            },
                        )
                        .await;
                        return Ok(if decision.should_attempt_next_backend(idx, max_attempts) {
                            BackendAttemptOutcome::Continue(Some(mapped))
                        } else {
                            BackendAttemptOutcome::Stop(mapped)
                        });
                    }
                    #[cfg(not(feature = "gateway-routing-advanced"))]
                    return Ok(BackendAttemptOutcome::Continue(Some(mapped)));
                }
            };

            #[cfg(feature = "gateway-metrics-prometheus")]
            if let Some(metrics) = state.proxy.metrics.as_ref() {
                let mut metrics = metrics.lock().await;
                metrics.record_proxy_backend_in_flight_dec(&backend_name);
                metrics.observe_proxy_backend_request_duration(
                    &backend_name,
                    shim_timer_start.elapsed(),
                );
            }

            let status = shim_response.status();

            #[cfg(feature = "gateway-routing-advanced")]
            let status_code = status.as_u16();
            #[cfg(feature = "gateway-routing-advanced")]
            let failure_kind = FailureKind::Status(status_code);
            #[cfg(feature = "gateway-routing-advanced")]
            let decision = retry_config.decision_for_failure(failure_kind);
            #[cfg(feature = "gateway-routing-advanced")]
            let should_record_status_failure =
                should_record_proxy_status_failure(state, retry_config, failure_kind, status);

            #[cfg(feature = "gateway-routing-advanced")]
            if should_record_status_failure {
                record_proxy_backend_failure(
                    state,
                    &backend_name,
                    _now_epoch_seconds,
                    failure_kind,
                    format!("status {}", status_code),
                )
                .await;
                emit_proxy_backend_decision_logs(
                    state,
                    decision,
                    ProxyDecisionLogContext {
                        request_id: &request_id,
                        backend_name: &backend_name,
                        path_and_query,
                        attempted_backends,
                        idx,
                        max_attempts,
                        status_code: Some(status_code),
                    },
                )
                .await;
            } else {
                record_proxy_backend_success(state, &backend_name).await;
            }

            #[cfg(feature = "gateway-routing-advanced")]
            if decision.should_attempt_next_backend(idx, max_attempts) {
                return Ok(BackendAttemptOutcome::Continue(Some(
                    openai_status_routing_error(status, decision),
                )));
            }

            let spend_tokens = status.is_success();

            #[cfg(feature = "gateway-metrics-prometheus")]
            if let Some(metrics) = state.proxy.metrics.as_ref() {
                let is_failure_status = {
                    #[cfg(feature = "gateway-routing-advanced")]
                    {
                        should_record_status_failure
                    }
                    #[cfg(not(feature = "gateway-routing-advanced"))]
                    {
                        status.is_server_error()
                    }
                };
                let duration = metrics_timer_start.elapsed();
                let mut metrics = metrics.lock().await;
                if is_failure_status {
                    metrics.record_proxy_backend_failure(&backend_name);
                } else {
                    metrics.record_proxy_backend_success(&backend_name);
                }
                metrics.record_proxy_response_status_by_path(metrics_path, status.as_u16());
                metrics.record_proxy_response_status_by_backend(&backend_name, status.as_u16());
                if let Some(model) = model.as_deref() {
                    metrics.record_proxy_response_status_by_model(model, status.as_u16());
                    metrics.observe_proxy_request_duration_by_model(model, duration);
                }
                metrics.observe_proxy_request_duration(metrics_path, duration);
            }

            #[cfg(any(
                feature = "gateway-store-sqlite",
                feature = "gateway-store-postgres",
                feature = "gateway-store-mysql",
                feature = "gateway-store-redis"
            ))]
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
                    state.spend_budget_tokens(&virtual_key_id, &budget, u64::from(charge_tokens));
                    if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                        state.spend_budget_tokens(scope, budget, u64::from(charge_tokens));
                    }
                    if let Some((scope, budget)) = project_budget_scope.as_ref() {
                        state.spend_budget_tokens(scope, budget, u64::from(charge_tokens));
                    }
                    if let Some((scope, budget)) = user_budget_scope.as_ref() {
                        state.spend_budget_tokens(scope, budget, u64::from(charge_tokens));
                    }

                    #[cfg(feature = "gateway-costing")]
                    if !use_persistent_budget {
                        if let Some(charge_cost_usd_micros) = charge_cost_usd_micros {
                            state.spend_budget_cost(
                                &virtual_key_id,
                                &budget,
                                charge_cost_usd_micros,
                            );
                            if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                                state.spend_budget_cost(scope, budget, charge_cost_usd_micros);
                            }
                            if let Some((scope, budget)) = project_budget_scope.as_ref() {
                                state.spend_budget_cost(scope, budget, charge_cost_usd_micros);
                            }
                            if let Some((scope, budget)) = user_budget_scope.as_ref() {
                                state.spend_budget_cost(scope, budget, charge_cost_usd_micros);
                            }
                        }
                    }
                }
            }
            #[cfg(not(any(
                feature = "gateway-store-sqlite",
                feature = "gateway-store-postgres",
                feature = "gateway-store-mysql",
                feature = "gateway-store-redis"
            )))]
            if let (Some(virtual_key_id), Some(budget)) = (virtual_key_id.clone(), budget.clone()) {
                if spend_tokens {
                    state.spend_budget_tokens(&virtual_key_id, &budget, u64::from(charge_tokens));
                    if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                        state.spend_budget_tokens(scope, budget, u64::from(charge_tokens));
                    }
                    if let Some((scope, budget)) = project_budget_scope.as_ref() {
                        state.spend_budget_tokens(scope, budget, u64::from(charge_tokens));
                    }
                    if let Some((scope, budget)) = user_budget_scope.as_ref() {
                        state.spend_budget_tokens(scope, budget, u64::from(charge_tokens));
                    }

                    #[cfg(feature = "gateway-costing")]
                    if let Some(charge_cost_usd_micros) = charge_cost_usd_micros {
                        state.spend_budget_cost(&virtual_key_id, &budget, charge_cost_usd_micros);
                        if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                            state.spend_budget_cost(scope, budget, charge_cost_usd_micros);
                        }
                        if let Some((scope, budget)) = project_budget_scope.as_ref() {
                            state.spend_budget_cost(scope, budget, charge_cost_usd_micros);
                        }
                        if let Some((scope, budget)) = user_budget_scope.as_ref() {
                            state.spend_budget_cost(scope, budget, charge_cost_usd_micros);
                        }
                    }
                }
            }

            #[cfg(all(
                feature = "gateway-costing",
                any(
                    feature = "gateway-store-sqlite",
                    feature = "gateway-store-postgres",
                    feature = "gateway-store-mysql",
                    feature = "gateway-store-redis"
                )
            ))]
            if !cost_budget_reservation_ids.is_empty() {
                settle_proxy_cost_budget_reservations(
                    state,
                    cost_budget_reservation_ids,
                    spend_tokens,
                    u64::MAX,
                )
                .await;
            }

            #[cfg(all(
                feature = "gateway-costing",
                any(
                    feature = "gateway-store-sqlite",
                    feature = "gateway-store-postgres",
                    feature = "gateway-store-mysql",
                    feature = "gateway-store-redis"
                )
            ))]
            if !_cost_budget_reserved && use_persistent_budget && spend_tokens {
                if let (Some(virtual_key_id), Some(charge_cost_usd_micros)) =
                    (virtual_key_id.as_deref(), charge_cost_usd_micros)
                {
                    #[cfg(feature = "gateway-store-sqlite")]
                    if let Some(store) = state.stores.sqlite.as_ref() {
                        let _ = store
                            .record_spent_cost_usd_micros(virtual_key_id, charge_cost_usd_micros)
                            .await;
                    }
                    #[cfg(feature = "gateway-store-postgres")]
                    if let Some(store) = state.stores.postgres.as_ref() {
                        let _ = store
                            .record_spent_cost_usd_micros(virtual_key_id, charge_cost_usd_micros)
                            .await;
                    }
                    #[cfg(feature = "gateway-store-mysql")]
                    if let Some(store) = state.stores.mysql.as_ref() {
                        let _ = store
                            .record_spent_cost_usd_micros(virtual_key_id, charge_cost_usd_micros)
                            .await;
                    }
                    #[cfg(feature = "gateway-store-redis")]
                    if let Some(store) = state.stores.redis.as_ref() {
                        let _ = store
                            .record_spent_cost_usd_micros(virtual_key_id, charge_cost_usd_micros)
                            .await;
                    }
                }
            }

            #[cfg(any(
                feature = "gateway-store-sqlite",
                feature = "gateway-store-postgres",
                feature = "gateway-store-mysql",
                feature = "gateway-store-redis"
            ))]
            {
                let payload = serde_json::json!({
                    "request_id": &request_id,
                    "provider": &protocol,
                    "virtual_key_id": virtual_key_id.as_deref(),
                    "backend": &backend_name,
                    "attempted_backends": &attempted_backends,
                    "method": parts.method.as_str(),
                    "path": path_and_query,
                    "model": &model,
                    "upstream_model": upstream_model.as_deref(),
                    "status": status.as_u16(),
                    "charge_tokens": charge_tokens,
                    "charge_cost_usd_micros": charge_cost_usd_micros,
                    "body_len": body.len(),
                    "shim": "responses_via_chat_completions",
                });

                append_audit_log(state, "proxy", payload).await;
            }

            emit_json_log(
                state,
                "proxy.response",
                serde_json::json!({
                    "request_id": &request_id,
                    "provider": &protocol,
                    "backend": &backend_name,
                    "status": status.as_u16(),
                    "attempted_backends": &attempted_backends,
                    "model": &model,
                    "upstream_model": upstream_model.as_deref(),
                }),
            );

            #[cfg(feature = "sdk")]
            emit_devtools_log(
                state,
                "proxy.response",
                serde_json::json!({
                    "request_id": &request_id,
                    "status": status.as_u16(),
                    "path": path_and_query,
                    "backend": &backend_name,
                }),
            );

            #[cfg(feature = "gateway-otel")]
            {
                tracing::Span::current().record("cache", tracing::field::display("miss"));
                tracing::Span::current().record("backend", tracing::field::display(&backend_name));
                tracing::Span::current().record("status", tracing::field::display(status.as_u16()));
            }

            if status.is_success() {
                match responses_shim_response(
                    ProxyResponseContext {
                        state,
                        backend: &backend_name,
                        request_id: &request_id,
                        #[cfg(feature = "gateway-metrics-prometheus")]
                        metrics_path,
                        cache_key: {
                            #[cfg(feature = "gateway-proxy-cache")]
                            {
                                proxy_cache_key.as_deref()
                            }
                            #[cfg(not(feature = "gateway-proxy-cache"))]
                            {
                                None
                            }
                        },
                        #[cfg(feature = "gateway-proxy-cache")]
                        cache_metadata: proxy_cache_metadata.as_ref(),
                    },
                    shim_response,
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
                return Ok(BackendAttemptOutcome::Response(
                    proxy_response(
                        ProxyResponseContext {
                            state,
                            backend: &backend_name,
                            request_id: &request_id,
                            #[cfg(feature = "gateway-metrics-prometheus")]
                            metrics_path,
                            cache_key: {
                                #[cfg(feature = "gateway-proxy-cache")]
                                {
                                    proxy_cache_key.as_deref()
                                }
                                #[cfg(not(feature = "gateway-proxy-cache"))]
                                {
                                    None
                                }
                            },
                            #[cfg(feature = "gateway-proxy-cache")]
                            cache_metadata: proxy_cache_metadata.as_ref(),
                        },
                        shim_response,
                        shim_permits,
                    )
                    .await,
                ));
            }
        }
    }

    #[cfg(feature = "gateway-routing-advanced")]
    let status_code = status.as_u16();
    #[cfg(feature = "gateway-routing-advanced")]
    let failure_kind = FailureKind::Status(status_code);
    #[cfg(feature = "gateway-routing-advanced")]
    let decision = retry_config.decision_for_failure(failure_kind);
    #[cfg(feature = "gateway-routing-advanced")]
    let should_record_status_failure =
        should_record_proxy_status_failure(state, retry_config, failure_kind, status);

    #[cfg(feature = "gateway-routing-advanced")]
    if should_record_status_failure {
        record_proxy_backend_failure(
            state,
            &backend_name,
            _now_epoch_seconds,
            failure_kind,
            format!("status {}", status_code),
        )
        .await;
        emit_proxy_backend_decision_logs(
            state,
            decision,
            ProxyDecisionLogContext {
                request_id: &request_id,
                backend_name: &backend_name,
                path_and_query,
                attempted_backends,
                idx,
                max_attempts,
                status_code: Some(status_code),
            },
        )
        .await;
    } else {
        record_proxy_backend_success(state, &backend_name).await;
    }

    #[cfg(feature = "gateway-routing-advanced")]
    if decision.should_attempt_next_backend(idx, max_attempts) {
        return Ok(BackendAttemptOutcome::Continue(Some(
            openai_status_routing_error(status, decision),
        )));
    }

    let spend_tokens = status.is_success();

    #[cfg(feature = "gateway-metrics-prometheus")]
    if let Some(metrics) = state.proxy.metrics.as_ref() {
        let is_failure_status = {
            #[cfg(feature = "gateway-routing-advanced")]
            {
                should_record_status_failure
            }
            #[cfg(not(feature = "gateway-routing-advanced"))]
            {
                status.is_server_error()
            }
        };
        let duration = metrics_timer_start.elapsed();
        let mut metrics = metrics.lock().await;
        if is_failure_status {
            metrics.record_proxy_backend_failure(&backend_name);
        } else {
            metrics.record_proxy_backend_success(&backend_name);
        }
        metrics.record_proxy_response_status_by_path(metrics_path, status.as_u16());
        metrics.record_proxy_response_status_by_backend(&backend_name, status.as_u16());
        if let Some(model) = model.as_deref() {
            metrics.record_proxy_response_status_by_model(model, status.as_u16());
            metrics.observe_proxy_request_duration_by_model(model, duration);
        }
        metrics.observe_proxy_request_duration(metrics_path, duration);
    }

    let upstream_headers = upstream_response.headers().clone();
    let content_type = upstream_headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let is_event_stream = content_type.starts_with("text/event-stream");

    if is_event_stream {
        include!("proxy_backend/stream.rs");
    }

    include!("proxy_backend/nonstream.rs")
}

#[cfg(feature = "gateway-routing-advanced")]
fn classify_proxy_backend_transport_failure(err: &GatewayError) -> FailureKind {
    match err {
        GatewayError::BackendTimeout { .. } => FailureKind::Timeout,
        _ => FailureKind::Network,
    }
}

#[cfg(feature = "gateway-routing-advanced")]
fn should_record_proxy_status_failure(
    state: &GatewayHttpState,
    retry_config: &crate::gateway::ProxyRetryConfig,
    kind: FailureKind,
    status: StatusCode,
) -> bool {
    status.is_server_error()
        || retry_config.action_for_failure(kind)
            != crate::gateway::proxy_routing::ProxyFailureAction::None
        || state
            .proxy
            .routing
            .as_ref()
            .map(|config| config.circuit_breaker.should_count_failure(kind))
            .unwrap_or(false)
}

#[cfg(feature = "gateway-routing-advanced")]
struct ProxyDecisionLogContext<'a> {
    request_id: &'a str,
    backend_name: &'a str,
    path_and_query: &'a str,
    attempted_backends: &'a [String],
    idx: usize,
    max_attempts: usize,
    status_code: Option<u16>,
}

#[cfg(feature = "gateway-routing-advanced")]
async fn emit_proxy_backend_decision_logs(
    state: &GatewayHttpState,
    decision: crate::gateway::proxy_routing::ProxyFailureDecision,
    ctx: ProxyDecisionLogContext<'_>,
) {
    let will_attempt_next_backend = decision.should_attempt_next_backend(ctx.idx, ctx.max_attempts);
    emit_json_log(
        state,
        decision.event_name(),
        serde_json::json!({
            "request_id": ctx.request_id,
            "backend": ctx.backend_name,
            "action": decision.action.as_str(),
            "failure_kind": decision.kind.as_str(),
            "reason": decision.reason_code(),
            "status": ctx.status_code.or_else(|| decision.kind.status_code()),
            "path": ctx.path_and_query,
            "will_attempt_next_backend": will_attempt_next_backend,
            "attempted_backends": ctx.attempted_backends,
        }),
    );

    #[cfg(feature = "sdk")]
    emit_devtools_log(
        state,
        decision.event_name(),
        serde_json::json!({
            "request_id": ctx.request_id,
            "backend": ctx.backend_name,
            "action": decision.action.as_str(),
            "failure_kind": decision.kind.as_str(),
            "reason": decision.reason_code(),
            "status": ctx.status_code.or_else(|| decision.kind.status_code()),
            "will_attempt_next_backend": will_attempt_next_backend,
            "path": ctx.path_and_query,
        }),
    );
}

#[cfg(feature = "gateway-routing-advanced")]
fn openai_status_routing_error(
    status: StatusCode,
    decision: crate::gateway::proxy_routing::ProxyFailureDecision,
) -> (StatusCode, Json<OpenAiErrorResponse>) {
    let message = match decision.action {
        crate::gateway::proxy_routing::ProxyFailureAction::Retry => {
            format!("retryable upstream status {}", status.as_u16())
        }
        crate::gateway::proxy_routing::ProxyFailureAction::Fallback => {
            format!("fallbackable upstream status {}", status.as_u16())
        }
        crate::gateway::proxy_routing::ProxyFailureAction::None => {
            format!("upstream status {}", status.as_u16())
        }
    };

    openai_error(status, "api_error", Some("backend_error"), message)
}
