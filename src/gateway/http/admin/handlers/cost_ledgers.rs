#[cfg(all(
    feature = "gateway-costing",
    any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
))]
async fn list_cost_ledgers(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Query(query): Query<LedgerQuery>,
) -> Result<Json<Vec<CostLedgerRecord>>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;
    let tenant_scopes = if let Some(tenant_id) = admin.tenant_id.as_deref() {
        let keys = { state.gateway.lock().await.list_virtual_keys() };
        Some(tenant_allowed_scopes(&keys, tenant_id))
    } else {
        None
    };

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
        if let Some(scopes) = tenant_scopes.as_ref() {
            ledgers.retain(|ledger| scopes.contains(ledger.key_id.as_str()));
        }
        if let Some(key_prefix) = key_prefix {
            ledgers.retain(|ledger| ledger.key_id.starts_with(key_prefix));
        }
        apply_admin_list_window(
            &mut ledgers,
            query.offset,
            query.limit,
            MAX_ADMIN_LEDGER_LIMIT,
        );
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
        if let Some(scopes) = tenant_scopes.as_ref() {
            ledgers.retain(|ledger| scopes.contains(ledger.key_id.as_str()));
        }
        if let Some(key_prefix) = key_prefix {
            ledgers.retain(|ledger| ledger.key_id.starts_with(key_prefix));
        }
        apply_admin_list_window(
            &mut ledgers,
            query.offset,
            query.limit,
            MAX_ADMIN_LEDGER_LIMIT,
        );
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
    let admin = ensure_admin_read(&state, &headers)?;

    let mut keys = { state.gateway.lock().await.list_virtual_keys() };
    if let Some(tenant_id) = admin.tenant_id.as_deref() {
        keys.retain(|key| key.tenant_id.as_deref() == Some(tenant_id));
    }

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
    let admin = ensure_admin_read(&state, &headers)?;

    let mut keys = { state.gateway.lock().await.list_virtual_keys() };
    if let Some(tenant_id) = admin.tenant_id.as_deref() {
        keys.retain(|key| key.tenant_id.as_deref() == Some(tenant_id));
    }

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
    let admin = ensure_admin_read(&state, &headers)?;

    let mut keys = { state.gateway.lock().await.list_virtual_keys() };
    if let Some(tenant_id) = admin.tenant_id.as_deref() {
        keys.retain(|key| key.tenant_id.as_deref() == Some(tenant_id));
    }

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
