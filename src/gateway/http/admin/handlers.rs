#[cfg(feature = "gateway-proxy-cache")]
async fn purge_proxy_cache(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Json(payload): Json<PurgeProxyCacheRequest>,
) -> Result<Json<PurgeProxyCacheResponse>, (StatusCode, Json<ErrorResponse>)> {
    ensure_admin(&state, &headers)?;

    let Some(cache) = state.proxy_cache.as_ref() else {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            "not_configured",
            "proxy cache not enabled",
        ));
    };

    if payload.all {
        {
            let mut cache = cache.lock().await;
            cache.clear();
        }

        #[cfg(feature = "gateway-store-redis")]
        if let Some(store) = state.redis_store.as_ref() {
            let deleted_redis = store.clear_proxy_cache().await.map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;
            #[cfg(feature = "gateway-metrics-prometheus")]
            if let Some(metrics) = state.prometheus_metrics.as_ref() {
                metrics.lock().await.record_proxy_cache_purge("all");
            }
            return Ok(Json(PurgeProxyCacheResponse {
                cleared_memory: true,
                deleted_redis: Some(deleted_redis),
            }));
        }

        #[cfg(feature = "gateway-metrics-prometheus")]
        if let Some(metrics) = state.prometheus_metrics.as_ref() {
            metrics.lock().await.record_proxy_cache_purge("all");
        }
        return Ok(Json(PurgeProxyCacheResponse {
            cleared_memory: true,
            deleted_redis: None,
        }));
    }

    let Some(cache_key) = payload
        .cache_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "must set all=true or cache_key",
        ));
    };

    let removed_memory = {
        let mut cache = cache.lock().await;
        cache.remove(cache_key)
    };

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.redis_store.as_ref() {
        let deleted_redis = store
            .delete_proxy_cache_response(cache_key)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;
        #[cfg(feature = "gateway-metrics-prometheus")]
        if let Some(metrics) = state.prometheus_metrics.as_ref() {
            metrics.lock().await.record_proxy_cache_purge("key");
        }
        return Ok(Json(PurgeProxyCacheResponse {
            cleared_memory: removed_memory,
            deleted_redis: Some(deleted_redis),
        }));
    }

    #[cfg(feature = "gateway-metrics-prometheus")]
    if let Some(metrics) = state.prometheus_metrics.as_ref() {
        metrics.lock().await.record_proxy_cache_purge("key");
    }
    Ok(Json(PurgeProxyCacheResponse {
        cleared_memory: removed_memory,
        deleted_redis: None,
    }))
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
#[derive(Debug, Deserialize)]
struct AuditQuery {
    #[serde(default = "default_audit_limit")]
    limit: usize,
    #[serde(default)]
    since_ts_ms: Option<u64>,
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
#[derive(Debug, Deserialize)]
struct LedgerQuery {
    #[serde(default)]
    key_prefix: Option<String>,
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
fn default_audit_limit() -> usize {
    100
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
async fn list_audit_logs(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Query(query): Query<AuditQuery>,
) -> Result<Json<Vec<AuditLogRecord>>, (StatusCode, Json<ErrorResponse>)> {
    ensure_admin(&state, &headers)?;

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.sqlite_store.as_ref() {
        let logs = store
            .list_audit_logs(query.limit.min(1000), query.since_ts_ms)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;
        return Ok(Json(logs));
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.redis_store.as_ref() {
        let logs = store
            .list_audit_logs(query.limit.min(1000), query.since_ts_ms)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;
        return Ok(Json(logs));
    }

    Err(error_response(
        StatusCode::BAD_REQUEST,
        "not_configured",
        "store not configured",
    ))
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
async fn list_budget_ledgers(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Query(query): Query<LedgerQuery>,
) -> Result<Json<Vec<BudgetLedgerRecord>>, (StatusCode, Json<ErrorResponse>)> {
    ensure_admin(&state, &headers)?;

    let key_prefix = query
        .key_prefix
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.sqlite_store.as_ref() {
        let mut ledgers = store.list_budget_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        if let Some(key_prefix) = key_prefix {
            ledgers.retain(|ledger| ledger.key_id.starts_with(key_prefix));
        }
        return Ok(Json(ledgers));
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.redis_store.as_ref() {
        let mut ledgers = store.list_budget_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        if let Some(key_prefix) = key_prefix {
            ledgers.retain(|ledger| ledger.key_id.starts_with(key_prefix));
        }
        return Ok(Json(ledgers));
    }

    Err(error_response(
        StatusCode::BAD_REQUEST,
        "not_configured",
        "store not configured",
    ))
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
#[derive(Debug, Serialize)]
struct ProjectBudgetLedger {
    project_id: Option<String>,
    spent_tokens: u64,
    reserved_tokens: u64,
    key_count: usize,
    updated_at_ms: u64,
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
#[derive(Debug, Serialize)]
struct UserBudgetLedger {
    user_id: Option<String>,
    spent_tokens: u64,
    reserved_tokens: u64,
    key_count: usize,
    updated_at_ms: u64,
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
#[derive(Debug, Serialize)]
struct TenantBudgetLedger {
    tenant_id: Option<String>,
    spent_tokens: u64,
    reserved_tokens: u64,
    key_count: usize,
    updated_at_ms: u64,
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
fn group_budget_ledgers_by_project(
    ledgers: &[BudgetLedgerRecord],
    keys: &[VirtualKeyConfig],
) -> Vec<ProjectBudgetLedger> {
    use std::collections::BTreeMap;

    let mut key_to_project = std::collections::HashMap::<&str, Option<&str>>::new();
    for key in keys {
        key_to_project.insert(key.id.as_str(), key.project_id.as_deref());
    }

    let mut grouped = BTreeMap::<Option<String>, (u64, u64, usize, u64)>::new();
    for ledger in ledgers {
        let ledger_key_id = ledger.key_id.as_str();
        let project_id = if let Some(project_id) = key_to_project.get(ledger_key_id).copied() {
            project_id.map(|id| id.to_string())
        } else if ledger_key_id.starts_with("tenant:")
            || ledger_key_id.starts_with("project:")
            || ledger_key_id.starts_with("user:")
        {
            continue;
        } else {
            None
        };
        let entry = grouped.entry(project_id).or_insert((0, 0, 0, 0));
        entry.0 = entry.0.saturating_add(ledger.spent_tokens);
        entry.1 = entry.1.saturating_add(ledger.reserved_tokens);
        entry.2 = entry.2.saturating_add(1);
        entry.3 = entry.3.max(ledger.updated_at_ms);
    }

    grouped
        .into_iter()
        .map(
            |(project_id, (spent_tokens, reserved_tokens, key_count, updated_at_ms))| {
                ProjectBudgetLedger {
                    project_id,
                    spent_tokens,
                    reserved_tokens,
                    key_count,
                    updated_at_ms,
                }
            },
        )
        .collect()
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
fn group_budget_ledgers_by_user(
    ledgers: &[BudgetLedgerRecord],
    keys: &[VirtualKeyConfig],
) -> Vec<UserBudgetLedger> {
    use std::collections::BTreeMap;

    let mut key_to_user = std::collections::HashMap::<&str, Option<&str>>::new();
    for key in keys {
        key_to_user.insert(key.id.as_str(), key.user_id.as_deref());
    }

    let mut grouped = BTreeMap::<Option<String>, (u64, u64, usize, u64)>::new();
    for ledger in ledgers {
        let ledger_key_id = ledger.key_id.as_str();
        let user_id = if let Some(user_id) = key_to_user.get(ledger_key_id).copied() {
            user_id.map(|id| id.to_string())
        } else if ledger_key_id.starts_with("tenant:")
            || ledger_key_id.starts_with("project:")
            || ledger_key_id.starts_with("user:")
        {
            continue;
        } else {
            None
        };
        let entry = grouped.entry(user_id).or_insert((0, 0, 0, 0));
        entry.0 = entry.0.saturating_add(ledger.spent_tokens);
        entry.1 = entry.1.saturating_add(ledger.reserved_tokens);
        entry.2 = entry.2.saturating_add(1);
        entry.3 = entry.3.max(ledger.updated_at_ms);
    }

    grouped
        .into_iter()
        .map(
            |(user_id, (spent_tokens, reserved_tokens, key_count, updated_at_ms))| {
                UserBudgetLedger {
                    user_id,
                    spent_tokens,
                    reserved_tokens,
                    key_count,
                    updated_at_ms,
                }
            },
        )
        .collect()
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
fn group_budget_ledgers_by_tenant(
    ledgers: &[BudgetLedgerRecord],
    keys: &[VirtualKeyConfig],
) -> Vec<TenantBudgetLedger> {
    use std::collections::BTreeMap;

    let mut key_to_tenant = std::collections::HashMap::<&str, Option<&str>>::new();
    for key in keys {
        key_to_tenant.insert(key.id.as_str(), key.tenant_id.as_deref());
    }

    let mut grouped = BTreeMap::<Option<String>, (u64, u64, usize, u64)>::new();
    for ledger in ledgers {
        let ledger_key_id = ledger.key_id.as_str();
        let tenant_id = if let Some(tenant_id) = key_to_tenant.get(ledger_key_id).copied() {
            tenant_id.map(|id| id.to_string())
        } else if ledger_key_id.starts_with("tenant:")
            || ledger_key_id.starts_with("project:")
            || ledger_key_id.starts_with("user:")
        {
            continue;
        } else {
            None
        };
        let entry = grouped.entry(tenant_id).or_insert((0, 0, 0, 0));
        entry.0 = entry.0.saturating_add(ledger.spent_tokens);
        entry.1 = entry.1.saturating_add(ledger.reserved_tokens);
        entry.2 = entry.2.saturating_add(1);
        entry.3 = entry.3.max(ledger.updated_at_ms);
    }

    grouped
        .into_iter()
        .map(
            |(tenant_id, (spent_tokens, reserved_tokens, key_count, updated_at_ms))| {
                TenantBudgetLedger {
                    tenant_id,
                    spent_tokens,
                    reserved_tokens,
                    key_count,
                    updated_at_ms,
                }
            },
        )
        .collect()
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
async fn list_project_budget_ledgers(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ProjectBudgetLedger>>, (StatusCode, Json<ErrorResponse>)> {
    ensure_admin(&state, &headers)?;

    let keys = { state.gateway.lock().await.list_virtual_keys() };

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.sqlite_store.as_ref() {
        let ledgers = store.list_budget_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_budget_ledgers_by_project(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.redis_store.as_ref() {
        let ledgers = store.list_budget_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_budget_ledgers_by_project(&ledgers, &keys)));
    }

    Err(error_response(
        StatusCode::BAD_REQUEST,
        "not_configured",
        "store not configured",
    ))
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
async fn list_user_budget_ledgers(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
) -> Result<Json<Vec<UserBudgetLedger>>, (StatusCode, Json<ErrorResponse>)> {
    ensure_admin(&state, &headers)?;

    let keys = { state.gateway.lock().await.list_virtual_keys() };

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.sqlite_store.as_ref() {
        let ledgers = store.list_budget_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_budget_ledgers_by_user(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.redis_store.as_ref() {
        let ledgers = store.list_budget_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_budget_ledgers_by_user(&ledgers, &keys)));
    }

    Err(error_response(
        StatusCode::BAD_REQUEST,
        "not_configured",
        "store not configured",
    ))
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
async fn list_tenant_budget_ledgers(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
) -> Result<Json<Vec<TenantBudgetLedger>>, (StatusCode, Json<ErrorResponse>)> {
    ensure_admin(&state, &headers)?;

    let keys = { state.gateway.lock().await.list_virtual_keys() };

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.sqlite_store.as_ref() {
        let ledgers = store.list_budget_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_budget_ledgers_by_tenant(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.redis_store.as_ref() {
        let ledgers = store.list_budget_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_budget_ledgers_by_tenant(&ledgers, &keys)));
    }

    Err(error_response(
        StatusCode::BAD_REQUEST,
        "not_configured",
        "store not configured",
    ))
}

#[cfg(all(
    feature = "gateway-costing",
    any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
))]
async fn list_cost_ledgers(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Query(query): Query<LedgerQuery>,
) -> Result<Json<Vec<CostLedgerRecord>>, (StatusCode, Json<ErrorResponse>)> {
    ensure_admin(&state, &headers)?;

    let key_prefix = query
        .key_prefix
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.sqlite_store.as_ref() {
        let mut ledgers = store.list_cost_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        if let Some(key_prefix) = key_prefix {
            ledgers.retain(|ledger| ledger.key_id.starts_with(key_prefix));
        }
        return Ok(Json(ledgers));
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.redis_store.as_ref() {
        let mut ledgers = store.list_cost_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        if let Some(key_prefix) = key_prefix {
            ledgers.retain(|ledger| ledger.key_id.starts_with(key_prefix));
        }
        return Ok(Json(ledgers));
    }

    Err(error_response(
        StatusCode::BAD_REQUEST,
        "not_configured",
        "store not configured",
    ))
}

#[cfg(all(
    feature = "gateway-costing",
    any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
))]
#[derive(Debug, Serialize)]
struct ProjectCostLedger {
    project_id: Option<String>,
    spent_usd_micros: u64,
    reserved_usd_micros: u64,
    key_count: usize,
    updated_at_ms: u64,
}

#[cfg(all(
    feature = "gateway-costing",
    any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
))]
#[derive(Debug, Serialize)]
struct UserCostLedger {
    user_id: Option<String>,
    spent_usd_micros: u64,
    reserved_usd_micros: u64,
    key_count: usize,
    updated_at_ms: u64,
}

#[cfg(all(
    feature = "gateway-costing",
    any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
))]
#[derive(Debug, Serialize)]
struct TenantCostLedger {
    tenant_id: Option<String>,
    spent_usd_micros: u64,
    reserved_usd_micros: u64,
    key_count: usize,
    updated_at_ms: u64,
}

#[cfg(all(
    feature = "gateway-costing",
    any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
))]
fn group_cost_ledgers_by_project(
    ledgers: &[CostLedgerRecord],
    keys: &[VirtualKeyConfig],
) -> Vec<ProjectCostLedger> {
    use std::collections::BTreeMap;

    let mut key_to_project = std::collections::HashMap::<&str, Option<&str>>::new();
    for key in keys {
        key_to_project.insert(key.id.as_str(), key.project_id.as_deref());
    }

    let mut grouped = BTreeMap::<Option<String>, (u64, u64, usize, u64)>::new();
    for ledger in ledgers {
        let ledger_key_id = ledger.key_id.as_str();
        let project_id = if let Some(project_id) = key_to_project.get(ledger_key_id).copied() {
            project_id.map(|id| id.to_string())
        } else if ledger_key_id.starts_with("tenant:")
            || ledger_key_id.starts_with("project:")
            || ledger_key_id.starts_with("user:")
        {
            continue;
        } else {
            None
        };
        let entry = grouped.entry(project_id).or_insert((0, 0, 0, 0));
        entry.0 = entry.0.saturating_add(ledger.spent_usd_micros);
        entry.1 = entry.1.saturating_add(ledger.reserved_usd_micros);
        entry.2 = entry.2.saturating_add(1);
        entry.3 = entry.3.max(ledger.updated_at_ms);
    }

    grouped
        .into_iter()
        .map(
            |(project_id, (spent_usd_micros, reserved_usd_micros, key_count, updated_at_ms))| {
                ProjectCostLedger {
                    project_id,
                    spent_usd_micros,
                    reserved_usd_micros,
                    key_count,
                    updated_at_ms,
                }
            },
        )
        .collect()
}

#[cfg(all(
    feature = "gateway-costing",
    any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
))]
fn group_cost_ledgers_by_user(
    ledgers: &[CostLedgerRecord],
    keys: &[VirtualKeyConfig],
) -> Vec<UserCostLedger> {
    use std::collections::BTreeMap;

    let mut key_to_user = std::collections::HashMap::<&str, Option<&str>>::new();
    for key in keys {
        key_to_user.insert(key.id.as_str(), key.user_id.as_deref());
    }

    let mut grouped = BTreeMap::<Option<String>, (u64, u64, usize, u64)>::new();
    for ledger in ledgers {
        let ledger_key_id = ledger.key_id.as_str();
        let user_id = if let Some(user_id) = key_to_user.get(ledger_key_id).copied() {
            user_id.map(|id| id.to_string())
        } else if ledger_key_id.starts_with("tenant:")
            || ledger_key_id.starts_with("project:")
            || ledger_key_id.starts_with("user:")
        {
            continue;
        } else {
            None
        };
        let entry = grouped.entry(user_id).or_insert((0, 0, 0, 0));
        entry.0 = entry.0.saturating_add(ledger.spent_usd_micros);
        entry.1 = entry.1.saturating_add(ledger.reserved_usd_micros);
        entry.2 = entry.2.saturating_add(1);
        entry.3 = entry.3.max(ledger.updated_at_ms);
    }

    grouped
        .into_iter()
        .map(
            |(user_id, (spent_usd_micros, reserved_usd_micros, key_count, updated_at_ms))| {
                UserCostLedger {
                    user_id,
                    spent_usd_micros,
                    reserved_usd_micros,
                    key_count,
                    updated_at_ms,
                }
            },
        )
        .collect()
}

