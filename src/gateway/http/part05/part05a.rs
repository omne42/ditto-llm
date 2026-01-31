fn openai_error(
    status: StatusCode,
    kind: &'static str,
    code: Option<&'static str>,
    message: impl ToString,
) -> (StatusCode, Json<OpenAiErrorResponse>) {
    (
        status,
        Json(OpenAiErrorResponse {
            error: OpenAiErrorDetail {
                message: message.to_string(),
                kind,
                code,
            },
        }),
    )
}

async fn list_keys(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Query(query): Query<ListKeysQuery>,
) -> Result<Json<Vec<VirtualKeyConfig>>, (StatusCode, Json<ErrorResponse>)> {
    ensure_admin(&state, &headers)?;
    let gateway = state.gateway.lock().await;
    let mut keys = gateway.list_virtual_keys();
    if !query.include_tokens {
        for key in &mut keys {
            key.token = "redacted".to_string();
        }
    }
    Ok(Json(keys))
}

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

async fn upsert_key(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Json(key): Json<VirtualKeyConfig>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    ensure_admin(&state, &headers)?;
    if let Err(err) = key.guardrails.validate() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            format!("invalid guardrails config: {err}"),
        ));
    }
    let (inserted, persisted_keys) = {
        let mut gateway = state.gateway.lock().await;
        let inserted = gateway.upsert_virtual_key(key.clone());
        (inserted, gateway.list_virtual_keys())
    };
    persist_virtual_keys(&state, &persisted_keys).await?;

    #[cfg(feature = "sdk")]
    if let Some(logger) = state.devtools.as_ref() {
        let _ = logger.log_event(
            "admin.key.upsert",
            serde_json::json!({
                "key_id": &key.id,
                "enabled": key.enabled,
                "inserted": inserted,
            }),
        );
    }

    let status = if inserted {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(key)))
}

async fn upsert_key_with_id(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(mut key): Json<VirtualKeyConfig>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    ensure_admin(&state, &headers)?;
    key.id = id;
    if let Err(err) = key.guardrails.validate() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            format!("invalid guardrails config: {err}"),
        ));
    }
    let (inserted, persisted_keys) = {
        let mut gateway = state.gateway.lock().await;
        let inserted = gateway.upsert_virtual_key(key.clone());
        (inserted, gateway.list_virtual_keys())
    };
    persist_virtual_keys(&state, &persisted_keys).await?;

    #[cfg(feature = "sdk")]
    if let Some(logger) = state.devtools.as_ref() {
        let _ = logger.log_event(
            "admin.key.upsert",
            serde_json::json!({
                "key_id": &key.id,
                "enabled": key.enabled,
                "inserted": inserted,
            }),
        );
    }

    let status = if inserted {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(key)))
}

async fn delete_key(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    ensure_admin(&state, &headers)?;
    let (removed, persisted_keys) = {
        let mut gateway = state.gateway.lock().await;
        let removed = gateway.remove_virtual_key(&id).is_some();
        (removed, gateway.list_virtual_keys())
    };
    if removed {
        persist_virtual_keys(&state, &persisted_keys).await?;

        #[cfg(feature = "sdk")]
        if let Some(logger) = state.devtools.as_ref() {
            let _ = logger.log_event(
                "admin.key.delete",
                serde_json::json!({
                    "key_id": &id,
                }),
            );
        }

        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "virtual key not found",
        ))
    }
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
) -> Result<Json<Vec<BudgetLedgerRecord>>, (StatusCode, Json<ErrorResponse>)> {
    ensure_admin(&state, &headers)?;

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.sqlite_store.as_ref() {
        let ledgers = store.list_budget_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(ledgers));
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
        } else if ledger_key_id.starts_with("project:") || ledger_key_id.starts_with("user:") {
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
        } else if ledger_key_id.starts_with("project:") || ledger_key_id.starts_with("user:") {
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

#[cfg(all(
    feature = "gateway-costing",
    any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
))]
async fn list_cost_ledgers(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
) -> Result<Json<Vec<CostLedgerRecord>>, (StatusCode, Json<ErrorResponse>)> {
    ensure_admin(&state, &headers)?;

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.sqlite_store.as_ref() {
        let ledgers = store.list_cost_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(ledgers));
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
        } else if ledger_key_id.starts_with("project:") || ledger_key_id.starts_with("user:") {
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
        } else if ledger_key_id.starts_with("project:") || ledger_key_id.starts_with("user:") {
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
    test,
    any(feature = "gateway-store-sqlite", feature = "gateway-store-redis")
))]
mod ledger_grouping_tests {
    use super::*;

