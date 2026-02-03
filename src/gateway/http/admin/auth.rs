#[derive(Clone, Copy, Debug)]
enum AdminPermission {
    Read,
    Write,
}

#[derive(Clone, Debug)]
struct AdminContext {
    tenant_id: Option<String>,
}

fn ensure_admin_read(
    state: &GatewayHttpState,
    headers: &HeaderMap,
) -> Result<AdminContext, (StatusCode, Json<ErrorResponse>)> {
    ensure_admin(state, headers, AdminPermission::Read)
}

fn ensure_admin_write(
    state: &GatewayHttpState,
    headers: &HeaderMap,
) -> Result<AdminContext, (StatusCode, Json<ErrorResponse>)> {
    ensure_admin(state, headers, AdminPermission::Write)
}

fn ensure_admin(
    state: &GatewayHttpState,
    headers: &HeaderMap,
    permission: AdminPermission,
) -> Result<AdminContext, (StatusCode, Json<ErrorResponse>)> {
    let write_token = state.admin_token.as_deref();
    let read_token = state.admin_read_token.as_deref();
    let has_tenant_tokens = !state.admin_tenant_tokens.is_empty();

    if write_token.is_none() && read_token.is_none() && !has_tenant_tokens {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            "not_configured",
            "admin auth not configured",
        ));
    }

    let provided = extract_bearer(headers)
        .or_else(|| extract_header(headers, "x-admin-token"))
        .unwrap_or_default();

    if write_token.is_some_and(|expected| provided == expected) {
        return Ok(AdminContext { tenant_id: None });
    }

    if let AdminPermission::Read = permission {
        if read_token.is_some_and(|expected| provided == expected) {
            return Ok(AdminContext { tenant_id: None });
        }
    }

    if has_tenant_tokens {
        for binding in &state.admin_tenant_tokens {
            if provided != binding.token {
                continue;
            }
            if let AdminPermission::Write = permission {
                if binding.read_only {
                    break;
                }
            }
            return Ok(AdminContext {
                tenant_id: Some(binding.tenant_id.clone()),
            });
        }
    }

    Err(error_response(
        StatusCode::UNAUTHORIZED,
        "unauthorized",
        "invalid admin token",
    ))
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
async fn append_admin_audit_log(state: &GatewayHttpState, kind: &str, payload: serde_json::Value) {
    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.sqlite_store.as_ref() {
        let _ = store.append_audit_log(kind, payload.clone()).await;
        return;
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.redis_store.as_ref() {
        let _ = store.append_audit_log(kind, payload).await;
    }
}

fn extract_header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn extract_bearer(headers: &HeaderMap) -> Option<String> {
    let auth = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())?
        .trim()
        .to_string();
    let rest = auth
        .strip_prefix("Bearer ")
        .or_else(|| auth.strip_prefix("bearer "))?;
    let token = rest.trim();
    (!token.is_empty()).then(|| token.to_string())
}

fn extract_litellm_api_key(headers: &HeaderMap) -> Option<String> {
    let raw = extract_header(headers, "x-litellm-api-key")?;
    let token = raw
        .strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
        .unwrap_or(raw.as_str())
        .trim();
    (!token.is_empty()).then(|| token.to_string())
}

fn extract_virtual_key(headers: &HeaderMap) -> Option<String> {
    extract_litellm_api_key(headers)
        .or_else(|| extract_bearer(headers))
        .or_else(|| extract_header(headers, "x-ditto-virtual-key"))
        .or_else(|| extract_header(headers, "x-api-key"))
}

fn map_gateway_error(err: GatewayError) -> (StatusCode, Json<ErrorResponse>) {
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
        GatewayError::InvalidRequest { reason } => {
            error_response(StatusCode::BAD_REQUEST, "invalid_request", reason)
        }
    }
}

fn error_response(
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

fn persist_state_file(
    path: &StdPath,
    keys: &[VirtualKeyConfig],
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    GatewayStateFile {
        virtual_keys: keys.to_vec(),
    }
    .save(path)
    .map_err(|err| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            err.to_string(),
        )
    })
}

async fn persist_virtual_keys(
    state: &GatewayHttpState,
    keys: &[VirtualKeyConfig],
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.sqlite_store.as_ref() {
        store.replace_virtual_keys(keys).await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(());
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.redis_store.as_ref() {
        store.replace_virtual_keys(keys).await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        return Ok(());
    }

    if let Some(path) = state.state_file.as_ref() {
        persist_state_file(path.as_path(), keys)?;
    }

    Ok(())
}

#[cfg(test)]
mod admin_auth_tests {
    use super::*;

    fn test_state() -> GatewayHttpState {
        let config = crate::gateway::GatewayConfig {
            backends: Vec::new(),
            virtual_keys: Vec::new(),
            router: crate::gateway::RouterConfig {
                default_backend: "default".to_string(),
                default_backends: Vec::new(),
                rules: Vec::new(),
            },
            a2a_agents: Vec::new(),
            mcp_servers: Vec::new(),
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
