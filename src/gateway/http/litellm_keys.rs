static LITELLM_KEY_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Deserialize)]
struct LitellmKeyGenerateRequest {
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    key_alias: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    team_id: Option<String>,
    #[serde(default)]
    organization_id: Option<String>,
    #[serde(default)]
    models: Vec<String>,
    #[serde(default)]
    max_budget: Option<f64>,
    #[serde(default)]
    rpm_limit: Option<u32>,
    #[serde(default)]
    tpm_limit: Option<u32>,
}

#[derive(Debug, Serialize)]
struct LitellmKeyGenerateResponse {
    key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    key_alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    key_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    team_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    organization_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_budget: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rpm_limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tpm_limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    models: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct LitellmKeyDeleteRequest {
    #[serde(default)]
    keys: Option<Vec<String>>,
    #[serde(default)]
    key_aliases: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
struct LitellmKeyDeleteResponse {
    deleted_keys: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct LitellmKeyInfoQuery {
    #[serde(default)]
    key: Option<String>,
}

#[derive(Debug, Serialize)]
struct LitellmKeyInfoResponse {
    key: String,
    info: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct LitellmKeyListQuery {
    #[serde(default)]
    page: Option<usize>,
    #[serde(default)]
    size: Option<usize>,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    team_id: Option<String>,
    #[serde(default)]
    organization_id: Option<String>,
    #[serde(default)]
    key_alias: Option<String>,
    #[serde(default)]
    return_full_object: Option<bool>,
}

#[derive(Debug, Serialize)]
struct LitellmKeyListResponse {
    keys: Vec<serde_json::Value>,
    total_count: usize,
    current_page: usize,
    total_pages: usize,
}

fn litellm_key_info_value(key: &VirtualKeyConfig) -> serde_json::Value {
    serde_json::json!({
        "key_alias": key.id,
        "key_name": key.id,
        "user_id": key.user_id,
        "team_id": key.tenant_id,
        "enabled": key.enabled,
        "blocked": !key.enabled,
        "tpm_limit": key.limits.tpm,
        "rpm_limit": key.limits.rpm,
        "max_budget": key.budget.total_usd_micros.map(|v| (v as f64) / 1_000_000.0),
        "models": key.guardrails.allow_models,
    })
}

fn litellm_key_full_value(key: &VirtualKeyConfig) -> serde_json::Value {
    let mut value = litellm_key_info_value(key);
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "token".to_string(),
            serde_json::Value::String(key.token.clone()),
        );
    }
    value
}

fn litellm_key_router() -> Router<GatewayHttpState> {
    Router::new()
        .route("/key/generate", post(litellm_key_generate))
        .route("/key/update", post(litellm_key_update))
        .route("/key/delete", post(litellm_key_delete))
        .route("/key/info", get(litellm_key_info))
        .route("/key/list", get(litellm_key_list))
        .route("/key/regenerate", post(litellm_key_regenerate))
        .route("/key/:key/regenerate", post(litellm_key_regenerate_path))
}

async fn litellm_key_generate(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Json(payload): Json<LitellmKeyGenerateRequest>,
) -> Result<Json<LitellmKeyGenerateResponse>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_write(&state, &headers)?;

    let key_alias = payload
        .key_alias
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .unwrap_or_else(generate_key_id);

    let key = payload
        .key
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .unwrap_or_else(generate_key_token);

    let tenant_id = payload
        .organization_id
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .or_else(|| {
            payload
                .team_id
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(|v| v.to_string())
        });

    let mut virtual_key = VirtualKeyConfig::new(key_alias.clone(), key.clone());
    virtual_key.enabled = true;
    virtual_key.tenant_id = tenant_id.clone();
    virtual_key.user_id = payload.user_id.clone();
    virtual_key.limits.rpm = payload.rpm_limit;
    virtual_key.limits.tpm = payload.tpm_limit;

    if !payload.models.is_empty() {
        virtual_key.guardrails.allow_models = payload.models.clone();
    }

    if let Some(max_budget) = payload.max_budget {
        if !max_budget.is_finite() || max_budget < 0.0 {
            return Err(error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "max_budget must be a non-negative finite number",
            ));
        }
        let micros = (max_budget * 1_000_000.0).round();
        if micros > 0.0 {
            virtual_key.budget.total_usd_micros = Some(micros as u64);
        }
    }

    if let Some(admin_tenant) = admin.tenant_id.as_deref() {
        if tenant_id.as_deref().is_some_and(|t| t != admin_tenant) {
            return Err(error_response(
                StatusCode::FORBIDDEN,
                "forbidden",
                "cannot generate keys for a different tenant",
            ));
        }
        if virtual_key.tenant_id.is_none() {
            virtual_key.tenant_id = Some(admin_tenant.to_string());
        }
    }

