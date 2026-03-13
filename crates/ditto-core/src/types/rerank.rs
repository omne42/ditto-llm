use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::contracts::Warning;
use crate::provider_options::ProviderOptionsEnvelope;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RerankDocument {
    Text(String),
    Json(Value),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RerankRequest {
    pub query: String,
    pub documents: Vec<RerankDocument>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_n: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<ProviderOptionsEnvelope>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RerankResult {
    #[serde(default)]
    pub index: u32,
    #[serde(default)]
    pub relevance_score: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RerankResponse {
    #[serde(default)]
    pub ranking: Vec<RerankResult>,
    #[serde(default)]
    pub warnings: Vec<Warning>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<Value>,
}
