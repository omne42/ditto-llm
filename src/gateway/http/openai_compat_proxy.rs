#[derive(Clone, Copy)]
struct ProxyAttemptParams<'a> {
    state: &'a GatewayHttpState,
    parts: &'a axum::http::request::Parts,
    body: &'a Bytes,
    parsed_json: &'a Option<serde_json::Value>,
    model: &'a Option<String>,
    service_tier: &'a Option<String>,
    request_id: &'a str,
    path_and_query: &'a str,
    now_epoch_seconds: u64,
    charge_tokens: u32,
    max_output_tokens: u32,
    stream_requested: bool,
    strip_authorization: bool,
    use_persistent_budget: bool,
    virtual_key_id: &'a Option<String>,
    budget: &'a Option<super::BudgetConfig>,
    project_budget_scope: &'a Option<(String, super::BudgetConfig)>,
    user_budget_scope: &'a Option<(String, super::BudgetConfig)>,
    charge_cost_usd_micros: Option<u64>,
    token_budget_reserved: bool,
    #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
    token_budget_reservation_ids: &'a [String],
    cost_budget_reserved: bool,
    #[cfg(all(
        feature = "gateway-costing",
        any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
    ))]
    cost_budget_reservation_ids: &'a [String],
    max_attempts: usize,
    #[cfg(feature = "gateway-routing-advanced")]
    retry_config: &'a super::ProxyRetryConfig,
    #[cfg(feature = "gateway-proxy-cache")]
    proxy_cache_key: &'a Option<String>,
    #[cfg(feature = "gateway-metrics-prometheus")]
    metrics_path: &'a str,
    #[cfg(feature = "gateway-metrics-prometheus")]
    metrics_timer_start: Instant,
}

enum BackendAttemptOutcome {
    Response(axum::response::Response),
    Continue(Option<(StatusCode, Json<OpenAiErrorResponse>)>),
}

