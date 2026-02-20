type ResolvedGatewayContext = (
    Option<String>,
    Option<super::LimitsConfig>,
    Option<super::BudgetConfig>,
    Option<(String, super::BudgetConfig)>,
    Option<(String, super::BudgetConfig)>,
    Option<(String, super::BudgetConfig)>,
    Option<(String, super::LimitsConfig)>,
    Option<(String, super::LimitsConfig)>,
    Option<(String, super::LimitsConfig)>,
    Vec<String>,
    bool,
    Option<u64>,
);

#[allow(clippy::too_many_arguments)]
async fn resolve_openai_compat_proxy_gateway_context(
    state: &GatewayHttpState,
    parts: &axum::http::request::Parts,
    body: &Bytes,
    parsed_json: &Option<serde_json::Value>,
    request_id: &str,
    path_and_query: &str,
    model: &Option<String>,
    _service_tier: &Option<String>,
    input_tokens_estimate: u32,
    _max_output_tokens: u32,
    charge_tokens: u32,
    minute: u64,
    use_redis_budget: bool,
    use_persistent_budget: bool,
    #[cfg(feature = "gateway-metrics-prometheus")] metrics_path: String,
    #[cfg(feature = "gateway-metrics-prometheus")] metrics_timer_start: Instant,
) -> Result<ResolvedGatewayContext, (StatusCode, Json<OpenAiErrorResponse>)> {
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
    ) = {
        let mut gateway = state.gateway.lock().await;
        gateway.observability.record_request();

        let strip_authorization = !gateway.config.virtual_keys.is_empty();
        let key = if gateway.config.virtual_keys.is_empty() {
            None
        } else {
            let token = extract_virtual_key(&parts.headers).ok_or_else(|| {
                openai_error(
                    StatusCode::UNAUTHORIZED,
                    "authentication_error",
                    Some("invalid_api_key"),
                    "missing virtual key",
                )
            })?;
            let key = gateway.virtual_key_by_token(&token).cloned().ok_or_else(|| {
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
            Some(key)
        };

        if let Some(key) = key.as_ref() {
            let virtual_key_id = Some(key.id.clone());
            let limits = Some(key.limits.clone());

            let tenant_scope = key
                .tenant_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(|id| format!("tenant:{id}"));
            let tenant_budget_scope = tenant_scope.as_ref().and_then(|scope| {
                key.tenant_budget
                    .as_ref()
                    .map(|budget| (scope.clone(), budget.clone()))
            });
            let tenant_limits_scope = tenant_scope.as_ref().and_then(|scope| {
                key.tenant_limits
                    .as_ref()
                    .map(|limits| (scope.clone(), limits.clone()))
            });

            let project_scope = key
                .project_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(|id| format!("project:{id}"));
            let project_budget_scope = project_scope.as_ref().and_then(|scope| {
                key.project_budget
                    .as_ref()
                    .map(|budget| (scope.clone(), budget.clone()))
            });
            let project_limits_scope = project_scope.as_ref().and_then(|scope| {
                key.project_limits
                    .as_ref()
                    .map(|limits| (scope.clone(), limits.clone()))
            });

            let user_scope = key
                .user_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(|id| format!("user:{id}"));
            let user_budget_scope = user_scope.as_ref().and_then(|scope| {
                key.user_budget
                    .as_ref()
                    .map(|budget| (scope.clone(), budget.clone()))
            });
            let user_limits_scope = user_scope.as_ref().and_then(|scope| {
                key.user_limits
                    .as_ref()
                    .map(|limits| (scope.clone(), limits.clone()))
            });

            #[cfg(feature = "gateway-costing")]
            let (has_cost_budget, cost_budget_policy) = {
                let has_cost_budget = key.budget.total_usd_micros.is_some()
                    || tenant_budget_scope
                        .as_ref()
                        .is_some_and(|(_, budget)| budget.total_usd_micros.is_some())
                    || project_budget_scope
                        .as_ref()
                        .is_some_and(|(_, budget)| budget.total_usd_micros.is_some())
                    || user_budget_scope
                        .as_ref()
                        .is_some_and(|(_, budget)| budget.total_usd_micros.is_some());

                let cost_budget_policy = if has_cost_budget {
                    Some(cost_budget_endpoint_policy(&parts.method, path_and_query))
                } else {
                    None
                };

                (has_cost_budget, cost_budget_policy)
            };

            #[cfg(feature = "gateway-costing")]
            if has_cost_budget
                && matches!(
                    cost_budget_policy,
                    Some(CostBudgetEndpointPolicy::Unsupported)
                )
            {
                let path = path_and_query
                    .split_once('?')
                    .map(|(path, _)| path)
                    .unwrap_or(path_and_query)
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

            if !use_redis_budget {
                if let Err(err) = gateway
                    .limits
                    .check_and_consume(&key.id, &key.limits, charge_tokens, minute)
                {
                    gateway.observability.record_rate_limited();
                    let mapped = map_openai_gateway_error(err);
                    #[cfg(feature = "gateway-metrics-prometheus")]
                    if let Some(metrics) = state.prometheus_metrics.as_ref() {
                        let duration = metrics_timer_start.elapsed();
                        let status = mapped.0.as_u16();
                        drop(gateway);
                        let mut metrics = metrics.lock().await;
                        metrics.record_proxy_request(Some(&key.id), model.as_deref(), &metrics_path);
                        metrics.record_proxy_rate_limited(
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
                    return Err(mapped);
                }
                if let Some((scope, limits)) = tenant_limits_scope.as_ref() {
                    if let Err(err) =
                        gateway
                            .limits
                            .check_and_consume(scope, limits, charge_tokens, minute)
                    {
                        gateway.observability.record_rate_limited();
                        let mapped = map_openai_gateway_error(err);
                        #[cfg(feature = "gateway-metrics-prometheus")]
                        if let Some(metrics) = state.prometheus_metrics.as_ref() {
                            let duration = metrics_timer_start.elapsed();
                            let status = mapped.0.as_u16();
                            drop(gateway);
                            let mut metrics = metrics.lock().await;
                            metrics.record_proxy_request(
                                Some(&key.id),
                                model.as_deref(),
                                &metrics_path,
                            );
                            metrics.record_proxy_rate_limited(
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
                        return Err(mapped);
                    }
                }
                if let Some((scope, limits)) = project_limits_scope.as_ref() {
                    if let Err(err) =
                        gateway
                            .limits
                            .check_and_consume(scope, limits, charge_tokens, minute)
                    {
                        gateway.observability.record_rate_limited();
                        let mapped = map_openai_gateway_error(err);
                        #[cfg(feature = "gateway-metrics-prometheus")]
                        if let Some(metrics) = state.prometheus_metrics.as_ref() {
                            let duration = metrics_timer_start.elapsed();
                            let status = mapped.0.as_u16();
                            drop(gateway);
                            let mut metrics = metrics.lock().await;
                            metrics.record_proxy_request(
                                Some(&key.id),
                                model.as_deref(),
                                &metrics_path,
                            );
                            metrics.record_proxy_rate_limited(
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
                        return Err(mapped);
                    }
                }
                if let Some((scope, limits)) = user_limits_scope.as_ref() {
                    if let Err(err) =
                        gateway
                            .limits
                            .check_and_consume(scope, limits, charge_tokens, minute)
                    {
                        gateway.observability.record_rate_limited();
                        let mapped = map_openai_gateway_error(err);
                        #[cfg(feature = "gateway-metrics-prometheus")]
                        if let Some(metrics) = state.prometheus_metrics.as_ref() {
                            let duration = metrics_timer_start.elapsed();
                            let status = mapped.0.as_u16();
                            drop(gateway);
                            let mut metrics = metrics.lock().await;
                            metrics.record_proxy_request(
                                Some(&key.id),
                                model.as_deref(),
                                &metrics_path,
                            );
                            metrics.record_proxy_rate_limited(
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
                        return Err(mapped);
                    }
                }
            }

            let guardrails = model
                .as_deref()
                .and_then(|model| {
                    gateway
                        .router
                        .rule_for_model(model, Some(key))
                        .and_then(|rule| rule.guardrails.as_ref())
                })
                .unwrap_or(&key.guardrails)
                .clone();

            if let Some(model_id) = model.as_deref() {
                if let Some(reason) = guardrails.check_model(model_id) {
                    gateway.observability.record_guardrail_blocked();
                    let err = openai_error(
                        StatusCode::FORBIDDEN,
                        "policy_error",
                        Some("guardrail_rejected"),
                        reason,
                    );
                    #[cfg(feature = "gateway-metrics-prometheus")]
                    if let Some(metrics) = state.prometheus_metrics.as_ref() {
                        let duration = metrics_timer_start.elapsed();
                        let status = err.0.as_u16();
                        drop(gateway);
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

            if let Some(limit) = guardrails.max_input_tokens {
                if input_tokens_estimate > limit {
                    gateway.observability.record_guardrail_blocked();
                    let err = openai_error(
                        StatusCode::FORBIDDEN,
                        "policy_error",
                        Some("guardrail_rejected"),
                        format!("input_tokens>{limit}"),
                    );
                    #[cfg(feature = "gateway-metrics-prometheus")]
                    if let Some(metrics) = state.prometheus_metrics.as_ref() {
                        let duration = metrics_timer_start.elapsed();
                        let status = err.0.as_u16();
                        drop(gateway);
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

            drop(gateway);

            if guardrails.validate_schema {
                let reason = if let Some(body_json) = parsed_json.as_ref() {
                    validate_openai_request_schema(path_and_query, body_json)
                } else if parts.method == axum::http::Method::POST {
                    validate_openai_multipart_request_schema(
                        path_and_query,
                        parts
                            .headers
                            .get("content-type")
                            .and_then(|value| value.to_str().ok()),
                        body,
                    )
                } else {
                    None
                };
                if let Some(reason) = reason {
                    {
                        let mut gateway = state.gateway.lock().await;
                        gateway.observability.record_guardrail_blocked();
                    }
                    let err = openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        reason,
                    );
                    #[cfg(feature = "gateway-metrics-prometheus")]
                    if let Some(metrics) = state.prometheus_metrics.as_ref() {
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

            if guardrails.has_text_filters() {
                if let Ok(text) = std::str::from_utf8(body) {
                    if let Some(reason) = guardrails.check_text(text) {
                        {
                            let mut gateway = state.gateway.lock().await;
                            gateway.observability.record_guardrail_blocked();
                        }
                        let err = openai_error(
                            StatusCode::FORBIDDEN,
                            "policy_error",
                            Some("guardrail_rejected"),
                            reason,
                        );
                        #[cfg(feature = "gateway-metrics-prometheus")]
                        if let Some(metrics) = state.prometheus_metrics.as_ref() {
                            let duration = metrics_timer_start.elapsed();
                            let status = err.0.as_u16();
                            let mut metrics = metrics.lock().await;
                            metrics.record_proxy_request(
                                Some(&key.id),
                                model.as_deref(),
                                &metrics_path,
                            );
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
            }

            let mut gateway = state.gateway.lock().await;

            if !use_persistent_budget {
                if let Err(err) =
                    gateway
                        .budget
                        .can_spend(&key.id, &key.budget, u64::from(charge_tokens))
                {
                    gateway.observability.record_budget_exceeded();
                    let mapped = map_openai_gateway_error(err);
                    #[cfg(feature = "gateway-metrics-prometheus")]
                    if let Some(metrics) = state.prometheus_metrics.as_ref() {
                        let duration = metrics_timer_start.elapsed();
                        let status = mapped.0.as_u16();
                        drop(gateway);
                        let mut metrics = metrics.lock().await;
                        metrics.record_proxy_request(Some(&key.id), model.as_deref(), &metrics_path);
                        metrics.record_proxy_budget_exceeded(
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
                    return Err(mapped);
                }

                if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                    if let Err(err) =
                        gateway
                            .budget
                            .can_spend(scope, budget, u64::from(charge_tokens))
                    {
                        gateway.observability.record_budget_exceeded();
                        let mapped = map_openai_gateway_error(err);
                        #[cfg(feature = "gateway-metrics-prometheus")]
                        if let Some(metrics) = state.prometheus_metrics.as_ref() {
                            let duration = metrics_timer_start.elapsed();
                            let status = mapped.0.as_u16();
                            drop(gateway);
                            let mut metrics = metrics.lock().await;
                            metrics.record_proxy_request(
                                Some(&key.id),
                                model.as_deref(),
                                &metrics_path,
                            );
                            metrics.record_proxy_budget_exceeded(
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
                        return Err(mapped);
                    }
                }

                if let Some((scope, budget)) = project_budget_scope.as_ref() {
                    if let Err(err) =
                        gateway
                            .budget
                            .can_spend(scope, budget, u64::from(charge_tokens))
                    {
                        gateway.observability.record_budget_exceeded();
                        let mapped = map_openai_gateway_error(err);
                        #[cfg(feature = "gateway-metrics-prometheus")]
                        if let Some(metrics) = state.prometheus_metrics.as_ref() {
                            let duration = metrics_timer_start.elapsed();
                            let status = mapped.0.as_u16();
                            drop(gateway);
                            let mut metrics = metrics.lock().await;
                            metrics.record_proxy_request(
                                Some(&key.id),
                                model.as_deref(),
                                &metrics_path,
                            );
                            metrics.record_proxy_budget_exceeded(
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
                        return Err(mapped);
                    }
                }

                if let Some((scope, budget)) = user_budget_scope.as_ref() {
                    if let Err(err) =
                        gateway
                            .budget
                            .can_spend(scope, budget, u64::from(charge_tokens))
                    {
                        gateway.observability.record_budget_exceeded();
                        let mapped = map_openai_gateway_error(err);
                        #[cfg(feature = "gateway-metrics-prometheus")]
                        if let Some(metrics) = state.prometheus_metrics.as_ref() {
                            let duration = metrics_timer_start.elapsed();
                            let status = mapped.0.as_u16();
                            drop(gateway);
                            let mut metrics = metrics.lock().await;
                            metrics.record_proxy_request(
                                Some(&key.id),
                                model.as_deref(),
                                &metrics_path,
                            );
                            metrics.record_proxy_budget_exceeded(
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
                        return Err(mapped);
                    }
                }
            }

            let budget = Some(key.budget.clone());

            let backends = gateway
                .router
                .select_backends_for_model_seeded(
                    model.as_deref().unwrap_or_default(),
                    Some(key),
                    Some(request_id),
                )
                .map_err(map_openai_gateway_error)?;

            #[cfg(feature = "gateway-costing")]
            let charge_cost_usd_micros = {
                if has_cost_budget {
                    match cost_budget_policy.unwrap_or(CostBudgetEndpointPolicy::Unsupported) {
                        CostBudgetEndpointPolicy::Free => Some(0),
                        CostBudgetEndpointPolicy::TokenBased => {
                            if state.pricing.is_none() {
                                return Err(openai_error(
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    "api_error",
                                    Some("pricing_not_configured"),
                                    "pricing not configured for cost budgets",
                                ));
                            }
                            if model.as_deref().is_none() {
                                return Err(openai_error(
                                    StatusCode::BAD_REQUEST,
                                    "invalid_request_error",
                                    Some("invalid_request"),
                                    "missing field `model`",
                                ));
                            }

                            estimate_charge_cost_usd_micros(
                                state,
                                &gateway,
                                model.as_deref(),
                                input_tokens_estimate,
                                _max_output_tokens,
                                _service_tier.as_deref(),
                                &backends,
                            )
                        }
                        CostBudgetEndpointPolicy::Unsupported => {
                            return Err(openai_error(
                                StatusCode::BAD_REQUEST,
                                "invalid_request_error",
                                Some("cost_budget_unsupported_endpoint"),
                                "cost budgets are token-based and do not support this endpoint (disable total_usd_micros or use token budgets)",
                            ));
                        }
                    }
                } else {
                    estimate_charge_cost_usd_micros(
                        state,
                        &gateway,
                        model.as_deref(),
                        input_tokens_estimate,
                        _max_output_tokens,
                        _service_tier.as_deref(),
                        &backends,
                    )
                }
            };
            #[cfg(not(feature = "gateway-costing"))]
            let charge_cost_usd_micros: Option<u64> = None;

            if !use_persistent_budget {
                #[cfg(feature = "gateway-costing")]
                if key.budget.total_usd_micros.is_some() {
                    let Some(charge_cost_usd_micros) = charge_cost_usd_micros else {
                        return Err(openai_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "api_error",
                            Some("pricing_not_configured"),
                            "pricing not configured for cost budgets",
                        ));
                    };

                    if let Err(err) = gateway.budget.can_spend_cost_usd_micros(
                        &key.id,
                        &key.budget,
                        charge_cost_usd_micros,
                    ) {
                        gateway.observability.record_budget_exceeded();
                        let mapped = map_openai_gateway_error(err);
                        #[cfg(feature = "gateway-metrics-prometheus")]
                        if let Some(metrics) = state.prometheus_metrics.as_ref() {
                            let duration = metrics_timer_start.elapsed();
                            let status = mapped.0.as_u16();
                            drop(gateway);
                            let mut metrics = metrics.lock().await;
                            metrics.record_proxy_request(Some(&key.id), model.as_deref(), &metrics_path);
                            metrics.record_proxy_budget_exceeded(
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
                        return Err(mapped);
                    }
                }

                #[cfg(feature = "gateway-costing")]
                if tenant_budget_scope
                    .as_ref()
                    .is_some_and(|(_, budget)| budget.total_usd_micros.is_some())
                    || project_budget_scope
                    .as_ref()
                    .is_some_and(|(_, budget)| budget.total_usd_micros.is_some())
                    || user_budget_scope
                        .as_ref()
                        .is_some_and(|(_, budget)| budget.total_usd_micros.is_some())
                {
                    let Some(charge_cost_usd_micros) = charge_cost_usd_micros else {
                        return Err(openai_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "api_error",
                            Some("pricing_not_configured"),
                            "pricing not configured for cost budgets",
                        ));
                    };

                    if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                        if let Some(_limit) = budget.total_usd_micros {
                            if let Err(err) = gateway.budget.can_spend_cost_usd_micros(
                                scope,
                                budget,
                                charge_cost_usd_micros,
                            ) {
                                gateway.observability.record_budget_exceeded();
                                let mapped = map_openai_gateway_error(err);
                                #[cfg(feature = "gateway-metrics-prometheus")]
                                if let Some(metrics) = state.prometheus_metrics.as_ref() {
                                    let duration = metrics_timer_start.elapsed();
                                    let status = mapped.0.as_u16();
                                    drop(gateway);
                                    let mut metrics = metrics.lock().await;
                                    metrics.record_proxy_request(
                                        Some(&key.id),
                                        model.as_deref(),
                                        &metrics_path,
                                    );
                                    metrics.record_proxy_budget_exceeded(
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
                                return Err(mapped);
                            }
                        }
                    }

                    if let Some((scope, budget)) = project_budget_scope.as_ref() {
                        if let Some(_limit) = budget.total_usd_micros {
                            if let Err(err) = gateway.budget.can_spend_cost_usd_micros(
                                scope,
                                budget,
                                charge_cost_usd_micros,
                            ) {
                                gateway.observability.record_budget_exceeded();
                                let mapped = map_openai_gateway_error(err);
                                #[cfg(feature = "gateway-metrics-prometheus")]
                                if let Some(metrics) = state.prometheus_metrics.as_ref() {
                                    let duration = metrics_timer_start.elapsed();
                                    let status = mapped.0.as_u16();
                                    drop(gateway);
                                    let mut metrics = metrics.lock().await;
                                    metrics.record_proxy_request(
                                        Some(&key.id),
                                        model.as_deref(),
                                        &metrics_path,
                                    );
                                    metrics.record_proxy_budget_exceeded(
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
                                return Err(mapped);
                            }
                        }
                    }

                    if let Some((scope, budget)) = user_budget_scope.as_ref() {
                        if let Some(_limit) = budget.total_usd_micros {
                            if let Err(err) = gateway.budget.can_spend_cost_usd_micros(
                                scope,
                                budget,
                                charge_cost_usd_micros,
                            ) {
                                gateway.observability.record_budget_exceeded();
                                let mapped = map_openai_gateway_error(err);
                                #[cfg(feature = "gateway-metrics-prometheus")]
                                if let Some(metrics) = state.prometheus_metrics.as_ref() {
                                    let duration = metrics_timer_start.elapsed();
                                    let status = mapped.0.as_u16();
                                    drop(gateway);
                                    let mut metrics = metrics.lock().await;
                                    metrics.record_proxy_request(
                                        Some(&key.id),
                                        model.as_deref(),
                                        &metrics_path,
                                    );
                                    metrics.record_proxy_budget_exceeded(
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
                                return Err(mapped);
                            }
                        }
                    }
                }
            }

            drop(gateway);
            (
                virtual_key_id,
                limits,
                budget,
                tenant_budget_scope,
                project_budget_scope,
                user_budget_scope,
                tenant_limits_scope,
                project_limits_scope,
                user_limits_scope,
                backends,
                strip_authorization,
                charge_cost_usd_micros,
            )
        } else {
            let backends = gateway
                .router
                .select_backends_for_model_seeded(
                    model.as_deref().unwrap_or_default(),
                    None,
                    Some(request_id),
                )
                .map_err(map_openai_gateway_error)?;

            #[cfg(feature = "gateway-costing")]
            let charge_cost_usd_micros = estimate_charge_cost_usd_micros(
                state,
                &gateway,
                model.as_deref(),
                input_tokens_estimate,
                _max_output_tokens,
                _service_tier.as_deref(),
                &backends,
            );
            #[cfg(not(feature = "gateway-costing"))]
            let charge_cost_usd_micros: Option<u64> = None;

            drop(gateway);
            (
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                backends,
                strip_authorization,
                charge_cost_usd_micros,
            )
        }
    };

    Ok((
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
    ))
}
