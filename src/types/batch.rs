use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::contracts::Warning;
use crate::provider_options::ProviderOptionsEnvelope;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BatchStatus {
    Validating,
    Failed,
    InProgress,
    Finalizing,
    Completed,
    Expired,
    Cancelling,
    Cancelled,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BatchRequestCounts {
    #[serde(default)]
    pub total: u32,
    #[serde(default)]
    pub completed: u32,
    #[serde(default)]
    pub failed: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Batch {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub status: BatchStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_window: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_file_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_file_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_file_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_progress_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finalizing_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expired_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancelling_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancelled_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_counts: Option<BatchRequestCounts>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub errors: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchCreateRequest {
    pub input_file_id: String,
    pub endpoint: String,
    pub completion_window: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<ProviderOptionsEnvelope>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BatchResponse {
    pub batch: Batch,
    #[serde(default)]
    pub warnings: Vec<Warning>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BatchListResponse {
    #[serde(default)]
    pub batches: Vec<Batch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_more: Option<bool>,
    #[serde(default)]
    pub warnings: Vec<Warning>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<Value>,
}