fn validate_openai_multipart_request_schema(
    path_and_query: &str,
    content_type: Option<&str>,
    body: &Bytes,
) -> Option<String> {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query)
        .trim_end_matches('/');

    let endpoint = if path == "/v1/audio/transcriptions" {
        "audio/transcriptions"
    } else if path == "/v1/audio/translations" {
        "audio/translations"
    } else if path == "/v1/files" {
        "files"
    } else {
        return None;
    };

    let Some(content_type) = content_type else {
        return Some(format!("{endpoint} request missing content-type"));
    };
    if !content_type
        .to_ascii_lowercase()
        .starts_with("multipart/form-data")
    {
        return Some(format!("{endpoint} request must be multipart/form-data"));
    }

    let parts = match super::multipart::parse_multipart_form(content_type, body) {
        Ok(parts) => parts,
        Err(err) => return Some(err),
    };

    if endpoint.starts_with("audio/") {
        let mut has_file = false;
        let mut has_model = false;
        for part in parts {
            match part.name.as_str() {
                "file" => has_file = true,
                "model" if part.filename.is_none() => {
                    let value = String::from_utf8_lossy(part.data.as_ref())
                        .trim()
                        .to_string();
                    if !value.is_empty() {
                        has_model = true;
                    }
                }
                _ => {}
            }
        }

        if !has_file {
            return Some(format!("{endpoint} request missing file"));
        }
        if !has_model {
            return Some(format!("{endpoint} request missing model"));
        }
        return None;
    }

    let mut has_file = false;
    let mut has_purpose = false;
    for part in parts {
        match part.name.as_str() {
            "file" => has_file = true,
            "purpose" if part.filename.is_none() => {
                let value = String::from_utf8_lossy(part.data.as_ref())
                    .trim()
                    .to_string();
                if !value.is_empty() {
                    has_purpose = true;
                }
            }
            _ => {}
        }
    }

    if !has_file {
        return Some("files request missing file".to_string());
    }
    if !has_purpose {
        return Some("files request missing purpose".to_string());
    }
    None
}
async fn handle_openai_compat_proxy(
    State(state): State<GatewayHttpState>,
    Path(_path): Path<String>,
    req: axum::http::Request<Body>,
) -> Result<axum::response::Response, (StatusCode, Json<OpenAiErrorResponse>)> {
    let max_body_bytes = state.proxy_max_body_bytes;

    let (parts, body) = req.into_parts();
    let body = to_bytes(body, max_body_bytes)
        .await
        .map_err(|err| openai_error(StatusCode::BAD_REQUEST, "invalid_request_error", None, err))?;

    let request_id =
        extract_header(&parts.headers, "x-request-id").unwrap_or_else(generate_request_id);

    let path_and_query = parts
        .uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or_else(|| parts.uri.path());

    #[cfg(feature = "gateway-otel")]
    let proxy_span = tracing::info_span!(
        "ditto.gateway.proxy",
        request_id = %request_id,
        method = %parts.method,
        path = %path_and_query,
        model = tracing::field::Empty,
        virtual_key_id = tracing::field::Empty,
        backend = tracing::field::Empty,
        status = tracing::field::Empty,
        cache = tracing::field::Empty,
    );
    #[cfg(feature = "gateway-otel")]
    let _proxy_span_guard = proxy_span.enter();

    let parsed_json: Option<serde_json::Value> = parts
        .headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .filter(|ct| ct.to_ascii_lowercase().starts_with("application/json"))
        .and_then(|_| serde_json::from_slice(&body).ok());

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

    let (
        virtual_key_id,
        limits,
        budget,
        project_budget_scope,
        user_budget_scope,
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
            let token = extract_bearer(&parts.headers)
                .or_else(|| extract_header(&parts.headers, "x-ditto-virtual-key"))
                .or_else(|| extract_header(&parts.headers, "x-api-key"))
                .ok_or_else(|| {
                    openai_error(
                        StatusCode::UNAUTHORIZED,
                        "authentication_error",
                        Some("invalid_api_key"),
                        "missing virtual key",
                    )
                })?;
            let key = gateway
                .config
                .virtual_keys
                .iter()
                .find(|key| key.token == token)
                .cloned()
                .ok_or_else(|| {
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
            let project_budget_scope = key
                .project_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .and_then(|id| {
                    key.project_budget
                        .as_ref()
                        .map(|budget| (format!("project:{id}"), budget.clone()))
                });
            let user_budget_scope = key
                .user_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .and_then(|id| {
                    key.user_budget
                        .as_ref()
                        .map(|budget| (format!("user:{id}"), budget.clone()))
                });

            if !use_redis_budget {
                if let Err(err) =
                    gateway
                        .limits
                        .check_and_consume(&key.id, &key.limits, charge_tokens, minute)
                {
                    gateway.observability.record_rate_limited();
                    return Err(map_openai_gateway_error(err));
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
                .unwrap_or(&key.guardrails);

            if let Some(model) = model.as_deref() {
                if let Some(reason) = guardrails.check_model(model) {
                    gateway.observability.record_guardrail_blocked();
                    return Err(openai_error(
                        StatusCode::FORBIDDEN,
                        "policy_error",
                        Some("guardrail_rejected"),
                        reason,
                    ));
                }
            }

            if let Some(limit) = guardrails.max_input_tokens {
                if input_tokens_estimate > limit {
                    gateway.observability.record_guardrail_blocked();
                    return Err(openai_error(
                        StatusCode::FORBIDDEN,
                        "policy_error",
                        Some("guardrail_rejected"),
                        format!("input_tokens>{limit}"),
                    ));
                }
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
                        &body,
                    )
                } else {
                    None
                };
                if let Some(reason) = reason {
                    gateway.observability.record_guardrail_blocked();
                    return Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        reason,
                    ));
                }
            }

            if guardrails.has_text_filters() {
                if let Ok(text) = std::str::from_utf8(&body) {
                    if let Some(reason) = guardrails.check_text(text) {
                        gateway.observability.record_guardrail_blocked();
                        return Err(openai_error(
                            StatusCode::FORBIDDEN,
                            "policy_error",
                            Some("guardrail_rejected"),
                            reason,
                        ));
                    }
                }
            }

            if !use_persistent_budget {
                if let Err(err) =
                    gateway
                        .budget
                        .can_spend(&key.id, &key.budget, u64::from(charge_tokens))
                {
                    gateway.observability.record_budget_exceeded();
                    return Err(map_openai_gateway_error(err));
                }

                if let Some((scope, budget)) = project_budget_scope.as_ref() {
                    if let Err(err) =
                        gateway
                            .budget
                            .can_spend(scope, budget, u64::from(charge_tokens))
                    {
                        gateway.observability.record_budget_exceeded();
                        return Err(map_openai_gateway_error(err));
                    }
                }

                if let Some((scope, budget)) = user_budget_scope.as_ref() {
                    if let Err(err) =
                        gateway
                            .budget
                            .can_spend(scope, budget, u64::from(charge_tokens))
                    {
                        gateway.observability.record_budget_exceeded();
                        return Err(map_openai_gateway_error(err));
                    }
                }
            }

            let budget = Some(key.budget.clone());

            let backends = gateway
                .router
                .select_backends_for_model_seeded(
                    model.as_deref().unwrap_or_default(),
                    Some(key),
                    Some(&request_id),
                )
                .map_err(map_openai_gateway_error)?;

            #[cfg(feature = "gateway-costing")]
            let charge_cost_usd_micros = model.as_deref().and_then(|request_model| {
                state.pricing.as_ref().and_then(|pricing| {
                    let mut cost = pricing.estimate_cost_usd_micros_for_service_tier(
                        request_model,
                        input_tokens_estimate,
                        max_output_tokens,
                        service_tier.as_deref(),
                    );
                    for backend_name in &backends {
                        if !state.proxy_backends.contains_key(backend_name) {
                            continue;
                        }
                        let mapped_model = gateway
                            .config
                            .backends
                            .iter()
                            .find(|backend| backend.name == backend_name.as_str())
                            .and_then(|backend| backend.model_map.get(request_model))
                            .map(|model| model.as_str());
                        if let Some(mapped_model) = mapped_model {
                            cost = max_option_u64(
                                cost,
                                pricing.estimate_cost_usd_micros_for_service_tier(
                                    mapped_model,
                                    input_tokens_estimate,
                                    max_output_tokens,
                                    service_tier.as_deref(),
                                ),
                            );
                        }
                    }
                    cost
                })
            });
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
                        return Err(map_openai_gateway_error(err));
                    }
                }

                #[cfg(feature = "gateway-costing")]
                if project_budget_scope
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

                    if let Some((scope, budget)) = project_budget_scope.as_ref() {
                        if let Some(_limit) = budget.total_usd_micros {
                            if let Err(err) = gateway.budget.can_spend_cost_usd_micros(
                                scope,
                                budget,
                                charge_cost_usd_micros,
                            ) {
                                gateway.observability.record_budget_exceeded();
                                return Err(map_openai_gateway_error(err));
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
                                return Err(map_openai_gateway_error(err));
                            }
                        }
                    }
                }
            }

            (
                virtual_key_id,
                limits,
                budget,
                project_budget_scope,
                user_budget_scope,
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
                    Some(&request_id),
                )
                .map_err(map_openai_gateway_error)?;

            #[cfg(feature = "gateway-costing")]
            let charge_cost_usd_micros = model.as_deref().and_then(|request_model| {
                state.pricing.as_ref().and_then(|pricing| {
                    let mut cost = pricing.estimate_cost_usd_micros_for_service_tier(
                        request_model,
                        input_tokens_estimate,
                        max_output_tokens,
                        service_tier.as_deref(),
                    );
                    for backend_name in &backends {
                        if !state.proxy_backends.contains_key(backend_name) {
                            continue;
                        }
                        let mapped_model = gateway
                            .config
                            .backends
                            .iter()
                            .find(|backend| backend.name == backend_name.as_str())
                            .and_then(|backend| backend.model_map.get(request_model))
                            .map(|model| model.as_str());
                        if let Some(mapped_model) = mapped_model {
                            cost = max_option_u64(
                                cost,
                                pricing.estimate_cost_usd_micros_for_service_tier(
                                    mapped_model,
                                    input_tokens_estimate,
                                    max_output_tokens,
                                    service_tier.as_deref(),
                                ),
                            );
                        }
                    }
                    cost
                })
            });
            #[cfg(not(feature = "gateway-costing"))]
            let charge_cost_usd_micros: Option<u64> = None;

            (
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

    #[cfg(feature = "gateway-store-redis")]
    if let (Some(store), Some(virtual_key_id), Some(limits)) =
        (state.redis_store.as_ref(), virtual_key_id.as_deref(), limits.as_ref())
    {
        if let Err(err) = store
            .check_and_consume_rate_limits(virtual_key_id, limits, charge_tokens, minute)
            .await
        {
            if matches!(err, GatewayError::RateLimited { .. }) {
                let mut gateway = state.gateway.lock().await;
                gateway.observability.record_rate_limited();
            }
            return Err(map_openai_gateway_error(err));
        }
    }

    #[cfg(feature = "gateway-otel")]
    if let Some(virtual_key_id) = virtual_key_id.as_deref() {
        proxy_span.record("virtual_key_id", tracing::field::display(virtual_key_id));
    }

    #[cfg(feature = "gateway-metrics-prometheus")]
    let metrics_path = super::metrics_prometheus::normalize_proxy_path_label(path_and_query);
    #[cfg(feature = "gateway-metrics-prometheus")]
    let metrics_timer_start = Instant::now();

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
    if let (Some(cache), Some(cache_key)) = (state.proxy_cache.as_ref(), proxy_cache_key.as_ref()) {
        #[cfg(feature = "gateway-metrics-prometheus")]
        if let Some(metrics) = state.prometheus_metrics.as_ref() {
            metrics
                .lock()
                .await
                .record_proxy_cache_lookup(&metrics_path);
        }

        let mut cache_source = "memory";
        let mut cached = { cache.lock().await.get(cache_key, _now_epoch_seconds) };
        #[cfg(feature = "gateway-store-redis")]
        if cached.is_none() {
            if let Some(store) = state.redis_store.as_ref() {
                if let Ok(redis_cached) = store.get_proxy_cache_response(cache_key).await {
                    if redis_cached.is_some() {
                        cache_source = "redis";
                    }
                    cached = redis_cached;
                }
            }
        }
        if let Some(cached) = cached {
            if cache_source == "redis" {
                let mut cache = cache.lock().await;
                cache.insert(cache_key.to_string(), cached.clone(), _now_epoch_seconds);
            }
            {
                let mut gateway = state.gateway.lock().await;
                gateway.observability.record_cache_hit();
            }

            emit_json_log(
                &state,
                "proxy.cache_hit",
                serde_json::json!({
                    "request_id": &request_id,
                    "cache": cache_source,
                    "backend": &cached.backend,
                    "path": path_and_query,
                }),
            );

            #[cfg(feature = "gateway-otel")]
            {
                proxy_span.record("cache", tracing::field::display("hit"));
                proxy_span.record("backend", tracing::field::display(&cached.backend));
                proxy_span.record("status", tracing::field::display(cached.status));
            }

            #[cfg(feature = "gateway-metrics-prometheus")]
            if let Some(metrics) = state.prometheus_metrics.as_ref() {
                let mut metrics = metrics.lock().await;
                metrics.record_proxy_cache_hit();
                metrics.record_proxy_cache_hit_by_source(cache_source);
                metrics.record_proxy_cache_hit_by_path(&metrics_path);
                metrics.record_proxy_response_status_by_path(&metrics_path, cached.status);
                metrics
                    .observe_proxy_request_duration(&metrics_path, metrics_timer_start.elapsed());
            }

            let mut response = cached_proxy_response(cached, request_id.clone());
            if let Ok(value) = axum::http::HeaderValue::from_str(cache_key) {
                response.headers_mut().insert("x-ditto-cache-key", value);
            }
            if let Ok(value) = axum::http::HeaderValue::from_str(cache_source) {
                response.headers_mut().insert("x-ditto-cache-source", value);
            }
            return Ok(response);
        }

        #[cfg(feature = "gateway-metrics-prometheus")]
        if let Some(metrics) = state.prometheus_metrics.as_ref() {
            metrics.lock().await.record_proxy_cache_miss(&metrics_path);
        }
    }


    #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
    let budget_reservation_params = ProxyBudgetReservationParams {
        state: &state,
        use_persistent_budget,
        virtual_key_id: virtual_key_id.as_deref(),
        budget: budget.as_ref(),
        project_budget_scope: &project_budget_scope,
        user_budget_scope: &user_budget_scope,
        request_id: &request_id,
        path_and_query,
        model: &model,
        charge_tokens,
    };

    #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
    let (_token_budget_reserved, token_budget_reservation_ids) =
        reserve_proxy_token_budgets_for_request(budget_reservation_params).await?;

    #[cfg(not(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis")))]
    let _token_budget_reserved = false;

    #[cfg(all(
        feature = "gateway-costing",
        any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
    ))]
    let (_cost_budget_reserved, cost_budget_reservation_ids) = reserve_proxy_cost_budgets_for_request(
        budget_reservation_params,
        charge_cost_usd_micros,
        &token_budget_reservation_ids,
    )
    .await?;

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
        max_output_tokens,
        stream_requested: _stream_requested,
        strip_authorization,
        use_persistent_budget,
        virtual_key_id: &virtual_key_id,
        budget: &budget,
        project_budget_scope: &project_budget_scope,
        user_budget_scope: &user_budget_scope,
        charge_cost_usd_micros,
        token_budget_reserved: _token_budget_reserved,
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

    #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
    {
        let (status, err_kind, err_code, err_message) = match last_err.as_ref() {
            Some((status, body)) => (
                Some(status.as_u16()),
                Some(body.0.error.kind),
                body.0.error.code,
                Some(body.0.error.message.as_str()),
            ),
            None => (None, None, None, None),
        };
        let payload = serde_json::json!({
            "request_id": &request_id,
            "virtual_key_id": virtual_key_id.as_deref(),
            "attempted_backends": &attempted_backends,
            "method": parts.method.as_str(),
            "path": path_and_query,
            "model": &model,
            "charge_tokens": charge_tokens,
            "charge_cost_usd_micros": charge_cost_usd_micros,
            "body_len": body.len(),
            "status": status,
            "error_type": err_kind,
            "error_code": err_code,
            "error_message": err_message,
        });

        #[cfg(feature = "gateway-store-sqlite")]
        if let Some(store) = state.sqlite_store.as_ref() {
            let _ = store.append_audit_log("proxy.error", payload.clone()).await;
        }
        #[cfg(feature = "gateway-store-redis")]
        if let Some(store) = state.redis_store.as_ref() {
            let _ = store.append_audit_log("proxy.error", payload.clone()).await;
        }
    }

    emit_json_log(
        &state,
        "proxy.error",
        serde_json::json!({
            "request_id": &request_id,
            "attempted_backends": &attempted_backends,
            "status": last_err.as_ref().map(|(status, _)| status.as_u16()),
        }),
    );

    #[cfg(feature = "gateway-metrics-prometheus")]
    if let Some(metrics) = state.prometheus_metrics.as_ref() {
        let status = last_err
            .as_ref()
            .map(|(status, _)| status.as_u16())
            .unwrap_or(StatusCode::BAD_GATEWAY.as_u16());
        let mut metrics = metrics.lock().await;
        metrics.record_proxy_response_status_by_path(&metrics_path, status);
        metrics.observe_proxy_request_duration(&metrics_path, metrics_timer_start.elapsed());
    }

    Err(last_err.unwrap_or_else(|| {
        openai_error(
            StatusCode::BAD_GATEWAY,
            "api_error",
            Some("backend_error"),
            "all backends failed",
        )
    }))
}
