use super::*;
use crate::gateway::RouteBackend;

#[tokio::test]
async fn sqlite_store_round_trips_virtual_keys() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("gateway.sqlite");
    let store = SqliteStore::new(&path);
    store.init().await.expect("init");
    store.verify_schema().await.expect("verify schema");

    let key = VirtualKeyConfig::new("key-1", "vk-1");
    store
        .replace_virtual_keys(std::slice::from_ref(&key))
        .await
        .expect("persist");

    let loaded = store.load_virtual_keys().await.expect("load");
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].id, "key-1");
    assert_eq!(loaded[0].token, key.sanitized_for_persistence().token);

    store
        .replace_virtual_keys(&[])
        .await
        .expect("persist empty");
    let loaded = store.load_virtual_keys().await.expect("load");
    assert!(loaded.is_empty());
}

#[tokio::test]
async fn sqlite_store_round_trips_router_config() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("gateway.sqlite");
    let store = SqliteStore::new(&path);
    store.init().await.expect("init");

    let router = RouterConfig {
        default_backends: Vec::new(),
        rules: Vec::new(),
    };

    store
        .replace_router_config(&router)
        .await
        .expect("persist router");

    let loaded = store.load_router_config().await.expect("load router");
    assert!(loaded.is_some());
    let loaded = loaded.expect("router");
    assert_eq!(loaded.default_backends.len(), 0);
    assert_eq!(loaded.rules.len(), 0);
}

#[tokio::test]
async fn sqlite_store_round_trips_control_plane_snapshot() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("gateway.sqlite");
    let store = SqliteStore::new(&path);
    store.init().await.expect("init");

    let key = VirtualKeyConfig::new("key-1", "vk-1");
    let router = RouterConfig {
        default_backends: vec![RouteBackend {
            backend: "primary".to_string(),
            weight: 1.0,
        }],
        rules: Vec::new(),
    };

    store
        .replace_control_plane_snapshot(std::slice::from_ref(&key), &router)
        .await
        .expect("persist snapshot");

    let loaded_keys = store.load_virtual_keys().await.expect("load keys");
    assert_eq!(loaded_keys.len(), 1);
    assert_eq!(loaded_keys[0].id, "key-1");

    let loaded_router = store
        .load_router_config()
        .await
        .expect("load router")
        .expect("router");
    assert_eq!(loaded_router.default_backends.len(), 1);
    assert_eq!(loaded_router.default_backends[0].backend, "primary");
}

#[tokio::test]
async fn sqlite_store_control_plane_snapshot_rolls_back_if_router_write_fails() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("gateway.sqlite");
    let store = SqliteStore::new(&path);
    store.init().await.expect("init");

    let original_key = VirtualKeyConfig::new("key-1", "vk-1");
    let original_router = RouterConfig {
        default_backends: vec![RouteBackend {
            backend: "primary".to_string(),
            weight: 1.0,
        }],
        rules: Vec::new(),
    };
    store
        .replace_control_plane_snapshot(std::slice::from_ref(&original_key), &original_router)
        .await
        .expect("seed snapshot");

    let trigger_path = path.clone();
    tokio::task::spawn_blocking(move || -> Result<(), SqliteStoreError> {
        let conn = open_connection(trigger_path)?;
        init_schema(&conn)?;
        conn.execute_batch(
            "CREATE TRIGGER fail_router_snapshot_insert
             BEFORE INSERT ON config_state
             WHEN NEW.key = 'router'
             BEGIN
                 SELECT RAISE(FAIL, 'router write failed');
             END;
             CREATE TRIGGER fail_router_snapshot_update
             BEFORE UPDATE ON config_state
             WHEN NEW.key = 'router'
             BEGIN
                 SELECT RAISE(FAIL, 'router write failed');
             END;",
        )?;
        Ok(())
    })
    .await
    .expect("join")
    .expect("install trigger");

    let new_key = VirtualKeyConfig::new("key-2", "vk-2");
    let new_router = RouterConfig {
        default_backends: vec![RouteBackend {
            backend: "secondary".to_string(),
            weight: 1.0,
        }],
        rules: Vec::new(),
    };

    let err = store
        .replace_control_plane_snapshot(std::slice::from_ref(&new_key), &new_router)
        .await
        .expect_err("router trigger should abort snapshot write");
    assert!(err.to_string().contains("router write failed"));

    let loaded_keys = store.load_virtual_keys().await.expect("load keys");
    assert_eq!(loaded_keys.len(), 1);
    assert_eq!(loaded_keys[0].id, "key-1");

    let loaded_router = store
        .load_router_config()
        .await
        .expect("load router")
        .expect("router");
    assert_eq!(loaded_router.default_backends.len(), 1);
    assert_eq!(loaded_router.default_backends[0].backend, "primary");
}

