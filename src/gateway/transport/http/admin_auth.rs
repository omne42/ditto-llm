use super::*;

#[derive(Clone, Copy, Debug)]
enum AdminPermission {
    Read,
    Write,
}

#[derive(Clone, Debug)]
pub(super) struct AdminContext {
    pub(super) tenant_id: Option<String>,
}

pub(super) fn ensure_admin_read(
    state: &GatewayHttpState,
    headers: &HeaderMap,
) -> Result<AdminContext, (StatusCode, Json<ErrorResponse>)> {
    ensure_admin(state, headers, AdminPermission::Read)
}

pub(super) fn ensure_admin_write(
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
    let write_token = state.admin.admin_token.as_deref();
    let read_token = state.admin.admin_read_token.as_deref();
    let has_tenant_tokens = !state.admin.admin_tenant_tokens.is_empty();

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
        for binding in &state.admin.admin_tenant_tokens {
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
