use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use super::ProviderAuth;
use super::generated_catalogs::generated_anthropic_model_catalog;

static ANTHROPIC_MODEL_CATALOG: OnceLock<AnthropicModelCatalog> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct AnthropicModelCatalog {
    pub provider: AnthropicCatalogProvider,
    #[serde(default)]
    pub models: BTreeMap<String, AnthropicModelCatalogEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct AnthropicCatalogProvider {
    pub id: String,
    pub display_name: String,
    pub base_url: String,
    pub protocol: String,
    pub source_url: String,
    pub auth: ProviderAuth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnthropicModelStatus {
    Active,
    Legacy,
    Deprecated,
    Retired,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct AnthropicModelCatalogEntry {
    pub source_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle_source_url: Option<String>,
    pub display_name: String,
    pub api_model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bedrock_model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertex_model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub input_modalities: Vec<String>,
    #[serde(default)]
    pub output_modalities: Vec<String>,
    #[serde(default)]
    pub features: BTreeMap<String, bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comparative_latency: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beta_context_window_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reliable_knowledge_cutoff: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub training_data_cutoff: Option<String>,
    #[serde(default)]
    pub beta_headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<AnthropicModelPricing>,
    pub status: AnthropicModelStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecated_on: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retirement_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_retired_before: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended_replacement: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct AnthropicModelPricing {
    pub input_usd_per_mtok: String,
    pub output_usd_per_mtok: String,
}

pub fn anthropic_model_catalog() -> &'static AnthropicModelCatalog {
    ANTHROPIC_MODEL_CATALOG.get_or_init(generated_anthropic_model_catalog)
}

pub fn anthropic_model_catalog_entry(model: &str) -> Option<&'static AnthropicModelCatalogEntry> {
    anthropic_model_catalog().models.get(model)
}

pub fn anthropic_model_catalog_entry_by_model(
    model: &str,
) -> Option<&'static AnthropicModelCatalogEntry> {
    anthropic_model_catalog().models.values().find(|entry| {
        entry.api_model_id == model
            || entry.api_alias.as_deref() == Some(model)
            || entry.bedrock_model_id.as_deref() == Some(model)
            || entry.vertex_model_id.as_deref() == Some(model)
    })
}
