#[derive(Clone, Copy, Debug)]
enum AdminPermission {
    Read,
    Write,
}

fn ensure_admin_read(
    state: &GatewayHttpState,
    headers: &HeaderMap,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    ensure_admin(state, headers, AdminPermission::Read)
}

fn ensure_admin_write(
    state: &GatewayHttpState,
    headers: &HeaderMap,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    ensure_admin(state, headers, AdminPermission::Write)
}

fn ensure_admin(
    state: &GatewayHttpState,
    headers: &HeaderMap,
    permission: AdminPermission,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let write_token = state.admin_token.as_deref();
    let read_token = state.admin_read_token.as_deref();

    if write_token.is_none() && read_token.is_none() {
        return Ok(());
    }

    let provided = extract_bearer(headers)
        .or_else(|| extract_header(headers, "x-admin-token"))
        .unwrap_or_default();

    let ok = match permission {
        AdminPermission::Read => {
            write_token.is_some_and(|expected| provided == expected)
                || read_token.is_some_and(|expected| provided == expected)
        }
        AdminPermission::Write => write_token.is_some_and(|expected| provided == expected),
    };

    ok.then_some(())
        .ok_or_else(|| error_response(StatusCode::UNAUTHORIZED, "unauthorized", "invalid admin token"))
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
