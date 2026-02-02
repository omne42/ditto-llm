#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
#[derive(Clone, Copy)]
struct ProxyBudgetReservationParams<'a> {
    state: &'a GatewayHttpState,
    use_persistent_budget: bool,
    virtual_key_id: Option<&'a str>,
    budget: Option<&'a super::BudgetConfig>,
    tenant_budget_scope: &'a Option<(String, super::BudgetConfig)>,
    project_budget_scope: &'a Option<(String, super::BudgetConfig)>,
    user_budget_scope: &'a Option<(String, super::BudgetConfig)>,
    request_id: &'a str,
    path_and_query: &'a str,
    model: &'a Option<String>,
    charge_tokens: u32,
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
async fn reserve_proxy_token_budgets_for_request(
    params: ProxyBudgetReservationParams<'_>,
) -> Result<(bool, Vec<String>), (StatusCode, Json<OpenAiErrorResponse>)> {
    let ProxyBudgetReservationParams {
        state,
        use_persistent_budget,
        virtual_key_id,
        budget,
        tenant_budget_scope,
        project_budget_scope,
        user_budget_scope,
        request_id,
        path_and_query,
        model,
        charge_tokens,
    } = params;

    let token_budget_reserved = if use_persistent_budget {
        if let (Some(virtual_key_id), Some(budget)) = (virtual_key_id, budget) {
            if let Some(limit) = budget.total_tokens {
                #[cfg(feature = "gateway-store-sqlite")]
                {
                    if let Some(store) = state.sqlite_store.as_ref() {
                        match store
                            .reserve_budget_tokens(
                                request_id,
                                virtual_key_id,
                                limit,
                                u64::from(charge_tokens),
                            )
                            .await
                        {
                            Ok(()) => true,
                            Err(SqliteStoreError::BudgetExceeded { limit, attempted }) => {
                                let _ = store
                                    .append_audit_log(
                                        "proxy.blocked",
                                        serde_json::json!({
                                            "request_id": request_id,
                                            "virtual_key_id": virtual_key_id,
                                            "reason": "budget_exceeded",
                                            "limit": limit,
                                            "attempted": attempted,
                                            "charge_tokens": charge_tokens,
                                            "path": path_and_query,
                                            "model": model,
                                        }),
                                    )
                                    .await;
                                emit_json_log(
                                    state,
                                    "proxy.blocked",
                                    serde_json::json!({
                                        "request_id": request_id,
                                        "virtual_key_id": virtual_key_id,
                                        "reason": "budget_exceeded",
                                        "limit": limit,
                                        "attempted": attempted,
                                    }),
                                );
                                return Err(map_openai_gateway_error(GatewayError::BudgetExceeded {
                                    limit,
                                    attempted,
                                }));
                            }
                            Err(err) => {
                                let _ = store
                                    .append_audit_log(
                                        "proxy.blocked",
                                        serde_json::json!({
                                            "request_id": request_id,
                                            "virtual_key_id": virtual_key_id,
                                            "reason": "storage_error",
                                            "error": err.to_string(),
                                            "path": path_and_query,
                                            "model": model,
                                        }),
                                    )
                                    .await;
                                return Err(openai_error(
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    "api_error",
                                    Some("storage_error"),
                                    err.to_string(),
                                ));
                            }
                        }
                    } else {
                        #[cfg(feature = "gateway-store-redis")]
                        {
                            if let Some(store) = state.redis_store.as_ref() {
                                match store
                                    .reserve_budget_tokens(
                                        request_id,
                                        virtual_key_id,
                                        limit,
                                        u64::from(charge_tokens),
                                    )
                                    .await
                                {
                                    Ok(()) => true,
                                    Err(RedisStoreError::BudgetExceeded { limit, attempted }) => {
                                        let _ = store
                                            .append_audit_log(
                                                "proxy.blocked",
                                                serde_json::json!({
                                                    "request_id": request_id,
                                                    "virtual_key_id": virtual_key_id,
                                                    "reason": "budget_exceeded",
                                                    "limit": limit,
                                                    "attempted": attempted,
                                                    "charge_tokens": charge_tokens,
                                                    "path": path_and_query,
                                                    "model": model,
                                                }),
                                            )
                                            .await;
                                        emit_json_log(
                                            state,
                                            "proxy.blocked",
                                            serde_json::json!({
                                                "request_id": request_id,
                                                "virtual_key_id": virtual_key_id,
                                                "reason": "budget_exceeded",
                                                "limit": limit,
                                                "attempted": attempted,
                                            }),
                                        );
                                        return Err(map_openai_gateway_error(
                                            GatewayError::BudgetExceeded { limit, attempted },
                                        ));
                                    }
                                    Err(err) => {
                                        let _ = store
                                            .append_audit_log(
                                                "proxy.blocked",
                                                serde_json::json!({
                                                    "request_id": request_id,
                                                    "virtual_key_id": virtual_key_id,
                                                    "reason": "storage_error",
                                                    "error": err.to_string(),
                                                    "path": path_and_query,
                                                    "model": model,
                                                }),
                                            )
                                            .await;
                                        return Err(openai_error(
                                            StatusCode::INTERNAL_SERVER_ERROR,
                                            "api_error",
                                            Some("storage_error"),
                                            err.to_string(),
                                        ));
                                    }
                                }
                            } else {
                                false
                            }
                        }
                        #[cfg(not(feature = "gateway-store-redis"))]
                        {
                            false
                        }
                    }
                }
                #[cfg(not(feature = "gateway-store-sqlite"))]
                {
                    #[cfg(feature = "gateway-store-redis")]
                    {
                        if let Some(store) = state.redis_store.as_ref() {
                            match store
                                .reserve_budget_tokens(
                                    request_id,
                                    virtual_key_id,
                                    limit,
                                    u64::from(charge_tokens),
                                )
                                .await
                            {
                                Ok(()) => true,
                                Err(RedisStoreError::BudgetExceeded { limit, attempted }) => {
                                    let _ = store
                                        .append_audit_log(
                                            "proxy.blocked",
                                            serde_json::json!({
                                                "request_id": request_id,
                                                "virtual_key_id": virtual_key_id,
                                                "reason": "budget_exceeded",
                                                "limit": limit,
                                                "attempted": attempted,
                                                "charge_tokens": charge_tokens,
                                                "path": path_and_query,
                                                "model": model,
                                            }),
                                        )
                                        .await;
                                    emit_json_log(
                                        state,
                                        "proxy.blocked",
                                        serde_json::json!({
                                            "request_id": request_id,
                                            "virtual_key_id": virtual_key_id,
                                            "reason": "budget_exceeded",
                                            "limit": limit,
                                            "attempted": attempted,
                                        }),
                                    );
                                    return Err(map_openai_gateway_error(
                                        GatewayError::BudgetExceeded { limit, attempted },
                                    ));
                                }
                                Err(err) => {
                                    let _ = store
                                        .append_audit_log(
                                            "proxy.blocked",
                                            serde_json::json!({
                                                "request_id": request_id,
                                                "virtual_key_id": virtual_key_id,
                                                "reason": "storage_error",
                                                "error": err.to_string(),
                                                "path": path_and_query,
                                                "model": model,
                                            }),
                                        )
                                        .await;
                                    return Err(openai_error(
                                        StatusCode::INTERNAL_SERVER_ERROR,
                                        "api_error",
                                        Some("storage_error"),
                                        err.to_string(),
                                    ));
                                }
                            }
                        } else {
                            false
                        }
                    }
                    #[cfg(not(feature = "gateway-store-redis"))]
                    {
                        false
                    }
                }
            } else {
                false
            }
        } else {
            false
        }
    } else {
        false
    };

    let mut token_budget_reservation_ids: Vec<String> = Vec::new();
    if token_budget_reserved {
        token_budget_reservation_ids.push(request_id.to_string());
    }

    if use_persistent_budget {
        if let Some(virtual_key_id) = virtual_key_id {
            if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                if let Some(limit) = budget.total_tokens {
                    let reservation_id = format!("{request_id}::budget::{scope}");
                    let ctx = ProxyBudgetReservationContext {
                        state,
                        reservation_id: &reservation_id,
                        budget_scope: scope,
                        request_id,
                        virtual_key_id,
                        path_and_query,
                        model,
                    };
                    if let Err(err) = reserve_proxy_token_budget(ctx, limit, charge_tokens).await {
                        rollback_proxy_token_budget_reservations(
                            state,
                            &token_budget_reservation_ids,
                        )
                        .await;
                        return Err(err);
                    }
                    token_budget_reservation_ids.push(reservation_id);
                }
            }

            if let Some((scope, budget)) = project_budget_scope.as_ref() {
                if let Some(limit) = budget.total_tokens {
                    let reservation_id = format!("{request_id}::budget::{scope}");
                    let ctx = ProxyBudgetReservationContext {
                        state,
                        reservation_id: &reservation_id,
                        budget_scope: scope,
                        request_id,
                        virtual_key_id,
                        path_and_query,
                        model,
                    };
                    if let Err(err) = reserve_proxy_token_budget(ctx, limit, charge_tokens).await {
                        rollback_proxy_token_budget_reservations(
                            state,
                            &token_budget_reservation_ids,
                        )
                        .await;
                        return Err(err);
                    }
                    token_budget_reservation_ids.push(reservation_id);
                }
            }

            if let Some((scope, budget)) = user_budget_scope.as_ref() {
                if let Some(limit) = budget.total_tokens {
                    let reservation_id = format!("{request_id}::budget::{scope}");
                    let ctx = ProxyBudgetReservationContext {
                        state,
                        reservation_id: &reservation_id,
                        budget_scope: scope,
                        request_id,
                        virtual_key_id,
                        path_and_query,
                        model,
                    };
                    if let Err(err) = reserve_proxy_token_budget(ctx, limit, charge_tokens).await {
                        rollback_proxy_token_budget_reservations(
                            state,
                            &token_budget_reservation_ids,
                        )
                        .await;
                        return Err(err);
                    }
                    token_budget_reservation_ids.push(reservation_id);
                }
            }
        }
    }

    Ok((token_budget_reserved, token_budget_reservation_ids))
}

