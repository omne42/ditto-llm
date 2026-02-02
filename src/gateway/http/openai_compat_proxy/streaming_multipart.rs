fn should_stream_large_multipart_request(
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

async fn handle_openai_compat_proxy_streaming_multipart(
    state: GatewayHttpState,
    parts: axum::http::request::Parts,
    body: Body,
    request_id: String,
    path_and_query: String,
) -> Result<axum::response::Response, (StatusCode, Json<OpenAiErrorResponse>)> {
    let model: Option<String> = None;
    let content_length = parts
        .headers
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|raw| raw.parse::<usize>().ok())
        .unwrap_or(0);
    let charge_tokens = estimate_tokens_from_length(content_length);

    #[cfg(feature = "gateway-store-sqlite")]
    let use_sqlite_budget = state.sqlite_store.is_some();
    #[cfg(not(feature = "gateway-store-sqlite"))]
    let use_sqlite_budget = false;

    #[cfg(feature = "gateway-store-redis")]
    let use_redis_budget = state.redis_store.is_some();
    #[cfg(not(feature = "gateway-store-redis"))]
    let use_redis_budget = false;

    let use_persistent_budget = use_sqlite_budget || use_redis_budget;

    #[cfg(feature = "gateway-costing")]
    let charge_cost_usd_micros: Option<u64> = Some(0);
    #[cfg(not(feature = "gateway-costing"))]
    let charge_cost_usd_micros: Option<u64> = None;

    let now_epoch_seconds = now_epoch_seconds();
    let minute = now_epoch_seconds / 60;
    let rate_limit_route = normalize_rate_limit_route(&path_and_query);

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

        if !use_redis_budget {
            if let (Some(key), Some(limits)) = (key.as_ref(), limits.as_ref()) {
                if let Err(err) =
                    gateway
                        .limits
                        .check_and_consume(&key.id, limits, charge_tokens, minute)
                {
                    gateway.observability.record_rate_limited();
                    return Err(map_openai_gateway_error(err));
                }
            }
            if let Some((scope, limits)) = tenant_limits_scope.as_ref() {
                if let Err(err) =
                    gateway
                        .limits
                        .check_and_consume(scope, limits, charge_tokens, minute)
                {
                    gateway.observability.record_rate_limited();
                    return Err(map_openai_gateway_error(err));
                }
            }
            if let Some((scope, limits)) = project_limits_scope.as_ref() {
                if let Err(err) =
                    gateway
                        .limits
                        .check_and_consume(scope, limits, charge_tokens, minute)
                {
                    gateway.observability.record_rate_limited();
                    return Err(map_openai_gateway_error(err));
                }
            }
            if let Some((scope, limits)) = user_limits_scope.as_ref() {
                if let Err(err) =
                    gateway
                        .limits
                        .check_and_consume(scope, limits, charge_tokens, minute)
                {
                    gateway.observability.record_rate_limited();
                    return Err(map_openai_gateway_error(err));
                }
            }
        }

        if !use_persistent_budget {
            if let (Some(key), Some(budget)) = (key.as_ref(), key.as_ref().map(|key| &key.budget)) {
                if let Err(err) = gateway
                    .budget
                    .can_spend(&key.id, budget, u64::from(charge_tokens))
                {
                    gateway.observability.record_budget_exceeded();
                    return Err(map_openai_gateway_error(err));
                }
            }
            if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                if let Err(err) = gateway
                    .budget
                    .can_spend(scope, budget, u64::from(charge_tokens))
                {
                    gateway.observability.record_budget_exceeded();
                    return Err(map_openai_gateway_error(err));
                }
            }
            if let Some((scope, budget)) = project_budget_scope.as_ref() {
                if let Err(err) = gateway
                    .budget
                    .can_spend(scope, budget, u64::from(charge_tokens))
                {
                    gateway.observability.record_budget_exceeded();
                    return Err(map_openai_gateway_error(err));
                }
            }
            if let Some((scope, budget)) = user_budget_scope.as_ref() {
                if let Err(err) = gateway
                    .budget
                    .can_spend(scope, budget, u64::from(charge_tokens))
                {
                    gateway.observability.record_budget_exceeded();
                    return Err(map_openai_gateway_error(err));
                }
            }
        }

        let budget = key.as_ref().map(|key| key.budget.clone());
        let backends = gateway
            .router
            .select_backends_for_model_seeded("", key.as_ref(), Some(&request_id))
            .map_err(map_openai_gateway_error)?;

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
        )
    };

    #[cfg(feature = "gateway-store-redis")]
    if use_redis_budget {
        if let Some(store) = state.redis_store.as_ref() {
            if let Some(limits) = limits.as_ref() {
                if let Some(virtual_key_id) = virtual_key_id.as_deref() {
                    if let Err(err) = store
                        .check_and_consume_rate_limits(
                            virtual_key_id,
                            &rate_limit_route,
                            limits,
                            charge_tokens,
                            now_epoch_seconds,
                        )
                        .await
                    {
                        if matches!(err, GatewayError::RateLimited { .. }) {
                            let mut gateway = state.gateway.lock().await;
                            gateway.observability.record_rate_limited();
                        }
                        return Err(map_openai_gateway_error(err));
                    }
                }
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
                    if matches!(err, GatewayError::RateLimited { .. }) {
                        let mut gateway = state.gateway.lock().await;
                        gateway.observability.record_rate_limited();
                    }
                    return Err(map_openai_gateway_error(err));
                }
            }
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
            path_and_query: &path_and_query,
            model: &model,
            charge_tokens,
        };

    #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
    let (_token_budget_reserved, token_budget_reservation_ids) =
        reserve_proxy_token_budgets_for_request(budget_reservation_params).await?;
    #[cfg(not(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis")))]
    let (_token_budget_reserved, token_budget_reservation_ids): (bool, Vec<String>) =
        (false, Vec::new());

    #[cfg(all(
        feature = "gateway-costing",
        any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
    ))]
    let (cost_budget_reserved, cost_budget_reservation_ids) =
        reserve_proxy_cost_budgets_for_request(
            budget_reservation_params,
            charge_cost_usd_micros,
            &token_budget_reservation_ids,
        )
        .await?;
    #[cfg(not(all(
        feature = "gateway-costing",
        any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
    )))]
    let (cost_budget_reserved, cost_budget_reservation_ids): (bool, Vec<String>) = (false, Vec::new());

    #[cfg(all(
        not(feature = "gateway-costing"),
        any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
    ))]
    let _ = (&cost_budget_reservation_ids, cost_budget_reserved);

    #[cfg(not(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis")))]
    let _ = (
        &token_budget_reservation_ids,
        &cost_budget_reservation_ids,
        cost_budget_reserved,
    );

    let Some(backend_name) = backend_candidates.first().cloned() else {
        #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
        rollback_proxy_token_budget_reservations(&state, &token_budget_reservation_ids).await;
        #[cfg(all(
            feature = "gateway-costing",
            any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
        ))]
        rollback_proxy_cost_budget_reservations(&state, &cost_budget_reservation_ids).await;
        return Err(openai_error(
            StatusCode::BAD_GATEWAY,
            "api_error",
            Some("backend_error"),
            "no backends available",
        ));
    };

    #[cfg(feature = "gateway-translation")]
    if state.translation_backends.contains_key(&backend_name) {
        #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
        rollback_proxy_token_budget_reservations(&state, &token_budget_reservation_ids).await;
        #[cfg(all(
            feature = "gateway-costing",
            any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
        ))]
        rollback_proxy_cost_budget_reservations(&state, &cost_budget_reservation_ids).await;
        return Err(openai_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            Some("request_too_large"),
            "large multipart requests require a proxy backend (not a translation backend)",
        ));
    }

    let backend = match state.proxy_backends.get(&backend_name) {
        Some(backend) => backend.clone(),
        None => {
            #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
            rollback_proxy_token_budget_reservations(&state, &token_budget_reservation_ids).await;
            #[cfg(all(
                feature = "gateway-costing",
                any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
            ))]
            rollback_proxy_cost_budget_reservations(&state, &cost_budget_reservation_ids).await;
            return Err(openai_error(
                StatusCode::BAD_GATEWAY,
                "api_error",
                Some("backend_not_found"),
                format!("backend not found: {backend_name}"),
            ));
        }
    };

    let mut proxy_permits = match try_acquire_proxy_permits(&state, &backend_name)? {
        ProxyPermitOutcome::Acquired(permits) => permits,
        ProxyPermitOutcome::BackendRateLimited(err) => return Err(err),
    };

    let mut outgoing_headers = parts.headers.clone();
    sanitize_proxy_headers(&mut outgoing_headers, strip_authorization);
    apply_backend_headers(&mut outgoing_headers, backend.headers());
    insert_request_id(&mut outgoing_headers, &request_id);

    let data_stream = body
        .into_data_stream()
        .map(|result| result.map_err(|err| std::io::Error::other(err.to_string())));
    let outgoing_body = reqwest::Body::wrap_stream(data_stream);

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
            #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
            rollback_proxy_token_budget_reservations(&state, &token_budget_reservation_ids).await;
            #[cfg(all(
                feature = "gateway-costing",
                any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
            ))]
            rollback_proxy_cost_budget_reservations(&state, &cost_budget_reservation_ids).await;
            return Err(map_openai_gateway_error(err));
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

    #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
    if !token_budget_reservation_ids.is_empty() {
        settle_proxy_token_budget_reservations(
            &state,
            &token_budget_reservation_ids,
            spend_tokens,
            spent_tokens,
        )
        .await;
    } else if let (Some(virtual_key_id), Some(budget)) = (virtual_key_id.clone(), budget.clone()) {
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
                        gateway.budget.spend_cost_usd_micros(scope, budget, spent_cost_usd_micros);
                    }
                    if let Some((scope, budget)) = project_budget_scope.as_ref() {
                        gateway
                            .budget
                            .spend_cost_usd_micros(scope, budget, spent_cost_usd_micros);
                    }
                    if let Some((scope, budget)) = user_budget_scope.as_ref() {
                        gateway.budget.spend_cost_usd_micros(scope, budget, spent_cost_usd_micros);
                    }
                }
            }
        }
    }

    #[cfg(all(
        feature = "gateway-costing",
        any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
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
        any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
    ))]
    if !cost_budget_reserved && use_persistent_budget && spend_tokens {
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
        &state,
        upstream_response,
        backend_name,
        request_id,
        None,
        proxy_permits.take(),
    )
    .await)
}
