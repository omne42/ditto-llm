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
    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    let model: Option<String> = None;
    let content_length = parts
        .headers
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|raw| raw.parse::<usize>().ok())
        .unwrap_or(0);
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
    } = {
        state.record_request();

        let strip_authorization = true;
        let token = extract_virtual_key(&parts.headers).ok_or_else(|| {
            openai_error(
                StatusCode::UNAUTHORIZED,
                "authentication_error",
                Some("invalid_api_key"),
                "missing virtual key",
            )
        })?;
        let key = state.virtual_key_by_token(&token).ok_or_else(|| {
            openai_error(
                StatusCode::UNAUTHORIZED,
                "authentication_error",
                Some("invalid_api_key"),
                "unauthorized virtual key",
            )
        })?;
        if !key.enabled {
            return Err(openai_error(
                StatusCode::UNAUTHORIZED,
                "authentication_error",
                Some("invalid_api_key"),
                "virtual key disabled",
            ));
        }
        let key = Some(key);

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

        if !use_redis_budget {
            if let (Some(key), Some(limits)) = (key.as_ref(), limits.as_ref())
                && let Err(err) =
                    state.check_and_consume_rate_limit(&key.id, limits, charge_tokens, minute)
            {
                state.record_rate_limited();
                let mapped = map_openai_gateway_error(err);
                #[cfg(feature = "gateway-metrics-prometheus")]
                if let Some(metrics) = state.proxy.metrics.as_ref() {
                    let duration = metrics_timer_start.elapsed();
                    let status = mapped.0.as_u16();
                    let mut metrics = metrics.lock().await;
                    metrics.record_proxy_request(Some(&key.id), None, &metrics_path);
                    metrics.record_proxy_rate_limited(Some(&key.id), None, &metrics_path);
                    metrics.record_proxy_response_status_by_path(&metrics_path, status);
                    metrics.observe_proxy_request_duration(&metrics_path, duration);
                }
                return Err(mapped);
            }
            if let Some((scope, limits)) = tenant_limits_scope.as_ref()
                && let Err(err) =
                    state.check_and_consume_rate_limit(scope, limits, charge_tokens, minute)
            {
                state.record_rate_limited();
                let mapped = map_openai_gateway_error(err);
                #[cfg(feature = "gateway-metrics-prometheus")]
                if let Some(metrics) = state.proxy.metrics.as_ref() {
                    let duration = metrics_timer_start.elapsed();
                    let status = mapped.0.as_u16();
                    let mut metrics = metrics.lock().await;
                    metrics.record_proxy_request(virtual_key_id.as_deref(), None, &metrics_path);
                    metrics.record_proxy_rate_limited(
                        virtual_key_id.as_deref(),
                        None,
                        &metrics_path,
                    );
                    metrics.record_proxy_response_status_by_path(&metrics_path, status);
                    metrics.observe_proxy_request_duration(&metrics_path, duration);
                }
                return Err(mapped);
            }
            if let Some((scope, limits)) = project_limits_scope.as_ref()
                && let Err(err) =
                    state.check_and_consume_rate_limit(scope, limits, charge_tokens, minute)
            {
                state.record_rate_limited();
                let mapped = map_openai_gateway_error(err);
                #[cfg(feature = "gateway-metrics-prometheus")]
                if let Some(metrics) = state.proxy.metrics.as_ref() {
                    let duration = metrics_timer_start.elapsed();
                    let status = mapped.0.as_u16();
                    let mut metrics = metrics.lock().await;
                    metrics.record_proxy_request(virtual_key_id.as_deref(), None, &metrics_path);
                    metrics.record_proxy_rate_limited(
                        virtual_key_id.as_deref(),
                        None,
                        &metrics_path,
                    );
                    metrics.record_proxy_response_status_by_path(&metrics_path, status);
                    metrics.observe_proxy_request_duration(&metrics_path, duration);
                }
                return Err(mapped);
            }
            if let Some((scope, limits)) = user_limits_scope.as_ref()
                && let Err(err) =
                    state.check_and_consume_rate_limit(scope, limits, charge_tokens, minute)
            {
                state.record_rate_limited();
                let mapped = map_openai_gateway_error(err);
                #[cfg(feature = "gateway-metrics-prometheus")]
                if let Some(metrics) = state.proxy.metrics.as_ref() {
                    let duration = metrics_timer_start.elapsed();
                    let status = mapped.0.as_u16();
                    let mut metrics = metrics.lock().await;
                    metrics.record_proxy_request(virtual_key_id.as_deref(), None, &metrics_path);
                    metrics.record_proxy_rate_limited(
                        virtual_key_id.as_deref(),
                        None,
                        &metrics_path,
                    );
                    metrics.record_proxy_response_status_by_path(&metrics_path, status);
                    metrics.observe_proxy_request_duration(&metrics_path, duration);
                }
                return Err(mapped);
            }
        }

        if !use_persistent_budget {
            if let (Some(key), Some(budget)) = (key.as_ref(), key.as_ref().map(|key| &key.budget))
                && let Err(err) =
                    state.can_spend_budget_tokens(&key.id, budget, u64::from(charge_tokens))
            {
                state.record_budget_exceeded();
                let mapped = map_openai_gateway_error(err);
                #[cfg(feature = "gateway-metrics-prometheus")]
                if let Some(metrics) = state.proxy.metrics.as_ref() {
                    let duration = metrics_timer_start.elapsed();
                    let status = mapped.0.as_u16();
                    let mut metrics = metrics.lock().await;
                    metrics.record_proxy_request(Some(&key.id), None, &metrics_path);
                    metrics.record_proxy_budget_exceeded(Some(&key.id), None, &metrics_path);
                    metrics.record_proxy_response_status_by_path(&metrics_path, status);
                    metrics.observe_proxy_request_duration(&metrics_path, duration);
                }
                return Err(mapped);
            }
            if let Some((scope, budget)) = tenant_budget_scope.as_ref()
                && let Err(err) =
                    state.can_spend_budget_tokens(scope, budget, u64::from(charge_tokens))
            {
                state.record_budget_exceeded();
                let mapped = map_openai_gateway_error(err);
                #[cfg(feature = "gateway-metrics-prometheus")]
                if let Some(metrics) = state.proxy.metrics.as_ref() {
                    let duration = metrics_timer_start.elapsed();
                    let status = mapped.0.as_u16();
                    let mut metrics = metrics.lock().await;
                    metrics.record_proxy_request(virtual_key_id.as_deref(), None, &metrics_path);
                    metrics.record_proxy_budget_exceeded(
                        virtual_key_id.as_deref(),
                        None,
                        &metrics_path,
                    );
                    metrics.record_proxy_response_status_by_path(&metrics_path, status);
                    metrics.observe_proxy_request_duration(&metrics_path, duration);
                }
                return Err(mapped);
            }
            if let Some((scope, budget)) = project_budget_scope.as_ref()
                && let Err(err) =
                    state.can_spend_budget_tokens(scope, budget, u64::from(charge_tokens))
            {
                state.record_budget_exceeded();
                let mapped = map_openai_gateway_error(err);
                #[cfg(feature = "gateway-metrics-prometheus")]
                if let Some(metrics) = state.proxy.metrics.as_ref() {
                    let duration = metrics_timer_start.elapsed();
                    let status = mapped.0.as_u16();
                    let mut metrics = metrics.lock().await;
                    metrics.record_proxy_request(virtual_key_id.as_deref(), None, &metrics_path);
                    metrics.record_proxy_budget_exceeded(
                        virtual_key_id.as_deref(),
                        None,
                        &metrics_path,
                    );
                    metrics.record_proxy_response_status_by_path(&metrics_path, status);
                    metrics.observe_proxy_request_duration(&metrics_path, duration);
                }
                return Err(mapped);
            }
            if let Some((scope, budget)) = user_budget_scope.as_ref()
                && let Err(err) =
                    state.can_spend_budget_tokens(scope, budget, u64::from(charge_tokens))
            {
                state.record_budget_exceeded();
                let mapped = map_openai_gateway_error(err);
                #[cfg(feature = "gateway-metrics-prometheus")]
                if let Some(metrics) = state.proxy.metrics.as_ref() {
                    let duration = metrics_timer_start.elapsed();
                    let status = mapped.0.as_u16();
                    let mut metrics = metrics.lock().await;
                    metrics.record_proxy_request(virtual_key_id.as_deref(), None, &metrics_path);
                    metrics.record_proxy_budget_exceeded(
                        virtual_key_id.as_deref(),
                        None,
                        &metrics_path,
                    );
                    metrics.record_proxy_response_status_by_path(&metrics_path, status);
                    metrics.observe_proxy_request_duration(&metrics_path, duration);
                }
                return Err(mapped);
            }
        }

        let budget = key.as_ref().map(|key| key.budget.clone());
        let backends = state
            .select_backends_for_model_seeded("", key.as_ref(), Some(&request_id))
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
    if use_redis_budget && let Some(store) = state.stores.redis.as_ref() {
        if let Some(limits) = limits.as_ref()
            && let Some(vk_id) = virtual_key_id.as_deref()
            && let Err(err) = store
                .check_and_consume_rate_limits(
                    vk_id,
                    &rate_limit_route,
                    limits,
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
                metrics.record_proxy_request(Some(vk_id), None, &metrics_path);
                metrics.record_proxy_rate_limited(Some(vk_id), None, &metrics_path);
                metrics.record_proxy_response_status_by_path(&metrics_path, status);
                metrics.observe_proxy_request_duration(&metrics_path, duration);
            }
            return Err(mapped);
        }

        for scope_and_limits in [
            tenant_limits_scope.as_ref(),
            project_limits_scope.as_ref(),
            user_limits_scope.as_ref(),
        ] {
            let Some((scope, limits)) = scope_and_limits else {
                continue;
            };
            if let Err(err) = store
                .check_and_consume_rate_limits(
                    scope,
                    &rate_limit_route,
                    limits,
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
                    metrics.record_proxy_request(virtual_key_id.as_deref(), None, &metrics_path);
                    metrics.record_proxy_rate_limited(
                        virtual_key_id.as_deref(),
                        None,
                        &metrics_path,
                    );
                    metrics.record_proxy_response_status_by_path(&metrics_path, status);
                    metrics.observe_proxy_request_duration(&metrics_path, duration);
                }
                return Err(mapped);
            }
        }
    }

    #[cfg(feature = "gateway-metrics-prometheus")]
    if let Some(metrics) = state.proxy.metrics.as_ref() {
        metrics
            .lock()
            .await
            .record_proxy_request(virtual_key_id.as_deref(), None, &metrics_path);
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

    let data_stream = body
        .into_data_stream()
        .map(|result| result.map_err(|err| std::io::Error::other(err.to_string())));
    let outgoing_body = reqwest::Body::wrap_stream(data_stream);

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
        && spend_tokens
        && !use_persistent_budget
        && let (Some(virtual_key_id), Some(budget)) = (virtual_key_id.clone(), budget.clone())
    {
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
            "model": Value::Null,
            "status": status.as_u16(),
            "charge_tokens": charge_tokens,
            "spent_tokens": spent_tokens,
            "charge_cost_usd_micros": charge_cost_usd_micros,
            "spent_cost_usd_micros": spent_cost_usd_micros,
            "body_len": content_length,
        });
        append_audit_log(&state, "proxy", payload).await;
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
