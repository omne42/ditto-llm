use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::GatewayError;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BudgetConfig {
    pub total_tokens: Option<u64>,
    #[serde(default)]
    pub total_usd_micros: Option<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct BudgetTracker {
    spent_tokens: HashMap<String, u64>,
    spent_usd_micros: HashMap<String, u64>,
}

impl BudgetTracker {
    fn validate_token_reservation(
        current: u64,
        budget: &BudgetConfig,
        tokens: u64,
    ) -> Result<Option<u64>, GatewayError> {
        let Some(limit) = budget.total_tokens else {
            return Ok(None);
        };
        let attempted = current.saturating_add(tokens);
        if attempted > limit {
            return Err(GatewayError::BudgetExceeded { limit, attempted });
        }
        Ok(Some(attempted))
    }

    fn validate_cost_reservation(
        current: u64,
        budget: &BudgetConfig,
        usd_micros: u64,
    ) -> Result<Option<u64>, GatewayError> {
        let Some(limit_usd_micros) = budget.total_usd_micros else {
            return Ok(None);
        };
        let attempted = current.saturating_add(usd_micros);
        if attempted > limit_usd_micros {
            return Err(GatewayError::CostBudgetExceeded {
                limit_usd_micros,
                attempted_usd_micros: attempted,
            });
        }
        Ok(Some(attempted))
    }

    pub fn can_spend(
        &self,
        key_id: &str,
        budget: &BudgetConfig,
        tokens: u64,
    ) -> Result<(), GatewayError> {
        let spent = self.spent_tokens.get(key_id).copied().unwrap_or(0);
        Self::validate_token_reservation(spent, budget, tokens)?;
        Ok(())
    }

    pub fn reserve_many<'a, I>(&mut self, scopes: I, tokens: u64) -> Result<(), GatewayError>
    where
        I: IntoIterator<Item = (&'a str, &'a BudgetConfig)>,
    {
        let mut proposed = HashMap::<String, u64>::new();

        for (scope, budget) in scopes {
            let current = proposed
                .get(scope)
                .copied()
                .unwrap_or_else(|| self.spent_tokens.get(scope).copied().unwrap_or(0));
            let Some(next) = Self::validate_token_reservation(current, budget, tokens)? else {
                continue;
            };
            proposed.insert(scope.to_string(), next);
        }

        for (scope, next) in proposed {
            if next == 0 {
                self.spent_tokens.remove(&scope);
            } else {
                self.spent_tokens.insert(scope, next);
            }
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

    pub fn refund_many<'a, I>(&mut self, scopes: I, tokens: u64)
    where
        I: IntoIterator<Item = (&'a str, &'a BudgetConfig)>,
    {
        for (scope, budget) in scopes {
            self.refund(scope, budget, tokens);
        }
    }

    pub fn settle_many<'a, I>(&mut self, scopes: I, reserved_tokens: u64, actual_tokens: u64)
    where
        I: IntoIterator<Item = (&'a str, &'a BudgetConfig)>,
    {
        if actual_tokens == u64::MAX {
            return;
        }

        match actual_tokens.cmp(&reserved_tokens) {
            std::cmp::Ordering::Less => {
                self.refund_many(scopes, reserved_tokens - actual_tokens);
            }
            std::cmp::Ordering::Greater => {
                for (scope, budget) in scopes {
                    self.spend(scope, budget, actual_tokens - reserved_tokens);
                }
            }
            std::cmp::Ordering::Equal => {}
        }
    }

    pub fn can_spend_cost_usd_micros(
        &self,
        key_id: &str,
        budget: &BudgetConfig,
        usd_micros: u64,
    ) -> Result<(), GatewayError> {
        let spent = self.spent_usd_micros.get(key_id).copied().unwrap_or(0);
        Self::validate_cost_reservation(spent, budget, usd_micros)?;
        Ok(())
    }

    pub fn reserve_cost_many<'a, I>(
        &mut self,
        scopes: I,
        usd_micros: u64,
    ) -> Result<(), GatewayError>
    where
        I: IntoIterator<Item = (&'a str, &'a BudgetConfig)>,
    {
        let mut proposed = HashMap::<String, u64>::new();

        for (scope, budget) in scopes {
            let current = proposed
                .get(scope)
                .copied()
                .unwrap_or_else(|| self.spent_usd_micros.get(scope).copied().unwrap_or(0));
            let Some(next) = Self::validate_cost_reservation(current, budget, usd_micros)? else {
                continue;
            };
            proposed.insert(scope.to_string(), next);
        }

        for (scope, next) in proposed {
            if next == 0 {
                self.spent_usd_micros.remove(&scope);
            } else {
                self.spent_usd_micros.insert(scope, next);
            }
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

    pub fn refund_cost_usd_micros(&mut self, key_id: &str, budget: &BudgetConfig, usd_micros: u64) {
        if budget.total_usd_micros.is_none() || usd_micros == 0 {
            return;
        }
        let Some(entry) = self.spent_usd_micros.get_mut(key_id) else {
            return;
        };
        *entry = entry.saturating_sub(usd_micros);
        if *entry == 0 {
            self.spent_usd_micros.remove(key_id);
        }
    }

    pub fn refund_cost_many<'a, I>(&mut self, scopes: I, usd_micros: u64)
    where
        I: IntoIterator<Item = (&'a str, &'a BudgetConfig)>,
    {
        for (scope, budget) in scopes {
            self.refund_cost_usd_micros(scope, budget, usd_micros);
        }
    }

    pub fn settle_cost_many<'a, I>(
        &mut self,
        scopes: I,
        reserved_usd_micros: u64,
        actual_usd_micros: Option<u64>,
    ) where
        I: IntoIterator<Item = (&'a str, &'a BudgetConfig)>,
    {
        let Some(actual_usd_micros) = actual_usd_micros else {
            return;
        };

        match actual_usd_micros.cmp(&reserved_usd_micros) {
            std::cmp::Ordering::Less => {
                self.refund_cost_many(scopes, reserved_usd_micros - actual_usd_micros);
            }
            std::cmp::Ordering::Greater => {
                for (scope, budget) in scopes {
                    self.spend_cost_usd_micros(
                        scope,
                        budget,
                        actual_usd_micros - reserved_usd_micros,
                    );
                }
            }
            std::cmp::Ordering::Equal => {}
        }
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

    #[test]
    fn reserve_many_is_atomic_across_scopes() {
        let mut tracker = BudgetTracker::default();
        let wide = BudgetConfig {
            total_tokens: Some(100),
            total_usd_micros: None,
        };
        let tight = BudgetConfig {
            total_tokens: Some(5),
            total_usd_micros: None,
        };

        tracker
            .reserve_many([("key", &wide), ("tenant:t1", &wide)], 4)
            .unwrap();
        let err = tracker.reserve_many([("key", &wide), ("tenant:t1", &tight)], 2);
        assert!(matches!(err, Err(GatewayError::BudgetExceeded { .. })));
        assert_eq!(tracker.spent_tokens.get("key"), Some(&4));
        assert_eq!(tracker.spent_tokens.get("tenant:t1"), Some(&4));
    }

    #[test]
    fn reserve_cost_many_is_atomic_across_scopes() {
        let mut tracker = BudgetTracker::default();
        let wide = BudgetConfig {
            total_tokens: None,
            total_usd_micros: Some(1_000),
        };
        let tight = BudgetConfig {
            total_tokens: None,
            total_usd_micros: Some(200),
        };

        tracker
            .reserve_cost_many([("key", &wide), ("tenant:t1", &wide)], 150)
            .unwrap();
        let err = tracker.reserve_cost_many([("key", &wide), ("tenant:t1", &tight)], 100);
        assert!(matches!(err, Err(GatewayError::CostBudgetExceeded { .. })));
        assert_eq!(tracker.spent_usd_micros.get("key"), Some(&150));
        assert_eq!(tracker.spent_usd_micros.get("tenant:t1"), Some(&150));
    }
}