    let persisted_keys = {
        let mut gateway = state.gateway.lock().await;
        if gateway
            .list_virtual_keys()
            .iter()
            .any(|existing| existing.id == virtual_key.id)
        {
            return Err(error_response(
                StatusCode::CONFLICT,
                "conflict",
                "key_alias already exists",
            ));
        }
        gateway.upsert_virtual_key(virtual_key.clone());
        gateway.list_virtual_keys()
    };
    persist_virtual_keys(&state, &persisted_keys).await?;

    #[cfg(feature = "sdk")]
    emit_devtools_log(
        &state,
        "litellm.key.generate",
        serde_json::json!({
            "key_id": &virtual_key.id,
            "tenant_id": virtual_key.tenant_id.as_deref(),
        }),
    );

    #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
    append_admin_audit_log(
        &state,
        "litellm.key.generate",
        serde_json::json!({
            "key_id": &virtual_key.id,
            "tenant_id": virtual_key.tenant_id.as_deref(),
            "user_id": virtual_key.user_id.as_deref(),
        }),
    )
    .await;

    Ok(Json(LitellmKeyGenerateResponse {
        key: key.clone(),
        token: Some(key),
        key_alias: Some(key_alias),
        key_name: Some(virtual_key.id),
        user_id: payload.user_id,
        team_id: payload.team_id,
        organization_id: payload.organization_id,
        max_budget: payload.max_budget,
        rpm_limit: payload.rpm_limit,
        tpm_limit: payload.tpm_limit,
        models: payload.models,
    }))
}

#[derive(Debug, Deserialize)]
struct LitellmKeyUpdateRequest {
    key: String,
    #[serde(default)]
    key_alias: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    team_id: Option<String>,
    #[serde(default)]
    organization_id: Option<String>,
    #[serde(default)]
    models: Option<Vec<String>>,
    #[serde(default)]
    max_budget: Option<f64>,
    #[serde(default)]
    rpm_limit: Option<u32>,
    #[serde(default)]
    tpm_limit: Option<u32>,
    #[serde(default)]
    blocked: Option<bool>,
}

fn litellm_generate_response_from_virtual_key(key: &VirtualKeyConfig) -> LitellmKeyGenerateResponse {
    LitellmKeyGenerateResponse {
        key: key.token.clone(),
        token: Some(key.token.clone()),
        key_alias: Some(key.id.clone()),
        key_name: Some(key.id.clone()),
        user_id: key.user_id.clone(),
        team_id: key.tenant_id.clone(),
        organization_id: key.tenant_id.clone(),
        max_budget: key
            .budget
            .total_usd_micros
            .map(|v| (v as f64) / 1_000_000.0),
        rpm_limit: key.limits.rpm,
        tpm_limit: key.limits.tpm,
        models: key.guardrails.allow_models.clone(),
    }
}

