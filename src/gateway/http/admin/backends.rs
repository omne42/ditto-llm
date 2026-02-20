#[cfg(feature = "gateway-routing-advanced")]
async fn list_backends(
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
async fn reset_backend(
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

    let Some(health) = state.proxy_backend_health.as_ref() else {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "not_configured",
            "proxy routing not enabled",
        ));
    };

    let mut health = health.lock().await;
    health.remove(name.as_str());
    drop(health);

    #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
    append_admin_audit_log(
        &state,
        "admin.backend.reset",
        serde_json::json!({
            "backend": &name,
        }),
    )
    .await;

    Ok(Json(BackendHealth::default().snapshot(&name)))
}
