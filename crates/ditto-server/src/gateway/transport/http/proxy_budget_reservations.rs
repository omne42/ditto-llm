use super::*;

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
#[derive(Clone, Copy)]
pub(super) struct ProxyBudgetReservationParams<'a> {
    pub(super) state: &'a GatewayHttpState,
    pub(super) use_persistent_budget: bool,
    pub(super) virtual_key_id: Option<&'a str>,
    pub(super) budget: Option<&'a super::BudgetConfig>,
    pub(super) tenant_budget_scope: &'a Option<(String, super::BudgetConfig)>,
    pub(super) project_budget_scope: &'a Option<(String, super::BudgetConfig)>,
    pub(super) user_budget_scope: &'a Option<(String, super::BudgetConfig)>,
    pub(super) request_id: &'a str,
    pub(super) path_and_query: &'a str,
    pub(super) model: &'a Option<String>,
    pub(super) charge_tokens: u32,
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
pub(super) async fn reserve_proxy_token_budgets_for_request(
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
                let ctx = ProxyBudgetReservationContext {
                    state,
                    reservation_id: request_id,
                    budget_scope: virtual_key_id,
                    request_id,
                    virtual_key_id,
                    path_and_query,
                    model,
                };
                reserve_proxy_token_budget(ctx, limit, charge_tokens).await?;
                true
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

    if use_persistent_budget && let Some(virtual_key_id) = virtual_key_id {
        if let Some((scope, budget)) = tenant_budget_scope.as_ref()
            && let Some(limit) = budget.total_tokens
        {
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
                rollback_proxy_token_budget_reservations(state, &token_budget_reservation_ids)
                    .await;
                return Err(err);
            }
            token_budget_reservation_ids.push(reservation_id);
        }

        if let Some((scope, budget)) = project_budget_scope.as_ref()
            && let Some(limit) = budget.total_tokens
        {
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
                rollback_proxy_token_budget_reservations(state, &token_budget_reservation_ids)
                    .await;
                return Err(err);
            }
            token_budget_reservation_ids.push(reservation_id);
        }

        if let Some((scope, budget)) = user_budget_scope.as_ref()
            && let Some(limit) = budget.total_tokens
        {
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
                rollback_proxy_token_budget_reservations(state, &token_budget_reservation_ids)
                    .await;
                return Err(err);
            }
            token_budget_reservation_ids.push(reservation_id);
        }
    }

    Ok((token_budget_reserved, token_budget_reservation_ids))
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
pub(super) async fn reserve_proxy_cost_budgets_for_request(
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

                let ctx = ProxyBudgetReservationContext {
                    state,
                    reservation_id: request_id,
                    budget_scope: virtual_key_id,
                    request_id,
                    virtual_key_id,
                    path_and_query,
                    model,
                };
                if let Err(err) =
                    reserve_proxy_cost_budget(ctx, limit_usd_micros, charge_cost_usd_micros).await
                {
                    rollback_proxy_token_budget_reservations(state, token_budget_reservation_ids)
                        .await;
                    return Err(err);
                }
                true
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

    if use_persistent_budget && let Some(virtual_key_id) = virtual_key_id {
        let mut cost_scopes: Vec<(String, u64)> = Vec::new();
        if let Some((scope, budget)) = tenant_budget_scope.as_ref()
            && let Some(limit) = budget.total_usd_micros
        {
            cost_scopes.push((scope.clone(), limit));
        }
        if let Some((scope, budget)) = project_budget_scope.as_ref()
            && let Some(limit) = budget.total_usd_micros
        {
            cost_scopes.push((scope.clone(), limit));
        }
        if let Some((scope, budget)) = user_budget_scope.as_ref()
            && let Some(limit) = budget.total_usd_micros
        {
            cost_scopes.push((scope.clone(), limit));
        }

        if !cost_scopes.is_empty() {
            let Some(charge_cost_usd_micros) = charge_cost_usd_micros else {
                rollback_proxy_cost_budget_reservations(state, &cost_budget_reservation_ids).await;
                rollback_proxy_token_budget_reservations(state, token_budget_reservation_ids).await;
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
                    reserve_proxy_cost_budget(ctx, limit_usd_micros, charge_cost_usd_micros).await
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

    Ok((cost_budget_reserved, cost_budget_reservation_ids))
}
// end inline: proxy/budget_reservations.rs
// inlined from proxy/budget_reservation.rs
#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
#[derive(Clone, Copy)]
struct ProxyBudgetReservationContext<'a> {
    state: &'a GatewayHttpState,
    reservation_id: &'a str,
    budget_scope: &'a str,
    request_id: &'a str,
    virtual_key_id: &'a str,
    path_and_query: &'a str,
    model: &'a Option<String>,
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
async fn reserve_proxy_token_budget(
    ctx: ProxyBudgetReservationContext<'_>,
    limit: u64,
    charge_tokens: u32,
) -> Result<(), (StatusCode, Json<OpenAiErrorResponse>)> {
    let ProxyBudgetReservationContext {
        state,
        reservation_id,
        budget_scope,
        request_id,
        virtual_key_id,
        path_and_query,
        model,
    } = ctx;
    let charge_tokens_u64 = u64::from(charge_tokens);

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.stores.sqlite.as_ref() {
        match store
            .reserve_budget_tokens(reservation_id, budget_scope, limit, charge_tokens_u64)
            .await
        {
            Ok(()) => return Ok(()),
            Err(SqliteStoreError::BudgetExceeded { limit, attempted }) => {
                let _ = append_audit_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
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
                        "budget_scope": budget_scope,
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
                let _ = append_audit_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
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
    }

    #[cfg(feature = "gateway-store-postgres")]
    if let Some(store) = state.stores.postgres.as_ref() {
        match store
            .reserve_budget_tokens(reservation_id, budget_scope, limit, charge_tokens_u64)
            .await
        {
            Ok(()) => return Ok(()),
            Err(PostgresStoreError::BudgetExceeded { limit, attempted }) => {
                let _ = append_audit_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
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
                        "budget_scope": budget_scope,
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
                let _ = append_audit_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
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
    }

    #[cfg(feature = "gateway-store-mysql")]
    if let Some(store) = state.stores.mysql.as_ref() {
        match store
            .reserve_budget_tokens(reservation_id, budget_scope, limit, charge_tokens_u64)
            .await
        {
            Ok(()) => return Ok(()),
            Err(MySqlStoreError::BudgetExceeded { limit, attempted }) => {
                let _ = append_audit_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
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
                        "budget_scope": budget_scope,
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
                let _ = append_audit_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
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
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.stores.redis.as_ref() {
        match store
            .reserve_budget_tokens(reservation_id, budget_scope, limit, charge_tokens_u64)
            .await
        {
            Ok(()) => return Ok(()),
            Err(RedisStoreError::BudgetExceeded { limit, attempted }) => {
                let _ = append_audit_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
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
                        "budget_scope": budget_scope,
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
                let _ = append_audit_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
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
    }

    Ok(())
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
fn budget_storage_error(operation: &str, target: &str, err: impl std::fmt::Display) -> String {
    format!("{operation} failed for {target}: {err}")
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
fn report_budget_storage_error(state: &GatewayHttpState, operation: &str, target: &str, err: &str) {
    emit_json_log(
        state,
        "proxy.storage_error",
        serde_json::json!({
            "operation": operation,
            "target": target,
            "error": err,
        }),
    );
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
pub(super) async fn rollback_proxy_token_budget_reservations_checked(
    state: &GatewayHttpState,
    reservation_ids: &[String],
) -> Result<(), String> {
    for reservation_id in reservation_ids {
        #[cfg(feature = "gateway-store-sqlite")]
        if let Some(store) = state.stores.sqlite.as_ref() {
            store
                .rollback_budget_reservation(reservation_id)
                .await
                .map_err(|err| {
                    budget_storage_error("rollback_budget_reservation", reservation_id, err)
                })?;
        }
        #[cfg(feature = "gateway-store-postgres")]
        if let Some(store) = state.stores.postgres.as_ref() {
            store
                .rollback_budget_reservation(reservation_id)
                .await
                .map_err(|err| {
                    budget_storage_error("rollback_budget_reservation", reservation_id, err)
                })?;
        }
        #[cfg(feature = "gateway-store-mysql")]
        if let Some(store) = state.stores.mysql.as_ref() {
            store
                .rollback_budget_reservation(reservation_id)
                .await
                .map_err(|err| {
                    budget_storage_error("rollback_budget_reservation", reservation_id, err)
                })?;
        }
        #[cfg(feature = "gateway-store-redis")]
        if let Some(store) = state.stores.redis.as_ref() {
            store
                .rollback_budget_reservation(reservation_id)
                .await
                .map_err(|err| {
                    budget_storage_error("rollback_budget_reservation", reservation_id, err)
                })?;
        }
    }
    Ok(())
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
pub(super) async fn rollback_proxy_token_budget_reservations(
    state: &GatewayHttpState,
    reservation_ids: &[String],
) {
    if let Err(err) = rollback_proxy_token_budget_reservations_checked(state, reservation_ids).await
    {
        report_budget_storage_error(state, "rollback_budget_reservation", "budget_tokens", &err);
    }
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
pub(super) async fn settle_proxy_token_budget_reservations_checked(
    state: &GatewayHttpState,
    reservation_ids: &[String],
    spend_tokens: bool,
    spent_tokens: u64,
) -> Result<(), String> {
    if reservation_ids.is_empty() {
        return Ok(());
    }
    for reservation_id in reservation_ids {
        #[cfg(feature = "gateway-store-sqlite")]
        if let Some(store) = state.stores.sqlite.as_ref() {
            if spend_tokens {
                store
                    .commit_budget_reservation_with_tokens(reservation_id, spent_tokens)
                    .await
                    .map_err(|err| {
                        budget_storage_error(
                            "commit_budget_reservation_with_tokens",
                            reservation_id,
                            err,
                        )
                    })?;
            } else {
                store
                    .rollback_budget_reservation(reservation_id)
                    .await
                    .map_err(|err| {
                        budget_storage_error("rollback_budget_reservation", reservation_id, err)
                    })?;
            }
        }
        #[cfg(feature = "gateway-store-postgres")]
        if let Some(store) = state.stores.postgres.as_ref() {
            if spend_tokens {
                store
                    .commit_budget_reservation_with_tokens(reservation_id, spent_tokens)
                    .await
                    .map_err(|err| {
                        budget_storage_error(
                            "commit_budget_reservation_with_tokens",
                            reservation_id,
                            err,
                        )
                    })?;
            } else {
                store
                    .rollback_budget_reservation(reservation_id)
                    .await
                    .map_err(|err| {
                        budget_storage_error("rollback_budget_reservation", reservation_id, err)
                    })?;
            }
        }
        #[cfg(feature = "gateway-store-mysql")]
        if let Some(store) = state.stores.mysql.as_ref() {
            if spend_tokens {
                store
                    .commit_budget_reservation_with_tokens(reservation_id, spent_tokens)
                    .await
                    .map_err(|err| {
                        budget_storage_error(
                            "commit_budget_reservation_with_tokens",
                            reservation_id,
                            err,
                        )
                    })?;
            } else {
                store
                    .rollback_budget_reservation(reservation_id)
                    .await
                    .map_err(|err| {
                        budget_storage_error("rollback_budget_reservation", reservation_id, err)
                    })?;
            }
        }
        #[cfg(feature = "gateway-store-redis")]
        if let Some(store) = state.stores.redis.as_ref() {
            if spend_tokens {
                store
                    .commit_budget_reservation_with_tokens(reservation_id, spent_tokens)
                    .await
                    .map_err(|err| {
                        budget_storage_error(
                            "commit_budget_reservation_with_tokens",
                            reservation_id,
                            err,
                        )
                    })?;
            } else {
                store
                    .rollback_budget_reservation(reservation_id)
                    .await
                    .map_err(|err| {
                        budget_storage_error("rollback_budget_reservation", reservation_id, err)
                    })?;
            }
        }
    }
    Ok(())
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
async fn reserve_proxy_cost_budget(
    ctx: ProxyBudgetReservationContext<'_>,
    limit_usd_micros: u64,
    charge_cost_usd_micros: u64,
) -> Result<(), (StatusCode, Json<OpenAiErrorResponse>)> {
    let ProxyBudgetReservationContext {
        state,
        reservation_id,
        budget_scope,
        request_id,
        virtual_key_id,
        path_and_query,
        model,
    } = ctx;
    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.stores.sqlite.as_ref() {
        match store
            .reserve_cost_usd_micros(
                reservation_id,
                budget_scope,
                limit_usd_micros,
                charge_cost_usd_micros,
            )
            .await
        {
            Ok(()) => return Ok(()),
            Err(SqliteStoreError::CostBudgetExceeded {
                limit_usd_micros,
                attempted_usd_micros,
            }) => {
                let _ = append_audit_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
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
                        "budget_scope": budget_scope,
                        "reason": "cost_budget_exceeded",
                        "limit_usd_micros": limit_usd_micros,
                        "attempted_usd_micros": attempted_usd_micros,
                    }),
                );
                return Err(map_openai_gateway_error(GatewayError::CostBudgetExceeded {
                    limit_usd_micros,
                    attempted_usd_micros,
                }));
            }
            Err(err) => {
                let _ = append_audit_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
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
    }

    #[cfg(feature = "gateway-store-postgres")]
    if let Some(store) = state.stores.postgres.as_ref() {
        match store
            .reserve_cost_usd_micros(
                reservation_id,
                budget_scope,
                limit_usd_micros,
                charge_cost_usd_micros,
            )
            .await
        {
            Ok(()) => return Ok(()),
            Err(PostgresStoreError::CostBudgetExceeded {
                limit_usd_micros,
                attempted_usd_micros,
            }) => {
                let _ = append_audit_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
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
                        "budget_scope": budget_scope,
                        "reason": "cost_budget_exceeded",
                        "limit_usd_micros": limit_usd_micros,
                        "attempted_usd_micros": attempted_usd_micros,
                    }),
                );
                return Err(map_openai_gateway_error(GatewayError::CostBudgetExceeded {
                    limit_usd_micros,
                    attempted_usd_micros,
                }));
            }
            Err(err) => {
                let _ = append_audit_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
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
    }

    #[cfg(feature = "gateway-store-mysql")]
    if let Some(store) = state.stores.mysql.as_ref() {
        match store
            .reserve_cost_usd_micros(
                reservation_id,
                budget_scope,
                limit_usd_micros,
                charge_cost_usd_micros,
            )
            .await
        {
            Ok(()) => return Ok(()),
            Err(MySqlStoreError::CostBudgetExceeded {
                limit_usd_micros,
                attempted_usd_micros,
            }) => {
                let _ = append_audit_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
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
                        "budget_scope": budget_scope,
                        "reason": "cost_budget_exceeded",
                        "limit_usd_micros": limit_usd_micros,
                        "attempted_usd_micros": attempted_usd_micros,
                    }),
                );
                return Err(map_openai_gateway_error(GatewayError::CostBudgetExceeded {
                    limit_usd_micros,
                    attempted_usd_micros,
                }));
            }
            Err(err) => {
                let _ = append_audit_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
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
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.stores.redis.as_ref() {
        match store
            .reserve_cost_usd_micros(
                reservation_id,
                budget_scope,
                limit_usd_micros,
                charge_cost_usd_micros,
            )
            .await
        {
            Ok(()) => return Ok(()),
            Err(RedisStoreError::CostBudgetExceeded {
                limit_usd_micros,
                attempted_usd_micros,
            }) => {
                let _ = append_audit_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
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
                        "budget_scope": budget_scope,
                        "reason": "cost_budget_exceeded",
                        "limit_usd_micros": limit_usd_micros,
                        "attempted_usd_micros": attempted_usd_micros,
                    }),
                );
                return Err(map_openai_gateway_error(GatewayError::CostBudgetExceeded {
                    limit_usd_micros,
                    attempted_usd_micros,
                }));
            }
            Err(err) => {
                let _ = append_audit_log(
                    state,
                    "proxy.blocked",
                    serde_json::json!({
                        "request_id": request_id,
                        "virtual_key_id": virtual_key_id,
                        "budget_scope": budget_scope,
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
    }

    Ok(())
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
pub(super) async fn rollback_proxy_cost_budget_reservations_checked(
    state: &GatewayHttpState,
    reservation_ids: &[String],
) -> Result<(), String> {
    for reservation_id in reservation_ids {
        #[cfg(feature = "gateway-store-sqlite")]
        if let Some(store) = state.stores.sqlite.as_ref() {
            store
                .rollback_cost_reservation(reservation_id)
                .await
                .map_err(|err| {
                    budget_storage_error("rollback_cost_reservation", reservation_id, err)
                })?;
        }
        #[cfg(feature = "gateway-store-postgres")]
        if let Some(store) = state.stores.postgres.as_ref() {
            store
                .rollback_cost_reservation(reservation_id)
                .await
                .map_err(|err| {
                    budget_storage_error("rollback_cost_reservation", reservation_id, err)
                })?;
        }
        #[cfg(feature = "gateway-store-mysql")]
        if let Some(store) = state.stores.mysql.as_ref() {
            store
                .rollback_cost_reservation(reservation_id)
                .await
                .map_err(|err| {
                    budget_storage_error("rollback_cost_reservation", reservation_id, err)
                })?;
        }
        #[cfg(feature = "gateway-store-redis")]
        if let Some(store) = state.stores.redis.as_ref() {
            store
                .rollback_cost_reservation(reservation_id)
                .await
                .map_err(|err| {
                    budget_storage_error("rollback_cost_reservation", reservation_id, err)
                })?;
        }
    }
    Ok(())
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
pub(super) async fn rollback_proxy_cost_budget_reservations(
    state: &GatewayHttpState,
    reservation_ids: &[String],
) {
    if let Err(err) = rollback_proxy_cost_budget_reservations_checked(state, reservation_ids).await
    {
        report_budget_storage_error(state, "rollback_cost_reservation", "budget_cost", &err);
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
pub(super) async fn settle_proxy_cost_budget_reservations_checked(
    state: &GatewayHttpState,
    reservation_ids: &[String],
    spend_tokens: bool,
    spent_cost_usd_micros: u64,
) -> Result<(), String> {
    if reservation_ids.is_empty() {
        return Ok(());
    }

    for reservation_id in reservation_ids {
        #[cfg(feature = "gateway-store-sqlite")]
        if let Some(store) = state.stores.sqlite.as_ref() {
            if spend_tokens {
                store
                    .commit_cost_reservation_with_usd_micros(reservation_id, spent_cost_usd_micros)
                    .await
                    .map_err(|err| {
                        budget_storage_error(
                            "commit_cost_reservation_with_usd_micros",
                            reservation_id,
                            err,
                        )
                    })?;
            } else {
                store
                    .rollback_cost_reservation(reservation_id)
                    .await
                    .map_err(|err| {
                        budget_storage_error("rollback_cost_reservation", reservation_id, err)
                    })?;
            }
        }
        #[cfg(feature = "gateway-store-postgres")]
        if let Some(store) = state.stores.postgres.as_ref() {
            if spend_tokens {
                store
                    .commit_cost_reservation_with_usd_micros(reservation_id, spent_cost_usd_micros)
                    .await
                    .map_err(|err| {
                        budget_storage_error(
                            "commit_cost_reservation_with_usd_micros",
                            reservation_id,
                            err,
                        )
                    })?;
            } else {
                store
                    .rollback_cost_reservation(reservation_id)
                    .await
                    .map_err(|err| {
                        budget_storage_error("rollback_cost_reservation", reservation_id, err)
                    })?;
            }
        }
        #[cfg(feature = "gateway-store-mysql")]
        if let Some(store) = state.stores.mysql.as_ref() {
            if spend_tokens {
                store
                    .commit_cost_reservation_with_usd_micros(reservation_id, spent_cost_usd_micros)
                    .await
                    .map_err(|err| {
                        budget_storage_error(
                            "commit_cost_reservation_with_usd_micros",
                            reservation_id,
                            err,
                        )
                    })?;
            } else {
                store
                    .rollback_cost_reservation(reservation_id)
                    .await
                    .map_err(|err| {
                        budget_storage_error("rollback_cost_reservation", reservation_id, err)
                    })?;
            }
        }
        #[cfg(feature = "gateway-store-redis")]
        if let Some(store) = state.stores.redis.as_ref() {
            if spend_tokens {
                store
                    .commit_cost_reservation_with_usd_micros(reservation_id, spent_cost_usd_micros)
                    .await
                    .map_err(|err| {
                        budget_storage_error(
                            "commit_cost_reservation_with_usd_micros",
                            reservation_id,
                            err,
                        )
                    })?;
            } else {
                store
                    .rollback_cost_reservation(reservation_id)
                    .await
                    .map_err(|err| {
                        budget_storage_error("rollback_cost_reservation", reservation_id, err)
                    })?;
            }
        }
    }
    Ok(())
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
pub(super) async fn record_proxy_spent_cost_usd_micros_checked(
    state: &GatewayHttpState,
    virtual_key_id: &str,
    spent_cost_usd_micros: u64,
) -> Result<(), String> {
    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.stores.sqlite.as_ref() {
        store
            .record_spent_cost_usd_micros(virtual_key_id, spent_cost_usd_micros)
            .await
            .map_err(|err| {
                budget_storage_error("record_spent_cost_usd_micros", virtual_key_id, err)
            })?;
    }
    #[cfg(feature = "gateway-store-postgres")]
    if let Some(store) = state.stores.postgres.as_ref() {
        store
            .record_spent_cost_usd_micros(virtual_key_id, spent_cost_usd_micros)
            .await
            .map_err(|err| {
                budget_storage_error("record_spent_cost_usd_micros", virtual_key_id, err)
            })?;
    }
    #[cfg(feature = "gateway-store-mysql")]
    if let Some(store) = state.stores.mysql.as_ref() {
        store
            .record_spent_cost_usd_micros(virtual_key_id, spent_cost_usd_micros)
            .await
            .map_err(|err| {
                budget_storage_error("record_spent_cost_usd_micros", virtual_key_id, err)
            })?;
    }
    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.stores.redis.as_ref() {
        store
            .record_spent_cost_usd_micros(virtual_key_id, spent_cost_usd_micros)
            .await
            .map_err(|err| {
                budget_storage_error("record_spent_cost_usd_micros", virtual_key_id, err)
            })?;
    }
    Ok(())
}

#[cfg(all(test, feature = "gateway-store-sqlite"))]
mod tests {
    use super::*;
    use crate::gateway::{Gateway, GatewayConfig, RouterConfig, SqliteStore};

    #[tokio::test]
    async fn checked_settle_token_budget_reservations_returns_storage_errors() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("gateway.sqlite");
        let store = SqliteStore::new(&path);
        store.init().await.expect("init");
        store
            .reserve_budget_tokens("r1", "key-1", 10, 7)
            .await
            .expect("reserve");

        let conn = rusqlite::Connection::open(store.path()).expect("open sqlite");
        conn.execute_batch(
            "CREATE TRIGGER fail_budget_commit
             BEFORE UPDATE ON budget_ledger
             BEGIN
                 SELECT RAISE(FAIL, 'budget commit failed');
             END;",
        )
        .expect("install trigger");

        let state = GatewayHttpState::new(Gateway::new(GatewayConfig {
            backends: Vec::new(),
            virtual_keys: Vec::new(),
            router: RouterConfig::default(),
            a2a_agents: Vec::new(),
            mcp_servers: Vec::new(),
            observability: Default::default(),
        }))
        .with_sqlite_store(store);

        let err =
            settle_proxy_token_budget_reservations_checked(&state, &[String::from("r1")], true, 3)
                .await
                .expect_err("commit should fail");
        assert!(err.contains("commit_budget_reservation_with_tokens"));
        assert!(err.contains("budget commit failed"));
    }
}

pub(super) enum ProxyPermitOutcome {
    Acquired(ProxyPermits),
    BackendRateLimited((StatusCode, Json<OpenAiErrorResponse>)),
}

pub(super) fn try_acquire_proxy_permits(
    state: &GatewayHttpState,
    backend: &str,
) -> Result<ProxyPermitOutcome, (StatusCode, Json<OpenAiErrorResponse>)> {
    let proxy_permit = if let Some(limit) = state.proxy.backpressure.as_ref() {
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

    let backend_permit = if let Some(limit) = state.proxy.backend_backpressure.get(backend) {
        match limit.clone().try_acquire_owned() {
            Ok(permit) => Some(permit),
            Err(_) => {
                return Ok(ProxyPermitOutcome::BackendRateLimited(openai_error(
                    StatusCode::TOO_MANY_REQUESTS,
                    "rate_limit_error",
                    Some("inflight_limit_backend"),
                    format!("too many in-flight proxy requests for backend {backend}"),
                )));
            }
        }
    } else {
        None
    };

    Ok(ProxyPermitOutcome::Acquired(ProxyPermits::new(
        proxy_permit,
        backend_permit,
    )))
}
