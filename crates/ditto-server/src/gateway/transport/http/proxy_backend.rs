use super::*;

pub(super) async fn attempt_proxy_backend(
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
        // inlined from proxy_backend/stream.rs
        {
            const SSE_USAGE_TRACKER_MAX_BUFFER_BYTES: usize = 512 * 1024;
            const SSE_USAGE_TRACKER_TAIL_BYTES: usize = 128 * 1024;
            const PROXY_SSE_ABORT_FINALIZER_WORKERS: usize = 2;
            const PROXY_SSE_ABORT_FINALIZER_QUEUE_CAPACITY: usize = 1024;

            #[derive(Default)]
            struct SseUsageTracker {
                buffer: bytes::BytesMut,
                observed_usage: Option<ObservedUsage>,
            }

            impl SseUsageTracker {
                fn ingest(&mut self, chunk: &Bytes) {
                    self.buffer.extend_from_slice(chunk.as_ref());

                    loop {
                        let Some((pos, delimiter_len)) = find_sse_delimiter(self.buffer.as_ref())
                        else {
                            break;
                        };

                        let event_bytes = self.buffer.split_to(pos);
                        let _ = self.buffer.split_to(delimiter_len);

                        let Some(data) = extract_sse_data(event_bytes.as_ref()) else {
                            continue;
                        };
                        let trimmed = trim_ascii_whitespace(&data);
                        if trimmed == b"[DONE]" {
                            continue;
                        }

                        if trimmed.starts_with(b"{") {
                            if let Some(usage) = extract_openai_usage_from_slice(trimmed) {
                                self.observed_usage = Some(usage);
                            }
                        }
                    }

                    if self.buffer.len() > SSE_USAGE_TRACKER_MAX_BUFFER_BYTES {
                        let keep_from = self
                            .buffer
                            .len()
                            .saturating_sub(SSE_USAGE_TRACKER_TAIL_BYTES);
                        self.buffer = self.buffer.split_off(keep_from);
                    }
                }

                fn observed_usage(&self) -> Option<ObservedUsage> {
                    self.observed_usage
                }
            }

            fn find_sse_delimiter(buf: &[u8]) -> Option<(usize, usize)> {
                if buf.len() < 2 {
                    return None;
                }

                // Use a single forward scan so mixed newline styles still split at the earliest
                // event boundary instead of whichever delimiter pattern we searched first.
                let mut idx = 0usize;
                while idx + 1 < buf.len() {
                    if buf[idx] == b'\n' && buf[idx + 1] == b'\n' {
                        return Some((idx, 2));
                    }
                    if idx + 3 < buf.len()
                        && buf[idx] == b'\r'
                        && buf[idx + 1] == b'\n'
                        && buf[idx + 2] == b'\r'
                        && buf[idx + 3] == b'\n'
                    {
                        return Some((idx, 4));
                    }
                    idx += 1;
                }

                None
            }

            fn extract_sse_data(event: &[u8]) -> Option<Vec<u8>> {
                let mut out = Vec::<u8>::new();
                for line in event.split(|b| *b == b'\n') {
                    let line = line.strip_suffix(b"\r").unwrap_or(line);
                    let Some(rest) = line.strip_prefix(b"data:") else {
                        continue;
                    };
                    let rest = trim_ascii_whitespace(rest);
                    if rest.is_empty() {
                        continue;
                    }
                    if !out.is_empty() {
                        out.push(b'\n');
                    }
                    out.extend_from_slice(rest);
                }
                (!out.is_empty()).then_some(out)
            }

            fn trim_ascii_whitespace(bytes: &[u8]) -> &[u8] {
                let start = bytes
                    .iter()
                    .position(|b| !b.is_ascii_whitespace())
                    .unwrap_or(bytes.len());
                let end = bytes
                    .iter()
                    .rposition(|b| !b.is_ascii_whitespace())
                    .map(|pos| pos + 1)
                    .unwrap_or(start);
                &bytes[start..end]
            }

            #[derive(Clone, Copy, Debug)]
            enum StreamEnd {
                Completed,
                Error,
                Aborted,
            }

            struct ProxySseFinalizer {
                state: GatewayHttpState,
                backend_name: String,
                attempted_backends: Vec<String>,
                request_id: String,
                provider: String,
                #[cfg(any(
                    feature = "gateway-store-sqlite",
                    feature = "gateway-store-postgres",
                    feature = "gateway-store-mysql",
                    feature = "gateway-store-redis",
                    feature = "sdk"
                ))]
                method: String,
                #[cfg(any(
                    feature = "gateway-store-sqlite",
                    feature = "gateway-store-postgres",
                    feature = "gateway-store-mysql",
                    feature = "gateway-store-redis",
                    feature = "sdk"
                ))]
                path_and_query: String,
                #[cfg(feature = "gateway-metrics-prometheus")]
                metrics_path: String,
                model: Option<String>,
                upstream_model: Option<String>,
                service_tier: Option<String>,
                backend_model_map: BTreeMap<String, String>,
                status: u16,
                charge_tokens: u32,
                charge_cost_usd_micros: Option<u64>,
                spend_tokens: bool,
                #[cfg(any(
                    feature = "gateway-store-sqlite",
                    feature = "gateway-store-postgres",
                    feature = "gateway-store-mysql",
                    feature = "gateway-store-redis"
                ))]
                use_persistent_budget: bool,
                virtual_key_id: Option<String>,
                budget: Option<super::BudgetConfig>,
                tenant_budget_scope: Option<(String, super::BudgetConfig)>,
                project_budget_scope: Option<(String, super::BudgetConfig)>,
                user_budget_scope: Option<(String, super::BudgetConfig)>,
                #[cfg(any(
                    feature = "gateway-store-sqlite",
                    feature = "gateway-store-postgres",
                    feature = "gateway-store-mysql",
                    feature = "gateway-store-redis"
                ))]
                token_budget_reservation_ids: Vec<String>,
                #[cfg(all(
                    feature = "gateway-costing",
                    any(
                        feature = "gateway-store-sqlite",
                        feature = "gateway-store-postgres",
                        feature = "gateway-store-mysql",
                        feature = "gateway-store-redis"
                    )
                ))]
                cost_budget_reserved: bool,
                #[cfg(all(
                    feature = "gateway-costing",
                    any(
                        feature = "gateway-store-sqlite",
                        feature = "gateway-store-postgres",
                        feature = "gateway-store-mysql",
                        feature = "gateway-store-redis"
                    )
                ))]
                cost_budget_reservation_ids: Vec<String>,
                #[cfg(any(
                    feature = "gateway-store-sqlite",
                    feature = "gateway-store-postgres",
                    feature = "gateway-store-mysql",
                    feature = "gateway-store-redis",
                    feature = "sdk"
                ))]
                request_body_len: usize,
            }

            impl ProxySseFinalizer {
                async fn finalize(
                    self,
                    observed_usage: Option<ObservedUsage>,
                    end: StreamEnd,
                    stream_bytes: u64,
                ) {
                    #[cfg(not(feature = "gateway-metrics-prometheus"))]
                    let _ = (&end, stream_bytes);

                    #[cfg(feature = "gateway-metrics-prometheus")]
                    if let Some(metrics) = self.state.proxy.metrics.as_ref() {
                        let mut metrics = metrics.lock().await;
                        metrics.record_proxy_stream_close(&self.backend_name, &self.metrics_path);
                        metrics.record_proxy_stream_bytes(
                            &self.backend_name,
                            &self.metrics_path,
                            stream_bytes,
                        );
                        match end {
                            StreamEnd::Completed => {
                                metrics.record_proxy_stream_completed(
                                    &self.backend_name,
                                    &self.metrics_path,
                                );
                            }
                            StreamEnd::Error => {
                                metrics.record_proxy_stream_error(
                                    &self.backend_name,
                                    &self.metrics_path,
                                );
                            }
                            StreamEnd::Aborted => {
                                metrics.record_proxy_stream_aborted(
                                    &self.backend_name,
                                    &self.metrics_path,
                                );
                            }
                        }
                    }

                    let spent_tokens = if self.spend_tokens {
                        observed_usage
                            .and_then(|usage| usage.total_tokens)
                            .unwrap_or_else(|| u64::from(self.charge_tokens))
                    } else {
                        0
                    };

                    #[cfg(feature = "gateway-costing")]
                    let spent_cost_usd_micros = if self.spend_tokens {
                        self.model
                            .as_deref()
                            .map(|request_model| {
                                self.backend_model_map
                                    .get(request_model)
                                    .map(|model| model.as_str())
                                    .unwrap_or(request_model)
                            })
                            .and_then(|cost_model| {
                                self.state.proxy.pricing.as_ref().and_then(|pricing| {
                                    let usage = observed_usage?;
                                    let input = usage.input_tokens?;
                                    let output = usage.output_tokens?;
                                    pricing.estimate_cost_usd_micros_with_cache_for_service_tier(
                                        cost_model,
                                        clamp_u64_to_u32(input),
                                        usage.cache_input_tokens.map(clamp_u64_to_u32),
                                        usage.cache_creation_input_tokens.map(clamp_u64_to_u32),
                                        clamp_u64_to_u32(output),
                                        self.service_tier.as_deref(),
                                    )
                                })
                            })
                            .or(self.charge_cost_usd_micros)
                    } else {
                        None
                    };
                    #[cfg(not(feature = "gateway-costing"))]
                    let spent_cost_usd_micros: Option<u64> = None;

                    #[cfg(not(feature = "gateway-costing"))]
                    let _ = (
                        spent_cost_usd_micros,
                        &self.model,
                        &self.service_tier,
                        &self.backend_model_map,
                        self.charge_cost_usd_micros,
                    );

                    #[cfg(any(
                        feature = "gateway-store-sqlite",
                        feature = "gateway-store-postgres",
                        feature = "gateway-store-mysql",
                        feature = "gateway-store-redis",
                        feature = "sdk",
                    ))]
                    let _ = (&self.method, &self.path_and_query, self.request_body_len);

                    #[cfg(any(
                        feature = "gateway-store-sqlite",
                        feature = "gateway-store-postgres",
                        feature = "gateway-store-mysql",
                        feature = "gateway-store-redis"
                    ))]
                    let _ = (
                        self.use_persistent_budget,
                        &self.token_budget_reservation_ids,
                    );

                    #[cfg(all(
                        feature = "gateway-costing",
                        any(
                            feature = "gateway-store-sqlite",
                            feature = "gateway-store-postgres",
                            feature = "gateway-store-mysql",
                            feature = "gateway-store-redis"
                        )
                    ))]
                    let _ = (self.cost_budget_reserved, &self.cost_budget_reservation_ids);

                    #[cfg(any(
                        feature = "gateway-store-sqlite",
                        feature = "gateway-store-postgres",
                        feature = "gateway-store-mysql",
                        feature = "gateway-store-redis"
                    ))]
                    if !self.token_budget_reservation_ids.is_empty() {
                        settle_proxy_token_budget_reservations(
                            &self.state,
                            &self.token_budget_reservation_ids,
                            self.spend_tokens,
                            spent_tokens,
                        )
                        .await;
                    } else if let (Some(virtual_key_id), Some(budget)) =
                        (self.virtual_key_id.clone(), self.budget.clone())
                    {
                        if self.spend_tokens {
                            self.state
                                .spend_budget_tokens(&virtual_key_id, &budget, spent_tokens);
                            if let Some((scope, budget)) = self.tenant_budget_scope.as_ref() {
                                self.state.spend_budget_tokens(scope, budget, spent_tokens);
                            }
                            if let Some((scope, budget)) = self.project_budget_scope.as_ref() {
                                self.state.spend_budget_tokens(scope, budget, spent_tokens);
                            }
                            if let Some((scope, budget)) = self.user_budget_scope.as_ref() {
                                self.state.spend_budget_tokens(scope, budget, spent_tokens);
                            }

                            #[cfg(feature = "gateway-costing")]
                            if !self.use_persistent_budget {
                                if let Some(spent_cost_usd_micros) = spent_cost_usd_micros {
                                    self.state.spend_budget_cost(
                                        &virtual_key_id,
                                        &budget,
                                        spent_cost_usd_micros,
                                    );
                                    if let Some((scope, budget)) = self.tenant_budget_scope.as_ref()
                                    {
                                        self.state.spend_budget_cost(
                                            scope,
                                            budget,
                                            spent_cost_usd_micros,
                                        );
                                    }
                                    if let Some((scope, budget)) =
                                        self.project_budget_scope.as_ref()
                                    {
                                        self.state.spend_budget_cost(
                                            scope,
                                            budget,
                                            spent_cost_usd_micros,
                                        );
                                    }
                                    if let Some((scope, budget)) = self.user_budget_scope.as_ref() {
                                        self.state.spend_budget_cost(
                                            scope,
                                            budget,
                                            spent_cost_usd_micros,
                                        );
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
                    if let (Some(virtual_key_id), Some(budget)) =
                        (self.virtual_key_id.clone(), self.budget.clone())
                    {
                        if self.spend_tokens {
                            self.state
                                .spend_budget_tokens(&virtual_key_id, &budget, spent_tokens);
                            if let Some((scope, budget)) = self.tenant_budget_scope.as_ref() {
                                self.state.spend_budget_tokens(scope, budget, spent_tokens);
                            }
                            if let Some((scope, budget)) = self.project_budget_scope.as_ref() {
                                self.state.spend_budget_tokens(scope, budget, spent_tokens);
                            }
                            if let Some((scope, budget)) = self.user_budget_scope.as_ref() {
                                self.state.spend_budget_tokens(scope, budget, spent_tokens);
                            }

                            #[cfg(feature = "gateway-costing")]
                            if let Some(spent_cost_usd_micros) = spent_cost_usd_micros {
                                self.state.spend_budget_cost(
                                    &virtual_key_id,
                                    &budget,
                                    spent_cost_usd_micros,
                                );
                                if let Some((scope, budget)) = self.tenant_budget_scope.as_ref() {
                                    self.state.spend_budget_cost(
                                        scope,
                                        budget,
                                        spent_cost_usd_micros,
                                    );
                                }
                                if let Some((scope, budget)) = self.project_budget_scope.as_ref() {
                                    self.state.spend_budget_cost(
                                        scope,
                                        budget,
                                        spent_cost_usd_micros,
                                    );
                                }
                                if let Some((scope, budget)) = self.user_budget_scope.as_ref() {
                                    self.state.spend_budget_cost(
                                        scope,
                                        budget,
                                        spent_cost_usd_micros,
                                    );
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
                        ),
                    ))]
                    if !self.cost_budget_reservation_ids.is_empty() {
                        settle_proxy_cost_budget_reservations(
                            &self.state,
                            &self.cost_budget_reservation_ids,
                            self.spend_tokens,
                            spent_cost_usd_micros.unwrap_or_default(),
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
                        ),
                    ))]
                    if !self.cost_budget_reserved && self.use_persistent_budget && self.spend_tokens
                    {
                        if let (Some(virtual_key_id), Some(spent_cost_usd_micros)) =
                            (self.virtual_key_id.as_deref(), spent_cost_usd_micros)
                        {
                            #[cfg(feature = "gateway-store-sqlite")]
                            if let Some(store) = self.state.stores.sqlite.as_ref() {
                                let _ = store
                                    .record_spent_cost_usd_micros(
                                        virtual_key_id,
                                        spent_cost_usd_micros,
                                    )
                                    .await;
                            }
                            #[cfg(feature = "gateway-store-postgres")]
                            if let Some(store) = self.state.stores.postgres.as_ref() {
                                let _ = store
                                    .record_spent_cost_usd_micros(
                                        virtual_key_id,
                                        spent_cost_usd_micros,
                                    )
                                    .await;
                            }
                            #[cfg(feature = "gateway-store-mysql")]
                            if let Some(store) = self.state.stores.mysql.as_ref() {
                                let _ = store
                                    .record_spent_cost_usd_micros(
                                        virtual_key_id,
                                        spent_cost_usd_micros,
                                    )
                                    .await;
                            }
                            #[cfg(feature = "gateway-store-redis")]
                            if let Some(store) = self.state.stores.redis.as_ref() {
                                let _ = store
                                    .record_spent_cost_usd_micros(
                                        virtual_key_id,
                                        spent_cost_usd_micros,
                                    )
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
                            "request_id": &self.request_id,
                            "provider": &self.provider,
                            "virtual_key_id": self.virtual_key_id.as_deref(),
                            "backend": &self.backend_name,
                            "attempted_backends": &self.attempted_backends,
                            "method": &self.method,
                            "path": &self.path_and_query,
                            "model": &self.model,
                            "upstream_model": self.upstream_model.as_deref(),
                            "status": self.status,
                            "charge_tokens": self.charge_tokens,
                            "input_tokens": observed_usage.and_then(|usage| usage.input_tokens),
                            "cache_input_tokens": observed_usage.and_then(|usage| usage.cache_input_tokens),
                            "cache_creation_input_tokens": observed_usage.and_then(|usage| usage.cache_creation_input_tokens),
                            "output_tokens": observed_usage.and_then(|usage| usage.output_tokens),
                            "reasoning_tokens": observed_usage.and_then(|usage| usage.reasoning_tokens),
                            "total_tokens": observed_usage.and_then(|usage| usage.total_tokens),
                            "spent_tokens": spent_tokens,
                            "charge_cost_usd_micros": self.charge_cost_usd_micros,
                            "spent_cost_usd_micros": spent_cost_usd_micros,
                            "body_len": self.request_body_len,
                            "stream": true,
                        });
                        append_audit_log(&self.state, "proxy", payload).await;
                    }

                    emit_json_log(
                        &self.state,
                        "proxy.response",
                        serde_json::json!({
                            "request_id": &self.request_id,
                            "provider": &self.provider,
                            "backend": &self.backend_name,
                            "status": self.status,
                            "attempted_backends": &self.attempted_backends,
                            "model": &self.model,
                            "upstream_model": self.upstream_model.as_deref(),
                            "input_tokens": observed_usage.and_then(|usage| usage.input_tokens),
                            "cache_input_tokens": observed_usage.and_then(|usage| usage.cache_input_tokens),
                            "cache_creation_input_tokens": observed_usage.and_then(|usage| usage.cache_creation_input_tokens),
                            "output_tokens": observed_usage.and_then(|usage| usage.output_tokens),
                            "reasoning_tokens": observed_usage.and_then(|usage| usage.reasoning_tokens),
                            "total_tokens": observed_usage.and_then(|usage| usage.total_tokens),
                            "spent_tokens": spent_tokens,
                        }),
                    );

                    #[cfg(feature = "sdk")]
                    emit_devtools_log(
                        &self.state,
                        "proxy.response",
                        serde_json::json!({
                            "request_id": &self.request_id,
                            "status": self.status,
                            "path": &self.path_and_query,
                            "backend": &self.backend_name,
                            "spent_tokens": spent_tokens,
                        }),
                    );
                }
            }

            struct ProxySseAbortFinalizeJob {
                finalizer: ProxySseFinalizer,
                observed: Option<ObservedUsage>,
                bytes_sent: u64,
            }

            struct ProxySseAbortFinalizerPool {
                senders: Vec<std::sync::mpsc::SyncSender<ProxySseAbortFinalizeJob>>,
                next_sender: std::sync::atomic::AtomicUsize,
            }

            fn proxy_sse_abort_finalizer_pool() -> &'static ProxySseAbortFinalizerPool {
                static POOL: std::sync::OnceLock<ProxySseAbortFinalizerPool> =
                    std::sync::OnceLock::new();
                POOL.get_or_init(|| {
                    let workers = PROXY_SSE_ABORT_FINALIZER_WORKERS.max(1);
                    let capacity = PROXY_SSE_ABORT_FINALIZER_QUEUE_CAPACITY.max(1);
                    let mut senders = Vec::with_capacity(workers);

                    for worker in 0..workers {
                        let (tx, rx) =
                            std::sync::mpsc::sync_channel::<ProxySseAbortFinalizeJob>(capacity);
                        let thread_name = format!("ditto-proxy-sse-finalizer-{worker}");
                        let spawn_result =
                            std::thread::Builder::new()
                                .name(thread_name)
                                .spawn(move || {
                                    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                                        .enable_all()
                                        .build()
                                    else {
                                        return;
                                    };
                                    while let Ok(job) = rx.recv() {
                                        runtime.block_on(async move {
                                            job.finalizer
                                                .finalize(
                                                    job.observed,
                                                    StreamEnd::Aborted,
                                                    job.bytes_sent,
                                                )
                                                .await;
                                        });
                                    }
                                });

                        if spawn_result.is_ok() {
                            senders.push(tx);
                        }
                    }

                    ProxySseAbortFinalizerPool {
                        senders,
                        next_sender: std::sync::atomic::AtomicUsize::new(0),
                    }
                })
            }

            fn enqueue_proxy_sse_abort_finalize(
                finalizer: ProxySseFinalizer,
                observed: Option<ObservedUsage>,
                bytes_sent: u64,
            ) {
                fn spawn_proxy_sse_abort_finalize(job: ProxySseAbortFinalizeJob) {
                    match tokio::runtime::Handle::try_current() {
                        Ok(handle) => {
                            handle.spawn(async move {
                                job.finalizer
                                    .finalize(job.observed, StreamEnd::Aborted, job.bytes_sent)
                                    .await;
                            });
                        }
                        Err(_) => {
                            let _ = std::thread::Builder::new()
                                .name("ditto-proxy-sse-finalizer-fallback".to_string())
                                .spawn(move || {
                                    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                                        .enable_all()
                                        .build()
                                    else {
                                        return;
                                    };
                                    runtime.block_on(async move {
                                        job.finalizer
                                            .finalize(
                                                job.observed,
                                                StreamEnd::Aborted,
                                                job.bytes_sent,
                                            )
                                            .await;
                                    });
                                });
                        }
                    }
                }

                let job = ProxySseAbortFinalizeJob {
                    finalizer,
                    observed,
                    bytes_sent,
                };

                let pool = proxy_sse_abort_finalizer_pool();
                if pool.senders.is_empty() {
                    spawn_proxy_sse_abort_finalize(job);
                    return;
                }

                let idx = pool.next_sender.fetch_add(1, Ordering::Relaxed) % pool.senders.len();
                if let Err(err) = pool.senders[idx].try_send(job) {
                    let job = match err {
                        std::sync::mpsc::TrySendError::Full(job) => job,
                        std::sync::mpsc::TrySendError::Disconnected(job) => job,
                    };
                    spawn_proxy_sse_abort_finalize(job);
                }
            }

            struct ProxySseStreamState {
                upstream: ProxyBodyStream,
                tracker: SseUsageTracker,
                bytes_sent: u64,
                finalizer: Option<ProxySseFinalizer>,
                #[cfg(feature = "gateway-proxy-cache")]
                cache_completion: Option<ProxyCompletedStreamCacheWrite>,
                _permits: ProxyPermits,
            }

            impl Drop for ProxySseStreamState {
                fn drop(&mut self) {
                    let Some(finalizer) = self.finalizer.take() else {
                        return;
                    };
                    let observed = self.tracker.observed_usage();
                    let bytes_sent = self.bytes_sent;
                    enqueue_proxy_sse_abort_finalize(finalizer, observed, bytes_sent);
                }
            }

            impl ProxySseStreamState {
                async fn finalize(&mut self, end: StreamEnd) {
                    #[cfg(feature = "gateway-proxy-cache")]
                    if matches!(end, StreamEnd::Completed) {
                        if let Some(cache_completion) = self.cache_completion.take() {
                            cache_completion.finish().await;
                        }
                    }

                    let Some(finalizer) = self.finalizer.take() else {
                        return;
                    };
                    let observed = self.tracker.observed_usage();
                    let bytes_sent = self.bytes_sent;
                    finalizer.finalize(observed, end, bytes_sent).await;
                }
            }

            let mut headers = upstream_headers;
            apply_proxy_response_headers(&mut headers, &backend_name, &request_id, false);
            #[cfg(feature = "gateway-proxy-cache")]
            if let Some(cache_key) = proxy_cache_key.as_ref() {
                if let Ok(value) = axum::http::HeaderValue::from_str(cache_key) {
                    headers.insert("x-ditto-cache-key", value);
                }
            }

            #[cfg(feature = "gateway-otel")]
            {
                tracing::Span::current().record("cache", tracing::field::display("miss"));
                tracing::Span::current().record("backend", tracing::field::display(&backend_name));
                tracing::Span::current().record("status", tracing::field::display(status.as_u16()));
            }

            let upstream_stream: ProxyBodyStream = upstream_response
                .bytes_stream()
                .map(|chunk| chunk.map_err(std::io::Error::other))
                .boxed();

            #[cfg(any(
                feature = "gateway-store-sqlite",
                feature = "gateway-store-postgres",
                feature = "gateway-store-mysql",
                feature = "gateway-store-redis"
            ))]
            let token_budget_reservation_ids = token_budget_reservation_ids.to_vec();

            #[cfg(all(
                feature = "gateway-costing",
                any(
                    feature = "gateway-store-sqlite",
                    feature = "gateway-store-postgres",
                    feature = "gateway-store-mysql",
                    feature = "gateway-store-redis"
                ),
            ))]
            let cost_budget_reservation_ids = cost_budget_reservation_ids.to_vec();

            let finalizer = ProxySseFinalizer {
                state: state.to_owned(),
                backend_name: backend_name.clone(),
                attempted_backends: attempted_backends.to_vec(),
                request_id: request_id.clone(),
                provider: protocol.clone(),
                #[cfg(any(
                    feature = "gateway-store-sqlite",
                    feature = "gateway-store-postgres",
                    feature = "gateway-store-mysql",
                    feature = "gateway-store-redis",
                    feature = "sdk"
                ))]
                method: parts.method.as_str().to_string(),
                #[cfg(any(
                    feature = "gateway-store-sqlite",
                    feature = "gateway-store-postgres",
                    feature = "gateway-store-mysql",
                    feature = "gateway-store-redis",
                    feature = "sdk"
                ))]
                path_and_query: path_and_query.to_string(),
                #[cfg(feature = "gateway-metrics-prometheus")]
                metrics_path: metrics_path.to_string(),
                model: model.to_owned(),
                upstream_model: upstream_model.clone(),
                service_tier: service_tier.to_owned(),
                backend_model_map: backend_model_map.clone(),
                status: status.as_u16(),
                charge_tokens,
                charge_cost_usd_micros,
                spend_tokens,
                #[cfg(any(
                    feature = "gateway-store-sqlite",
                    feature = "gateway-store-postgres",
                    feature = "gateway-store-mysql",
                    feature = "gateway-store-redis"
                ))]
                use_persistent_budget,
                virtual_key_id: virtual_key_id.to_owned(),
                budget: budget.to_owned(),
                tenant_budget_scope: tenant_budget_scope.to_owned(),
                project_budget_scope: project_budget_scope.to_owned(),
                user_budget_scope: user_budget_scope.to_owned(),
                #[cfg(any(
                    feature = "gateway-store-sqlite",
                    feature = "gateway-store-postgres",
                    feature = "gateway-store-mysql",
                    feature = "gateway-store-redis"
                ))]
                token_budget_reservation_ids,
                #[cfg(all(
                    feature = "gateway-costing",
                    any(
                        feature = "gateway-store-sqlite",
                        feature = "gateway-store-postgres",
                        feature = "gateway-store-mysql",
                        feature = "gateway-store-redis"
                    )
                ))]
                cost_budget_reserved: _cost_budget_reserved,
                #[cfg(all(
                    feature = "gateway-costing",
                    any(
                        feature = "gateway-store-sqlite",
                        feature = "gateway-store-postgres",
                        feature = "gateway-store-mysql",
                        feature = "gateway-store-redis"
                    )
                ))]
                cost_budget_reservation_ids,
                #[cfg(any(
                    feature = "gateway-store-sqlite",
                    feature = "gateway-store-postgres",
                    feature = "gateway-store-mysql",
                    feature = "gateway-store-redis",
                    feature = "sdk"
                ))]
                request_body_len: body.len(),
            };

            #[cfg(feature = "gateway-metrics-prometheus")]
            if let Some(metrics) = state.proxy.metrics.as_ref() {
                metrics
                    .lock()
                    .await
                    .record_proxy_stream_open(&backend_name, metrics_path);
            }

            let state = ProxySseStreamState {
                upstream: upstream_stream,
                tracker: SseUsageTracker::default(),
                bytes_sent: 0,
                finalizer: Some(finalizer),
                #[cfg(feature = "gateway-proxy-cache")]
                cache_completion: ProxyCompletedStreamCacheWrite::new(
                    state,
                    &backend_name,
                    status,
                    &headers,
                    proxy_cache_key.as_deref(),
                    proxy_cache_metadata.as_ref(),
                ),
                _permits: proxy_permits.take(),
            };

            let stream = futures_util::stream::try_unfold(state, |mut state| async move {
                match state.upstream.next().await {
                    Some(Ok(chunk)) => {
                        state.bytes_sent = state.bytes_sent.saturating_add(chunk.len() as u64);
                        state.tracker.ingest(&chunk);
                        #[cfg(feature = "gateway-proxy-cache")]
                        if let Some(cache_completion) = state.cache_completion.as_mut() {
                            cache_completion.ingest(&chunk);
                        }
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
            return Ok(BackendAttemptOutcome::Response(response));
        }
        // end inline: proxy_backend/stream.rs
    }

    // inlined from proxy_backend/nonstream.rs
    {
        let content_length = upstream_headers
            .get("content-length")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<usize>().ok());

        #[cfg(feature = "gateway-proxy-cache")]
        let should_attempt_buffer_for_cache =
            status.is_success() && proxy_cache_key.is_some() && state.proxy.cache_config.is_some();

        let should_attempt_buffer_for_usage =
            content_type.starts_with("application/json") && state.proxy.usage_max_body_bytes > 0;

        let cache_max_buffer_bytes = {
            #[cfg(feature = "gateway-proxy-cache")]
            {
                if should_attempt_buffer_for_cache {
                    state
                        .proxy
                        .cache_config
                        .as_ref()
                        .map(|config| config.max_body_bytes)
                        .unwrap_or(1024 * 1024)
                } else {
                    0
                }
            }
            #[cfg(not(feature = "gateway-proxy-cache"))]
            {
                0
            }
        };

        let usage_max_buffer_bytes = if should_attempt_buffer_for_usage {
            state.proxy.usage_max_body_bytes
        } else {
            0
        };

        let max_buffer_bytes = cache_max_buffer_bytes.max(usage_max_buffer_bytes);
        let should_try_buffer =
            max_buffer_bytes > 0 && content_length.is_none_or(|len| len <= max_buffer_bytes);

        enum ProxyResponseBody {
            Bytes(Bytes),
            Stream(ProxyBodyStream),
        }

        let response_body = if should_try_buffer {
            let mut upstream_stream = upstream_response.bytes_stream();
            let initial_capacity = content_length
                .map(|len| len.min(max_buffer_bytes))
                .unwrap_or(0);
            let mut buffered = bytes::BytesMut::with_capacity(initial_capacity);
            let mut first_unbuffered: Option<Bytes> = None;
            let mut stream_error: Option<std::io::Error> = None;

            while let Some(next) = upstream_stream.next().await {
                match next {
                    Ok(chunk) => {
                        if buffered.len().saturating_add(chunk.len()) <= max_buffer_bytes {
                            buffered.extend_from_slice(chunk.as_ref());
                        } else {
                            first_unbuffered = Some(chunk);
                            break;
                        }
                    }
                    Err(err) => {
                        stream_error = Some(std::io::Error::other(err));
                        break;
                    }
                }
            }

            match (first_unbuffered, stream_error) {
                (None, None) => ProxyResponseBody::Bytes(buffered.freeze()),
                (Some(chunk), _) => {
                    let prefix_bytes = buffered.freeze();
                    let prefix: ProxyBodyStream = if prefix_bytes.is_empty() {
                        futures_util::stream::empty().boxed()
                    } else {
                        futures_util::stream::once(async move {
                            Ok::<Bytes, std::io::Error>(prefix_bytes)
                        })
                        .boxed()
                    };
                    let first =
                        futures_util::stream::once(
                            async move { Ok::<Bytes, std::io::Error>(chunk) },
                        );
                    let rest = upstream_stream.map(|chunk| chunk.map_err(std::io::Error::other));
                    let stream = prefix.chain(first).chain(rest).boxed();
                    ProxyResponseBody::Stream(stream)
                }
                (None, Some(err)) => {
                    let prefix_bytes = buffered.freeze();
                    let prefix: ProxyBodyStream = if prefix_bytes.is_empty() {
                        futures_util::stream::empty().boxed()
                    } else {
                        futures_util::stream::once(async move {
                            Ok::<Bytes, std::io::Error>(prefix_bytes)
                        })
                        .boxed()
                    };
                    let err_stream =
                        futures_util::stream::once(
                            async move { Err::<Bytes, std::io::Error>(err) },
                        );
                    let stream = prefix.chain(err_stream).boxed();
                    ProxyResponseBody::Stream(stream)
                }
            }
        } else {
            let stream = upstream_response
                .bytes_stream()
                .map(|chunk| chunk.map_err(std::io::Error::other))
                .boxed();
            ProxyResponseBody::Stream(stream)
        };

        let observed_usage = if should_attempt_buffer_for_usage {
            match &response_body {
                ProxyResponseBody::Bytes(bytes) => extract_openai_usage_from_bytes(bytes),
                ProxyResponseBody::Stream(_) => None,
            }
        } else {
            None
        };

        let spent_tokens = if spend_tokens {
            observed_usage
                .and_then(|usage| usage.total_tokens)
                .unwrap_or_else(|| u64::from(charge_tokens))
        } else {
            0
        };

        #[cfg(feature = "gateway-costing")]
        let spent_cost_usd_micros = if spend_tokens {
            model
                .as_deref()
                .map(|request_model| {
                    backend_model_map
                        .get(request_model)
                        .map(|model| model.as_str())
                        .unwrap_or(request_model)
                })
                .and_then(|cost_model| {
                    state.proxy.pricing.as_ref().and_then(|pricing| {
                        let usage = observed_usage?;
                        let input = usage.input_tokens?;
                        let output = usage.output_tokens?;
                        pricing.estimate_cost_usd_micros_with_cache_for_service_tier(
                            cost_model,
                            clamp_u64_to_u32(input),
                            usage.cache_input_tokens.map(clamp_u64_to_u32),
                            usage.cache_creation_input_tokens.map(clamp_u64_to_u32),
                            clamp_u64_to_u32(output),
                            service_tier.as_deref(),
                        )
                    })
                })
                .or(charge_cost_usd_micros)
        } else {
            None
        };
        #[cfg(not(feature = "gateway-costing"))]
        let spent_cost_usd_micros: Option<u64> = None;

        #[cfg(not(any(
            feature = "gateway-costing",
            feature = "gateway-store-sqlite",
            feature = "gateway-store-redis"
        )))]
        let _ = spent_cost_usd_micros;

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
                spent_tokens,
            )
            .await;
        } else if let (Some(virtual_key_id), Some(budget)) =
            (virtual_key_id.clone(), budget.clone())
        {
            if spend_tokens {
                state.spend_budget_tokens(&virtual_key_id, &budget, spent_tokens);
                if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                    state.spend_budget_tokens(scope, budget, spent_tokens);
                }
                if let Some((scope, budget)) = project_budget_scope.as_ref() {
                    state.spend_budget_tokens(scope, budget, spent_tokens);
                }
                if let Some((scope, budget)) = user_budget_scope.as_ref() {
                    state.spend_budget_tokens(scope, budget, spent_tokens);
                }

                #[cfg(feature = "gateway-costing")]
                if !use_persistent_budget {
                    if let Some(spent_cost_usd_micros) = spent_cost_usd_micros {
                        state.spend_budget_cost(&virtual_key_id, &budget, spent_cost_usd_micros);
                        if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                            state.spend_budget_cost(scope, budget, spent_cost_usd_micros);
                        }
                        if let Some((scope, budget)) = project_budget_scope.as_ref() {
                            state.spend_budget_cost(scope, budget, spent_cost_usd_micros);
                        }
                        if let Some((scope, budget)) = user_budget_scope.as_ref() {
                            state.spend_budget_cost(scope, budget, spent_cost_usd_micros);
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
                state.spend_budget_tokens(&virtual_key_id, &budget, spent_tokens);
                if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                    state.spend_budget_tokens(scope, budget, spent_tokens);
                }
                if let Some((scope, budget)) = project_budget_scope.as_ref() {
                    state.spend_budget_tokens(scope, budget, spent_tokens);
                }
                if let Some((scope, budget)) = user_budget_scope.as_ref() {
                    state.spend_budget_tokens(scope, budget, spent_tokens);
                }

                #[cfg(feature = "gateway-costing")]
                if let Some(spent_cost_usd_micros) = spent_cost_usd_micros {
                    state.spend_budget_cost(&virtual_key_id, &budget, spent_cost_usd_micros);
                    if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                        state.spend_budget_cost(scope, budget, spent_cost_usd_micros);
                    }
                    if let Some((scope, budget)) = project_budget_scope.as_ref() {
                        state.spend_budget_cost(scope, budget, spent_cost_usd_micros);
                    }
                    if let Some((scope, budget)) = user_budget_scope.as_ref() {
                        state.spend_budget_cost(scope, budget, spent_cost_usd_micros);
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
            ),
        ))]
        if !cost_budget_reservation_ids.is_empty() {
            settle_proxy_cost_budget_reservations(
                state,
                cost_budget_reservation_ids,
                spend_tokens,
                spent_cost_usd_micros.unwrap_or_default(),
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
            ),
        ))]
        if !_cost_budget_reserved && use_persistent_budget && spend_tokens {
            if let (Some(virtual_key_id), Some(spent_cost_usd_micros)) =
                (virtual_key_id.as_deref(), spent_cost_usd_micros)
            {
                #[cfg(feature = "gateway-store-sqlite")]
                if let Some(store) = state.stores.sqlite.as_ref() {
                    let _ = store
                        .record_spent_cost_usd_micros(virtual_key_id, spent_cost_usd_micros)
                        .await;
                }
                #[cfg(feature = "gateway-store-postgres")]
                if let Some(store) = state.stores.postgres.as_ref() {
                    let _ = store
                        .record_spent_cost_usd_micros(virtual_key_id, spent_cost_usd_micros)
                        .await;
                }
                #[cfg(feature = "gateway-store-mysql")]
                if let Some(store) = state.stores.mysql.as_ref() {
                    let _ = store
                        .record_spent_cost_usd_micros(virtual_key_id, spent_cost_usd_micros)
                        .await;
                }
                #[cfg(feature = "gateway-store-redis")]
                if let Some(store) = state.stores.redis.as_ref() {
                    let _ = store
                        .record_spent_cost_usd_micros(virtual_key_id, spent_cost_usd_micros)
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
                "service_tier": service_tier.as_deref(),
                "status": status.as_u16(),
                "charge_tokens": charge_tokens,
                "input_tokens": observed_usage.and_then(|usage| usage.input_tokens),
                "cache_input_tokens": observed_usage.and_then(|usage| usage.cache_input_tokens),
                "cache_creation_input_tokens": observed_usage.and_then(|usage| usage.cache_creation_input_tokens),
                "output_tokens": observed_usage.and_then(|usage| usage.output_tokens),
                "reasoning_tokens": observed_usage.and_then(|usage| usage.reasoning_tokens),
                "total_tokens": observed_usage.and_then(|usage| usage.total_tokens),
                "spent_tokens": spent_tokens,
                "charge_cost_usd_micros": charge_cost_usd_micros,
                "spent_cost_usd_micros": spent_cost_usd_micros,
                "body_len": body.len(),
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
                "input_tokens": observed_usage.and_then(|usage| usage.input_tokens),
                "cache_input_tokens": observed_usage.and_then(|usage| usage.cache_input_tokens),
                "cache_creation_input_tokens": observed_usage.and_then(|usage| usage.cache_creation_input_tokens),
                "output_tokens": observed_usage.and_then(|usage| usage.output_tokens),
                "reasoning_tokens": observed_usage.and_then(|usage| usage.reasoning_tokens),
                "total_tokens": observed_usage.and_then(|usage| usage.total_tokens),
                "spent_tokens": spent_tokens,
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

        #[cfg(feature = "gateway-proxy-cache")]
        if should_attempt_buffer_for_cache && status.is_success() {
            if let (Some(cache_key), Some(cache_metadata)) =
                (proxy_cache_key.as_deref(), proxy_cache_metadata.as_ref())
            {
                if let ProxyResponseBody::Bytes(bytes) = &response_body {
                    let cached = CachedProxyResponse {
                        status: status.as_u16(),
                        headers: upstream_headers.clone(),
                        body: bytes.clone(),
                        backend: backend_name.clone(),
                    };
                    store_proxy_cache_response(
                        state,
                        cache_key,
                        cached,
                        cache_metadata,
                        now_epoch_seconds(),
                    )
                    .await;
                }
            }
        }

        let mut headers = upstream_headers;
        apply_proxy_response_headers(&mut headers, &backend_name, &request_id, false);
        #[cfg(feature = "gateway-proxy-cache")]
        if let Some(cache_key) = proxy_cache_key.as_deref() {
            if let Ok(value) = axum::http::HeaderValue::from_str(cache_key) {
                headers.insert("x-ditto-cache-key", value);
            }
        }
        match response_body {
            ProxyResponseBody::Bytes(bytes) => {
                let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
                let mut response = axum::response::Response::new(body);
                *response.status_mut() = status;
                *response.headers_mut() = headers;
                Ok(BackendAttemptOutcome::Response(response))
            }
            ProxyResponseBody::Stream(stream) => {
                headers.remove("content-length");
                let stream = ProxyBodyStreamWithPermit {
                    inner: stream,
                    _permits: proxy_permits.take(),
                };
                let mut response = axum::response::Response::new(Body::from_stream(stream));
                *response.status_mut() = status;
                *response.headers_mut() = headers;
                Ok(BackendAttemptOutcome::Response(response))
            }
        }
    }
    // end inline: proxy_backend/nonstream.rs
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
