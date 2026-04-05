use super::*;

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
#[derive(Clone, Copy)]
enum GatewayStoreTarget {
    #[cfg(feature = "gateway-store-sqlite")]
    Sqlite,
    #[cfg(feature = "gateway-store-postgres")]
    Postgres,
    #[cfg(feature = "gateway-store-mysql")]
    Mysql,
    #[cfg(feature = "gateway-store-redis")]
    Redis,
}

#[derive(Clone, Copy)]
enum ControlPlanePersistenceTarget {
    StateFile,
    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    Store(GatewayStoreTarget),
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
fn configured_control_plane_persistence_target_names(
    state: &GatewayHttpState,
) -> Vec<&'static str> {
    let mut configured = Vec::new();

    if state.admin.state_file.is_some() {
        configured.push("state_file");
    }
    #[cfg(feature = "gateway-store-sqlite")]
    if state.stores.sqlite.is_some() {
        configured.push("sqlite");
    }
    #[cfg(feature = "gateway-store-postgres")]
    if state.stores.postgres.is_some() {
        configured.push("postgres");
    }
    #[cfg(feature = "gateway-store-mysql")]
    if state.stores.mysql.is_some() {
        configured.push("mysql");
    }
    #[cfg(feature = "gateway-store-redis")]
    if state.stores.redis.is_some() {
        configured.push("redis");
    }

    configured
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
fn control_plane_persistence_topology_error(
    state: &GatewayHttpState,
) -> (StatusCode, Json<ErrorResponse>) {
    error_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        "storage_error",
        format!(
            "multiple control-plane persistence targets configured: {}; choose exactly one target",
            configured_control_plane_persistence_target_names(state).join(", ")
        ),
    )
}

fn selected_control_plane_persistence_target(
    state: &GatewayHttpState,
) -> Result<Option<ControlPlanePersistenceTarget>, (StatusCode, Json<ErrorResponse>)> {
    let mut selected = None;

    if state.admin.state_file.is_some() {
        selected = Some(ControlPlanePersistenceTarget::StateFile);
    }
    #[cfg(feature = "gateway-store-sqlite")]
    if state.stores.sqlite.is_some() {
        if selected.is_some() {
            return Err(control_plane_persistence_topology_error(state));
        }
        selected = Some(ControlPlanePersistenceTarget::Store(
            GatewayStoreTarget::Sqlite,
        ));
    }
    #[cfg(feature = "gateway-store-postgres")]
    if state.stores.postgres.is_some() {
        if selected.is_some() {
            return Err(control_plane_persistence_topology_error(state));
        }
        selected = Some(ControlPlanePersistenceTarget::Store(
            GatewayStoreTarget::Postgres,
        ));
    }
    #[cfg(feature = "gateway-store-mysql")]
    if state.stores.mysql.is_some() {
        if selected.is_some() {
            return Err(control_plane_persistence_topology_error(state));
        }
        selected = Some(ControlPlanePersistenceTarget::Store(
            GatewayStoreTarget::Mysql,
        ));
    }
    #[cfg(feature = "gateway-store-redis")]
    if state.stores.redis.is_some() {
        if selected.is_some() {
            return Err(control_plane_persistence_topology_error(state));
        }
        selected = Some(ControlPlanePersistenceTarget::Store(
            GatewayStoreTarget::Redis,
        ));
    }

    Ok(selected)
}