#[tokio::test]
async fn sqlite_store_budget_reservations_enforce_limit() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("gateway.sqlite");
    let store = SqliteStore::new(&path);
    store.init().await.expect("init");

    store
        .reserve_budget_tokens("r1", "key-1", 5, 3)
        .await
        .expect("reserve r1");
    let err = store.reserve_budget_tokens("r2", "key-1", 5, 3).await;
    assert!(matches!(err, Err(SqliteStoreError::BudgetExceeded { .. })));

    store
        .rollback_budget_reservation("r1")
        .await
        .expect("rollback r1");

    store
        .reserve_budget_tokens("r3", "key-1", 5, 3)
        .await
        .expect("reserve r3");
    store
        .commit_budget_reservation("r3")
        .await
        .expect("commit r3");

    let err = store.reserve_budget_tokens("r4", "key-1", 5, 3).await;
    assert!(matches!(err, Err(SqliteStoreError::BudgetExceeded { .. })));
}

#[tokio::test]
async fn sqlite_store_commit_budget_reservation_with_tokens_releases_difference() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("gateway.sqlite");
    let store = SqliteStore::new(&path);
    store.init().await.expect("init");

    store
        .reserve_budget_tokens("r1", "key-1", 10, 7)
        .await
        .expect("reserve r1");
    store
        .commit_budget_reservation_with_tokens("r1", 3)
        .await
        .expect("commit r1");

    let ledgers = store.list_budget_ledgers().await.expect("ledgers");
    assert_eq!(ledgers.len(), 1);
    assert_eq!(ledgers[0].key_id, "key-1");
    assert_eq!(ledgers[0].spent_tokens, 3);
    assert_eq!(ledgers[0].reserved_tokens, 0);

    store
        .reserve_budget_tokens("r2", "key-1", 10, 7)
        .await
        .expect("reserve r2");
    let err = store.reserve_budget_tokens("r3", "key-1", 10, 1).await;
    assert!(matches!(err, Err(SqliteStoreError::BudgetExceeded { .. })));

    store
        .reserve_budget_tokens("r4", "key-2", 10, 2)
        .await
        .expect("reserve r4");
    store
        .commit_budget_reservation_with_tokens("r4", 5)
        .await
        .expect("commit r4");

    let ledgers = store.list_budget_ledgers().await.expect("ledgers 2");
    assert_eq!(ledgers.len(), 2);
    assert_eq!(ledgers[0].key_id, "key-1");
    assert_eq!(ledgers[1].key_id, "key-2");
    assert_eq!(ledgers[1].spent_tokens, 5);
    assert_eq!(ledgers[1].reserved_tokens, 0);
}

#[tokio::test]
async fn sqlite_store_appends_audit_logs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("gateway.sqlite");
    let store = SqliteStore::new(&path);
    store.init().await.expect("init");

    store
        .append_audit_log("test", serde_json::json!({"ok": true}))
        .await
        .expect("append");

    let logs = store.list_audit_logs(10, None).await.expect("list");
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].kind, "test");
    assert_eq!(logs[0].payload["ok"], true);
}

#[tokio::test]
async fn sqlite_store_records_cost_ledgers() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("gateway.sqlite");
    let store = SqliteStore::new(&path);
    store.init().await.expect("init");

    store
        .reserve_cost_usd_micros("req-1", "key-1", 10, 5)
        .await
        .expect("reserve");
    store
        .commit_cost_reservation("req-1")
        .await
        .expect("commit");

    let ledgers = store.list_cost_ledgers().await.expect("ledgers");
    assert_eq!(ledgers.len(), 1);
    assert_eq!(ledgers[0].key_id, "key-1");
    assert_eq!(ledgers[0].spent_usd_micros, 5);
    assert_eq!(ledgers[0].reserved_usd_micros, 0);
}

