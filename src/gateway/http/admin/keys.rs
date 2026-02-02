async fn list_keys(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Query(query): Query<ListKeysQuery>,
) -> Result<Json<Vec<VirtualKeyConfig>>, (StatusCode, Json<ErrorResponse>)> {
    ensure_admin_read(&state, &headers)?;
    let gateway = state.gateway.lock().await;
    let mut keys = gateway.list_virtual_keys();

    if let Some(enabled) = query.enabled {
        keys.retain(|key| key.enabled == enabled);
    }

    if let Some(prefix) = query.id_prefix.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        keys.retain(|key| key.id.starts_with(prefix));
    }

    if let Some(tenant_id) = query
        .tenant_id
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
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

    if let Some(limit) = query.limit.map(|limit| limit.min(MAX_ADMIN_LIST_LIMIT)) {
        if keys.len() > limit {
            keys.truncate(limit);
        }
    }

    if !query.include_tokens {
        for key in &mut keys {
            key.token = "redacted".to_string();
        }
    }
    Ok(Json(keys))
}

async fn upsert_key(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Json(key): Json<VirtualKeyConfig>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    ensure_admin_write(&state, &headers)?;
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

    #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
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
    .await;

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
    ensure_admin_write(&state, &headers)?;
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

    #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
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
    .await;

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
    ensure_admin_write(&state, &headers)?;
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

        #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
        append_admin_audit_log(
            &state,
            "admin.key.delete",
            serde_json::json!({
                "key_id": &id,
            }),
        )
        .await;

        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "virtual key not found",
        ))
    }
}

const MAX_ADMIN_LIST_LIMIT: usize = 10_000;

#[derive(Debug, Deserialize)]
struct ListKeysQuery {
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
