#[derive(Debug, Deserialize)]
struct LitellmKeyRegenerateRequest {
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    new_key: Option<String>,
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

async fn litellm_key_regenerate(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    payload: Option<Json<LitellmKeyRegenerateRequest>>,
) -> Result<Json<LitellmKeyGenerateResponse>, (StatusCode, Json<ErrorResponse>)> {
    litellm_key_regenerate_inner(state, headers, None, payload.map(|Json(v)| v)).await
}

async fn litellm_key_regenerate_path(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Path(path_key): Path<String>,
    payload: Option<Json<LitellmKeyRegenerateRequest>>,
) -> Result<Json<LitellmKeyGenerateResponse>, (StatusCode, Json<ErrorResponse>)> {
    litellm_key_regenerate_inner(state, headers, Some(path_key), payload.map(|Json(v)| v)).await
}

async fn litellm_key_regenerate_inner(
    state: GatewayHttpState,
    headers: HeaderMap,
    path_key: Option<String>,
    payload: Option<LitellmKeyRegenerateRequest>,
) -> Result<Json<LitellmKeyGenerateResponse>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_write(&state, &headers)?;

    let payload = payload.unwrap_or(LitellmKeyRegenerateRequest {
        key: None,
        new_key: None,
        key_alias: None,
        user_id: None,
        team_id: None,
        organization_id: None,
        models: None,
        max_budget: None,
        rpm_limit: None,
        tpm_limit: None,
        blocked: None,
    });

    let token = path_key
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .or_else(|| {
            payload
                .key
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(|v| v.to_string())
        })
        .ok_or_else(|| {
            error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "key is required",
            )
        })?;

    let new_token = if let Some(new_key) = payload
        .new_key
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        if !new_key.starts_with("sk-") {
            return Err(error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "new_key must start with 'sk-'",
            ));
        }
        new_key.to_string()
    } else {
        generate_key_token()
    };

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

        let Some(existing) = keys.iter().find(|key| key.token == token).cloned() else {
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
                    "cannot regenerate keys for a different tenant",
                ));
            }
        }

        if keys
            .iter()
            .any(|candidate| candidate.token == new_token && candidate.id != existing.id)
        {
            return Err(error_response(
                StatusCode::CONFLICT,
                "conflict",
                "new_key already exists",
            ));
        }

        let mut key = existing.clone();
        key.token = new_token.clone();

        if let Some(user_id) = payload.user_id.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
            key.user_id = Some(user_id.to_string());
        }

        if let Some(tenant_id) = tenant_id.as_deref() {
            if let Some(admin_tenant) = admin.tenant_id.as_deref() {
                if tenant_id != admin_tenant {
                    return Err(error_response(
                        StatusCode::FORBIDDEN,
                        "forbidden",
                        "cannot regenerate keys for a different tenant",
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
    if let Some(logger) = state.devtools.as_ref() {
        let _ = logger.log_event(
            "litellm.key.regenerate",
            serde_json::json!({
                "key_id": &key.id,
                "tenant_id": key.tenant_id.as_deref(),
            }),
        );
    }

    #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
    append_admin_audit_log(
        &state,
        "litellm.key.regenerate",
        serde_json::json!({
            "key_id": &key.id,
            "tenant_id": key.tenant_id.as_deref(),
            "user_id": key.user_id.as_deref(),
        }),
    )
    .await;

    Ok(Json(litellm_generate_response_from_virtual_key(&key)))
}
