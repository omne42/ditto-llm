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
        if budget.total_tokens.is_none() {
            return;
        }
        let entry = self.spent_tokens.entry(key_id.to_string()).or_insert(0);
        *entry = entry.saturating_add(tokens);
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
        if budget.total_usd_micros.is_none() {
            return;
        }
        let entry = self.spent_usd_micros.entry(key_id.to_string()).or_insert(0);
        *entry = entry.saturating_add(usd_micros);
    }

    pub fn retain_scopes(&mut self, scopes: &HashSet<String>) {
        self.spent_tokens.retain(|scope, _| scopes.contains(scope));
        self.spent_usd_micros
            .retain(|scope, _| scopes.contains(scope));
    }
}
