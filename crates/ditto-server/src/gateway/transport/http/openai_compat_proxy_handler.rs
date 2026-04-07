use super::*;

fn is_non_billable_openai_meta_path(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    matches!(
        path,
        "/v1/responses/input_tokens" | "/v1/responses/input_tokens/"
    )
}

pub(super) async fn handle_openai_compat_proxy(
    State(state): State<GatewayHttpState>,
    Path(_path): Path<String>,
    req: axum::http::Request<Body>,
) -> Result<axum::response::Response, (StatusCode, Json<OpenAiErrorResponse>)> {
    let max_body_bytes = state.proxy.max_body_bytes;
    let (parts, incoming_body) = req.into_parts();
    let client_supplied_request_id = parts.headers.contains_key("x-request-id");
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
    let metrics_path = super::super::metrics_prometheus::normalize_proxy_path_label(path_and_query);
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
        if client_supplied_request_id && !parts.method.is_safe() {
            return Err(openai_error(
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                Some("unsupported_idempotency"),
                "x-request-id idempotency is not supported for streaming multipart proxy requests",
            ));
        }
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
    let body = {
        let _buffering_permit = if let Some(limit) = state.proxy.backpressure.as_ref() {
            Some(limit.clone().try_acquire_owned().map_err(|_| {
                openai_error(
                    StatusCode::TOO_MANY_REQUESTS,
                    "rate_limit_error",
                    Some("inflight_limit"),
                    "too many in-flight proxy requests",
                )
            })?)
        } else {
            None
        };
        to_bytes(incoming_body, max_body_bytes)
            .await
            .map_err(|err| {
                openai_error(StatusCode::BAD_REQUEST, "invalid_request_error", None, err)
            })?
    };

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

    let _stream_requested = parsed_json
        .as_ref()
        .and_then(|value| value.get("stream"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

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
    let charge_tokens = if is_non_billable_openai_meta_path(path_and_query) {
        0
    } else {
        input_tokens_estimate.saturating_add(max_output_tokens)
    };

    #[cfg(feature = "gateway-store-sqlite")]
    let use_sqlite_budget = state.stores.sqlite.is_some();
    #[cfg(not(feature = "gateway-store-sqlite"))]
    let use_sqlite_budget = false;

    #[cfg(feature = "gateway-store-postgres")]
    let use_postgres_budget = state.stores.postgres.is_some();
    #[cfg(not(feature = "gateway-store-postgres"))]
    let use_postgres_budget = false;

    #[cfg(feature = "gateway-store-mysql")]
    let use_mysql_budget = state.stores.mysql.is_some();
    #[cfg(not(feature = "gateway-store-mysql"))]
    let use_mysql_budget = false;

    #[cfg(feature = "gateway-store-redis")]
    let use_redis_budget = state.stores.redis.is_some();
    #[cfg(not(feature = "gateway-store-redis"))]
    let use_redis_budget = false;

    let use_persistent_budget =
        use_sqlite_budget || use_postgres_budget || use_mysql_budget || use_redis_budget;

    let _now_epoch_seconds = now_epoch_seconds();
    let minute = _now_epoch_seconds / 60;
    #[cfg(feature = "gateway-store-redis")]
    let rate_limit_route = normalize_rate_limit_route(path_and_query);

    let ResolvedGatewayContext {
        virtual_key_id,
        #[cfg(feature = "gateway-translation")]
        response_owner,
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
        local_rate_limit_reserved,
        local_token_budget_reserved,
        #[cfg(feature = "gateway-costing")]
        local_cost_budget_reserved,
    } = resolve_openai_compat_proxy_gateway_context(
        ResolveOpenAiCompatProxyGatewayContextRequest {
            state: &state,
            parts: &parts,
            body: &body,
            parsed_json: &parsed_json,
            request_id: &request_id,
            path_and_query,
            model: &model,
            service_tier: &service_tier,
            input_tokens_estimate,
            max_output_tokens,
            charge_tokens,
            minute,
            use_redis_budget,
            use_persistent_budget,
            #[cfg(feature = "gateway-metrics-prometheus")]
            metrics_path: &metrics_path,
            #[cfg(feature = "gateway-metrics-prometheus")]
            metrics_timer_start,
        },
    )
    .await?;

    let mut request_dedup_leader =
        match prepare_proxy_request_dedup(PrepareProxyRequestDedupInput {
            state: &state,
            method: &parts.method,
            path_and_query,
            headers: &parts.headers,
            body: &body,
            request_id: &request_id,
            client_supplied_request_id,
            virtual_key_id: virtual_key_id.as_deref(),
        })
        .await?
        {
            ProxyRequestDedupDecision::Disabled => None,
            ProxyRequestDedupDecision::Replay(result) => return result,
            ProxyRequestDedupDecision::Leader(leader) => Some(leader),
        };

    state.record_request();

    #[cfg(not(feature = "gateway-store-redis"))]
    let _ = (
        &limits,
        &tenant_limits_scope,
        &project_limits_scope,
        &user_limits_scope,
        local_rate_limit_reserved,
    );

    #[cfg(feature = "gateway-store-redis")]
    let redis_rate_limit_scopes = redis_rate_limit_scopes(
        virtual_key_id.as_deref(),
        limits.as_ref(),
        tenant_limits_scope.as_ref(),
        project_limits_scope.as_ref(),
        user_limits_scope.as_ref(),
    );

    #[cfg(feature = "gateway-store-redis")]
    let mut redis_rate_limit_reserved = false;

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.stores.redis.as_ref()
        && let Err(err) = store
            .check_and_consume_rate_limits_many(
                redis_rate_limit_scopes.iter().copied(),
                &rate_limit_route,
                charge_tokens,
                _now_epoch_seconds,
            )
            .await
    {
        let is_rate_limited = matches!(err, GatewayError::RateLimited { .. });
        if is_rate_limited {
            state.record_rate_limited();
        }
        let mapped = map_openai_gateway_error(err);
        #[cfg(feature = "gateway-metrics-prometheus")]
        if is_rate_limited && let Some(metrics) = state.proxy.metrics.as_ref() {
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
        return finish_proxy_request_dedup_result(request_dedup_leader.take(), Err(mapped)).await;
    }
    #[cfg(feature = "gateway-store-redis")]
    if state.stores.redis.is_some() && !redis_rate_limit_scopes.is_empty() {
        redis_rate_limit_reserved = true;
    }

    #[cfg(feature = "gateway-otel")]
    if let Some(virtual_key_id) = virtual_key_id.as_deref() {
        proxy_span.record("virtual_key_id", tracing::field::display(virtual_key_id));
    }

    #[cfg(feature = "gateway-metrics-prometheus")]
    if let Some(metrics) = state.proxy.metrics.as_ref() {
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
    let streaming_cache_enabled = state
        .proxy
        .cache_config
        .as_ref()
        .is_some_and(ProxyCacheConfig::streaming_cache_enabled);

    #[cfg(feature = "gateway-proxy-cache")]
    let (proxy_cache_key, proxy_cache_metadata) = if state.proxy.cache.is_some()
        && proxy_cache_can_read(&parts.method)
        && (!_stream_requested || streaming_cache_enabled)
        && !proxy_cache_bypass(&parts.headers)
        && (parts.method == axum::http::Method::GET || parsed_json.is_some())
    {
        let scope = proxy_cache_scope(virtual_key_id.as_deref(), &parts.headers);
        let route_partition = proxy_cache_route_partition(&backend_candidates);
        (
            Some(proxy_cache_key(
                &parts.method,
                path_and_query,
                &body,
                &scope,
                &route_partition,
                &parts.headers,
            )),
            Some(ProxyCacheEntryMetadata::new(
                scope,
                &parts.method,
                path_and_query,
                model.as_deref(),
                Some(&route_partition),
            )),
        )
    } else {
        (None, None)
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
            return finish_proxy_request_dedup_result(request_dedup_leader.take(), Ok(response))
                .await;
        }
    }

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
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

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    let (_token_budget_reserved, token_budget_reservation_ids) =
        match reserve_proxy_token_budgets_for_request(budget_reservation_params).await {
            Ok(reserved) => reserved,
            Err(err) => {
                if err.0 == StatusCode::PAYMENT_REQUIRED {
                    state.record_budget_exceeded();
                }

                #[cfg(feature = "gateway-metrics-prometheus")]
                if let Some(metrics) = state.proxy.metrics.as_ref() {
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
                #[cfg(feature = "gateway-store-redis")]
                if redis_rate_limit_reserved && let Some(store) = state.stores.redis.as_ref() {
                    let _ = store
                        .refund_rate_limits_many(
                            redis_rate_limit_scopes.iter().copied(),
                            &rate_limit_route,
                            charge_tokens,
                            _now_epoch_seconds,
                        )
                        .await;
                }
                return finish_proxy_request_dedup_result(request_dedup_leader.take(), Err(err))
                    .await;
            }
        };

    #[cfg(not(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    )))]
    let _token_budget_reserved = false;

    #[cfg(all(
        feature = "gateway-costing",
        any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        ),
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
                    state.record_budget_exceeded();
                }

                #[cfg(feature = "gateway-metrics-prometheus")]
                if let Some(metrics) = state.proxy.metrics.as_ref() {
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
                #[cfg(feature = "gateway-store-redis")]
                if redis_rate_limit_reserved && let Some(store) = state.stores.redis.as_ref() {
                    let _ = store
                        .refund_rate_limits_many(
                            redis_rate_limit_scopes.iter().copied(),
                            &rate_limit_route,
                            charge_tokens,
                            _now_epoch_seconds,
                        )
                        .await;
                }
                return finish_proxy_request_dedup_result(request_dedup_leader.take(), Err(err))
                    .await;
            }
        };

    #[cfg(not(all(
        feature = "gateway-costing",
        any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        ),
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
        .proxy
        .routing
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
        #[cfg(feature = "gateway-routing-advanced")]
        client_supplied_request_id,
        path_and_query,
        now_epoch_seconds: _now_epoch_seconds,
        charge_tokens,
        stream_requested: _stream_requested,
        strip_authorization,
        use_persistent_budget,
        virtual_key_id: &virtual_key_id,
        limits: &limits,
        #[cfg(feature = "gateway-translation")]
        response_owner: &response_owner,
        budget: &budget,
        tenant_budget_scope: &tenant_budget_scope,
        project_budget_scope: &project_budget_scope,
        user_budget_scope: &user_budget_scope,
        tenant_limits_scope: &tenant_limits_scope,
        project_limits_scope: &project_limits_scope,
        user_limits_scope: &user_limits_scope,
        local_rate_limit_reserved,
        #[cfg(feature = "gateway-store-redis")]
        redis_rate_limit_reserved,
        #[cfg(feature = "gateway-store-redis")]
        rate_limit_route: &rate_limit_route,
        local_token_budget_reserved,
        #[cfg(feature = "gateway-costing")]
        local_cost_budget_reserved,
        charge_cost_usd_micros,
        #[cfg(any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        ))]
        token_budget_reservation_ids: &token_budget_reservation_ids,
        cost_budget_reserved: _cost_budget_reserved,
        #[cfg(all(
            feature = "gateway-costing",
            any(
                feature = "gateway-store-sqlite",
                feature = "gateway-store-postgres",
                feature = "gateway-store-mysql",
                feature = "gateway-store-redis"
            ),
        ))]
        cost_budget_reservation_ids: &cost_budget_reservation_ids,
        max_attempts,
        #[cfg(feature = "gateway-routing-advanced")]
        retry_config: &retry_config,
        #[cfg(feature = "gateway-proxy-cache")]
        proxy_cache_key: &proxy_cache_key,
        #[cfg(feature = "gateway-proxy-cache")]
        proxy_cache_metadata: &proxy_cache_metadata,
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
        if let Some(translation_backend) = state
            .backends
            .translation_backends
            .get(&backend_name)
            .cloned()
        {
            match attempt_translation_backend(
                attempt_params,
                &backend_name,
                translation_backend,
                &attempted_backends,
            )
            .await?
            {
                BackendAttemptOutcome::Response(response) => {
                    return finish_proxy_request_dedup_result(
                        request_dedup_leader.take(),
                        Ok(response),
                    )
                    .await;
                }
                BackendAttemptOutcome::Continue(err) => {
                    if let Some(err) = err {
                        last_err = Some(err);
                    }
                    continue;
                }
                BackendAttemptOutcome::Stop(err) => {
                    last_err = Some(err);
                    break;
                }
            }
        }

        match attempt_proxy_backend(attempt_params, &backend_name, idx, &attempted_backends).await?
        {
            BackendAttemptOutcome::Response(response) => {
                return finish_proxy_request_dedup_result(
                    request_dedup_leader.take(),
                    Ok(response),
                )
                .await;
            }
            BackendAttemptOutcome::Continue(err) => {
                if let Some(err) = err {
                    last_err = Some(err);
                }
                continue;
            }
            BackendAttemptOutcome::Stop(err) => {
                last_err = Some(err);
                break;
            }
        }
    }

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    rollback_proxy_token_budget_reservations(&state, &token_budget_reservation_ids).await;

    #[cfg(all(
        feature = "gateway-costing",
        any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        ),
    ))]
    rollback_proxy_cost_budget_reservations(&state, &cost_budget_reservation_ids).await;

    if local_token_budget_reserved {
        let budget_scopes = collect_budget_scopes(
            virtual_key_id.as_deref(),
            budget.as_ref(),
            &tenant_budget_scope,
            &project_budget_scope,
            &user_budget_scope,
        );
        state.rollback_budget_tokens(budget_scopes.clone(), u64::from(charge_tokens));
        #[cfg(feature = "gateway-costing")]
        if local_cost_budget_reserved {
            state.rollback_budget_cost(budget_scopes, charge_cost_usd_micros.unwrap_or_default());
        }
    }

    if local_rate_limit_reserved {
        let rate_limit_scopes = collect_limit_scopes(
            virtual_key_id.as_deref(),
            limits.as_ref(),
            &tenant_limits_scope,
            &project_limits_scope,
            &user_limits_scope,
        );
        state.rollback_rate_limits(rate_limit_scopes, charge_tokens, minute);
    }

    #[cfg(feature = "gateway-store-redis")]
    if redis_rate_limit_reserved && let Some(store) = state.stores.redis.as_ref() {
        let _ = store
            .refund_rate_limits_many(
                redis_rate_limit_scopes.iter().copied(),
                &rate_limit_route,
                charge_tokens,
                _now_epoch_seconds,
            )
            .await;
    }

    #[cfg(feature = "gateway-metrics-prometheus")]
    let proxy_metrics = Some((metrics_path.as_str(), metrics_timer_start));
    #[cfg(not(feature = "gateway-metrics-prometheus"))]
    let proxy_metrics = None;

    finish_proxy_request_dedup_result(
        request_dedup_leader.take(),
        Err(finalize_openai_compat_proxy_failure(
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
        .await),
    )
    .await
}
// end inline: ../../http/openai_compat_proxy.rs