#[tokio::test]
async fn sqlite_store_commit_cost_reservation_with_usd_micros_releases_difference() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("gateway.sqlite");
    let store = SqliteStore::new(&path);
    store.init().await.expect("init");

    store
        .reserve_cost_usd_micros("req-1", "key-1", 10, 7)
        .await
        .expect("reserve req-1");
    store
        .commit_cost_reservation_with_usd_micros("req-1", 3)
        .await
        .expect("commit req-1");

    let ledgers = store.list_cost_ledgers().await.expect("ledgers");
    assert_eq!(ledgers.len(), 1);
    assert_eq!(ledgers[0].key_id, "key-1");
    assert_eq!(ledgers[0].spent_usd_micros, 3);
    assert_eq!(ledgers[0].reserved_usd_micros, 0);

    store
        .reserve_cost_usd_micros("req-2", "key-1", 10, 7)
        .await
        .expect("reserve req-2");
    let err = store.reserve_cost_usd_micros("req-3", "key-1", 10, 1).await;
    assert!(matches!(
        err,
        Err(SqliteStoreError::CostBudgetExceeded { .. })
    ));

    store
        .reserve_cost_usd_micros("req-4", "key-2", 10, 2)
        .await
        .expect("reserve req-4");
    store
        .commit_cost_reservation_with_usd_micros("req-4", 5)
        .await
        .expect("commit req-4");

    let ledgers = store.list_cost_ledgers().await.expect("ledgers 2");
    assert_eq!(ledgers.len(), 2);
    assert_eq!(ledgers[0].key_id, "key-1");
    assert_eq!(ledgers[1].key_id, "key-2");
    assert_eq!(ledgers[1].spent_usd_micros, 5);
    assert_eq!(ledgers[1].reserved_usd_micros, 0);
}

#[tokio::test]
async fn sqlite_store_reaps_stale_budget_reservations() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("gateway.sqlite");
    let store = SqliteStore::new(&path);
    store.init().await.expect("init");

    store
        .reserve_budget_tokens("req-1", "key-1", 20, 4)
        .await
        .expect("reserve");

    let (scanned, reaped, released) = store
        .reap_stale_budget_reservations(u64::MAX, 100, true)
        .await
        .expect("dry run reap");
    assert_eq!((scanned, reaped, released), (1, 1, 4));
    let ledgers = store.list_budget_ledgers().await.expect("ledgers dry");
    assert_eq!(ledgers.len(), 1);
    assert_eq!(ledgers[0].reserved_tokens, 4);

    let (scanned, reaped, released) = store
        .reap_stale_budget_reservations(u64::MAX, 100, false)
        .await
        .expect("reap");
    assert_eq!((scanned, reaped, released), (1, 1, 4));
    let ledgers = store.list_budget_ledgers().await.expect("ledgers");
    assert_eq!(ledgers.len(), 1);
    assert_eq!(ledgers[0].reserved_tokens, 0);
}

#[tokio::test]
async fn sqlite_store_reaps_stale_cost_reservations() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("gateway.sqlite");
    let store = SqliteStore::new(&path);
    store.init().await.expect("init");

    store
        .reserve_cost_usd_micros("req-1", "key-1", 20, 7)
        .await
        .expect("reserve");

    let (scanned, reaped, released) = store
        .reap_stale_cost_reservations(u64::MAX, 100, true)
        .await
        .expect("dry run reap");
    assert_eq!((scanned, reaped, released), (1, 1, 7));
    let ledgers = store.list_cost_ledgers().await.expect("ledgers dry");
    assert_eq!(ledgers.len(), 1);
    assert_eq!(ledgers[0].reserved_usd_micros, 7);

    let (scanned, reaped, released) = store
        .reap_stale_cost_reservations(u64::MAX, 100, false)
        .await
        .expect("reap");
    assert_eq!((scanned, reaped, released), (1, 1, 7));
    let ledgers = store.list_cost_ledgers().await.expect("ledgers");
    assert_eq!(ledgers.len(), 1);
    assert_eq!(ledgers[0].reserved_usd_micros, 0);
}