#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
fn report_admin_audit_append_failure(store: &str, kind: &str, err: &impl std::fmt::Display) {
    eprintln!("failed to append {store} admin audit log `{kind}`: {err}");
}

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
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let selected_target = selected_control_plane_persistence_target(state)?;
    let Some(payload) = state.prepare_observability_event(
        crate::gateway::observability::GatewayObservabilitySink::Audit,
        payload,
    ) else {
        return Ok(());
    };

    match selected_target {
        Some(ControlPlanePersistenceTarget::StateFile) | None => {}
        #[cfg(feature = "gateway-store-sqlite")]
        Some(ControlPlanePersistenceTarget::Store(GatewayStoreTarget::Sqlite)) => {
            let store = state
                .stores
                .sqlite
                .as_ref()
                .expect("selected sqlite target must exist");
            if let Err(err) = store.append_audit_log(kind, payload).await {
                report_admin_audit_append_failure("sqlite", kind, &err);
                return Err(error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    format!("failed to append sqlite admin audit log `{kind}`: {err}"),
                ));
            }
        }
        #[cfg(feature = "gateway-store-postgres")]
        Some(ControlPlanePersistenceTarget::Store(GatewayStoreTarget::Postgres)) => {
            let store = state
                .stores
                .postgres
                .as_ref()
                .expect("selected postgres target must exist");
            if let Err(err) = store.append_audit_log(kind, payload).await {
                report_admin_audit_append_failure("postgres", kind, &err);
                return Err(error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    format!("failed to append postgres admin audit log `{kind}`: {err}"),
                ));
            }
        }
        #[cfg(feature = "gateway-store-mysql")]
        Some(ControlPlanePersistenceTarget::Store(GatewayStoreTarget::Mysql)) => {
            let store = state
                .stores
                .mysql
                .as_ref()
                .expect("selected mysql target must exist");
            if let Err(err) = store.append_audit_log(kind, payload).await {
                report_admin_audit_append_failure("mysql", kind, &err);
                return Err(error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    format!("failed to append mysql admin audit log `{kind}`: {err}"),
                ));
            }
        }
        #[cfg(feature = "gateway-store-redis")]
        Some(ControlPlanePersistenceTarget::Store(GatewayStoreTarget::Redis)) => {
            let store = state
                .stores
                .redis
                .as_ref()
                .expect("selected redis target must exist");
            if let Err(err) = store.append_audit_log(kind, payload).await {
                report_admin_audit_append_failure("redis", kind, &err);
                return Err(error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    format!("failed to append redis admin audit log `{kind}`: {err}"),
                ));
            }
        }
    }

    Ok(())
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
    match selected_control_plane_persistence_target(state)? {
        None => {}
        Some(ControlPlanePersistenceTarget::StateFile) => {
            let path = state
                .admin
                .state_file
                .as_ref()
                .expect("selected state file target must exist");
            persist_state_file(path.as_path(), &stored_keys, router)?;
        }
        #[cfg(feature = "gateway-store-sqlite")]
        Some(ControlPlanePersistenceTarget::Store(GatewayStoreTarget::Sqlite)) => {
            let store = state
                .stores
                .sqlite
                .as_ref()
                .expect("selected sqlite target must exist");
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
        Some(ControlPlanePersistenceTarget::Store(GatewayStoreTarget::Postgres)) => {
            let store = state
                .stores
                .postgres
                .as_ref()
                .expect("selected postgres target must exist");
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
        Some(ControlPlanePersistenceTarget::Store(GatewayStoreTarget::Mysql)) => {
            let store = state
                .stores
                .mysql
                .as_ref()
                .expect("selected mysql target must exist");
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
        Some(ControlPlanePersistenceTarget::Store(GatewayStoreTarget::Redis)) => {
            let store = state
                .stores
                .redis
                .as_ref()
                .expect("selected redis target must exist");
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
    }

    Ok(())
}

fn restore_gateway_runtime(
    state: &GatewayHttpState,
    snapshot: &crate::gateway::GatewayRuntimeSnapshot,
) {
    state.gateway.restore_runtime_snapshot(snapshot);
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
    let previous = state.gateway.runtime_snapshot();
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
    let backend_names = state
        .backend_names_snapshot()
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    if let Err(err) = current.validate_with_backend_names(&backend_names) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    #[cfg(feature = "gateway-store-sqlite")]
    use crate::gateway::SqliteStore;
    use crate::gateway::{
        Backend, BudgetConfig, Gateway, GatewayConfig, GatewayError, GatewayRequest,
        GatewayResponse, RouteBackend, RouterConfig, VirtualKeyConfig,
    };

    struct EchoBackend;

    #[async_trait]
    impl Backend for EchoBackend {
        async fn call(&self, request: &GatewayRequest) -> Result<GatewayResponse, GatewayError> {
            Ok(GatewayResponse {
                content: format!("echo: {}", request.prompt),
                output_tokens: 1,
                backend: String::new(),
                cached: false,
            })
        }
    }

    #[tokio::test]
    async fn persistence_failure_restores_budget_runtime_state() {
        let dir = tempfile::tempdir().expect("tempdir");
        let invalid_state_path = dir.path().to_path_buf();

        let mut key = VirtualKeyConfig::new("key-1", "vk-1");
        key.budget = BudgetConfig {
            total_tokens: Some(5),
            total_usd_micros: None,
        };

        let config = GatewayConfig {
            backends: Vec::new(),
            virtual_keys: vec![key],
            router: RouterConfig {
                default_backends: vec![RouteBackend {
                    backend: "primary".to_string(),
                    weight: 1.0,
                }],
                rules: Vec::new(),
            },
            a2a_agents: Vec::new(),
            mcp_servers: Vec::new(),
            observability: Default::default(),
        };

        let mut gateway = Gateway::new(config);
        gateway.register_backend("primary", EchoBackend);
        let state = GatewayHttpState::new(gateway).with_state_file(invalid_state_path);

        state
            .gateway
            .handle(GatewayRequest {
                virtual_key: "vk-1".to_string(),
                model: "gpt-test".to_string(),
                prompt: "hello".to_string(),
                input_tokens: 4,
                max_output_tokens: 1,
                passthrough: false,
            })
            .await
            .expect("initial request should spend budget");

        let err = apply_control_plane_change(&state, "test.persistence_failure", |gateway| {
            gateway.remove_virtual_key("key-1");
            Ok(())
        })
        .await
        .expect_err("directory-backed state path should fail persistence");
        assert_eq!(err.0, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(state.list_virtual_keys_snapshot().len(), 1);

        let second = state
            .gateway
            .handle(GatewayRequest {
                virtual_key: "vk-1".to_string(),
                model: "gpt-test".to_string(),
                prompt: "again".to_string(),
                input_tokens: 2,
                max_output_tokens: 1,
                passthrough: false,
            })
            .await
            .expect_err("restored budget should still block overspend");

        assert!(matches!(second, GatewayError::BudgetExceeded { .. }));
    }

    #[cfg(feature = "gateway-store-sqlite")]
    #[tokio::test]
    async fn rejects_multiple_control_plane_persistence_targets() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state_path = dir.path().join("gateway-state.json");
        let sqlite_path = dir.path().join("gateway.sqlite");
        let store = SqliteStore::new(&sqlite_path);
        store.init().await.expect("init");
        store.verify_schema().await.expect("verify schema");

        let config = GatewayConfig {
            backends: Vec::new(),
            virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
            router: RouterConfig {
                default_backends: vec![RouteBackend {
                    backend: "primary".to_string(),
                    weight: 1.0,
                }],
                rules: Vec::new(),
            },
            a2a_agents: Vec::new(),
            mcp_servers: Vec::new(),
            observability: Default::default(),
        };

        let mut gateway = Gateway::new(config);
        gateway.register_backend("primary", EchoBackend);
        let state = GatewayHttpState::new(gateway)
            .with_state_file(state_path.clone())
            .with_sqlite_store(store.clone());

        let err = apply_control_plane_change(&state, "test.multi_target", |gateway| {
            gateway.remove_virtual_key("key-1");
            Ok(())
        })
        .await
        .expect_err("multiple persistence targets should be rejected");

        assert_eq!(err.0, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(storage_error_message(&err).contains("multiple control-plane persistence targets"));
        assert_eq!(state.list_virtual_keys_snapshot().len(), 1);
        assert!(!state_path.exists(), "state file should not be written");
        assert!(
            store
                .load_virtual_keys()
                .await
                .expect("load keys")
                .is_empty(),
            "sqlite store should not be mutated"
        );
    }
}
