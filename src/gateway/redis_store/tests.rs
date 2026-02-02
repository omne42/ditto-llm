#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[cfg(feature = "gateway-proxy-cache")]
    #[test]
    fn proxy_cache_record_round_trips_headers_and_body() {
        let mut headers = axum::http::HeaderMap::new();
        headers.append("content-type", "application/json".parse().unwrap());
        headers.append("set-cookie", "a=b".parse().unwrap());

        let cached = CachedProxyResponse {
            status: 200,
            headers: headers.clone(),
            body: bytes::Bytes::from_static(b"ok"),
            backend: "primary".to_string(),
        };

        let record = CachedProxyResponseRecord::from_cached(&cached);
        let raw = serde_json::to_vec(&record).expect("serialize");
        let decoded: CachedProxyResponseRecord = serde_json::from_slice(&raw).expect("decode");
        let round_tripped = decoded.into_cached();

        assert_eq!(round_tripped.status, cached.status);
        assert_eq!(round_tripped.backend, cached.backend);
        assert_eq!(round_tripped.body, cached.body);

        assert_eq!(
            round_tripped.headers.get("content-type"),
            headers.get("content-type")
        );
        assert_eq!(
            round_tripped
                .headers
                .get_all("set-cookie")
                .iter()
                .collect::<Vec<_>>(),
            headers.get_all("set-cookie").iter().collect::<Vec<_>>(),
        );
    }

    fn env_nonempty(key: &str) -> Option<String> {
        std::env::var(key)
            .ok()
            .filter(|value| !value.trim().is_empty())
    }

    fn redis_url() -> Option<String> {
        env_nonempty("DITTO_REDIS_URL").or_else(|| env_nonempty("REDIS_URL"))
    }

    static PREFIX_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn test_prefix() -> String {
        let n = PREFIX_COUNTER.fetch_add(1, Ordering::Relaxed);
        format!("ditto_test:{}:{n}", now_millis())
    }

    #[tokio::test]
    async fn redis_store_round_trips_virtual_keys_and_budget_ledgers() {
        let Some(url) = redis_url() else {
            return;
        };

        let store = RedisStore::new(url)
            .expect("store")
            .with_prefix(test_prefix());
        store.ping().await.expect("ping");

        let key = VirtualKeyConfig::new("key-1", "vk-1");
        store
            .replace_virtual_keys(std::slice::from_ref(&key))
            .await
            .expect("persist");
        let loaded = store.load_virtual_keys().await.expect("load");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "key-1");

        store
            .reserve_budget_tokens("req-1", "key-1", 10, 5)
            .await
            .expect("reserve");
        store
            .commit_budget_reservation("req-1")
            .await
            .expect("commit");

        let ledgers = store.list_budget_ledgers().await.expect("ledgers");
        assert_eq!(ledgers.len(), 1);
        assert_eq!(ledgers[0].key_id, "key-1");
        assert_eq!(ledgers[0].spent_tokens, 5);
        assert_eq!(ledgers[0].reserved_tokens, 0);
    }

    #[tokio::test]
    async fn redis_store_commit_budget_reservation_with_tokens_releases_difference() {
        let Some(url) = redis_url() else {
            return;
        };

        let store = RedisStore::new(url)
            .expect("store")
            .with_prefix(test_prefix());
        store.ping().await.expect("ping");

        store
            .reserve_budget_tokens("req-1", "key-1", 10, 7)
            .await
            .expect("reserve");
        store
            .commit_budget_reservation_with_tokens("req-1", 3)
            .await
            .expect("commit");

        let ledgers = store.list_budget_ledgers().await.expect("ledgers");
        assert_eq!(ledgers.len(), 1);
        assert_eq!(ledgers[0].key_id, "key-1");
        assert_eq!(ledgers[0].spent_tokens, 3);
        assert_eq!(ledgers[0].reserved_tokens, 0);

        store
            .reserve_budget_tokens("req-2", "key-1", 10, 7)
            .await
            .expect("reserve 2");
        let err = store.reserve_budget_tokens("req-3", "key-1", 10, 1).await;
        assert!(matches!(err, Err(RedisStoreError::BudgetExceeded { .. })));
    }

    #[tokio::test]
    async fn redis_store_commit_cost_reservation_with_usd_micros_releases_difference() {
        let Some(url) = redis_url() else {
            return;
        };

        let store = RedisStore::new(url)
            .expect("store")
            .with_prefix(test_prefix());
        store.ping().await.expect("ping");

        store
            .reserve_cost_usd_micros("req-1", "key-1", 10, 7)
            .await
            .expect("reserve");
        store
            .commit_cost_reservation_with_usd_micros("req-1", 3)
            .await
            .expect("commit");

        let ledgers = store.list_cost_ledgers().await.expect("ledgers");
        assert_eq!(ledgers.len(), 1);
        assert_eq!(ledgers[0].key_id, "key-1");
        assert_eq!(ledgers[0].spent_usd_micros, 3);
        assert_eq!(ledgers[0].reserved_usd_micros, 0);

        store
            .reserve_cost_usd_micros("req-2", "key-1", 10, 7)
            .await
            .expect("reserve 2");
        let err = store.reserve_cost_usd_micros("req-3", "key-1", 10, 1).await;
        assert!(matches!(
            err,
            Err(RedisStoreError::CostBudgetExceeded { .. })
        ));
    }

    #[tokio::test]
    async fn redis_store_reaps_stale_budget_reservations() {
        let Some(url) = redis_url() else {
            return;
        };

        let store = RedisStore::new(url)
            .expect("store")
            .with_prefix(test_prefix());
        store.ping().await.expect("ping");

        store
            .reserve_budget_tokens("req-1", "key-1", 10, 7)
            .await
            .expect("reserve");

        let ledgers = store.list_budget_ledgers().await.expect("ledgers");
        assert_eq!(ledgers.len(), 1);
        assert_eq!(ledgers[0].key_id, "key-1");
        assert_eq!(ledgers[0].reserved_tokens, 7);

        let cutoff_ts_ms = now_millis_u64().saturating_add(1);
        let (_scanned, reaped, released) = store
            .reap_stale_budget_reservations(cutoff_ts_ms, 1000, false)
            .await
            .expect("reap");
        assert_eq!(reaped, 1);
        assert_eq!(released, 7);

        let ledgers = store.list_budget_ledgers().await.expect("ledgers");
        assert_eq!(ledgers.len(), 1);
        assert_eq!(ledgers[0].reserved_tokens, 0);
        assert_eq!(ledgers[0].spent_tokens, 0);
    }

    #[tokio::test]
    async fn redis_store_reaps_stale_cost_reservations() {
        let Some(url) = redis_url() else {
            return;
        };

        let store = RedisStore::new(url)
            .expect("store")
            .with_prefix(test_prefix());
        store.ping().await.expect("ping");

        store
            .reserve_cost_usd_micros("req-1", "key-1", 10, 7)
            .await
            .expect("reserve");

        let ledgers = store.list_cost_ledgers().await.expect("ledgers");
        assert_eq!(ledgers.len(), 1);
        assert_eq!(ledgers[0].key_id, "key-1");
        assert_eq!(ledgers[0].reserved_usd_micros, 7);

        let cutoff_ts_ms = now_millis_u64().saturating_add(1);
        let (_scanned, reaped, released) = store
            .reap_stale_cost_reservations(cutoff_ts_ms, 1000, false)
            .await
            .expect("reap");
        assert_eq!(reaped, 1);
        assert_eq!(released, 7);

        let ledgers = store.list_cost_ledgers().await.expect("ledgers");
        assert_eq!(ledgers.len(), 1);
        assert_eq!(ledgers[0].reserved_usd_micros, 0);
        assert_eq!(ledgers[0].spent_usd_micros, 0);
    }

    #[tokio::test]
    async fn redis_store_rate_limits_enforce_rpm() {
        let Some(url) = redis_url() else {
            return;
        };

        let store = RedisStore::new(url)
            .expect("store")
            .with_prefix(test_prefix());
        store.ping().await.expect("ping");

        let minute = now_millis_u64() / 60_000;
        let limits = super::super::LimitsConfig {
            rpm: Some(2),
            tpm: Some(1000),
        };

        store
            .check_and_consume_rate_limits("key-rpm", &limits, 1, minute)
            .await
            .expect("first allowed");
        store
            .check_and_consume_rate_limits("key-rpm", &limits, 1, minute)
            .await
            .expect("second allowed");

        let err = store
            .check_and_consume_rate_limits("key-rpm", &limits, 1, minute)
            .await
            .expect_err("third blocked");
        assert!(matches!(err, super::super::GatewayError::RateLimited { .. }));
        if let super::super::GatewayError::RateLimited { limit } = err {
            assert!(limit.starts_with("rpm>"));
        }
    }

    #[tokio::test]
    async fn redis_store_rate_limits_enforce_tpm() {
        let Some(url) = redis_url() else {
            return;
        };

        let store = RedisStore::new(url)
            .expect("store")
            .with_prefix(test_prefix());
        store.ping().await.expect("ping");

        let minute = now_millis_u64() / 60_000;
        let limits = super::super::LimitsConfig {
            rpm: Some(1000),
            tpm: Some(3),
        };

        store
            .check_and_consume_rate_limits("key-tpm", &limits, 2, minute)
            .await
            .expect("first allowed");

        let err = store
            .check_and_consume_rate_limits("key-tpm", &limits, 2, minute)
            .await
            .expect_err("second blocked");
        assert!(matches!(err, super::super::GatewayError::RateLimited { .. }));
        if let super::super::GatewayError::RateLimited { limit } = err {
            assert!(limit.starts_with("tpm>"));
        }
    }
}
