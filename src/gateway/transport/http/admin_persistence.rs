use super::*;

pub(super) async fn append_admin_audit_log(
    state: &GatewayHttpState,
    kind: &str,
    payload: serde_json::Value,
) {
    let Some(payload) = state.prepare_observability_event(
        crate::gateway::observability::GatewayObservabilitySink::Audit,
        payload,
    ) else {
        return;
    };

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.stores.sqlite.as_ref() {
        let _ = store.append_audit_log(kind, payload.clone()).await;
    }

    #[cfg(feature = "gateway-store-postgres")]
    if let Some(store) = state.stores.postgres.as_ref() {
        let _ = store.append_audit_log(kind, payload.clone()).await;
    }

    #[cfg(feature = "gateway-store-mysql")]
    if let Some(store) = state.stores.mysql.as_ref() {
        let _ = store.append_audit_log(kind, payload.clone()).await;
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.stores.redis.as_ref() {
        let _ = store.append_audit_log(kind, payload).await;
    }
}

fn persist_state_file(
    path: &StdPath,
    keys: &[VirtualKeyConfig],
    router: &RouterConfig,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    GatewayStateFile {
        virtual_keys: keys.to_vec(),
        router: Some(router.clone()),
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

pub(super) async fn persist_virtual_keys(
    state: &GatewayHttpState,
    keys: &[VirtualKeyConfig],
    reason: &str,
) -> Result<ConfigVersionInfo, (StatusCode, Json<ErrorResponse>)> {
    let router = state.router_config_snapshot();

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.stores.sqlite.as_ref() {
        store.replace_virtual_keys(keys).await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        store.replace_router_config(&router).await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
    }

    #[cfg(feature = "gateway-store-postgres")]
    if let Some(store) = state.stores.postgres.as_ref() {
        store.replace_virtual_keys(keys).await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        store.replace_router_config(&router).await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
    }

    #[cfg(feature = "gateway-store-mysql")]
    if let Some(store) = state.stores.mysql.as_ref() {
        store.replace_virtual_keys(keys).await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        store.replace_router_config(&router).await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.stores.redis.as_ref() {
        store.replace_virtual_keys(keys).await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
        store.replace_router_config(&router).await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
    }

    if let Some(path) = state.admin.state_file.as_ref() {
        persist_state_file(path.as_path(), keys, &router)?;
    }

    let version = state
        .config_versions
        .lock()
        .await
        .push_snapshot(keys.to_vec(), router, reason);
    Ok(version)
}
