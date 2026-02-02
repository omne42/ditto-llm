#[cfg(all(
    test,
    any(feature = "gateway-store-sqlite", feature = "gateway-store-redis")
))]
mod ledger_grouping_tests {
    use super::*;

    #[test]
    fn budget_ledgers_group_by_project_and_user() {
        let mut key_1 = VirtualKeyConfig::new("key-1", "vk-1");
        key_1.tenant_id = Some("tenant-a".to_string());
        key_1.project_id = Some("proj-a".to_string());
        key_1.user_id = Some("user-a".to_string());

        let mut key_2 = VirtualKeyConfig::new("key-2", "vk-2");
        key_2.tenant_id = Some("tenant-a".to_string());
        key_2.project_id = Some("proj-a".to_string());
        key_2.user_id = Some("user-b".to_string());

        let ledgers = vec![
            BudgetLedgerRecord {
                key_id: "key-1".to_string(),
                spent_tokens: 10,
                reserved_tokens: 3,
                updated_at_ms: 100,
            },
            BudgetLedgerRecord {
                key_id: "key-2".to_string(),
                spent_tokens: 7,
                reserved_tokens: 0,
                updated_at_ms: 200,
            },
            BudgetLedgerRecord {
                key_id: "key-unknown".to_string(),
                spent_tokens: 1,
                reserved_tokens: 2,
                updated_at_ms: 50,
            },
        ];

        let keys = vec![key_1, key_2];

        let projects = group_budget_ledgers_by_project(&ledgers, &keys);
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[0].project_id, None);
        assert_eq!(projects[0].spent_tokens, 1);
        assert_eq!(projects[0].reserved_tokens, 2);
        assert_eq!(projects[0].key_count, 1);
        assert_eq!(projects[0].updated_at_ms, 50);
        assert_eq!(projects[1].project_id.as_deref(), Some("proj-a"));
        assert_eq!(projects[1].spent_tokens, 17);
        assert_eq!(projects[1].reserved_tokens, 3);
        assert_eq!(projects[1].key_count, 2);
        assert_eq!(projects[1].updated_at_ms, 200);

        let users = group_budget_ledgers_by_user(&ledgers, &keys);
        assert_eq!(users.len(), 3);
        assert_eq!(users[0].user_id, None);
        assert_eq!(users[0].spent_tokens, 1);
        assert_eq!(users[0].reserved_tokens, 2);
        assert_eq!(users[0].key_count, 1);
        assert_eq!(users[0].updated_at_ms, 50);
        assert_eq!(users[1].user_id.as_deref(), Some("user-a"));
        assert_eq!(users[1].spent_tokens, 10);
        assert_eq!(users[1].reserved_tokens, 3);
        assert_eq!(users[1].key_count, 1);
        assert_eq!(users[1].updated_at_ms, 100);
        assert_eq!(users[2].user_id.as_deref(), Some("user-b"));
        assert_eq!(users[2].spent_tokens, 7);
        assert_eq!(users[2].reserved_tokens, 0);
        assert_eq!(users[2].key_count, 1);
        assert_eq!(users[2].updated_at_ms, 200);

        let tenants = group_budget_ledgers_by_tenant(&ledgers, &keys);
        assert_eq!(tenants.len(), 2);
        assert_eq!(tenants[0].tenant_id, None);
        assert_eq!(tenants[0].spent_tokens, 1);
        assert_eq!(tenants[0].reserved_tokens, 2);
        assert_eq!(tenants[0].key_count, 1);
        assert_eq!(tenants[0].updated_at_ms, 50);
        assert_eq!(tenants[1].tenant_id.as_deref(), Some("tenant-a"));
        assert_eq!(tenants[1].spent_tokens, 17);
        assert_eq!(tenants[1].reserved_tokens, 3);
        assert_eq!(tenants[1].key_count, 2);
        assert_eq!(tenants[1].updated_at_ms, 200);
    }

    #[cfg(feature = "gateway-costing")]
    #[test]
    fn cost_ledgers_group_by_project_and_user() {
        let mut key_1 = VirtualKeyConfig::new("key-1", "vk-1");
        key_1.tenant_id = Some("tenant-a".to_string());
        key_1.project_id = Some("proj-a".to_string());
        key_1.user_id = Some("user-a".to_string());

        let ledgers = vec![
            CostLedgerRecord {
                key_id: "key-1".to_string(),
                spent_usd_micros: 10,
                reserved_usd_micros: 3,
                updated_at_ms: 100,
            },
            CostLedgerRecord {
                key_id: "key-unknown".to_string(),
                spent_usd_micros: 1,
                reserved_usd_micros: 2,
                updated_at_ms: 50,
            },
        ];

        let keys = vec![key_1];

        let projects = group_cost_ledgers_by_project(&ledgers, &keys);
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[0].project_id, None);
        assert_eq!(projects[0].spent_usd_micros, 1);
        assert_eq!(projects[0].reserved_usd_micros, 2);
        assert_eq!(projects[0].key_count, 1);
        assert_eq!(projects[0].updated_at_ms, 50);
        assert_eq!(projects[1].project_id.as_deref(), Some("proj-a"));
        assert_eq!(projects[1].spent_usd_micros, 10);
        assert_eq!(projects[1].reserved_usd_micros, 3);
        assert_eq!(projects[1].key_count, 1);
        assert_eq!(projects[1].updated_at_ms, 100);

        let users = group_cost_ledgers_by_user(&ledgers, &keys);
        assert_eq!(users.len(), 2);
        assert_eq!(users[0].user_id, None);
        assert_eq!(users[0].spent_usd_micros, 1);
        assert_eq!(users[0].reserved_usd_micros, 2);
        assert_eq!(users[0].key_count, 1);
        assert_eq!(users[0].updated_at_ms, 50);
        assert_eq!(users[1].user_id.as_deref(), Some("user-a"));
        assert_eq!(users[1].spent_usd_micros, 10);
        assert_eq!(users[1].reserved_usd_micros, 3);
        assert_eq!(users[1].key_count, 1);
        assert_eq!(users[1].updated_at_ms, 100);

        let tenants = group_cost_ledgers_by_tenant(&ledgers, &keys);
        assert_eq!(tenants.len(), 2);
        assert_eq!(tenants[0].tenant_id, None);
        assert_eq!(tenants[0].spent_usd_micros, 1);
        assert_eq!(tenants[0].reserved_usd_micros, 2);
        assert_eq!(tenants[0].key_count, 1);
        assert_eq!(tenants[0].updated_at_ms, 50);
        assert_eq!(tenants[1].tenant_id.as_deref(), Some("tenant-a"));
        assert_eq!(tenants[1].spent_usd_micros, 10);
        assert_eq!(tenants[1].reserved_usd_micros, 3);
        assert_eq!(tenants[1].key_count, 1);
        assert_eq!(tenants[1].updated_at_ms, 100);
    }
}

