use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::GatewayError;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BudgetConfig {
    pub total_tokens: Option<u64>,
    #[serde(default)]
    pub total_usd_micros: Option<u64>,
}

#[derive(Debug, Default)]
pub struct BudgetTracker {
    spent_tokens: HashMap<String, u64>,
    spent_usd_micros: HashMap<String, u64>,
}

impl BudgetTracker {
    pub fn can_spend(
        &self,
        key_id: &str,
        budget: &BudgetConfig,
        tokens: u64,
    ) -> Result<(), GatewayError> {
        let Some(limit) = budget.total_tokens else {
            return Ok(());
        };
        let spent = self.spent_tokens.get(key_id).copied().unwrap_or(0);
        let attempted = spent.saturating_add(tokens);
        if attempted > limit {
            return Err(GatewayError::BudgetExceeded { limit, attempted });
        }
        Ok(())
    }

    pub fn spend(&mut self, key_id: &str, budget: &BudgetConfig, tokens: u64) {
        if budget.total_tokens.is_none() || tokens == 0 {
            return;
        }
        if let Some(entry) = self.spent_tokens.get_mut(key_id) {
            *entry = entry.saturating_add(tokens);
            return;
        }
        self.spent_tokens.insert(key_id.to_string(), tokens);
    }

    pub fn refund(&mut self, key_id: &str, budget: &BudgetConfig, tokens: u64) {
        if budget.total_tokens.is_none() || tokens == 0 {
            return;
        }
        let Some(entry) = self.spent_tokens.get_mut(key_id) else {
            return;
        };
        *entry = entry.saturating_sub(tokens);
        if *entry == 0 {
            self.spent_tokens.remove(key_id);
        }
    }

    pub fn can_spend_cost_usd_micros(
        &self,
        key_id: &str,
        budget: &BudgetConfig,
        usd_micros: u64,
    ) -> Result<(), GatewayError> {
        let Some(limit_usd_micros) = budget.total_usd_micros else {
            return Ok(());
        };
        let spent = self.spent_usd_micros.get(key_id).copied().unwrap_or(0);
        let attempted = spent.saturating_add(usd_micros);
        if attempted > limit_usd_micros {
            return Err(GatewayError::CostBudgetExceeded {
                limit_usd_micros,
                attempted_usd_micros: attempted,
            });
        }
        Ok(())
    }

    pub fn spend_cost_usd_micros(&mut self, key_id: &str, budget: &BudgetConfig, usd_micros: u64) {
        if budget.total_usd_micros.is_none() || usd_micros == 0 {
            return;
        }
        if let Some(entry) = self.spent_usd_micros.get_mut(key_id) {
            *entry = entry.saturating_add(usd_micros);
            return;
        }
        self.spent_usd_micros.insert(key_id.to_string(), usd_micros);
    }

    pub fn retain_scopes(&mut self, scopes: &HashSet<String>) {
        self.spent_tokens.retain(|scope, _| scopes.contains(scope));
        self.spent_usd_micros
            .retain(|scope, _| scopes.contains(scope));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_spend_does_not_create_tracking_entries() {
        let mut tracker = BudgetTracker::default();
        let budget = BudgetConfig {
            total_tokens: Some(100),
            total_usd_micros: Some(1_000),
        };

        tracker.spend("vk_1", &budget, 0);
        tracker.spend_cost_usd_micros("vk_1", &budget, 0);

        assert!(!tracker.spent_tokens.contains_key("vk_1"));
        assert!(!tracker.spent_usd_micros.contains_key("vk_1"));
    }

    #[test]
    fn non_zero_spend_still_accumulates() {
        let mut tracker = BudgetTracker::default();
        let budget = BudgetConfig {
            total_tokens: Some(100),
            total_usd_micros: Some(1_000),
        };

        tracker.spend("vk_1", &budget, 10);
        tracker.spend("vk_1", &budget, 5);
        tracker.spend_cost_usd_micros("vk_1", &budget, 12);
        tracker.spend_cost_usd_micros("vk_1", &budget, 3);

        assert_eq!(tracker.spent_tokens.get("vk_1"), Some(&15));
        assert_eq!(tracker.spent_usd_micros.get("vk_1"), Some(&15));
    }
}