async fn litellm_key_update(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Json(payload): Json<LitellmKeyUpdateRequest>,
) -> Result<Json<LitellmKeyGenerateResponse>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_write(&state, &headers)?;

    let key_token = payload.key.trim();
    if key_token.is_empty() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "key is required",
        ));
    }

    let new_alias = payload
        .key_alias
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string());

    let tenant_id = payload
        .organization_id
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .or_else(|| {
            payload
                .team_id
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(|v| v.to_string())
        });

    let (key, persisted_keys) = {
        let mut gateway = state.gateway.lock().await;
        let keys = gateway.list_virtual_keys();

        let Some(existing) = keys.iter().find(|key| key.token == key_token).cloned() else {
            return Err(error_response(
                StatusCode::NOT_FOUND,
                "not_found",
                "virtual key not found",
            ));
        };

        if let Some(admin_tenant) = admin.tenant_id.as_deref() {
            if existing.tenant_id.as_deref() != Some(admin_tenant) {
                return Err(error_response(
                    StatusCode::FORBIDDEN,
                    "forbidden",
                    "cannot update keys for a different tenant",
                ));
            }
        }

        let mut key = existing.clone();

        if let Some(user_id) = payload.user_id.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
            key.user_id = Some(user_id.to_string());
        }

        if let Some(tenant_id) = tenant_id.as_deref() {
            if let Some(admin_tenant) = admin.tenant_id.as_deref() {
                if tenant_id != admin_tenant {
                    return Err(error_response(
                        StatusCode::FORBIDDEN,
                        "forbidden",
                        "cannot update keys for a different tenant",
                    ));
                }
            }
            key.tenant_id = Some(tenant_id.to_string());
        }

        if let Some(models) = payload.models.as_ref() {
            key.guardrails.allow_models = models.clone();
        }

        if let Some(max_budget) = payload.max_budget {
            if !max_budget.is_finite() || max_budget < 0.0 {
                return Err(error_response(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "max_budget must be a non-negative finite number",
                ));
            }
            let micros = (max_budget * 1_000_000.0).round();
            if micros <= 0.0 {
                key.budget.total_usd_micros = None;
            } else {
                key.budget.total_usd_micros = Some(micros as u64);
            }
        }

        if let Some(rpm) = payload.rpm_limit {
            key.limits.rpm = Some(rpm);
        }
        if let Some(tpm) = payload.tpm_limit {
            key.limits.tpm = Some(tpm);
        }

        if let Some(blocked) = payload.blocked {
            key.enabled = !blocked;
        }

        let old_id = existing.id.clone();
        if let Some(new_id) = new_alias.as_deref() {
            if new_id != old_id {
                if keys.iter().any(|candidate| candidate.id == new_id) {
                    return Err(error_response(
                        StatusCode::CONFLICT,
                        "conflict",
                        "key_alias already exists",
                    ));
                }
                gateway.remove_virtual_key(&old_id);
                key.id = new_id.to_string();
            }
        }

        gateway.upsert_virtual_key(key.clone());
        let persisted_keys = gateway.list_virtual_keys();
        (key, persisted_keys)
    };

    persist_virtual_keys(&state, &persisted_keys).await?;

    #[cfg(feature = "sdk")]
    emit_devtools_log(
        &state,
        "litellm.key.update",
        serde_json::json!({
            "key_id": &key.id,
            "tenant_id": key.tenant_id.as_deref(),
        }),
    );

    #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
    append_admin_audit_log(
        &state,
        "litellm.key.update",
        serde_json::json!({
            "key_id": &key.id,
            "tenant_id": key.tenant_id.as_deref(),
            "user_id": key.user_id.as_deref(),
        }),
    )
    .await;

    Ok(Json(litellm_generate_response_from_virtual_key(&key)))
}

async fn litellm_key_delete(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Json(payload): Json<LitellmKeyDeleteRequest>,
) -> Result<Json<LitellmKeyDeleteResponse>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_write(&state, &headers)?;

    let mut keys = payload.keys.unwrap_or_default();
    let mut aliases = payload.key_aliases.unwrap_or_default();
    keys.retain(|value| !value.trim().is_empty());
    aliases.retain(|value| !value.trim().is_empty());

    if keys.is_empty() && aliases.is_empty() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "expected keys or key_aliases",
        ));
    }

    let mut deleted_keys: Vec<String> = Vec::new();
    let mut deleted_key_ids: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    let persisted_keys = {
        let mut gateway = state.gateway.lock().await;
        let mut current = gateway.list_virtual_keys();

        for alias in aliases {
            let alias = alias.trim().to_string();
            if alias.is_empty() {
                continue;
            }
            let Some((found_id, found_tenant)) = current
                .iter()
                .find(|key| key.id == alias)
                .map(|key| (key.id.clone(), key.tenant_id.clone()))
            else {
                missing.push(alias);
                continue;
            };
            if let Some(admin_tenant) = admin.tenant_id.as_deref() {
                if found_tenant.as_deref() != Some(admin_tenant) {
                    return Err(error_response(
                        StatusCode::FORBIDDEN,
                        "forbidden",
                        "cannot delete keys for a different tenant",
                    ));
                }
            }
            if gateway.remove_virtual_key(&found_id).is_some() {
                deleted_keys.push(alias);
                deleted_key_ids.push(found_id.clone());
                current.retain(|key| key.id != found_id);
            } else {
                missing.push(alias);
            }
        }

        for token in keys {
            let token = token.trim().to_string();
            if token.is_empty() {
                continue;
            }
            let Some((found_id, found_tenant)) = current
                .iter()
                .find(|key| key.token == token)
                .map(|key| (key.id.clone(), key.tenant_id.clone()))
            else {
                missing.push(token);
                continue;
            };
            if let Some(admin_tenant) = admin.tenant_id.as_deref() {
                if found_tenant.as_deref() != Some(admin_tenant) {
                    return Err(error_response(
                        StatusCode::FORBIDDEN,
                        "forbidden",
                        "cannot delete keys for a different tenant",
                    ));
                }
            }
            if gateway.remove_virtual_key(&found_id).is_some() {
                deleted_keys.push(token);
                deleted_key_ids.push(found_id.clone());
                current.retain(|key| key.id != found_id);
            } else {
                missing.push(token);
            }
        }

        gateway.list_virtual_keys()
    };

    if !missing.is_empty() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "not all keys passed in were deleted",
        ));
    }

    persist_virtual_keys(&state, &persisted_keys).await?;

    #[cfg(feature = "sdk")]
    emit_devtools_log(
        &state,
        "litellm.key.delete",
        serde_json::json!({
            "deleted": deleted_keys.len(),
            "tenant_id": admin.tenant_id.as_deref(),
        }),
    );

    #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
    append_admin_audit_log(
        &state,
        "litellm.key.delete",
        serde_json::json!({
            "deleted": deleted_key_ids.len(),
            "deleted_key_ids": &deleted_key_ids,
            "tenant_id": admin.tenant_id.as_deref(),
        }),
    )
    .await;

    Ok(Json(LitellmKeyDeleteResponse {
        deleted_keys,
    }))
}

