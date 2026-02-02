use super::*;

#[tokio::test]
async fn sqlite_store_round_trips_virtual_keys() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("gateway.sqlite");
    let store = SqliteStore::new(&path);
    store.init().await.expect("init");

    let key = VirtualKeyConfig::new("key-1", "vk-1");
    store
        .replace_virtual_keys(std::slice::from_ref(&key))
        .await
        .expect("persist");

    let loaded = store.load_virtual_keys().await.expect("load");
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].id, "key-1");
    assert_eq!(loaded[0].token, "vk-1");

    store
        .replace_virtual_keys(&[])
        .await
        .expect("persist empty");
    let loaded = store.load_virtual_keys().await.expect("load");
    assert!(loaded.is_empty());
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
    assert_eq!(ledgers[1].spent_tokens, 2);
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
    assert_eq!(ledgers[1].spent_usd_micros, 2);
    assert_eq!(ledgers[1].reserved_usd_micros, 0);
}
