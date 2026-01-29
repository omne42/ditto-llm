use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuditLogRecord {
    pub id: i64,
    pub ts_ms: u64,
    pub kind: String,
    pub payload: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BudgetLedgerRecord {
    pub key_id: String,
    pub spent_tokens: u64,
    pub reserved_tokens: u64,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CostLedgerRecord {
    pub key_id: String,
    pub spent_usd_micros: u64,
    pub reserved_usd_micros: u64,
    pub updated_at_ms: u64,
}
