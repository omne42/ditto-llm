#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
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

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
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
    if let Some(store) = state.sqlite_store.as_ref() {
        match store
            .reserve_budget_tokens(reservation_id, budget_scope, limit, charge_tokens_u64)
            .await
        {
            Ok(()) => return Ok(()),
            Err(SqliteStoreError::BudgetExceeded { limit, attempted }) => {
                let _ = store
                    .append_audit_log(
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
                let _ = store
                    .append_audit_log(
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
    if let Some(store) = state.redis_store.as_ref() {
        match store
            .reserve_budget_tokens(reservation_id, budget_scope, limit, charge_tokens_u64)
            .await
        {
            Ok(()) => return Ok(()),
            Err(RedisStoreError::BudgetExceeded { limit, attempted }) => {
                let _ = store
                    .append_audit_log(
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
                let _ = store
                    .append_audit_log(
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

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
async fn rollback_proxy_token_budget_reservations(
    state: &GatewayHttpState,
    reservation_ids: &[String],
) {
    for reservation_id in reservation_ids {
        #[cfg(feature = "gateway-store-sqlite")]
        if let Some(store) = state.sqlite_store.as_ref() {
            let _ = store.rollback_budget_reservation(reservation_id).await;
        }
        #[cfg(feature = "gateway-store-redis")]
        if let Some(store) = state.redis_store.as_ref() {
            let _ = store.rollback_budget_reservation(reservation_id).await;
        }
    }
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
async fn settle_proxy_token_budget_reservations(
    state: &GatewayHttpState,
    reservation_ids: &[String],
    spend_tokens: bool,
    spent_tokens: u64,
) {
    if reservation_ids.is_empty() {
        return;
    }
    for reservation_id in reservation_ids {
        #[cfg(feature = "gateway-store-sqlite")]
        if let Some(store) = state.sqlite_store.as_ref() {
            if spend_tokens {
                let _ = store
                    .commit_budget_reservation_with_tokens(reservation_id, spent_tokens)
                    .await;
            } else {
                let _ = store.rollback_budget_reservation(reservation_id).await;
            }
        }
        #[cfg(feature = "gateway-store-redis")]
        if let Some(store) = state.redis_store.as_ref() {
            if spend_tokens {
                let _ = store
                    .commit_budget_reservation_with_tokens(reservation_id, spent_tokens)
                    .await;
            } else {
                let _ = store.rollback_budget_reservation(reservation_id).await;
            }
        }
    }
}

#[cfg(all(
    feature = "gateway-costing",
    any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
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
    if let Some(store) = state.sqlite_store.as_ref() {
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
                let _ = store
                    .append_audit_log(
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
                let _ = store
                    .append_audit_log(
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
    if let Some(store) = state.redis_store.as_ref() {
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
                let _ = store
                    .append_audit_log(
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
                let _ = store
                    .append_audit_log(
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
    any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
))]
async fn rollback_proxy_cost_budget_reservations(
    state: &GatewayHttpState,
    reservation_ids: &[String],
) {
    for reservation_id in reservation_ids {
        #[cfg(feature = "gateway-store-sqlite")]
        if let Some(store) = state.sqlite_store.as_ref() {
            let _ = store.rollback_cost_reservation(reservation_id).await;
        }
        #[cfg(feature = "gateway-store-redis")]
        if let Some(store) = state.redis_store.as_ref() {
            let _ = store.rollback_cost_reservation(reservation_id).await;
        }
    }
}

#[cfg(all(
    feature = "gateway-costing",
    any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
))]
async fn settle_proxy_cost_budget_reservations(
    state: &GatewayHttpState,
    reservation_ids: &[String],
    spend_tokens: bool,
    spent_cost_usd_micros: u64,
) {
    if reservation_ids.is_empty() {
        return;
    }

    for reservation_id in reservation_ids {
        #[cfg(feature = "gateway-store-sqlite")]
        if let Some(store) = state.sqlite_store.as_ref() {
            if spend_tokens {
                let _ = store
                    .commit_cost_reservation_with_usd_micros(reservation_id, spent_cost_usd_micros)
                    .await;
            } else {
                let _ = store.rollback_cost_reservation(reservation_id).await;
            }
        }
        #[cfg(feature = "gateway-store-redis")]
        if let Some(store) = state.redis_store.as_ref() {
            if spend_tokens {
                let _ = store
                    .commit_cost_reservation_with_usd_micros(reservation_id, spent_cost_usd_micros)
                    .await;
            } else {
                let _ = store.rollback_cost_reservation(reservation_id).await;
            }
        }
    }
}

enum ProxyPermitOutcome {
    Acquired(ProxyPermits),
    BackendRateLimited((StatusCode, Json<OpenAiErrorResponse>)),
}

fn try_acquire_proxy_permits(
    state: &GatewayHttpState,
    backend: &str,
) -> Result<ProxyPermitOutcome, (StatusCode, Json<OpenAiErrorResponse>)> {
    let proxy_permit = if let Some(limit) = state.proxy_backpressure.as_ref() {
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

    let backend_permit = if let Some(limit) = state.proxy_backend_backpressure.get(backend) {
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