    #[test]
    fn budget_ledgers_group_by_project_and_user() {
        let mut key_1 = VirtualKeyConfig::new("key-1", "vk-1");
        key_1.project_id = Some("proj-a".to_string());
        key_1.user_id = Some("user-a".to_string());

        let mut key_2 = VirtualKeyConfig::new("key-2", "vk-2");
        key_2.project_id = Some("proj-a".to_string());
        key_2.user_id = Some("user-b".to_string());

        let ledgers = vec![
            BudgetLedgerRecord {
                key_id: "key-1".to_string(),
                spent_tokens: 10,
                reserved_tokens: 3,
                updated_at_ms: 100,
            },
            BudgetLedgerRecord {
                key_id: "key-2".to_string(),
                spent_tokens: 7,
                reserved_tokens: 0,
                updated_at_ms: 200,
            },
            BudgetLedgerRecord {
                key_id: "key-unknown".to_string(),
                spent_tokens: 1,
                reserved_tokens: 2,
                updated_at_ms: 50,
            },
        ];

        let keys = vec![key_1, key_2];

        let projects = group_budget_ledgers_by_project(&ledgers, &keys);
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[0].project_id, None);
        assert_eq!(projects[0].spent_tokens, 1);
        assert_eq!(projects[0].reserved_tokens, 2);
        assert_eq!(projects[0].key_count, 1);
        assert_eq!(projects[0].updated_at_ms, 50);
        assert_eq!(projects[1].project_id.as_deref(), Some("proj-a"));
        assert_eq!(projects[1].spent_tokens, 17);
        assert_eq!(projects[1].reserved_tokens, 3);
        assert_eq!(projects[1].key_count, 2);
        assert_eq!(projects[1].updated_at_ms, 200);

        let users = group_budget_ledgers_by_user(&ledgers, &keys);
        assert_eq!(users.len(), 3);
        assert_eq!(users[0].user_id, None);
        assert_eq!(users[0].spent_tokens, 1);
        assert_eq!(users[0].reserved_tokens, 2);
        assert_eq!(users[0].key_count, 1);
        assert_eq!(users[0].updated_at_ms, 50);
        assert_eq!(users[1].user_id.as_deref(), Some("user-a"));
        assert_eq!(users[1].spent_tokens, 10);
        assert_eq!(users[1].reserved_tokens, 3);
        assert_eq!(users[1].key_count, 1);
        assert_eq!(users[1].updated_at_ms, 100);
        assert_eq!(users[2].user_id.as_deref(), Some("user-b"));
        assert_eq!(users[2].spent_tokens, 7);
        assert_eq!(users[2].reserved_tokens, 0);
        assert_eq!(users[2].key_count, 1);
        assert_eq!(users[2].updated_at_ms, 200);
    }

    #[cfg(feature = "gateway-costing")]
    #[test]
    fn cost_ledgers_group_by_project_and_user() {
        let mut key_1 = VirtualKeyConfig::new("key-1", "vk-1");
        key_1.project_id = Some("proj-a".to_string());
        key_1.user_id = Some("user-a".to_string());

        let ledgers = vec![
            CostLedgerRecord {
                key_id: "key-1".to_string(),
                spent_usd_micros: 10,
                reserved_usd_micros: 3,
                updated_at_ms: 100,
            },
            CostLedgerRecord {
                key_id: "key-unknown".to_string(),
                spent_usd_micros: 1,
                reserved_usd_micros: 2,
                updated_at_ms: 50,
            },
        ];

        let keys = vec![key_1];

        let projects = group_cost_ledgers_by_project(&ledgers, &keys);
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[0].project_id, None);
        assert_eq!(projects[0].spent_usd_micros, 1);
        assert_eq!(projects[0].reserved_usd_micros, 2);
        assert_eq!(projects[0].key_count, 1);
        assert_eq!(projects[0].updated_at_ms, 50);
        assert_eq!(projects[1].project_id.as_deref(), Some("proj-a"));
        assert_eq!(projects[1].spent_usd_micros, 10);
        assert_eq!(projects[1].reserved_usd_micros, 3);
        assert_eq!(projects[1].key_count, 1);
        assert_eq!(projects[1].updated_at_ms, 100);

        let users = group_cost_ledgers_by_user(&ledgers, &keys);
        assert_eq!(users.len(), 2);
        assert_eq!(users[0].user_id, None);
        assert_eq!(users[0].spent_usd_micros, 1);
        assert_eq!(users[0].reserved_usd_micros, 2);
        assert_eq!(users[0].key_count, 1);
        assert_eq!(users[0].updated_at_ms, 50);
        assert_eq!(users[1].user_id.as_deref(), Some("user-a"));
        assert_eq!(users[1].spent_usd_micros, 10);
        assert_eq!(users[1].reserved_usd_micros, 3);
        assert_eq!(users[1].key_count, 1);
        assert_eq!(users[1].updated_at_ms, 100);
    }
}

#[cfg(feature = "gateway-routing-advanced")]
async fn list_backends(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
) -> Result<Json<Vec<BackendHealthSnapshot>>, (StatusCode, Json<ErrorResponse>)> {
    ensure_admin(&state, &headers)?;

    let Some(health) = state.proxy_backend_health.as_ref() else {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "not_configured",
            "proxy routing not enabled",
        ));
    };

    let mut names: Vec<String> = state.proxy_backends.keys().cloned().collect();
    names.sort();

    let mut out = Vec::with_capacity(names.len());
    let health = health.lock().await;
    for name in names {
        let snapshot = health
            .get(name.as_str())
            .map(|entry| entry.snapshot(&name))
            .unwrap_or_else(|| BackendHealth::default().snapshot(&name));
        out.push(snapshot);
    }

    Ok(Json(out))
}

#[cfg(feature = "gateway-routing-advanced")]
async fn reset_backend(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<Json<BackendHealthSnapshot>, (StatusCode, Json<ErrorResponse>)> {
    ensure_admin(&state, &headers)?;

    let Some(health) = state.proxy_backend_health.as_ref() else {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "not_configured",
            "proxy routing not enabled",
        ));
    };

    let mut health = health.lock().await;
    health.remove(name.as_str());
    Ok(Json(BackendHealth::default().snapshot(&name)))
}

