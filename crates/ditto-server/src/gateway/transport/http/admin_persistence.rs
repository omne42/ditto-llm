use super::*;

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
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

async fn persist_control_plane_state(
    state: &GatewayHttpState,
    keys: &[VirtualKeyConfig],
    router: &RouterConfig,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let stored_keys = keys
        .iter()
        .map(VirtualKeyConfig::sanitized_for_persistence)
        .collect::<Vec<_>>();

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.stores.sqlite.as_ref() {
        store
            .replace_virtual_keys(&stored_keys)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;
        store.replace_router_config(router).await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
    }

    #[cfg(feature = "gateway-store-postgres")]
    if let Some(store) = state.stores.postgres.as_ref() {
        store
            .replace_virtual_keys(&stored_keys)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;
        store.replace_router_config(router).await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
    }

    #[cfg(feature = "gateway-store-mysql")]
    if let Some(store) = state.stores.mysql.as_ref() {
        store
            .replace_virtual_keys(&stored_keys)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;
        store.replace_router_config(router).await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.stores.redis.as_ref() {
        store
            .replace_virtual_keys(&stored_keys)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;
        store.replace_router_config(router).await.map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                err.to_string(),
            )
        })?;
    }

    if let Some(path) = state.admin.state_file.as_ref() {
        persist_state_file(path.as_path(), &stored_keys, router)?;
    }

    Ok(())
}

fn restore_gateway_runtime(state: &GatewayHttpState, snapshot: &GatewayConfig) {
    state.gateway.mutate_control_plane(|gateway| {
        gateway.replace_virtual_keys(snapshot.virtual_keys.clone());
        gateway.replace_router_config(snapshot.router.clone());
    });
    state.sync_control_plane_from_gateway();
}

fn storage_error_message(err: &(StatusCode, Json<ErrorResponse>)) -> String {
    err.1.error.message.clone()
}

pub(super) async fn apply_control_plane_change<T>(
    state: &GatewayHttpState,
    reason: &str,
    mutate: impl FnOnce(
        &mut crate::gateway::GatewayMutation<'_>,
    ) -> Result<T, (StatusCode, Json<ErrorResponse>)>,
) -> Result<(T, ConfigVersionInfo), (StatusCode, Json<ErrorResponse>)> {
    let previous = state.gateway.config_snapshot();
    let result = state.gateway.mutate_control_plane(mutate);

    let value = match result {
        Ok(value) => value,
        Err(err) => {
            restore_gateway_runtime(state, &previous);
            return Err(err);
        }
    };

    state.sync_control_plane_from_gateway();
    let current = state.gateway.config_snapshot();
    if let Err(err) = current.validate() {
        restore_gateway_runtime(state, &previous);
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            err.to_string(),
        ));
    }

    match persist_control_plane_state(state, &current.virtual_keys, &current.router).await {
        Ok(()) => {
            let version = state.config_versions.lock().await.push_snapshot(
                current.virtual_keys,
                current.router,
                reason,
            );
            Ok((value, version))
        }
        Err(err) => {
            restore_gateway_runtime(state, &previous);
            let rollback =
                persist_control_plane_state(state, &previous.virtual_keys, &previous.router).await;
            if let Err(rollback_err) = rollback {
                return Err(error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    format!(
                        "control-plane persistence failed and rollback also failed: persist={}, rollback={}",
                        storage_error_message(&err),
                        storage_error_message(&rollback_err),
                    ),
                ));
            }
            Err(err)
        }
    }
}