#[cfg(all(
    feature = "gateway-costing",
    any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
))]
fn group_cost_ledgers_by_tenant(
    ledgers: &[CostLedgerRecord],
    keys: &[VirtualKeyConfig],
) -> Vec<TenantCostLedger> {
    use std::collections::BTreeMap;

    let mut key_to_tenant = std::collections::HashMap::<&str, Option<&str>>::new();
    for key in keys {
        key_to_tenant.insert(key.id.as_str(), key.tenant_id.as_deref());
    }

    let mut grouped = BTreeMap::<Option<String>, (u64, u64, usize, u64)>::new();
    for ledger in ledgers {
        let ledger_key_id = ledger.key_id.as_str();
        let tenant_id = if let Some(tenant_id) = key_to_tenant.get(ledger_key_id).copied() {
            tenant_id.map(|id| id.to_string())
        } else if ledger_key_id.starts_with("tenant:")
            || ledger_key_id.starts_with("project:")
            || ledger_key_id.starts_with("user:")
        {
            continue;
        } else {
            None
        };
        let entry = grouped.entry(tenant_id).or_insert((0, 0, 0, 0));
        entry.0 = entry.0.saturating_add(ledger.spent_usd_micros);
        entry.1 = entry.1.saturating_add(ledger.reserved_usd_micros);
        entry.2 = entry.2.saturating_add(1);
        entry.3 = entry.3.max(ledger.updated_at_ms);
    }

    grouped
        .into_iter()
        .map(
            |(tenant_id, (spent_usd_micros, reserved_usd_micros, key_count, updated_at_ms))| {
                TenantCostLedger {
                    tenant_id,
                    spent_usd_micros,
                    reserved_usd_micros,
                    key_count,
                    updated_at_ms,
                }
            },
        )
        .collect()
}

