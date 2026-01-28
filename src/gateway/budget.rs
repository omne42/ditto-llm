use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::GatewayError;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BudgetConfig {
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Default)]
pub struct BudgetTracker {
    spent: HashMap<String, u64>,
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
        let spent = self.spent.get(key_id).copied().unwrap_or(0);
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
        let entry = self.spent.entry(key_id.to_string()).or_insert(0);
        *entry = entry.saturating_add(tokens);
    }
}
