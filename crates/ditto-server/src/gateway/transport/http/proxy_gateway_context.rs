use super::*;

#[derive(Debug, Clone)]
pub(super) struct ResolvedGatewayContext {
    pub(super) virtual_key_id: Option<String>,
    #[cfg(feature = "gateway-translation")]
    pub(super) response_owner: super::translation::TranslationResponseOwner,
    pub(super) limits: Option<super::LimitsConfig>,
    pub(super) budget: Option<super::BudgetConfig>,
    pub(super) tenant_budget_scope: Option<(String, super::BudgetConfig)>,
    pub(super) project_budget_scope: Option<(String, super::BudgetConfig)>,
    pub(super) user_budget_scope: Option<(String, super::BudgetConfig)>,
    pub(super) tenant_limits_scope: Option<(String, super::LimitsConfig)>,
    pub(super) project_limits_scope: Option<(String, super::LimitsConfig)>,
    pub(super) user_limits_scope: Option<(String, super::LimitsConfig)>,
    pub(super) backend_candidates: Vec<String>,
    pub(super) strip_authorization: bool,
    pub(super) charge_cost_usd_micros: Option<u64>,
    pub(super) local_token_budget_reserved: bool,
    #[cfg(feature = "gateway-costing")]
    pub(super) local_cost_budget_reserved: bool,
}

pub(super) struct ResolveOpenAiCompatProxyGatewayContextRequest<'a> {
    pub(super) state: &'a GatewayHttpState,
    pub(super) parts: &'a axum::http::request::Parts,
    pub(super) body: &'a Bytes,
    pub(super) parsed_json: &'a Option<serde_json::Value>,
    pub(super) request_id: &'a str,
    pub(super) path_and_query: &'a str,
    pub(super) model: &'a Option<String>,
    pub(super) service_tier: &'a Option<String>,
    pub(super) input_tokens_estimate: u32,
    pub(super) max_output_tokens: u32,
    pub(super) charge_tokens: u32,
    pub(super) minute: u64,
    pub(super) use_redis_budget: bool,
    pub(super) use_persistent_budget: bool,
    #[cfg(feature = "gateway-metrics-prometheus")]
    pub(super) metrics_path: &'a str,
    #[cfg(feature = "gateway-metrics-prometheus")]
    pub(super) metrics_timer_start: Instant,
}

#[derive(Debug, Clone)]
pub(super) struct OpenAiCompatProxyGatewayPreamble {
    pub(super) strip_authorization: bool,
    pub(super) key: Option<super::VirtualKeyConfig>,
}

struct ResolvedGatewayContextLocals {
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
    charge_cost_usd_micros: Option<u64>,
    local_token_budget_reserved: bool,
    #[cfg(feature = "gateway-costing")]
    local_cost_budget_reserved: bool,
}

fn model_from_request_path(path_and_query: &str) -> Option<&str> {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query)
        .trim_end_matches('/');
    let model = path.strip_prefix("/v1/models/")?.trim();
    if model.is_empty() { None } else { Some(model) }
}

fn effective_request_model<'a>(path_and_query: &'a str, model: Option<&'a str>) -> Option<&'a str> {
    model
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .or_else(|| model_from_request_path(path_and_query))
}

pub(super) async fn resolve_openai_compat_proxy_gateway_preamble(
    state: &GatewayHttpState,
    parts: &axum::http::request::Parts,
) -> Result<OpenAiCompatProxyGatewayPreamble, (StatusCode, Json<OpenAiErrorResponse>)> {
    // OpenAI-compatible proxy surfaces are always fail-closed. An empty virtual-key
    // set means "no request is authorized", not "anonymous relay mode", unless an
    // in-process protocol adapter has explicitly marked the request as a trusted
    // provider-auth passthrough.
    let strip_authorization =
        !internal_upstream_auth_passthrough_enabled(parts) || gateway_uses_virtual_keys(state);
    let key = if strip_authorization {
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
        Some(key)
    } else {
        None
    };

    Ok(OpenAiCompatProxyGatewayPreamble {
        strip_authorization,
        key,
    })
}