async fn litellm_key_info(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Query(query): Query<LitellmKeyInfoQuery>,
) -> Result<Json<LitellmKeyInfoResponse>, (StatusCode, Json<ErrorResponse>)> {
    let requested_token = query
        .key
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());

    let bearer_token = extract_bearer(&headers);

    let (admin, token) = if let Some(token) = requested_token {
        if bearer_token.as_deref() == Some(token) {
            (None, token.to_string())
        } else {
            let admin = ensure_admin_read(&state, &headers)?;
            (Some(admin), token.to_string())
        }
    } else {
        let token = bearer_token.ok_or_else(|| {
            error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "key is required",
            )
        })?;
        (None, token)
    };

    let gateway = state.gateway.lock().await;
    let key = gateway
        .list_virtual_keys()
        .into_iter()
        .find(|key| key.token == token)
        .ok_or_else(|| {
            error_response(
                StatusCode::NOT_FOUND,
                "not_found",
                "virtual key not found",
            )
        })?;

    if let Some(admin) = admin {
        if let Some(admin_tenant) = admin.tenant_id.as_deref() {
            if key.tenant_id.as_deref() != Some(admin_tenant) {
                return Err(error_response(
                    StatusCode::FORBIDDEN,
                    "forbidden",
                    "cannot access keys for a different tenant",
                ));
            }
        }
    }

    Ok(Json(LitellmKeyInfoResponse {
        key: token,
        info: litellm_key_info_value(&key),
    }))
}

async fn litellm_key_list(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Query(query): Query<LitellmKeyListQuery>,
) -> Result<Json<LitellmKeyListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;

    let return_full_object = query.return_full_object.unwrap_or(false);
    let page = query.page.unwrap_or(1).max(1);
    let size = query.size.unwrap_or(10).clamp(1, 100);
    let offset = (page - 1).saturating_mul(size);

    let tenant_filter = query
        .organization_id
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .or_else(|| {
            query.team_id
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(|v| v.to_string())
        });

    let user_filter = query
        .user_id
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string());

    let alias_filter = query
        .key_alias
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string());

    let gateway = state.gateway.lock().await;
    let mut keys = gateway.list_virtual_keys();
    drop(gateway);

    if let Some(admin_tenant) = admin.tenant_id.as_deref() {
        keys.retain(|key| key.tenant_id.as_deref() == Some(admin_tenant));
    }

    if let Some(tenant_id) = tenant_filter.as_deref() {
        keys.retain(|key| key.tenant_id.as_deref() == Some(tenant_id));
    }
    if let Some(user_id) = user_filter.as_deref() {
        keys.retain(|key| key.user_id.as_deref() == Some(user_id));
    }
    if let Some(alias) = alias_filter.as_deref() {
        keys.retain(|key| key.id == alias);
    }

    keys.sort_by(|a, b| a.id.cmp(&b.id));

    let total_count = keys.len();
    let total_pages = total_count.div_ceil(size);

    if offset >= keys.len() {
        keys.clear();
    } else {
        keys.drain(0..offset);
    }
    if keys.len() > size {
        keys.truncate(size);
    }

    let mut out = Vec::<serde_json::Value>::with_capacity(keys.len());
    for key in keys {
        if return_full_object {
            out.push(litellm_key_full_value(&key));
        } else {
            out.push(serde_json::Value::String(key.token));
        }
    }

    Ok(Json(LitellmKeyListResponse {
        keys: out,
        total_count,
        current_page: page,
        total_pages,
    }))
}

include!("litellm_keys/regenerate.rs");

fn generate_key_id() -> String {
    let ts_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let seq = LITELLM_KEY_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("sk_{ts_ms}_{seq}")
}

fn generate_key_token() -> String {
    let mut bytes = [0u8; 32];
    if getrandom::fill(&mut bytes).is_err() {
        let ts_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0);
        let seq = LITELLM_KEY_SEQ.fetch_add(1, Ordering::Relaxed);
        return format!("sk_fallback_{ts_ms}_{seq}");
    }
    format!("sk-{}", hex_encode(&bytes))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}
