use super::*;

// This file is intentionally split to keep each staged Rust file under the pre-commit size limit.
// inlined from streaming_multipart/preamble.rs
pub(super) fn should_stream_large_multipart_request(
    parts: &axum::http::request::Parts,
    path_and_query: &str,
    max_body_bytes: usize,
) -> bool {
    if parts.method != axum::http::Method::POST {
        return false;
    }

    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query)
        .trim_end_matches('/');
    if path != "/v1/files" && path != "/v1/audio/transcriptions" && path != "/v1/audio/translations"
    {
        return false;
    }

    let is_multipart = parts
        .headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|ct| ct.to_ascii_lowercase().starts_with("multipart/form-data"));
    if !is_multipart {
        return false;
    }

    let content_length = parts
        .headers
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|raw| raw.parse::<usize>().ok());
    content_length.is_some_and(|len| len > max_body_bytes)
}

fn estimate_tokens_from_length(len: usize) -> u32 {
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

async fn buffer_streaming_multipart_body(
    body: Body,
    content_length: usize,
) -> Result<Bytes, (StatusCode, Json<OpenAiErrorResponse>)> {
    let mut buffered = bytes::BytesMut::with_capacity(content_length.min(64 * 1024));
    let mut stream = body.into_data_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|err| {
            openai_error(
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                Some("invalid_request"),
                format!("failed to read multipart request body: {err}"),
            )
        })?;
        buffered.extend_from_slice(&chunk);
    }

    Ok(buffered.freeze())
}

fn multipart_request_model(
    path_and_query: &str,
    content_type: Option<&str>,
    body: &Bytes,
) -> Result<Option<String>, (StatusCode, Json<OpenAiErrorResponse>)> {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query)
        .trim_end_matches('/');
    if path != "/v1/audio/transcriptions" && path != "/v1/audio/translations" {
        return Ok(None);
    }

    let Some(content_type) = content_type else {
        return Ok(None);
    };

    let parts =
        super::super::multipart::parse_multipart_form(content_type, body).map_err(|err| {
            openai_error(
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                Some("invalid_request"),
                err,
            )
        })?;
    for part in parts {
        if part.name == "model" && part.filename.is_none() {
            let model = String::from_utf8_lossy(part.data.as_ref())
                .trim()
                .to_string();
            if !model.is_empty() {
                return Ok(Some(model));
            }
        }
    }
    Ok(None)
}

// end inline: streaming_multipart/preamble.rs
// inlined from streaming_multipart/handler.rs
struct ResolvedStreamingMultipartGatewayContext {
    virtual_key_id: Option<String>,
    limits: Option<super::LimitsConfig>,
    budget: Option<super::BudgetConfig>,
    tenant_budget_scope: Option<(String, super::BudgetConfig)>,
    project_budget_scope: Option<(String, super::BudgetConfig)>,
    user_budget_scope: Option<(String, super::BudgetConfig)>,
    tenant_limits_scope: Option<(String, super::LimitsConfig)>,
    project_limits_scope: Option<(String, super::LimitsConfig)>,
    user_limits_scope: Option<(String, super::LimitsConfig)>,
    backend_candidates: Vec<String>,
    strip_authorization: bool,
    local_token_budget_reserved: bool,
}

#[allow(clippy::too_many_arguments)]
fn rollback_local_streaming_multipart_budgets(
    state: &GatewayHttpState,
    virtual_key_id: Option<&str>,
    budget: Option<&super::BudgetConfig>,
    tenant_budget_scope: &Option<(String, super::BudgetConfig)>,
    project_budget_scope: &Option<(String, super::BudgetConfig)>,
    user_budget_scope: &Option<(String, super::BudgetConfig)>,
    charge_tokens: u32,
    #[cfg(feature = "gateway-costing")] charge_cost_usd_micros: Option<u64>,
) {
    let budget_scopes = collect_budget_scopes(
        virtual_key_id,
        budget,
        tenant_budget_scope,
        project_budget_scope,
        user_budget_scope,
    );
    state.rollback_budget_tokens(budget_scopes.clone(), u64::from(charge_tokens));
    #[cfg(feature = "gateway-costing")]
    if let Some(charge_cost_usd_micros) = charge_cost_usd_micros {
        state.rollback_budget_cost(budget_scopes, charge_cost_usd_micros);
    }
}

