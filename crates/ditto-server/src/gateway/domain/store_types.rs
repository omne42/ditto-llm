pub use crate::gateway::contracts::types::{AuditLogRecord, BudgetLedgerRecord, CostLedgerRecord};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StoredHttpHeader {
    pub name: String,
    pub value: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProxyRequestFingerprint {
    pub method: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub virtual_key_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub upstream_headers: Vec<StoredHttpHeader>,
    pub body_sha256: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyRequestIdempotencyState {
    InFlight,
    Completed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProxyRequestReplayError {
    pub message: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProxyRequestReplayResponse {
    pub status: u16,
    pub headers: Vec<StoredHttpHeader>,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProxyRequestReplayOutcome {
    Response(ProxyRequestReplayResponse),
    Error {
        status: u16,
        error: ProxyRequestReplayError,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProxyRequestIdempotencyRecord {
    pub request_id: String,
    pub fingerprint: ProxyRequestFingerprint,
    pub fingerprint_key: String,
    pub state: ProxyRequestIdempotencyState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_token: Option<String>,
    pub started_at_ms: u64,
    pub updated_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_until_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at_ms: Option<u64>,
    pub expires_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<ProxyRequestReplayOutcome>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum ProxyRequestIdempotencyBeginOutcome {
    Acquired,
    Replay {
        record: ProxyRequestIdempotencyRecord,
    },
    InFlight {
        record: ProxyRequestIdempotencyRecord,
    },
    Conflict {
        record: ProxyRequestIdempotencyRecord,
    },
}
