use serde::{Deserialize, Serialize};

pub const GATEWAY_CONTRACT_VERSION: &str = "0.1.0";
pub const GATEWAY_CONTRACT_ID: &str = "gateway-v0.1";

pub const GATEWAY_OPENAPI_V0_1_YAML: &str =
    include_str!("../../../contracts/gateway-contract-v0.1.openapi.yaml");

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorDetail {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: ErrorDetail,
}

pub type ProxyJsonEnvelope = serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListAuditLogsQuery {
    #[serde(default = "default_audit_limit")]
    pub limit: usize,
    #[serde(default)]
    pub since_ts_ms: Option<u64>,
}

fn default_audit_limit() -> usize {
    100
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListLedgersQuery {
    #[serde(default)]
    pub key_prefix: Option<String>,
    #[serde(default = "default_ledger_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

fn default_ledger_limit() -> usize {
    1000
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AuditLogRecord {
    pub id: i64,
    pub ts_ms: u64,
    pub kind: String,
    pub payload: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetLedgerRecord {
    pub key_id: String,
    pub spent_tokens: u64,
    pub reserved_tokens: u64,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostLedgerRecord {
    pub key_id: String,
    pub spent_usd_micros: u64,
    pub reserved_usd_micros: u64,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReapReservationsRequest {
    #[serde(default = "default_older_than_secs")]
    pub older_than_secs: u64,
    #[serde(default = "default_reap_limit")]
    pub limit: usize,
    #[serde(default)]
    pub dry_run: bool,
}

fn default_older_than_secs() -> u64 {
    24 * 60 * 60
}

fn default_reap_limit() -> usize {
    1000
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReapReservationsCounts {
    pub scanned: u64,
    pub reaped: u64,
    pub released: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReapReservationsResponse {
    pub store: String,
    pub dry_run: bool,
    pub cutoff_ts_ms: u64,
    pub budget: ReapReservationsCounts,
    pub cost: ReapReservationsCounts,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_yaml_exposes_expected_version_and_paths() {
        let doc: serde_yaml::Value =
            serde_yaml::from_str(GATEWAY_OPENAPI_V0_1_YAML).expect("parse openapi yaml");

        let version = doc
            .get("info")
            .and_then(|value| value.get("version"))
            .and_then(serde_yaml::Value::as_str)
            .expect("info.version present");
        assert_eq!(version, GATEWAY_CONTRACT_VERSION);

        let contract_id = doc
            .get("info")
            .and_then(|value| value.get("x-ditto-contract-id"))
            .and_then(serde_yaml::Value::as_str)
            .expect("contract id present");
        assert_eq!(contract_id, GATEWAY_CONTRACT_ID);

        let paths = doc.get("paths").expect("paths present");
        assert!(paths.get("/health").is_some());
        assert!(paths.get("/v1/chat/completions").is_some());
        assert!(paths.get("/admin/audit").is_some());
        assert!(paths.get("/admin/budgets").is_some());
        assert!(paths.get("/admin/costs").is_some());
        assert!(paths.get("/admin/reservations/reap").is_some());
    }

    #[test]
    fn reap_request_defaults_match_contract() {
        let req: ReapReservationsRequest =
            serde_json::from_str("{}").expect("deserialize with defaults");
        assert_eq!(req.older_than_secs, 24 * 60 * 60);
        assert_eq!(req.limit, 1000);
        assert!(!req.dry_run);
    }

    #[test]
    fn query_defaults_match_contract() {
        let audit: ListAuditLogsQuery =
            serde_json::from_str("{}").expect("deserialize audit query defaults");
        assert_eq!(audit.limit, 100);
        assert_eq!(audit.since_ts_ms, None);

        let ledgers: ListLedgersQuery =
            serde_json::from_str("{}").expect("deserialize ledgers query defaults");
        assert_eq!(ledgers.key_prefix, None);
        assert_eq!(ledgers.limit, 1000);
        assert_eq!(ledgers.offset, 0);
    }
}