pub(super) async fn handle_openai_compat_proxy_streaming_multipart(
    state: GatewayHttpState,
    parts: axum::http::request::Parts,
    body: Body,
    request_id: String,
    path_and_query: String,
) -> Result<axum::response::Response, (StatusCode, Json<OpenAiErrorResponse>)> {
    #[cfg(feature = "gateway-metrics-prometheus")]
    let metrics_path =
        super::super::metrics_prometheus::normalize_proxy_path_label(&path_and_query);
    #[cfg(feature = "gateway-metrics-prometheus")]
    let metrics_timer_start = Instant::now();
    let content_length = parts
        .headers
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|raw| raw.parse::<usize>().ok())
        .unwrap_or(0);
    let buffered_body = buffer_streaming_multipart_body(body, content_length).await?;
    let content_type = parts
        .headers
        .get("content-type")
        .and_then(|value| value.to_str().ok());
    let model = multipart_request_model(&path_and_query, content_type, &buffered_body)?;
    let charge_tokens = estimate_tokens_from_length(content_length);

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

    #[cfg(feature = "gateway-costing")]
    let mut charge_cost_usd_micros: Option<u64> = None;
    #[cfg(not(feature = "gateway-costing"))]
    let charge_cost_usd_micros: Option<u64> = None;

    let now_epoch_seconds = now_epoch_seconds();
    let minute = now_epoch_seconds / 60;
    #[cfg(feature = "gateway-store-redis")]
    let rate_limit_route = normalize_rate_limit_route(&path_and_query);

    let ResolvedStreamingMultipartGatewayContext {
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
        local_token_budget_reserved,
    } = {
        state.record_request();

        let gateway_preamble =
            super::proxy_gateway_context::resolve_openai_compat_proxy_gateway_preamble(
                &state, &parts,
            )
            .await?;
        let strip_authorization = gateway_preamble.strip_authorization;
        let key = gateway_preamble.key;

        let virtual_key_id = key.as_ref().map(|key| key.id.clone());
        let limits = key.as_ref().map(|key| key.limits.clone());

        let tenant_scope = key
            .as_ref()
            .and_then(|key| key.tenant_id.as_deref())
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(|id| format!("tenant:{id}"));
        let tenant_budget_scope = key.as_ref().and_then(|key| {
            tenant_scope.as_ref().and_then(|scope| {
                key.tenant_budget
                    .as_ref()
                    .map(|budget| (scope.clone(), budget.clone()))
            })
        });
        let tenant_limits_scope = key.as_ref().and_then(|key| {
            tenant_scope.as_ref().and_then(|scope| {
                key.tenant_limits
                    .as_ref()
                    .map(|limits| (scope.clone(), limits.clone()))
            })
        });

        let project_scope = key
            .as_ref()
            .and_then(|key| key.project_id.as_deref())
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(|id| format!("project:{id}"));
        let project_budget_scope = key.as_ref().and_then(|key| {
            project_scope.as_ref().and_then(|scope| {
                key.project_budget
                    .as_ref()
                    .map(|budget| (scope.clone(), budget.clone()))
            })
        });
        let project_limits_scope = key.as_ref().and_then(|key| {
            project_scope.as_ref().and_then(|scope| {
                key.project_limits
                    .as_ref()
                    .map(|limits| (scope.clone(), limits.clone()))
            })
        });

        let user_scope = key
            .as_ref()
            .and_then(|key| key.user_id.as_deref())
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(|id| format!("user:{id}"));
        let user_budget_scope = key.as_ref().and_then(|key| {
            user_scope.as_ref().and_then(|scope| {
                key.user_budget
                    .as_ref()
                    .map(|budget| (scope.clone(), budget.clone()))
            })
        });
        let user_limits_scope = key.as_ref().and_then(|key| {
            user_scope.as_ref().and_then(|scope| {
                key.user_limits
                    .as_ref()
                    .map(|limits| (scope.clone(), limits.clone()))
            })
        });

        #[cfg(feature = "gateway-costing")]
        {
            let has_cost_budget = key
                .as_ref()
                .is_some_and(|key| key.budget.total_usd_micros.is_some())
                || tenant_budget_scope
                    .as_ref()
                    .is_some_and(|(_, budget)| budget.total_usd_micros.is_some())
                || project_budget_scope
                    .as_ref()
                    .is_some_and(|(_, budget)| budget.total_usd_micros.is_some())
                || user_budget_scope
                    .as_ref()
                    .is_some_and(|(_, budget)| budget.total_usd_micros.is_some());

            if has_cost_budget {
                match cost_budget_endpoint_policy(&parts.method, &path_and_query) {
                    CostBudgetEndpointPolicy::Free => {
                        charge_cost_usd_micros = Some(0);
                    }
                    CostBudgetEndpointPolicy::TokenBased => {
                        charge_cost_usd_micros = Some(0);
                    }
                    CostBudgetEndpointPolicy::Unsupported => {
                        let path = path_and_query
                            .split_once('?')
                            .map(|(path, _)| path)
                            .unwrap_or(path_and_query.as_str())
                            .trim_end_matches('/');
                        return Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("cost_budget_unsupported_endpoint"),
                            format!(
                                "cost budgets are token-based and do not support {path} (disable total_usd_micros or use token budgets)"
                            ),
                        ));
                    }
                }
            }
        }

        if let Some(key) = key.as_ref() {
            let guardrails = state.guardrails_for_model(model.as_deref(), key);

            if let Some(model_id) = model.as_deref()
                && let Some(reason) = guardrails.check_model(model_id)
            {
                state.record_guardrail_blocked();
                let err = openai_error(
                    StatusCode::FORBIDDEN,
                    "policy_error",
                    Some("guardrail_rejected"),
                    reason,
                );
                #[cfg(feature = "gateway-metrics-prometheus")]
                if let Some(metrics) = state.proxy.metrics.as_ref() {
                    let duration = metrics_timer_start.elapsed();
                    let status = err.0.as_u16();
                    let mut metrics = metrics.lock().await;
                    metrics.record_proxy_request(Some(&key.id), model.as_deref(), &metrics_path);
                    metrics.record_proxy_guardrail_blocked(
                        Some(&key.id),
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
                return Err(err);
            }

            if let Some(limit) = guardrails.max_input_tokens
                && charge_tokens > limit
            {
                state.record_guardrail_blocked();
                let err = openai_error(
                    StatusCode::FORBIDDEN,
                    "policy_error",
                    Some("guardrail_rejected"),
                    format!("input_tokens>{limit}"),
                );
                #[cfg(feature = "gateway-metrics-prometheus")]
                if let Some(metrics) = state.proxy.metrics.as_ref() {
                    let duration = metrics_timer_start.elapsed();
                    let status = err.0.as_u16();
                    let mut metrics = metrics.lock().await;
                    metrics.record_proxy_request(Some(&key.id), model.as_deref(), &metrics_path);
                    metrics.record_proxy_guardrail_blocked(
                        Some(&key.id),
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
                return Err(err);
            }

            if guardrails.validate_schema
                && let Some(reason) = validate_openai_multipart_request_schema(
                    &path_and_query,
                    content_type,
                    &buffered_body,
                )
            {
                state.record_guardrail_blocked();
                let err = openai_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request_error",
                    Some("invalid_request"),
                    reason,
                );
                #[cfg(feature = "gateway-metrics-prometheus")]
                if let Some(metrics) = state.proxy.metrics.as_ref() {
                    let duration = metrics_timer_start.elapsed();
                    let status = err.0.as_u16();
                    let mut metrics = metrics.lock().await;
                    metrics.record_proxy_request(Some(&key.id), model.as_deref(), &metrics_path);
                    metrics.record_proxy_guardrail_blocked(
                        Some(&key.id),
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
                return Err(err);
            }

            if guardrails.has_text_filters()
                && let Ok(text) = std::str::from_utf8(&buffered_body)
                && let Some(reason) = guardrails.check_text(text)
            {
                state.record_guardrail_blocked();
                let err = openai_error(
                    StatusCode::FORBIDDEN,
                    "policy_error",
                    Some("guardrail_rejected"),
                    reason,
                );
                #[cfg(feature = "gateway-metrics-prometheus")]
                if let Some(metrics) = state.proxy.metrics.as_ref() {
                    let duration = metrics_timer_start.elapsed();
                    let status = err.0.as_u16();
                    let mut metrics = metrics.lock().await;
                    metrics.record_proxy_request(Some(&key.id), model.as_deref(), &metrics_path);
                    metrics.record_proxy_guardrail_blocked(
                        Some(&key.id),
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
                return Err(err);
            }
        }

        if !use_redis_budget {
            let mut rate_limit_scopes = Vec::new();
            if let (Some(key), Some(limits)) = (key.as_ref(), limits.as_ref()) {
                rate_limit_scopes.push((key.id.as_str(), limits));
            }
            if let Some((scope, limits)) = tenant_limits_scope.as_ref() {
                rate_limit_scopes.push((scope.as_str(), limits));
            }
            if let Some((scope, limits)) = project_limits_scope.as_ref() {
                rate_limit_scopes.push((scope.as_str(), limits));
            }
            if let Some((scope, limits)) = user_limits_scope.as_ref() {
                rate_limit_scopes.push((scope.as_str(), limits));
            }
            if let Err(err) = state.check_and_consume_rate_limits(
                rate_limit_scopes.into_iter(),
                charge_tokens,
                minute,
            ) {
                state.record_rate_limited();
                let mapped = map_openai_gateway_error(err);
                #[cfg(feature = "gateway-metrics-prometheus")]
                if let Some(metrics) = state.proxy.metrics.as_ref() {
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
                return Err(mapped);
            }
        }

        let budget = key.as_ref().map(|key| key.budget.clone());
        let mut local_token_budget_reserved = false;
        if !use_persistent_budget {
            let budget_scopes = collect_budget_scopes(
                virtual_key_id.as_deref(),
                budget.as_ref(),
                &tenant_budget_scope,
                &project_budget_scope,
                &user_budget_scope,
            );
            if !budget_scopes.is_empty()
                && let Err(err) =
                    state.reserve_budget_tokens(budget_scopes.clone(), u64::from(charge_tokens))
            {
                state.record_budget_exceeded();
                let mapped = map_openai_gateway_error(err);
                #[cfg(feature = "gateway-metrics-prometheus")]
                if let Some(metrics) = state.proxy.metrics.as_ref() {
                    let duration = metrics_timer_start.elapsed();
                    let status = mapped.0.as_u16();
                    let mut metrics = metrics.lock().await;
                    metrics.record_proxy_request(
                        virtual_key_id.as_deref(),
                        model.as_deref(),
                        &metrics_path,
                    );
                    metrics.record_proxy_budget_exceeded(
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
                return Err(mapped);
            }
            local_token_budget_reserved = !budget_scopes.is_empty();
        }

        let backends = state
            .select_backends_for_model_seeded(
                model.as_deref().unwrap_or_default(),
                key.as_ref(),
                Some(&request_id),
            )
            .map_err(map_openai_gateway_error)?;

        ResolvedStreamingMultipartGatewayContext {
            virtual_key_id,
            limits,
            budget,
            tenant_budget_scope,
            project_budget_scope,
            user_budget_scope,
            tenant_limits_scope,
            project_limits_scope,
            user_limits_scope,
            backend_candidates: backends,
            strip_authorization,
            local_token_budget_reserved,
        }
    };

    #[cfg(not(feature = "gateway-store-redis"))]
    let _ = (
        &limits,
        &tenant_limits_scope,
        &project_limits_scope,
        &user_limits_scope,
    );

    #[cfg(feature = "gateway-store-redis")]
    if use_redis_budget
        && let Some(store) = state.stores.redis.as_ref()
        && let Err(err) = store
            .check_and_consume_rate_limits_many(
                redis_rate_limit_scopes(
                    virtual_key_id.as_deref(),
                    limits.as_ref(),
                    tenant_limits_scope.as_ref(),
                    project_limits_scope.as_ref(),
                    user_limits_scope.as_ref(),
                ),
                &rate_limit_route,
                charge_tokens,
                now_epoch_seconds,
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
        return Err(mapped);
    }

    #[cfg(feature = "gateway-metrics-prometheus")]
    if let Some(metrics) = state.proxy.metrics.as_ref() {
        metrics.lock().await.record_proxy_request(
            virtual_key_id.as_deref(),
            model.as_deref(),
            &metrics_path,
        );
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
        path_and_query: &path_and_query,
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
                            None,
                            &metrics_path,
                        );
                    }
                    metrics.record_proxy_response_status_by_path(&metrics_path, status);
                    metrics.observe_proxy_request_duration(&metrics_path, duration);
                }

                return Err(err);
            }
        };
    #[cfg(not(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    )))]
    let (_token_budget_reserved, token_budget_reservation_ids): (bool, Vec<String>) =
        (false, Vec::new());

    #[cfg(all(
        feature = "gateway-costing",
        any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        ),
    ))]
    let (cost_budget_reserved, cost_budget_reservation_ids) =
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
                            None,
                            &metrics_path,
                        );
                    }
                    metrics.record_proxy_response_status_by_path(&metrics_path, status);
                    metrics.observe_proxy_request_duration(&metrics_path, duration);
                }

                return Err(err);
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
    let (cost_budget_reserved, cost_budget_reservation_ids): (bool, Vec<String>) =
        (false, Vec::new());

    #[cfg(all(
        not(feature = "gateway-costing"),
        any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        ),
    ))]
    let _ = (&cost_budget_reservation_ids, cost_budget_reserved);

    #[cfg(not(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    )))]
    let _ = (
        &token_budget_reservation_ids,
        &cost_budget_reservation_ids,
        cost_budget_reserved,
    );

    let Some(backend_name) = backend_candidates.first().cloned() else {
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
            rollback_local_streaming_multipart_budgets(
                &state,
                virtual_key_id.as_deref(),
                budget.as_ref(),
                &tenant_budget_scope,
                &project_budget_scope,
                &user_budget_scope,
                charge_tokens,
                #[cfg(feature = "gateway-costing")]
                charge_cost_usd_micros,
            );
        }
        let err = openai_error(
            StatusCode::BAD_GATEWAY,
            "api_error",
            Some("backend_error"),
            "no backends available",
        );
        #[cfg(feature = "gateway-metrics-prometheus")]
        if let Some(metrics) = state.proxy.metrics.as_ref() {
            let duration = metrics_timer_start.elapsed();
            let status = err.0.as_u16();
            let mut metrics = metrics.lock().await;
            metrics.record_proxy_response_status_by_path(&metrics_path, status);
            metrics.observe_proxy_request_duration(&metrics_path, duration);
        }
        return Err(err);
    };

    #[cfg(feature = "gateway-translation")]
    if state
        .backends
        .translation_backends
        .contains_key(&backend_name)
    {
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
            rollback_local_streaming_multipart_budgets(
                &state,
                virtual_key_id.as_deref(),
                budget.as_ref(),
                &tenant_budget_scope,
                &project_budget_scope,
                &user_budget_scope,
                charge_tokens,
                #[cfg(feature = "gateway-costing")]
                charge_cost_usd_micros,
            );
        }
        let err = openai_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            Some("request_too_large"),
            "large multipart requests require a proxy backend (not a translation backend)",
        );
        #[cfg(feature = "gateway-metrics-prometheus")]
        if let Some(metrics) = state.proxy.metrics.as_ref() {
            let duration = metrics_timer_start.elapsed();
            let status = err.0.as_u16();
            let mut metrics = metrics.lock().await;
            metrics.record_proxy_response_status_by_path(&metrics_path, status);
            metrics.observe_proxy_request_duration(&metrics_path, duration);
        }
        return Err(err);
    }

    let backend = match state.backends.proxy_backends.get(&backend_name) {
        Some(backend) => backend.clone(),
        None => {
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
                rollback_local_streaming_multipart_budgets(
                    &state,
                    virtual_key_id.as_deref(),
                    budget.as_ref(),
                    &tenant_budget_scope,
                    &project_budget_scope,
                    &user_budget_scope,
                    charge_tokens,
                    #[cfg(feature = "gateway-costing")]
                    charge_cost_usd_micros,
                );
            }
            let err = openai_error(
                StatusCode::BAD_GATEWAY,
                "api_error",
                Some("backend_not_found"),
                format!("backend not found: {backend_name}"),
            );
            #[cfg(feature = "gateway-metrics-prometheus")]
            if let Some(metrics) = state.proxy.metrics.as_ref() {
                let duration = metrics_timer_start.elapsed();
                let status = err.0.as_u16();
                let mut metrics = metrics.lock().await;
                metrics.record_proxy_response_status_by_path(&metrics_path, status);
                metrics.record_proxy_response_status_by_backend(&backend_name, status);
                metrics.observe_proxy_request_duration(&metrics_path, duration);
            }
            return Err(err);
        }
    };

    let mut proxy_permits = match try_acquire_proxy_permits(&state, &backend_name)? {
        ProxyPermitOutcome::Acquired(permits) => permits,
        ProxyPermitOutcome::BackendRateLimited(err) => {
            #[cfg(feature = "gateway-metrics-prometheus")]
            if let Some(metrics) = state.proxy.metrics.as_ref() {
                let duration = metrics_timer_start.elapsed();
                let status = err.0.as_u16();
                let mut metrics = metrics.lock().await;
                if err.0 == StatusCode::TOO_MANY_REQUESTS {
                    metrics.record_proxy_rate_limited(
                        virtual_key_id.as_deref(),
                        None,
                        &metrics_path,
                    );
                }
                metrics.record_proxy_response_status_by_path(&metrics_path, status);
                metrics.record_proxy_response_status_by_backend(&backend_name, status);
                metrics.observe_proxy_request_duration(&metrics_path, duration);
            }
            return Err(err);
        }
    };

    let mut outgoing_headers = parts.headers.clone();
    sanitize_proxy_headers(&mut outgoing_headers, strip_authorization);
    apply_backend_headers(&mut outgoing_headers, backend.headers());
    insert_request_id(&mut outgoing_headers, &request_id);

    let outgoing_body = reqwest::Body::from(buffered_body.clone());

    #[cfg(feature = "gateway-metrics-prometheus")]
    let backend_timer_start = Instant::now();
    #[cfg(feature = "gateway-metrics-prometheus")]
    if let Some(metrics) = state.proxy.metrics.as_ref() {
        let mut metrics = metrics.lock().await;
        metrics.record_proxy_backend_attempt(&backend_name);
        metrics.record_proxy_backend_in_flight_inc(&backend_name);
    }

    let upstream_response = match backend
        .request_stream(
            parts.method.clone(),
            &path_and_query,
            outgoing_headers,
            Some(outgoing_body),
        )
        .await
    {
        Ok(response) => response,
        Err(err) => {
            let mapped = map_openai_gateway_error(err);
            #[cfg(feature = "gateway-metrics-prometheus")]
            if let Some(metrics) = state.proxy.metrics.as_ref() {
                let duration = metrics_timer_start.elapsed();
                let status = mapped.0.as_u16();
                let mut metrics = metrics.lock().await;
                metrics.record_proxy_backend_in_flight_dec(&backend_name);
                metrics.observe_proxy_backend_request_duration(
                    &backend_name,
                    backend_timer_start.elapsed(),
                );
                metrics.record_proxy_backend_failure(&backend_name);
                metrics.record_proxy_response_status_by_path(&metrics_path, status);
                metrics.record_proxy_response_status_by_backend(&backend_name, status);
                metrics.observe_proxy_request_duration(&metrics_path, duration);
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
                rollback_local_streaming_multipart_budgets(
                    &state,
                    virtual_key_id.as_deref(),
                    budget.as_ref(),
                    &tenant_budget_scope,
                    &project_budget_scope,
                    &user_budget_scope,
                    charge_tokens,
                    #[cfg(feature = "gateway-costing")]
                    charge_cost_usd_micros,
                );
            }
            return Err(mapped);
        }
    };

    let status = upstream_response.status();
    let spend_tokens = status.is_success();
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

    #[cfg(feature = "gateway-metrics-prometheus")]
    if let Some(metrics) = state.proxy.metrics.as_ref() {
        let duration = metrics_timer_start.elapsed();
        let status_code = status.as_u16();
        let mut metrics = metrics.lock().await;
        metrics.record_proxy_backend_in_flight_dec(&backend_name);
        metrics
            .observe_proxy_backend_request_duration(&backend_name, backend_timer_start.elapsed());
        if spend_tokens {
            metrics.record_proxy_backend_success(&backend_name);
        } else {
            metrics.record_proxy_backend_failure(&backend_name);
        }
        metrics.record_proxy_response_status_by_path(&metrics_path, status_code);
        metrics.record_proxy_response_status_by_backend(&backend_name, status_code);
        metrics.observe_proxy_request_duration(&metrics_path, duration);
    }

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    if !token_budget_reservation_ids.is_empty() {
        settle_proxy_token_budget_reservations(
            &state,
            &token_budget_reservation_ids,
            spend_tokens,
            spent_tokens,
        )
        .await;
    }

    if token_budget_reservation_ids.is_empty()
        && !use_persistent_budget
        && local_token_budget_reserved
    {
        let budget_scopes = collect_budget_scopes(
            virtual_key_id.as_deref(),
            budget.as_ref(),
            &tenant_budget_scope,
            &project_budget_scope,
            &user_budget_scope,
        );
        if spend_tokens {
            state.settle_budget_tokens(
                budget_scopes.clone(),
                u64::from(charge_tokens),
                spent_tokens,
            );
        } else {
            state.rollback_budget_tokens(budget_scopes.clone(), u64::from(charge_tokens));
        }

        #[cfg(feature = "gateway-costing")]
        if spend_tokens {
            state.settle_budget_cost(
                budget_scopes,
                charge_cost_usd_micros.unwrap_or_default(),
                spent_cost_usd_micros,
            );
        } else {
            state.rollback_budget_cost(budget_scopes, charge_cost_usd_micros.unwrap_or_default());
        }
    }

    #[cfg(not(feature = "gateway-costing"))]
    let _ = &spent_cost_usd_micros;

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
            &state,
            &cost_budget_reservation_ids,
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
    if !cost_budget_reserved
        && use_persistent_budget
        && spend_tokens
        && let (Some(virtual_key_id), Some(spent_cost_usd_micros)) =
            (virtual_key_id.as_deref(), spent_cost_usd_micros)
    {
        #[cfg(feature = "gateway-store-sqlite")]
        if let Some(store) = state.stores.sqlite.as_ref() {
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

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    {
        let payload = serde_json::json!({
            "request_id": &request_id,
            "virtual_key_id": virtual_key_id.as_deref(),
            "backend": &backend_name,
            "attempted_backends": [&backend_name],
            "method": parts.method.as_str(),
            "path": &path_and_query,
            "model": model.as_deref(),
            "status": status.as_u16(),
            "charge_tokens": charge_tokens,
            "spent_tokens": spent_tokens,
            "charge_cost_usd_micros": charge_cost_usd_micros,
            "spent_cost_usd_micros": spent_cost_usd_micros,
            "body_len": content_length,
        });
        append_audit_log(&state, "proxy", payload)
            .await
            .map_err(openai_storage_error_response)?;
    }

    emit_json_log(
        &state,
        "proxy.response",
        serde_json::json!({
            "request_id": &request_id,
            "backend": &backend_name,
            "status": status.as_u16(),
        }),
    );

    #[cfg(feature = "gateway-otel")]
    {
        tracing::Span::current().record("cache", tracing::field::display("miss"));
        tracing::Span::current().record("backend", tracing::field::display(&backend_name));
        tracing::Span::current().record("status", tracing::field::display(status.as_u16()));
    }

    Ok(proxy_response(
        ProxyResponseContext {
            state: &state,
            backend: &backend_name,
            request_id: &request_id,
            #[cfg(feature = "gateway-metrics-prometheus")]
            metrics_path: metrics_path.as_str(),
            cache_key: None,
            #[cfg(feature = "gateway-proxy-cache")]
            cache_metadata: None,
        },
        upstream_response,
        proxy_permits.take(),
    )
    .await)
}
// end inline: streaming_multipart/handler.rs

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::{BTreeMap, HashMap};

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use httpmock::Method::POST;
    use httpmock::MockServer;

    use crate::gateway::{
        BackendConfig, Gateway, GatewayConfig, GuardrailsConfig, LimitsConfig, ProxyBackend,
        RouteBackend, RouteRule, RouterConfig, VirtualKeyConfig,
    };

    fn backend_config(name: &str, base_url: String) -> BackendConfig {
        BackendConfig {
            name: name.to_string(),
            base_url,
            max_in_flight: None,
            timeout_seconds: None,
            headers: BTreeMap::new(),
            query_params: BTreeMap::new(),
            provider: None,
            provider_config: None,
            model_map: BTreeMap::new(),
        }
    }

    fn multipart_audio_body(boundary: &str) -> Vec<u8> {
        format!(
            "--{boundary}\r\n\
Content-Disposition: form-data; name=\"model\"\r\n\r\n\
whisper-1\r\n\
--{boundary}\r\n\
Content-Disposition: form-data; name=\"file\"; filename=\"audio.wav\"\r\n\
Content-Type: audio/wav\r\n\r\n\
{}\r\n\
--{boundary}--\r\n",
            "x".repeat(64)
        )
        .into_bytes()
    }

    fn build_key(
        id: &str,
        token: &str,
        key_rpm: u32,
        tenant_limits: LimitsConfig,
    ) -> VirtualKeyConfig {
        let mut key = VirtualKeyConfig::new(id, token);
        key.tenant_id = Some("tenant-1".to_string());
        key.limits = LimitsConfig {
            rpm: Some(key_rpm),
            tpm: None,
        };
        key.tenant_limits = Some(tenant_limits);
        key
    }

    fn multipart_audio_body_without_model(boundary: &str) -> Vec<u8> {
        format!(
            "--{boundary}\r\n\
Content-Disposition: form-data; name=\"file\"; filename=\"audio.wav\"\r\n\
Content-Type: audio/wav\r\n\r\n\
{}\r\n\
--{boundary}--\r\n",
            "x".repeat(64)
        )
        .into_bytes()
    }

    fn streaming_request(token: &str, boundary: &str, body: Vec<u8>) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/v1/audio/transcriptions")
            .header("authorization", format!("Bearer {token}"))
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .header("content-length", body.len().to_string())
            .body(Body::from(body))
            .expect("multipart request")
    }

    fn passthrough_request(boundary: &str, body: Vec<u8>) -> Request<Body> {
        let mut request = Request::builder()
            .method("POST")
            .uri("/v1/audio/transcriptions")
            .header("authorization", "Bearer upstream-token")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .header("content-length", body.len().to_string())
            .body(Body::from(body))
            .expect("multipart request");
        request
            .extensions_mut()
            .insert(InternalUpstreamAuthPassthrough);
        request
    }

    #[tokio::test]
    async fn streaming_multipart_keeps_key_scope_unconsumed_when_tenant_scope_rejects() {
        if ditto_core::utils::test_support::should_skip_httpmock() {
            return;
        }

        let upstream = MockServer::start();
        let upstream_mock = upstream.mock(|when, then| {
            when.method(POST).path("/v1/audio/transcriptions");
            then.status(200)
                .header("content-type", "application/json")
                .body(r#"{"text":"ok"}"#);
        });

        let tenant_limits = LimitsConfig {
            rpm: Some(1),
            tpm: None,
        };
        let key_a = build_key("key-a", "vk-a", 1, tenant_limits.clone());
        let key_b = build_key("key-b", "vk-b", 4, tenant_limits);

        let config = GatewayConfig {
            backends: vec![backend_config("primary", upstream.base_url())],
            virtual_keys: vec![key_a.clone(), key_b],
            router: RouterConfig {
                default_backends: vec![RouteBackend {
                    backend: "primary".to_string(),
                    weight: 1.0,
                }],
                rules: Vec::new(),
            },
            a2a_agents: Vec::new(),
            mcp_servers: Vec::new(),
            observability: Default::default(),
        };

        let mut proxy_backends = HashMap::new();
        proxy_backends.insert(
            "primary".to_string(),
            ProxyBackend::new(upstream.base_url()).expect("proxy backend"),
        );

        let state = GatewayHttpState::new(Gateway::new(config))
            .with_proxy_backends(proxy_backends)
            .with_proxy_max_body_bytes(16);
        let minute = now_epoch_seconds() / 60;
        let boundary = "ditto-boundary";

        let first_request = streaming_request("vk-b", boundary, multipart_audio_body(boundary));
        let (first_parts, first_body) = first_request.into_parts();
        let first_response = handle_openai_compat_proxy_streaming_multipart(
            state.clone(),
            first_parts,
            first_body,
            "req-stream-prime".to_string(),
            "/v1/audio/transcriptions".to_string(),
        )
        .await
        .expect("first multipart request should pass");
        assert_eq!(first_response.status(), StatusCode::OK);

        let second_request = streaming_request("vk-a", boundary, multipart_audio_body(boundary));
        let (second_parts, second_body) = second_request.into_parts();
        let err = handle_openai_compat_proxy_streaming_multipart(
            state.clone(),
            second_parts,
            second_body,
            "req-stream-reject".to_string(),
            "/v1/audio/transcriptions".to_string(),
        )
        .await
        .expect_err("shared tenant limit should reject second multipart request");
        assert_eq!(err.0, StatusCode::TOO_MANY_REQUESTS);

        state
            .check_and_consume_rate_limits([("key-a", &key_a.limits)], 1, minute)
            .expect("key scope must remain available after multipart rejection");

        upstream_mock.assert_calls(1);
    }

    #[tokio::test]
    async fn streaming_multipart_uses_model_routing_and_passthrough_auth() {
        if ditto_core::utils::test_support::should_skip_httpmock() {
            return;
        }

        let primary = MockServer::start();
        let whisper = MockServer::start();
        let primary_mock = primary.mock(|when, then| {
            when.method(POST).path("/v1/audio/transcriptions");
            then.status(500);
        });
        let whisper_mock = whisper.mock(|when, then| {
            when.method(POST)
                .path("/v1/audio/transcriptions")
                .header("authorization", "Bearer upstream-token");
            then.status(200)
                .header("content-type", "application/json")
                .body(r#"{"text":"ok"}"#);
        });

        let config = GatewayConfig {
            backends: vec![
                backend_config("primary", primary.base_url()),
                backend_config("whisper", whisper.base_url()),
            ],
            virtual_keys: Vec::new(),
            router: RouterConfig {
                default_backends: vec![RouteBackend {
                    backend: "primary".to_string(),
                    weight: 1.0,
                }],
                rules: vec![RouteRule {
                    model_prefix: "whisper-1".to_string(),
                    exact: true,
                    backend: "whisper".to_string(),
                    backends: Vec::new(),
                    guardrails: None,
                }],
            },
            a2a_agents: Vec::new(),
            mcp_servers: Vec::new(),
            observability: Default::default(),
        };

        let mut proxy_backends = HashMap::new();
        proxy_backends.insert(
            "primary".to_string(),
            ProxyBackend::new(primary.base_url()).expect("primary backend"),
        );
        proxy_backends.insert(
            "whisper".to_string(),
            ProxyBackend::new(whisper.base_url()).expect("whisper backend"),
        );

        let state = GatewayHttpState::new(Gateway::new(config))
            .with_proxy_backends(proxy_backends)
            .with_proxy_max_body_bytes(16);
        let boundary = "ditto-boundary";
        let request = passthrough_request(boundary, multipart_audio_body(boundary));
        let (parts, body) = request.into_parts();
        let response = handle_openai_compat_proxy_streaming_multipart(
            state,
            parts,
            body,
            "req-stream-route".to_string(),
            "/v1/audio/transcriptions".to_string(),
        )
        .await
        .expect("multipart passthrough request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        primary_mock.assert_calls(0);
        whisper_mock.assert_calls(1);
    }

    #[tokio::test]
    async fn streaming_multipart_invalid_schema_does_not_consume_rate_limit() {
        let mut key = VirtualKeyConfig::new("key-1", "vk-1");
        key.limits = LimitsConfig {
            rpm: Some(1),
            tpm: None,
        };
        key.guardrails = GuardrailsConfig {
            validate_schema: true,
            ..GuardrailsConfig::default()
        };

        let config = GatewayConfig {
            backends: vec![backend_config("primary", "http://127.0.0.1:9".to_string())],
            virtual_keys: vec![key.clone()],
            router: RouterConfig {
                default_backends: vec![RouteBackend {
                    backend: "primary".to_string(),
                    weight: 1.0,
                }],
                rules: Vec::new(),
            },
            a2a_agents: Vec::new(),
            mcp_servers: Vec::new(),
            observability: Default::default(),
        };

        let state = GatewayHttpState::new(Gateway::new(config)).with_proxy_max_body_bytes(16);
        let boundary = "ditto-boundary";
        let request = streaming_request(
            "vk-1",
            boundary,
            multipart_audio_body_without_model(boundary),
        );
        let (parts, body) = request.into_parts();
        let err = handle_openai_compat_proxy_streaming_multipart(
            state.clone(),
            parts,
            body,
            "req-stream-invalid".to_string(),
            "/v1/audio/transcriptions".to_string(),
        )
        .await
        .expect_err("multipart request missing model should fail validation");

        assert_eq!(err.0, StatusCode::BAD_REQUEST);

        let minute = now_epoch_seconds() / 60;
        state
            .check_and_consume_rate_limits([("key-1", &key.limits)], 1, minute)
            .expect("invalid multipart request must not consume key rate limit");
    }
}