#[cfg(all(
    feature = "gateway-costing",
    any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
))]
async fn reserve_proxy_cost_budgets_for_request(
    params: ProxyBudgetReservationParams<'_>,
    charge_cost_usd_micros: Option<u64>,
    token_budget_reservation_ids: &[String],
) -> Result<(bool, Vec<String>), (StatusCode, Json<OpenAiErrorResponse>)> {
    let ProxyBudgetReservationParams {
        state,
        use_persistent_budget,
        virtual_key_id,
        budget,
        tenant_budget_scope,
        project_budget_scope,
        user_budget_scope,
        request_id,
        path_and_query,
        model,
        charge_tokens: _,
    } = params;

    let cost_budget_reserved = if use_persistent_budget {
        if let (Some(virtual_key_id), Some(budget)) = (virtual_key_id, budget) {
            if let Some(limit_usd_micros) = budget.total_usd_micros {
                let Some(charge_cost_usd_micros) = charge_cost_usd_micros else {
                    rollback_proxy_token_budget_reservations(state, token_budget_reservation_ids)
                        .await;
                    return Err(openai_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "api_error",
                        Some("pricing_not_configured"),
                        "pricing not configured for cost budgets",
                    ));
                };

                #[cfg(feature = "gateway-store-sqlite")]
                {
                    if let Some(store) = state.sqlite_store.as_ref() {
                        match store
                            .reserve_cost_usd_micros(
                                request_id,
                                virtual_key_id,
                                limit_usd_micros,
                                charge_cost_usd_micros,
                            )
                            .await
                        {
                            Ok(()) => true,
                            Err(SqliteStoreError::CostBudgetExceeded {
                                limit_usd_micros,
                                attempted_usd_micros,
                            }) => {
                                rollback_proxy_token_budget_reservations(
                                    state,
                                    token_budget_reservation_ids,
                                )
                                .await;
                                let _ = store
                                    .append_audit_log(
                                        "proxy.blocked",
                                        serde_json::json!({
                                            "request_id": request_id,
                                            "virtual_key_id": virtual_key_id,
                                            "reason": "cost_budget_exceeded",
                                            "limit_usd_micros": limit_usd_micros,
                                            "attempted_usd_micros": attempted_usd_micros,
                                            "charge_cost_usd_micros": charge_cost_usd_micros,
                                            "path": path_and_query,
                                            "model": model,
                                        }),
                                    )
                                    .await;
                                emit_json_log(
                                    state,
                                    "proxy.blocked",
                                    serde_json::json!({
                                        "request_id": request_id,
                                        "virtual_key_id": virtual_key_id,
                                        "reason": "cost_budget_exceeded",
                                        "limit_usd_micros": limit_usd_micros,
                                        "attempted_usd_micros": attempted_usd_micros,
                                    }),
                                );
                                return Err(map_openai_gateway_error(
                                    GatewayError::CostBudgetExceeded {
                                        limit_usd_micros,
                                        attempted_usd_micros,
                                    },
                                ));
                            }
                            Err(err) => {
                                rollback_proxy_token_budget_reservations(
                                    state,
                                    token_budget_reservation_ids,
                                )
                                .await;
                                let _ = store
                                    .append_audit_log(
                                        "proxy.blocked",
                                        serde_json::json!({
                                            "request_id": request_id,
                                            "virtual_key_id": virtual_key_id,
                                            "reason": "storage_error",
                                            "error": err.to_string(),
                                            "path": path_and_query,
                                            "model": model,
                                        }),
                                    )
                                    .await;
                                return Err(openai_error(
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    "api_error",
                                    Some("storage_error"),
                                    err.to_string(),
                                ));
                            }
                        }
                    } else {
                        #[cfg(feature = "gateway-store-redis")]
                        {
                            if let Some(store) = state.redis_store.as_ref() {
                                match store
                                    .reserve_cost_usd_micros(
                                        request_id,
                                        virtual_key_id,
                                        limit_usd_micros,
                                        charge_cost_usd_micros,
                                    )
                                    .await
                                {
                                    Ok(()) => true,
                                    Err(RedisStoreError::CostBudgetExceeded {
                                        limit_usd_micros,
                                        attempted_usd_micros,
                                    }) => {
                                        rollback_proxy_token_budget_reservations(
                                            state,
                                            token_budget_reservation_ids,
                                        )
                                        .await;
                                        let _ = store
                                            .append_audit_log(
                                                "proxy.blocked",
                                                serde_json::json!({
                                                    "request_id": request_id,
                                                    "virtual_key_id": virtual_key_id,
                                                    "reason": "cost_budget_exceeded",
                                                    "limit_usd_micros": limit_usd_micros,
                                                    "attempted_usd_micros": attempted_usd_micros,
                                                    "charge_cost_usd_micros": charge_cost_usd_micros,
                                                    "path": path_and_query,
                                                    "model": model,
                                                }),
                                            )
                                            .await;
                                        emit_json_log(
                                            state,
                                            "proxy.blocked",
                                            serde_json::json!({
                                                "request_id": request_id,
                                                "virtual_key_id": virtual_key_id,
                                                "reason": "cost_budget_exceeded",
                                                "limit_usd_micros": limit_usd_micros,
                                                "attempted_usd_micros": attempted_usd_micros,
                                            }),
                                        );
                                        return Err(map_openai_gateway_error(
                                            GatewayError::CostBudgetExceeded {
                                                limit_usd_micros,
                                                attempted_usd_micros,
                                            },
                                        ));
                                    }
                                    Err(err) => {
                                        rollback_proxy_token_budget_reservations(
                                            state,
                                            token_budget_reservation_ids,
                                        )
                                        .await;
                                        let _ = store
                                            .append_audit_log(
                                                "proxy.blocked",
                                                serde_json::json!({
                                                    "request_id": request_id,
                                                    "virtual_key_id": virtual_key_id,
                                                    "reason": "storage_error",
                                                    "error": err.to_string(),
                                                    "path": path_and_query,
                                                    "model": model,
                                                }),
                                            )
                                            .await;
                                        return Err(openai_error(
                                            StatusCode::INTERNAL_SERVER_ERROR,
                                            "api_error",
                                            Some("storage_error"),
                                            err.to_string(),
                                        ));
                                    }
                                }
                            } else {
                                false
                            }
                        }
                        #[cfg(not(feature = "gateway-store-redis"))]
                        {
                            false
                        }
                    }
                }
                #[cfg(not(feature = "gateway-store-sqlite"))]
                {
                    #[cfg(feature = "gateway-store-redis")]
                    {
                        if let Some(store) = state.redis_store.as_ref() {
                            match store
                                .reserve_cost_usd_micros(
                                    request_id,
                                    virtual_key_id,
                                    limit_usd_micros,
                                    charge_cost_usd_micros,
                                )
                                .await
                            {
                                Ok(()) => true,
                                Err(RedisStoreError::CostBudgetExceeded {
                                    limit_usd_micros,
                                    attempted_usd_micros,
                                }) => {
                                    rollback_proxy_token_budget_reservations(
                                        state,
                                        token_budget_reservation_ids,
                                    )
                                    .await;
                                    let _ = store
                                        .append_audit_log(
                                            "proxy.blocked",
                                            serde_json::json!({
                                                "request_id": request_id,
                                                "virtual_key_id": virtual_key_id,
                                                "reason": "cost_budget_exceeded",
                                                "limit_usd_micros": limit_usd_micros,
                                                "attempted_usd_micros": attempted_usd_micros,
                                                "charge_cost_usd_micros": charge_cost_usd_micros,
                                                "path": path_and_query,
                                                "model": model,
                                            }),
                                        )
                                        .await;
                                    emit_json_log(
                                        state,
                                        "proxy.blocked",
                                        serde_json::json!({
                                            "request_id": request_id,
                                            "virtual_key_id": virtual_key_id,
                                            "reason": "cost_budget_exceeded",
                                            "limit_usd_micros": limit_usd_micros,
                                            "attempted_usd_micros": attempted_usd_micros,
                                        }),
                                    );
                                    return Err(map_openai_gateway_error(
                                        GatewayError::CostBudgetExceeded {
                                            limit_usd_micros,
                                            attempted_usd_micros,
                                        },
                                    ));
                                }
                                Err(err) => {
                                    rollback_proxy_token_budget_reservations(
                                        state,
                                        token_budget_reservation_ids,
                                    )
                                    .await;
                                    let _ = store
                                        .append_audit_log(
                                            "proxy.blocked",
                                            serde_json::json!({
                                                "request_id": request_id,
                                                "virtual_key_id": virtual_key_id,
                                                "reason": "storage_error",
                                                "error": err.to_string(),
                                                "path": path_and_query,
                                                "model": model,
                                            }),
                                        )
                                        .await;
                                    return Err(openai_error(
                                        StatusCode::INTERNAL_SERVER_ERROR,
                                        "api_error",
                                        Some("storage_error"),
                                        err.to_string(),
                                    ));
                                }
                            }
                        } else {
                            false
                        }
                    }
                    #[cfg(not(feature = "gateway-store-redis"))]
                    {
                        false
                    }
                }
            } else {
                false
            }
        } else {
            false
        }
    } else {
        false
    };

    let mut cost_budget_reservation_ids: Vec<String> = Vec::new();
    if cost_budget_reserved {
        cost_budget_reservation_ids.push(request_id.to_string());
    }

    if use_persistent_budget {
        if let Some(virtual_key_id) = virtual_key_id {
            let mut cost_scopes: Vec<(String, u64)> = Vec::new();
            if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                if let Some(limit) = budget.total_usd_micros {
                    cost_scopes.push((scope.clone(), limit));
                }
            }
            if let Some((scope, budget)) = project_budget_scope.as_ref() {
                if let Some(limit) = budget.total_usd_micros {
                    cost_scopes.push((scope.clone(), limit));
                }
            }
            if let Some((scope, budget)) = user_budget_scope.as_ref() {
                if let Some(limit) = budget.total_usd_micros {
                    cost_scopes.push((scope.clone(), limit));
                }
            }

            if !cost_scopes.is_empty() {
                let Some(charge_cost_usd_micros) = charge_cost_usd_micros else {
                    rollback_proxy_cost_budget_reservations(state, &cost_budget_reservation_ids)
                        .await;
                    rollback_proxy_token_budget_reservations(state, token_budget_reservation_ids)
                        .await;
                    return Err(openai_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "api_error",
                        Some("pricing_not_configured"),
                        "pricing not configured for cost budgets",
                    ));
                };

                for (scope, limit_usd_micros) in cost_scopes {
                    let reservation_id = format!("{request_id}::cost::{scope}");
                    let ctx = ProxyBudgetReservationContext {
                        state,
                        reservation_id: &reservation_id,
                        budget_scope: &scope,
                        request_id,
                        virtual_key_id,
                        path_and_query,
                        model,
                    };
                    if let Err(err) =
                        reserve_proxy_cost_budget(ctx, limit_usd_micros, charge_cost_usd_micros)
                            .await
                    {
                        rollback_proxy_cost_budget_reservations(state, &cost_budget_reservation_ids)
                            .await;
                        rollback_proxy_token_budget_reservations(state, token_budget_reservation_ids)
                            .await;
                        return Err(err);
                    }
                    cost_budget_reservation_ids.push(reservation_id);
                }
            }
        }
    }

    Ok((cost_budget_reserved, cost_budget_reservation_ids))
}