#[cfg(all(
    feature = "gateway-costing",
    any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
))]
async fn list_project_cost_ledgers(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ProjectCostLedger>>, (StatusCode, Json<ErrorResponse>)> {
    ensure_admin(&state, &headers)?;

    let keys = { state.gateway.lock().await.list_virtual_keys() };

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.sqlite_store.as_ref() {
        let ledgers = store.list_cost_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_cost_ledgers_by_project(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.redis_store.as_ref() {
        let ledgers = store.list_cost_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_cost_ledgers_by_project(&ledgers, &keys)));
    }

    Err(error_response(
        StatusCode::BAD_REQUEST,
        "not_configured",
        "store not configured",
    ))
}

#[cfg(all(
    feature = "gateway-costing",
    any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
))]
async fn list_user_cost_ledgers(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
) -> Result<Json<Vec<UserCostLedger>>, (StatusCode, Json<ErrorResponse>)> {
    ensure_admin(&state, &headers)?;

    let keys = { state.gateway.lock().await.list_virtual_keys() };

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.sqlite_store.as_ref() {
        let ledgers = store.list_cost_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_cost_ledgers_by_user(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.redis_store.as_ref() {
        let ledgers = store.list_cost_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_cost_ledgers_by_user(&ledgers, &keys)));
    }

    Err(error_response(
        StatusCode::BAD_REQUEST,
        "not_configured",
        "store not configured",
    ))
}

#[cfg(all(
    feature = "gateway-costing",
    any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
))]
async fn list_tenant_cost_ledgers(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
) -> Result<Json<Vec<TenantCostLedger>>, (StatusCode, Json<ErrorResponse>)> {
    ensure_admin(&state, &headers)?;

    let keys = { state.gateway.lock().await.list_virtual_keys() };

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.sqlite_store.as_ref() {
        let ledgers = store.list_cost_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_cost_ledgers_by_tenant(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.redis_store.as_ref() {
        let ledgers = store.list_cost_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_cost_ledgers_by_tenant(&ledgers, &keys)));
    }

    Err(error_response(
        StatusCode::BAD_REQUEST,
        "not_configured",
        "store not configured",
    ))
}
