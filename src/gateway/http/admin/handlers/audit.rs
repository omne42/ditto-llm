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
struct AuditExportQuery {
    #[serde(default)]
    format: Option<String>,
    #[serde(default = "default_audit_export_limit")]
    limit: usize,
    #[serde(default)]
    since_ts_ms: Option<u64>,
    #[serde(default)]
    before_ts_ms: Option<u64>,
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
async fn list_audit_logs(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Query(query): Query<AuditQuery>,
) -> Result<Json<Vec<AuditLogRecord>>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.sqlite_store.as_ref() {
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
    if let Some(store) = state.redis_store.as_ref() {
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

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
fn audit_chain_hash(prev_hash: Option<&str>, record: &AuditLogRecord) -> String {
    use sha2::Digest as _;

    let mut hasher = sha2::Sha256::new();
    if let Some(prev_hash) = prev_hash {
        hasher.update(prev_hash.as_bytes());
    }
    hasher.update(b"\n");
    if let Ok(serialized) = serde_json::to_vec(record) {
        hasher.update(&serialized);
    }
    hex_lower(&hasher.finalize())
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
fn csv_escape(value: &str) -> String {
    if !value.contains([',', '"', '\n', '\r']) {
        return value.to_string();
    }
    let escaped = value.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
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

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
async fn export_audit_logs(
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
    if let Some(store) = state.sqlite_store.as_ref() {
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
    if let Some(store) = state.redis_store.as_ref() {
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

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
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
                let hash = audit_chain_hash(prev_hash.as_deref(), &log);
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

            let stream = stream::iter(lines.into_iter().map(|line| {
                Ok::<Bytes, std::io::Error>(Bytes::from(line))
            }));
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
                let hash = audit_chain_hash(prev_hash.as_deref(), &log);
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
            let stream = stream::iter(lines.into_iter().map(|line| {
                Ok::<Bytes, std::io::Error>(Bytes::from(line))
            }));
            let mut response = axum::response::Response::new(Body::from_stream(stream));
            response
                .headers_mut()
                .insert(
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
