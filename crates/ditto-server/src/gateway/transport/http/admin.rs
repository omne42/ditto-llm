#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
use super::admin_persistence::append_admin_audit_log;
use super::admin_persistence::apply_control_plane_change;
use super::*;

// inlined from admin/handlers.rs
// inlined from handlers/common.rs
#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
#[derive(Debug, Deserialize)]
pub(super) struct LedgerQuery {
    #[serde(default)]
    key_prefix: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: usize,
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
fn default_audit_limit() -> usize {
    100
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
fn default_audit_export_limit() -> usize {
    1000
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
const MAX_ADMIN_LEDGER_LIMIT: usize = 10_000;

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
fn apply_admin_list_window<T>(items: &mut Vec<T>, offset: usize, limit: Option<usize>, max: usize) {
    if offset > 0 {
        if offset >= items.len() {
            items.clear();
        } else {
            items.drain(0..offset);
        }
    }

    if let Some(limit) = limit.map(|limit| limit.min(max))
        && items.len() > limit
    {
        items.truncate(limit);
    }
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
fn tenant_allowed_scopes(
    keys: &[VirtualKeyConfig],
    tenant_id: &str,
) -> std::collections::HashSet<String> {
    let tenant_id = tenant_id.trim();
    let mut scopes = std::collections::HashSet::<String>::new();
    if !tenant_id.is_empty() {
        scopes.insert(format!("tenant:{tenant_id}"));
    }

    for key in keys {
        if key.tenant_id.as_deref() != Some(tenant_id) {
            continue;
        }
        scopes.insert(key.id.clone());

        if let Some(project_id) = key
            .project_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            scopes.insert(format!("project:{project_id}"));
        }

        if let Some(user_id) = key
            .user_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            scopes.insert(format!("user:{user_id}"));
        }
    }

    scopes
}
// end inline: handlers/common.rs
// inlined from handlers/proxy_cache.rs
#[cfg(feature = "gateway-proxy-cache")]
pub(super) async fn purge_proxy_cache(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Json(payload): Json<PurgeProxyCacheRequest>,
) -> Result<Json<PurgeProxyCacheResponse>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_write(&state, &headers)?;
    if admin.tenant_id.is_some() {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "tenant-scoped admin tokens cannot purge the proxy cache",
        ));
    }

    let Some(cache) = state.proxy.cache.as_ref() else {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            "not_configured",
            "proxy cache not enabled",
        ));
    };

    let selector = payload.selector.into_normalized();

    if payload.all {
        let deleted_memory = {
            let mut cache = cache.lock().await;
            let deleted = cache.len() as u64;
            cache.clear();
            deleted
        };

        let deleted_redis = {
            #[cfg(feature = "gateway-store-redis")]
            if let Some(store) = state.stores.redis.as_ref() {
                Some(store.clear_proxy_cache().await.map_err(|err| {
                    error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "storage_error",
                        err.to_string(),
                    )
                })?)
            } else {
                None
            }
            #[cfg(not(feature = "gateway-store-redis"))]
            {
                None
            }
        };

        #[cfg(feature = "gateway-metrics-prometheus")]
        if let Some(metrics) = state.proxy.metrics.as_ref() {
            metrics.lock().await.record_proxy_cache_purge("all");
        }

        #[cfg(any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        ))]
        append_admin_audit_log(
            &state,
            "admin.proxy_cache.purge",
            serde_json::json!({
                "all": true,
                "selector": selector,
                "deleted_memory": deleted_memory,
                "deleted_redis": deleted_redis,
            }),
        )
        .await?;

        return Ok(Json(PurgeProxyCacheResponse {
            cleared_memory: true,
            deleted_memory,
            deleted_redis,
        }));
    }

    if selector.is_empty() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "must set all=true or at least one of cache_key/scope/method/path/model",
        ));
    }

    let deleted_memory = {
        let mut cache = cache.lock().await;
        cache.purge_matching(&selector)
    };

    let deleted_redis = {
        #[cfg(feature = "gateway-store-redis")]
        if let Some(store) = state.stores.redis.as_ref() {
            Some(
                store
                    .purge_proxy_cache_matching(&selector)
                    .await
                    .map_err(|err| {
                        error_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "storage_error",
                            err.to_string(),
                        )
                    })?,
            )
        } else {
            None
        }
        #[cfg(not(feature = "gateway-store-redis"))]
        {
            None
        }
    };

    #[cfg(feature = "gateway-metrics-prometheus")]
    if let Some(metrics) = state.proxy.metrics.as_ref() {
        metrics
            .lock()
            .await
            .record_proxy_cache_purge(selector.kind_label());
    }

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    append_admin_audit_log(
        &state,
        "admin.proxy_cache.purge",
        serde_json::json!({
            "all": false,
            "selector": selector,
            "deleted_memory": deleted_memory,
            "deleted_redis": deleted_redis,
        }),
    )
    .await?;

    Ok(Json(PurgeProxyCacheResponse {
        cleared_memory: deleted_memory > 0,
        deleted_memory,
        deleted_redis,
    }))
}
// end inline: handlers/proxy_cache.rs
// inlined from handlers/audit.rs
#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
#[derive(Debug, Deserialize)]
pub(super) struct AuditQuery {
    #[serde(default = "default_audit_limit")]
    limit: usize,
    #[serde(default)]
    since_ts_ms: Option<u64>,
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
#[derive(Debug, Deserialize)]
pub(super) struct AuditExportQuery {
    #[serde(default)]
    format: Option<String>,
    #[serde(default = "default_audit_export_limit")]
    limit: usize,
    #[serde(default)]
    since_ts_ms: Option<u64>,
    #[serde(default)]
    before_ts_ms: Option<u64>,
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
pub(super) async fn list_audit_logs(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Query(query): Query<AuditQuery>,
) -> Result<Json<Vec<AuditLogRecord>>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.stores.sqlite.as_ref() {
        let mut logs = store
            .list_audit_logs(query.limit.min(1000), query.since_ts_ms)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;
        if let Some(tenant_id) = admin.tenant_id.as_deref() {
            logs.retain(|log| {
                log.payload
                    .get("tenant_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(tenant_id)
            });
        }
        for log in &mut logs {
            log.payload = state.redactor.redact(std::mem::take(&mut log.payload));
        }
        return Ok(Json(logs));
    }

    #[cfg(feature = "gateway-store-postgres")]
    if let Some(store) = state.stores.postgres.as_ref() {
        let mut logs = store
            .list_audit_logs(query.limit.min(1000), query.since_ts_ms)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;
        if let Some(tenant_id) = admin.tenant_id.as_deref() {
            logs.retain(|log| {
                log.payload
                    .get("tenant_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(tenant_id)
            });
        }
        for log in &mut logs {
            log.payload = state.redactor.redact(std::mem::take(&mut log.payload));
        }
        return Ok(Json(logs));
    }

    #[cfg(feature = "gateway-store-mysql")]
    if let Some(store) = state.stores.mysql.as_ref() {
        let mut logs = store
            .list_audit_logs(query.limit.min(1000), query.since_ts_ms)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;
        if let Some(tenant_id) = admin.tenant_id.as_deref() {
            logs.retain(|log| {
                log.payload
                    .get("tenant_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(tenant_id)
            });
        }
        for log in &mut logs {
            log.payload = state.redactor.redact(std::mem::take(&mut log.payload));
        }
        return Ok(Json(logs));
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.stores.redis.as_ref() {
        let mut logs = store
            .list_audit_logs(query.limit.min(1000), query.since_ts_ms)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;
        if let Some(tenant_id) = admin.tenant_id.as_deref() {
            logs.retain(|log| {
                log.payload
                    .get("tenant_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(tenant_id)
            });
        }
        for log in &mut logs {
            log.payload = state.redactor.redact(std::mem::take(&mut log.payload));
        }
        return Ok(Json(logs));
    }

    Err(error_response(
        StatusCode::BAD_REQUEST,
        "not_configured",
        "store not configured",
    ))
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
fn csv_escape(value: &str) -> String {
    if !value.contains([',', '"', '\n', '\r']) {
        return value.to_string();
    }
    let escaped = value.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
#[derive(Debug, Serialize)]
struct AuditExportRecord {
    id: i64,
    ts_ms: u64,
    kind: String,
    payload: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prev_hash: Option<String>,
    hash: String,
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
pub(super) async fn export_audit_logs(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Query(query): Query<AuditExportQuery>,
) -> Result<axum::response::Response, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;

    let format = query
        .format
        .as_deref()
        .unwrap_or("jsonl")
        .trim()
        .to_ascii_lowercase();
    let limit = query.limit.clamp(1, 10_000);

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.stores.sqlite.as_ref() {
        let mut logs = store
            .list_audit_logs_window(limit, query.since_ts_ms, query.before_ts_ms)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;
        if let Some(tenant_id) = admin.tenant_id.as_deref() {
            logs.retain(|log| {
                log.payload
                    .get("tenant_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(tenant_id)
            });
        }
        for log in &mut logs {
            log.payload = state.redactor.redact(std::mem::take(&mut log.payload));
        }
        return render_audit_export(&format, logs);
    }

    #[cfg(feature = "gateway-store-postgres")]
    if let Some(store) = state.stores.postgres.as_ref() {
        let mut logs = store
            .list_audit_logs_window(limit, query.since_ts_ms, query.before_ts_ms)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;
        if let Some(tenant_id) = admin.tenant_id.as_deref() {
            logs.retain(|log| {
                log.payload
                    .get("tenant_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(tenant_id)
            });
        }
        for log in &mut logs {
            log.payload = state.redactor.redact(std::mem::take(&mut log.payload));
        }
        return render_audit_export(&format, logs);
    }

    #[cfg(feature = "gateway-store-mysql")]
    if let Some(store) = state.stores.mysql.as_ref() {
        let mut logs = store
            .list_audit_logs_window(limit, query.since_ts_ms, query.before_ts_ms)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;
        if let Some(tenant_id) = admin.tenant_id.as_deref() {
            logs.retain(|log| {
                log.payload
                    .get("tenant_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(tenant_id)
            });
        }
        for log in &mut logs {
            log.payload = state.redactor.redact(std::mem::take(&mut log.payload));
        }
        return render_audit_export(&format, logs);
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.stores.redis.as_ref() {
        let mut logs = store
            .list_audit_logs_window(limit, query.since_ts_ms, query.before_ts_ms)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;
        if let Some(tenant_id) = admin.tenant_id.as_deref() {
            logs.retain(|log| {
                log.payload
                    .get("tenant_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(tenant_id)
            });
        }
        for log in &mut logs {
            log.payload = state.redactor.redact(std::mem::take(&mut log.payload));
        }
        return render_audit_export(&format, logs);
    }

    Err(error_response(
        StatusCode::BAD_REQUEST,
        "not_configured",
        "store not configured",
    ))
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
fn render_audit_export(
    format: &str,
    logs: Vec<AuditLogRecord>,
) -> Result<axum::response::Response, (StatusCode, Json<ErrorResponse>)> {
    use axum::body::Body;
    use bytes::Bytes;
    use futures_util::stream;

    let mut prev_hash: Option<String> = None;

    let mut lines = Vec::<String>::with_capacity(logs.len().saturating_add(1));

    match format {
        "jsonl" | "ndjson" => {
            for log in logs {
                let hash = crate::audit_integrity::audit_chain_hash(prev_hash.as_deref(), &log);
                let record = AuditExportRecord {
                    id: log.id,
                    ts_ms: log.ts_ms,
                    kind: log.kind,
                    payload: log.payload,
                    prev_hash: prev_hash.clone(),
                    hash: hash.clone(),
                };
                prev_hash = Some(hash);
                let mut line = serde_json::to_string(&record).map_err(|err| {
                    error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "encode_error",
                        err.to_string(),
                    )
                })?;
                line.push('\n');
                lines.push(line);
            }

            let stream = stream::iter(
                lines
                    .into_iter()
                    .map(|line| Ok::<Bytes, std::io::Error>(Bytes::from(line))),
            );
            let mut response = axum::response::Response::new(Body::from_stream(stream));
            response.headers_mut().insert(
                axum::http::header::CONTENT_TYPE,
                axum::http::HeaderValue::from_static("application/x-ndjson"),
            );
            Ok(response)
        }
        "csv" => {
            lines.push("id,ts_ms,kind,payload_json,prev_hash,hash\n".to_string());
            for log in logs {
                let hash = crate::audit_integrity::audit_chain_hash(prev_hash.as_deref(), &log);
                let payload_json = serde_json::to_string(&log.payload).unwrap_or_default();
                let line = format!(
                    "{},{},{},{},{},{}\n",
                    log.id,
                    log.ts_ms,
                    csv_escape(&log.kind),
                    csv_escape(&payload_json),
                    csv_escape(prev_hash.as_deref().unwrap_or("")),
                    csv_escape(&hash)
                );
                prev_hash = Some(hash);
                lines.push(line);
            }
            let stream = stream::iter(
                lines
                    .into_iter()
                    .map(|line| Ok::<Bytes, std::io::Error>(Bytes::from(line))),
            );
            let mut response = axum::response::Response::new(Body::from_stream(stream));
            response.headers_mut().insert(
                axum::http::header::CONTENT_TYPE,
                axum::http::HeaderValue::from_static("text/csv"),
            );
            Ok(response)
        }
        _ => Err(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            format!("unsupported export format: {format}"),
        )),
    }
}
// end inline: handlers/audit.rs
// inlined from handlers/budget_ledgers.rs
#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
pub(super) async fn list_budget_ledgers(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Query(query): Query<LedgerQuery>,
) -> Result<Json<Vec<BudgetLedgerRecord>>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;
    let tenant_scopes = if let Some(tenant_id) = admin.tenant_id.as_deref() {
        let keys = { state.list_virtual_keys_snapshot() };
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
    if let Some(store) = state.stores.sqlite.as_ref() {
        let mut ledgers = store.list_budget_ledgers().await.map_err(|err| {
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

    #[cfg(feature = "gateway-store-postgres")]
    if let Some(store) = state.stores.postgres.as_ref() {
        let mut ledgers = store.list_budget_ledgers().await.map_err(|err| {
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

    #[cfg(feature = "gateway-store-mysql")]
    if let Some(store) = state.stores.mysql.as_ref() {
        let mut ledgers = store.list_budget_ledgers().await.map_err(|err| {
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
    if let Some(store) = state.stores.redis.as_ref() {
        let mut ledgers = store.list_budget_ledgers().await.map_err(|err| {
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

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
#[derive(Debug, Serialize)]
pub(super) struct ProjectBudgetLedger {
    project_id: Option<String>,
    spent_tokens: u64,
    reserved_tokens: u64,
    key_count: usize,
    updated_at_ms: u64,
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
#[derive(Debug, Serialize)]
pub(super) struct UserBudgetLedger {
    user_id: Option<String>,
    spent_tokens: u64,
    reserved_tokens: u64,
    key_count: usize,
    updated_at_ms: u64,
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
#[derive(Debug, Serialize)]
pub(super) struct TenantBudgetLedger {
    tenant_id: Option<String>,
    spent_tokens: u64,
    reserved_tokens: u64,
    key_count: usize,
    updated_at_ms: u64,
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
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

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
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

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
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

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
pub(super) async fn list_project_budget_ledgers(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ProjectBudgetLedger>>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;

    let mut keys = { state.list_virtual_keys_snapshot() };
    if let Some(tenant_id) = admin.tenant_id.as_deref() {
        keys.retain(|key| key.tenant_id.as_deref() == Some(tenant_id));
    }

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.stores.sqlite.as_ref() {
        let ledgers = store.list_budget_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_budget_ledgers_by_project(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-postgres")]
    if let Some(store) = state.stores.postgres.as_ref() {
        let ledgers = store.list_budget_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_budget_ledgers_by_project(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-mysql")]
    if let Some(store) = state.stores.mysql.as_ref() {
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
    if let Some(store) = state.stores.redis.as_ref() {
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

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
pub(super) async fn list_user_budget_ledgers(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
) -> Result<Json<Vec<UserBudgetLedger>>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;

    let mut keys = { state.list_virtual_keys_snapshot() };
    if let Some(tenant_id) = admin.tenant_id.as_deref() {
        keys.retain(|key| key.tenant_id.as_deref() == Some(tenant_id));
    }

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.stores.sqlite.as_ref() {
        let ledgers = store.list_budget_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_budget_ledgers_by_user(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-postgres")]
    if let Some(store) = state.stores.postgres.as_ref() {
        let ledgers = store.list_budget_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_budget_ledgers_by_user(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-mysql")]
    if let Some(store) = state.stores.mysql.as_ref() {
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
    if let Some(store) = state.stores.redis.as_ref() {
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

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
pub(super) async fn list_tenant_budget_ledgers(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
) -> Result<Json<Vec<TenantBudgetLedger>>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;

    let mut keys = { state.list_virtual_keys_snapshot() };
    if let Some(tenant_id) = admin.tenant_id.as_deref() {
        keys.retain(|key| key.tenant_id.as_deref() == Some(tenant_id));
    }

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.stores.sqlite.as_ref() {
        let ledgers = store.list_budget_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_budget_ledgers_by_tenant(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-postgres")]
    if let Some(store) = state.stores.postgres.as_ref() {
        let ledgers = store.list_budget_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_budget_ledgers_by_tenant(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-mysql")]
    if let Some(store) = state.stores.mysql.as_ref() {
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
    if let Some(store) = state.stores.redis.as_ref() {
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
// end inline: handlers/budget_ledgers.rs
// inlined from handlers/cost_ledgers.rs
#[cfg(all(
    feature = "gateway-costing",
    any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ),
))]
pub(super) async fn list_cost_ledgers(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Query(query): Query<LedgerQuery>,
) -> Result<Json<Vec<CostLedgerRecord>>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;
    let tenant_scopes = if let Some(tenant_id) = admin.tenant_id.as_deref() {
        let keys = { state.list_virtual_keys_snapshot() };
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
    if let Some(store) = state.stores.sqlite.as_ref() {
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

    #[cfg(feature = "gateway-store-postgres")]
    if let Some(store) = state.stores.postgres.as_ref() {
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

    #[cfg(feature = "gateway-store-mysql")]
    if let Some(store) = state.stores.mysql.as_ref() {
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
    if let Some(store) = state.stores.redis.as_ref() {
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
    any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ),
))]
#[derive(Debug, Serialize)]
pub(super) struct ProjectCostLedger {
    project_id: Option<String>,
    spent_usd_micros: u64,
    reserved_usd_micros: u64,
    key_count: usize,
    updated_at_ms: u64,
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
#[derive(Debug, Serialize)]
pub(super) struct UserCostLedger {
    user_id: Option<String>,
    spent_usd_micros: u64,
    reserved_usd_micros: u64,
    key_count: usize,
    updated_at_ms: u64,
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
#[derive(Debug, Serialize)]
pub(super) struct TenantCostLedger {
    tenant_id: Option<String>,
    spent_usd_micros: u64,
    reserved_usd_micros: u64,
    key_count: usize,
    updated_at_ms: u64,
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
    any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ),
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
    any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ),
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
    any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ),
))]
pub(super) async fn list_project_cost_ledgers(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ProjectCostLedger>>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;

    let mut keys = { state.list_virtual_keys_snapshot() };
    if let Some(tenant_id) = admin.tenant_id.as_deref() {
        keys.retain(|key| key.tenant_id.as_deref() == Some(tenant_id));
    }

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.stores.sqlite.as_ref() {
        let ledgers = store.list_cost_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_cost_ledgers_by_project(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-postgres")]
    if let Some(store) = state.stores.postgres.as_ref() {
        let ledgers = store.list_cost_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_cost_ledgers_by_project(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-mysql")]
    if let Some(store) = state.stores.mysql.as_ref() {
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
    if let Some(store) = state.stores.redis.as_ref() {
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
    any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ),
))]
pub(super) async fn list_user_cost_ledgers(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
) -> Result<Json<Vec<UserCostLedger>>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;

    let mut keys = { state.list_virtual_keys_snapshot() };
    if let Some(tenant_id) = admin.tenant_id.as_deref() {
        keys.retain(|key| key.tenant_id.as_deref() == Some(tenant_id));
    }

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.stores.sqlite.as_ref() {
        let ledgers = store.list_cost_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_cost_ledgers_by_user(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-postgres")]
    if let Some(store) = state.stores.postgres.as_ref() {
        let ledgers = store.list_cost_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_cost_ledgers_by_user(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-mysql")]
    if let Some(store) = state.stores.mysql.as_ref() {
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
    if let Some(store) = state.stores.redis.as_ref() {
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
    any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ),
))]
pub(super) async fn list_tenant_cost_ledgers(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
) -> Result<Json<Vec<TenantCostLedger>>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;

    let mut keys = { state.list_virtual_keys_snapshot() };
    if let Some(tenant_id) = admin.tenant_id.as_deref() {
        keys.retain(|key| key.tenant_id.as_deref() == Some(tenant_id));
    }

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.stores.sqlite.as_ref() {
        let ledgers = store.list_cost_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_cost_ledgers_by_tenant(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-postgres")]
    if let Some(store) = state.stores.postgres.as_ref() {
        let ledgers = store.list_cost_ledgers().await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(Json(group_cost_ledgers_by_tenant(&ledgers, &keys)));
    }

    #[cfg(feature = "gateway-store-mysql")]
    if let Some(store) = state.stores.mysql.as_ref() {
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
    if let Some(store) = state.stores.redis.as_ref() {
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
// end inline: handlers/cost_ledgers.rs
// end inline: admin/handlers.rs
// inlined from admin/backends.rs
#[cfg(feature = "gateway-routing-advanced")]
pub(super) async fn list_backends(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
) -> Result<Json<Vec<BackendHealthSnapshot>>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;
    if admin.tenant_id.is_some() {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "tenant-scoped admin tokens cannot access backend health",
        ));
    }

    let Some(health) = state.proxy.backend_health.as_ref() else {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "not_configured",
            "proxy routing not enabled",
        ));
    };

    let mut names: Vec<String> = state.backends.proxy_backends.keys().cloned().collect();
    names.sort();

    let mut out = Vec::with_capacity(names.len());
    {
        let health = health.lock().await;
        for name in names {
            let snapshot = health
                .get(name.as_str())
                .map(|entry| entry.snapshot(&name))
                .unwrap_or_else(|| BackendHealth::default().snapshot(&name));
            out.push(snapshot);
        }
        drop(health);
    }

    Ok(Json(out))
}

#[cfg(feature = "gateway-routing-advanced")]
pub(super) async fn reset_backend(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<Json<BackendHealthSnapshot>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_write(&state, &headers)?;
    if admin.tenant_id.is_some() {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "tenant-scoped admin tokens cannot reset backends",
        ));
    }

    let Some(health) = state.proxy.backend_health.as_ref() else {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "not_configured",
            "proxy routing not enabled",
        ));
    };

    let mut health = health.lock().await;
    health.remove(name.as_str());
    drop(health);

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    append_admin_audit_log(
        &state,
        "admin.backend.reset",
        serde_json::json!({
            "backend": &name,
        }),
    )
    .await?;

    Ok(Json(BackendHealth::default().snapshot(&name)))
}
// end inline: admin/backends.rs
// inlined from admin/keys.rs
pub(super) async fn list_keys(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Query(query): Query<ListKeysQuery>,
) -> Result<Json<Vec<VirtualKeyConfig>>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;
    if query.include_tokens {
        ensure_admin_secret_access(&admin)?;
    }
    let mut keys = state.list_virtual_keys_snapshot();

    if let Some(enabled) = query.enabled {
        keys.retain(|key| key.enabled == enabled);
    }

    if let Some(prefix) = query
        .id_prefix
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        keys.retain(|key| key.id.starts_with(prefix));
    }

    let tenant_filter = if let Some(admin_tenant) = admin.tenant_id.as_deref() {
        if let Some(query_tenant) = query
            .tenant_id
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            && query_tenant != admin_tenant
        {
            return Err(error_response(
                StatusCode::FORBIDDEN,
                "forbidden",
                "cross-tenant admin access is not allowed",
            ));
        }
        Some(admin_tenant)
    } else {
        query
            .tenant_id
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
    };

    if let Some(tenant_id) = tenant_filter {
        keys.retain(|key| key.tenant_id.as_deref() == Some(tenant_id));
    }

    if let Some(project_id) = query
        .project_id
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        keys.retain(|key| key.project_id.as_deref() == Some(project_id));
    }

    if let Some(user_id) = query
        .user_id
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        keys.retain(|key| key.user_id.as_deref() == Some(user_id));
    }

    keys.sort_by(|a, b| a.id.cmp(&b.id));

    if query.offset > 0 {
        if query.offset >= keys.len() {
            keys.clear();
        } else {
            keys.drain(0..query.offset);
        }
    }

    if let Some(limit) = query.limit.map(|limit| limit.min(MAX_ADMIN_LIST_LIMIT))
        && keys.len() > limit
    {
        keys.truncate(limit);
    }

    if !query.include_tokens {
        for key in &mut keys {
            key.token = "redacted".to_string();
        }
    }
    Ok(Json(keys))
}

pub(super) async fn upsert_key(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Json(key): Json<VirtualKeyConfig>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_write(&state, &headers)?;
    let mut key = key;
    if let Some(admin_tenant) = admin.tenant_id.as_deref() {
        if let Some(tenant_id) = key.tenant_id.as_deref() {
            if tenant_id != admin_tenant {
                return Err(error_response(
                    StatusCode::FORBIDDEN,
                    "forbidden",
                    "cannot upsert keys for a different tenant",
                ));
            }
        } else {
            key.tenant_id = Some(admin_tenant.to_string());
        }
    }
    let backend_names = state
        .backend_names_snapshot()
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    crate::gateway::config::validate_virtual_key_payload(&key, 0, &backend_names)
        .map_err(map_gateway_error)?;
    let (inserted, _) = apply_control_plane_change(&state, "admin.key.upsert", |gateway| {
        Ok(gateway.upsert_virtual_key(key.clone()))
    })
    .await?;

    #[cfg(feature = "sdk")]
    emit_devtools_log(
        &state,
        "admin.key.upsert",
        serde_json::json!({
            "key_id": &key.id,
            "enabled": key.enabled,
            "inserted": inserted,
        }),
    );

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    append_admin_audit_log(
        &state,
        "admin.key.upsert",
        serde_json::json!({
            "key_id": &key.id,
            "enabled": key.enabled,
            "inserted": inserted,
            "tenant_id": key.tenant_id.as_deref(),
            "project_id": key.project_id.as_deref(),
            "user_id": key.user_id.as_deref(),
        }),
    )
    .await?;

    let status = if inserted {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(key)))
}

pub(super) async fn upsert_key_with_id(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(mut key): Json<VirtualKeyConfig>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_write(&state, &headers)?;
    key.id = id;
    if let Some(admin_tenant) = admin.tenant_id.as_deref() {
        if let Some(tenant_id) = key.tenant_id.as_deref() {
            if tenant_id != admin_tenant {
                return Err(error_response(
                    StatusCode::FORBIDDEN,
                    "forbidden",
                    "cannot upsert keys for a different tenant",
                ));
            }
        } else {
            key.tenant_id = Some(admin_tenant.to_string());
        }
    }
    let backend_names = state
        .backend_names_snapshot()
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    crate::gateway::config::validate_virtual_key_payload(&key, 0, &backend_names)
        .map_err(map_gateway_error)?;
    let (inserted, _) = apply_control_plane_change(&state, "admin.key.upsert", |gateway| {
        Ok(gateway.upsert_virtual_key(key.clone()))
    })
    .await?;

    #[cfg(feature = "sdk")]
    emit_devtools_log(
        &state,
        "admin.key.upsert",
        serde_json::json!({
            "key_id": &key.id,
            "enabled": key.enabled,
            "inserted": inserted,
        }),
    );

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    append_admin_audit_log(
        &state,
        "admin.key.upsert",
        serde_json::json!({
            "key_id": &key.id,
            "enabled": key.enabled,
            "inserted": inserted,
            "tenant_id": key.tenant_id.as_deref(),
            "project_id": key.project_id.as_deref(),
            "user_id": key.user_id.as_deref(),
        }),
    )
    .await?;

    let status = if inserted {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(key)))
}

pub(super) async fn delete_key(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_write(&state, &headers)?;
    apply_control_plane_change(&state, "admin.key.delete", |gateway| {
        if let Some(admin_tenant) = admin.tenant_id.as_deref() {
            let existing = gateway.list_virtual_keys();
            let Some(existing_key) = existing.iter().find(|key| key.id == id) else {
                return Err(error_response(
                    StatusCode::NOT_FOUND,
                    "not_found",
                    "virtual key not found",
                ));
            };
            if existing_key.tenant_id.as_deref() != Some(admin_tenant) {
                return Err(error_response(
                    StatusCode::FORBIDDEN,
                    "forbidden",
                    "cannot delete keys for a different tenant",
                ));
            }
        }
        gateway.remove_virtual_key(&id).ok_or_else(|| {
            error_response(StatusCode::NOT_FOUND, "not_found", "virtual key not found")
        })?;
        Ok(())
    })
    .await?;

    #[cfg(feature = "sdk")]
    emit_devtools_log(
        &state,
        "admin.key.delete",
        serde_json::json!({
            "key_id": &id,
        }),
    );

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    append_admin_audit_log(
        &state,
        "admin.key.delete",
        serde_json::json!({
            "key_id": &id,
            "tenant_id": admin.tenant_id.as_deref(),
        }),
    )
    .await?;

    Ok(StatusCode::NO_CONTENT)
}

const MAX_ADMIN_LIST_LIMIT: usize = 10_000;

#[derive(Debug, Deserialize)]
pub(super) struct ListKeysQuery {
    #[serde(default)]
    include_tokens: bool,
    #[serde(default)]
    tenant_id: Option<String>,
    #[serde(default)]
    project_id: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    id_prefix: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: usize,
}
// end inline: admin/keys.rs
// inlined from admin/maintenance.rs
#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
#[derive(Debug, Deserialize)]
pub(super) struct ReapReservationsRequest {
    #[serde(default = "default_reap_reservations_older_than_secs")]
    older_than_secs: u64,
    #[serde(default = "default_reap_reservations_limit")]
    limit: usize,
    #[serde(default)]
    dry_run: bool,
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
fn default_reap_reservations_older_than_secs() -> u64 {
    24 * 60 * 60
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
fn default_reap_reservations_limit() -> usize {
    1000
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
#[derive(Debug, Serialize)]
pub(super) struct ReapReservationsCounts {
    scanned: u64,
    reaped: u64,
    released: u64,
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
#[derive(Debug, Serialize)]
pub(super) struct ReapReservationsResponse {
    store: &'static str,
    dry_run: bool,
    cutoff_ts_ms: u64,
    budget: ReapReservationsCounts,
    cost: ReapReservationsCounts,
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
fn now_millis_u64() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
pub(super) async fn reap_reservations(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Json(payload): Json<ReapReservationsRequest>,
) -> Result<Json<ReapReservationsResponse>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_write(&state, &headers)?;
    if admin.tenant_id.is_some() {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "tenant-scoped admin tokens cannot reap reservations",
        ));
    }

    let now_ts_ms = now_millis_u64();
    let cutoff_ts_ms = now_ts_ms.saturating_sub(payload.older_than_secs.saturating_mul(1000));
    let limit = payload.limit.clamp(1, 100_000);
    let dry_run = payload.dry_run;

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.stores.sqlite.as_ref() {
        let (budget_scanned, budget_reaped, budget_released) = store
            .reap_stale_budget_reservations(cutoff_ts_ms, limit, dry_run)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;
        let (cost_scanned, cost_reaped, cost_released) = store
            .reap_stale_cost_reservations(cutoff_ts_ms, limit, dry_run)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;

        return Ok(Json(ReapReservationsResponse {
            store: "sqlite",
            dry_run,
            cutoff_ts_ms,
            budget: ReapReservationsCounts {
                scanned: budget_scanned,
                reaped: budget_reaped,
                released: budget_released,
            },
            cost: ReapReservationsCounts {
                scanned: cost_scanned,
                reaped: cost_reaped,
                released: cost_released,
            },
        }));
    }

    #[cfg(feature = "gateway-store-postgres")]
    if let Some(store) = state.stores.postgres.as_ref() {
        let (budget_scanned, budget_reaped, budget_released) = store
            .reap_stale_budget_reservations(cutoff_ts_ms, limit, dry_run)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;
        let (cost_scanned, cost_reaped, cost_released) = store
            .reap_stale_cost_reservations(cutoff_ts_ms, limit, dry_run)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;

        return Ok(Json(ReapReservationsResponse {
            store: "postgres",
            dry_run,
            cutoff_ts_ms,
            budget: ReapReservationsCounts {
                scanned: budget_scanned,
                reaped: budget_reaped,
                released: budget_released,
            },
            cost: ReapReservationsCounts {
                scanned: cost_scanned,
                reaped: cost_reaped,
                released: cost_released,
            },
        }));
    }

    #[cfg(feature = "gateway-store-mysql")]
    if let Some(store) = state.stores.mysql.as_ref() {
        let (budget_scanned, budget_reaped, budget_released) = store
            .reap_stale_budget_reservations(cutoff_ts_ms, limit, dry_run)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;
        let (cost_scanned, cost_reaped, cost_released) = store
            .reap_stale_cost_reservations(cutoff_ts_ms, limit, dry_run)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;

        return Ok(Json(ReapReservationsResponse {
            store: "mysql",
            dry_run,
            cutoff_ts_ms,
            budget: ReapReservationsCounts {
                scanned: budget_scanned,
                reaped: budget_reaped,
                released: budget_released,
            },
            cost: ReapReservationsCounts {
                scanned: cost_scanned,
                reaped: cost_reaped,
                released: cost_released,
            },
        }));
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.stores.redis.as_ref() {
        let (budget_scanned, budget_reaped, budget_released) = store
            .reap_stale_budget_reservations(cutoff_ts_ms, limit, dry_run)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;
        let (cost_scanned, cost_reaped, cost_released) = store
            .reap_stale_cost_reservations(cutoff_ts_ms, limit, dry_run)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;

        return Ok(Json(ReapReservationsResponse {
            store: "redis",
            dry_run,
            cutoff_ts_ms,
            budget: ReapReservationsCounts {
                scanned: budget_scanned,
                reaped: budget_reaped,
                released: budget_released,
            },
            cost: ReapReservationsCounts {
                scanned: cost_scanned,
                reaped: cost_reaped,
                released: cost_released,
            },
        }));
    }

    Err(error_response(
        StatusCode::BAD_REQUEST,
        "not_configured",
        "store not configured",
    ))
}
// end inline: admin/maintenance.rs
// inlined from admin/ledger_grouping_tests.rs
#[cfg(all(
    test,
    any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    )
))]
mod ledger_grouping_tests {
    use super::*;

    #[test]
    fn budget_ledgers_group_by_project_and_user() {
        let mut key_1 = VirtualKeyConfig::new("key-1", "vk-1");
        key_1.tenant_id = Some("tenant-a".to_string());
        key_1.project_id = Some("proj-a".to_string());
        key_1.user_id = Some("user-a".to_string());

        let mut key_2 = VirtualKeyConfig::new("key-2", "vk-2");
        key_2.tenant_id = Some("tenant-a".to_string());
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

        let tenants = group_budget_ledgers_by_tenant(&ledgers, &keys);
        assert_eq!(tenants.len(), 2);
        assert_eq!(tenants[0].tenant_id, None);
        assert_eq!(tenants[0].spent_tokens, 1);
        assert_eq!(tenants[0].reserved_tokens, 2);
        assert_eq!(tenants[0].key_count, 1);
        assert_eq!(tenants[0].updated_at_ms, 50);
        assert_eq!(tenants[1].tenant_id.as_deref(), Some("tenant-a"));
        assert_eq!(tenants[1].spent_tokens, 17);
        assert_eq!(tenants[1].reserved_tokens, 3);
        assert_eq!(tenants[1].key_count, 2);
        assert_eq!(tenants[1].updated_at_ms, 200);
    }

    #[cfg(feature = "gateway-costing")]
    #[test]
    fn cost_ledgers_group_by_project_and_user() {
        let mut key_1 = VirtualKeyConfig::new("key-1", "vk-1");
        key_1.tenant_id = Some("tenant-a".to_string());
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

        let tenants = group_cost_ledgers_by_tenant(&ledgers, &keys);
        assert_eq!(tenants.len(), 2);
        assert_eq!(tenants[0].tenant_id, None);
        assert_eq!(tenants[0].spent_usd_micros, 1);
        assert_eq!(tenants[0].reserved_usd_micros, 2);
        assert_eq!(tenants[0].key_count, 1);
        assert_eq!(tenants[0].updated_at_ms, 50);
        assert_eq!(tenants[1].tenant_id.as_deref(), Some("tenant-a"));
        assert_eq!(tenants[1].spent_usd_micros, 10);
        assert_eq!(tenants[1].reserved_usd_micros, 3);
        assert_eq!(tenants[1].key_count, 1);
        assert_eq!(tenants[1].updated_at_ms, 100);
    }
}
// end inline: admin/ledger_grouping_tests.rs
pub(super) fn map_gateway_error(err: GatewayError) -> (StatusCode, Json<ErrorResponse>) {
    match err {
        GatewayError::Unauthorized => error_response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "unauthorized virtual key",
        ),
        GatewayError::RateLimited { limit } => error_response(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            format!("rate limit exceeded: {limit}"),
        ),
        GatewayError::GuardrailRejected { reason } => error_response(
            StatusCode::FORBIDDEN,
            "guardrail_rejected",
            format!("guardrail rejected: {reason}"),
        ),
        GatewayError::BudgetExceeded { limit, attempted } => error_response(
            StatusCode::PAYMENT_REQUIRED,
            "budget_exceeded",
            format!("budget exceeded: limit={limit} attempted={attempted}"),
        ),
        GatewayError::CostBudgetExceeded {
            limit_usd_micros,
            attempted_usd_micros,
        } => error_response(
            StatusCode::PAYMENT_REQUIRED,
            "cost_budget_exceeded",
            format!(
                "cost budget exceeded: limit_usd_micros={limit_usd_micros} attempted_usd_micros={attempted_usd_micros}"
            ),
        ),
        GatewayError::BackendNotFound { name } => error_response(
            StatusCode::BAD_GATEWAY,
            "backend_not_found",
            format!("backend not found: {name}"),
        ),
        GatewayError::Backend { message } => {
            error_response(StatusCode::BAD_GATEWAY, "backend_error", message)
        }
        GatewayError::BackendTimeout { message } => {
            error_response(StatusCode::GATEWAY_TIMEOUT, "backend_timeout", message)
        }
        GatewayError::InvalidRequest { reason } => {
            error_response(StatusCode::BAD_REQUEST, "invalid_request", reason)
        }
    }
}

pub(super) fn error_response(
    status: StatusCode,
    code: &'static str,
    message: impl Into<String>,
) -> (StatusCode, Json<ErrorResponse>) {
    (
        status,
        Json(ErrorResponse {
            error: ErrorDetail {
                code,
                message: message.into(),
            },
        }),
    )
}

#[cfg(test)]
mod admin_auth_tests {
    use super::*;

    fn test_state() -> GatewayHttpState {
        let config = crate::gateway::GatewayConfig {
            backends: Vec::new(),
            virtual_keys: Vec::new(),
            router: crate::gateway::RouterConfig {
                default_backends: Vec::new(),
                rules: Vec::new(),
            },
            a2a_agents: Vec::new(),
            mcp_servers: Vec::new(),
            observability: Default::default(),
        };
        GatewayHttpState::new(crate::gateway::Gateway::new(config))
    }

    #[test]
    fn ensure_admin_read_rejects_when_not_configured() {
        let state = test_state();
        let headers = HeaderMap::new();
        let (status, Json(body)) = ensure_admin_read(&state, &headers).unwrap_err();
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body.error.code, "not_configured");
    }

    #[test]
    fn ensure_admin_write_rejects_when_not_configured() {
        let state = test_state();
        let headers = HeaderMap::new();
        let (status, Json(body)) = ensure_admin_write(&state, &headers).unwrap_err();
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body.error.code, "not_configured");
    }
}
// end inline: admin/auth.rs