pub(super) async fn resolve_openai_compat_proxy_gateway_context(
    request: ResolveOpenAiCompatProxyGatewayContextRequest<'_>,
) -> Result<ResolvedGatewayContext, (StatusCode, Json<OpenAiErrorResponse>)> {
    let ResolveOpenAiCompatProxyGatewayContextRequest {
        state,
        parts,
        body,
        parsed_json,
        request_id,
        path_and_query,
        model,
        service_tier,
        input_tokens_estimate,
        max_output_tokens,
        charge_tokens,
        minute,
        use_redis_budget,
        use_persistent_budget,
        #[cfg(feature = "gateway-metrics-prometheus")]
        metrics_path,
        #[cfg(feature = "gateway-metrics-prometheus")]
        metrics_timer_start,
    } = request;
    #[cfg(not(feature = "gateway-costing"))]
    let _ = (&service_tier, max_output_tokens);
    let routed_model = effective_request_model(path_and_query, model.as_deref());

    let gateway_preamble = resolve_openai_compat_proxy_gateway_preamble(state, parts).await?;
    let strip_authorization = gateway_preamble.strip_authorization;
    let key = gateway_preamble.key;
    #[cfg(feature = "gateway-translation")]
    let response_owner = key
        .as_ref()
        .map(|key| super::translation::TranslationResponseOwner {
            virtual_key_id: Some(key.id.clone()),
            tenant_id: key
                .tenant_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string),
            project_id: key
                .project_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string),
            user_id: key
                .user_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string),
        })
        .unwrap_or_default();

    let resolved = {
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

            let guardrails = state.guardrails_for_model(routed_model, key);

            if let Some(model_id) = routed_model
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
                    metrics.record_proxy_request(Some(&key.id), model.as_deref(), metrics_path);
                    metrics.record_proxy_guardrail_blocked(
                        Some(&key.id),
                        model.as_deref(),
                        metrics_path,
                    );
                    metrics.record_proxy_response_status_by_path(metrics_path, status);
                    if let Some(model) = model.as_deref() {
                        metrics.record_proxy_response_status_by_model(model, status);
                        metrics.observe_proxy_request_duration_by_model(model, duration);
                    }
                    metrics.observe_proxy_request_duration(metrics_path, duration);
                }
                return Err(err);
            }

            if let Some(limit) = guardrails.max_input_tokens
                && input_tokens_estimate > limit
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
                    metrics.record_proxy_request(Some(&key.id), model.as_deref(), metrics_path);
                    metrics.record_proxy_guardrail_blocked(
                        Some(&key.id),
                        model.as_deref(),
                        metrics_path,
                    );
                    metrics.record_proxy_response_status_by_path(metrics_path, status);
                    if let Some(model) = model.as_deref() {
                        metrics.record_proxy_response_status_by_model(model, status);
                        metrics.observe_proxy_request_duration_by_model(model, duration);
                    }
                    metrics.observe_proxy_request_duration(metrics_path, duration);
                }
                return Err(err);
            }

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
                        metrics.record_proxy_request(Some(&key.id), model.as_deref(), metrics_path);
                        metrics.record_proxy_guardrail_blocked(
                            Some(&key.id),
                            model.as_deref(),
                            metrics_path,
                        );
                        metrics.record_proxy_response_status_by_path(metrics_path, status);
                        if let Some(model) = model.as_deref() {
                            metrics.record_proxy_response_status_by_model(model, status);
                            metrics.observe_proxy_request_duration_by_model(model, duration);
                        }
                        metrics.observe_proxy_request_duration(metrics_path, duration);
                    }
                    return Err(err);
                }
            }

            if guardrails.has_text_filters()
                && let Ok(text) = std::str::from_utf8(body)
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
                    metrics.record_proxy_request(Some(&key.id), model.as_deref(), metrics_path);
                    metrics.record_proxy_guardrail_blocked(
                        Some(&key.id),
                        model.as_deref(),
                        metrics_path,
                    );
                    metrics.record_proxy_response_status_by_path(metrics_path, status);
                    if let Some(model) = model.as_deref() {
                        metrics.record_proxy_response_status_by_model(model, status);
                        metrics.observe_proxy_request_duration_by_model(model, duration);
                    }
                    metrics.observe_proxy_request_duration(metrics_path, duration);
                }
                return Err(err);
            }

            let budget = Some(key.budget.clone());

            let backends = state
                .select_backends_for_model_seeded(
                    routed_model.unwrap_or_default(),
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
                            if state.proxy.pricing.is_none() {
                                return Err(openai_error(
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    "api_error",
                                    Some("pricing_not_configured"),
                                    "pricing not configured for cost budgets",
                                ));
                            }
                            if routed_model.is_none() {
                                return Err(openai_error(
                                    StatusCode::BAD_REQUEST,
                                    "invalid_request_error",
                                    Some("invalid_request"),
                                    "missing field `model`",
                                ));
                            }

                            estimate_charge_cost_usd_micros(
                                state,
                                routed_model,
                                input_tokens_estimate,
                                max_output_tokens,
                                service_tier.as_deref(),
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
                        routed_model,
                        input_tokens_estimate,
                        max_output_tokens,
                        service_tier.as_deref(),
                        &backends,
                    )
                }
            };
            #[cfg(not(feature = "gateway-costing"))]
            let charge_cost_usd_micros: Option<u64> = None;

            if !use_redis_budget {
                let mut rate_limit_scopes = vec![(&key.id[..], &key.limits)];
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
                        metrics.record_proxy_request(Some(&key.id), model.as_deref(), metrics_path);
                        metrics.record_proxy_rate_limited(
                            Some(&key.id),
                            model.as_deref(),
                            metrics_path,
                        );
                        metrics.record_proxy_response_status_by_path(metrics_path, status);
                        if let Some(model) = model.as_deref() {
                            metrics.record_proxy_response_status_by_model(model, status);
                            metrics.observe_proxy_request_duration_by_model(model, duration);
                        }
                        metrics.observe_proxy_request_duration(metrics_path, duration);
                    }
                    return Err(mapped);
                }
            }

            let mut local_token_budget_reserved = false;
            #[cfg(feature = "gateway-costing")]
            let mut local_cost_budget_reserved = false;

            if !use_persistent_budget {
                let budget_scopes = collect_budget_scopes(
                    Some(&key.id),
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
                        metrics.record_proxy_request(Some(&key.id), model.as_deref(), metrics_path);
                        metrics.record_proxy_budget_exceeded(
                            Some(&key.id),
                            model.as_deref(),
                            metrics_path,
                        );
                        metrics.record_proxy_response_status_by_path(metrics_path, status);
                        if let Some(model) = model.as_deref() {
                            metrics.record_proxy_response_status_by_model(model, status);
                            metrics.observe_proxy_request_duration_by_model(model, duration);
                        }
                        metrics.observe_proxy_request_duration(metrics_path, duration);
                    }
                    return Err(mapped);
                }
                local_token_budget_reserved = !budget_scopes.is_empty();

                #[cfg(feature = "gateway-costing")]
                if budget
                    .as_ref()
                    .is_some_and(|budget| budget.total_usd_micros.is_some())
                    || tenant_budget_scope
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
                        if local_token_budget_reserved {
                            let budget_scopes = collect_budget_scopes(
                                Some(&key.id),
                                budget.as_ref(),
                                &tenant_budget_scope,
                                &project_budget_scope,
                                &user_budget_scope,
                            );
                            state.rollback_budget_tokens(budget_scopes, u64::from(charge_tokens));
                        }
                        return Err(openai_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "api_error",
                            Some("pricing_not_configured"),
                            "pricing not configured for cost budgets",
                        ));
                    };

                    let budget_scopes = collect_budget_scopes(
                        Some(&key.id),
                        budget.as_ref(),
                        &tenant_budget_scope,
                        &project_budget_scope,
                        &user_budget_scope,
                    );
                    if !budget_scopes.is_empty()
                        && let Err(err) =
                            state.reserve_budget_cost(budget_scopes.clone(), charge_cost_usd_micros)
                    {
                        if local_token_budget_reserved {
                            state.rollback_budget_tokens(
                                budget_scopes.clone(),
                                u64::from(charge_tokens),
                            );
                        }
                        state.record_budget_exceeded();
                        let mapped = map_openai_gateway_error(err);
                        #[cfg(feature = "gateway-metrics-prometheus")]
                        if let Some(metrics) = state.proxy.metrics.as_ref() {
                            let duration = metrics_timer_start.elapsed();
                            let status = mapped.0.as_u16();
                            let mut metrics = metrics.lock().await;
                            metrics.record_proxy_request(
                                Some(&key.id),
                                model.as_deref(),
                                metrics_path,
                            );
                            metrics.record_proxy_budget_exceeded(
                                Some(&key.id),
                                model.as_deref(),
                                metrics_path,
                            );
                            metrics.record_proxy_response_status_by_path(metrics_path, status);
                            if let Some(model) = model.as_deref() {
                                metrics.record_proxy_response_status_by_model(model, status);
                                metrics.observe_proxy_request_duration_by_model(model, duration);
                            }
                            metrics.observe_proxy_request_duration(metrics_path, duration);
                        }
                        return Err(mapped);
                    }
                    local_cost_budget_reserved = !budget_scopes.is_empty();
                }
            }

            ResolvedGatewayContextLocals {
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
                charge_cost_usd_micros,
                local_token_budget_reserved,
                #[cfg(feature = "gateway-costing")]
                local_cost_budget_reserved,
            }
        } else {
            let backends = state
                .select_backends_for_model_seeded(
                    routed_model.unwrap_or_default(),
                    None,
                    Some(request_id),
                )
                .map_err(map_openai_gateway_error)?;

            #[cfg(feature = "gateway-costing")]
            let charge_cost_usd_micros = estimate_charge_cost_usd_micros(
                state,
                routed_model,
                input_tokens_estimate,
                max_output_tokens,
                service_tier.as_deref(),
                &backends,
            );
            #[cfg(not(feature = "gateway-costing"))]
            let charge_cost_usd_micros: Option<u64> = None;

            ResolvedGatewayContextLocals {
                virtual_key_id: None,
                limits: None,
                budget: None,
                tenant_budget_scope: None,
                project_budget_scope: None,
                user_budget_scope: None,
                tenant_limits_scope: None,
                project_limits_scope: None,
                user_limits_scope: None,
                backend_candidates: backends,
                charge_cost_usd_micros,
                local_token_budget_reserved: false,
                #[cfg(feature = "gateway-costing")]
                local_cost_budget_reserved: false,
            }
        }
    };

    Ok(ResolvedGatewayContext {
        virtual_key_id: resolved.virtual_key_id,
        #[cfg(feature = "gateway-translation")]
        response_owner,
        limits: resolved.limits,
        budget: resolved.budget,
        tenant_budget_scope: resolved.tenant_budget_scope,
        project_budget_scope: resolved.project_budget_scope,
        user_budget_scope: resolved.user_budget_scope,
        tenant_limits_scope: resolved.tenant_limits_scope,
        project_limits_scope: resolved.project_limits_scope,
        user_limits_scope: resolved.user_limits_scope,
        backend_candidates: resolved.backend_candidates,
        strip_authorization,
        charge_cost_usd_micros: resolved.charge_cost_usd_micros,
        local_token_budget_reserved: resolved.local_token_budget_reserved,
        #[cfg(feature = "gateway-costing")]
        local_cost_budget_reserved: resolved.local_cost_budget_reserved,
    })
}
