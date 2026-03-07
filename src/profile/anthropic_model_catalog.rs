use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use super::ProviderAuth;

const EMBEDDED_ANTHROPIC_MODEL_CATALOG: &str =
    include_str!("../../catalog/provider_models/anthropic.toml");
const ANTHROPIC_MODEL_CATALOG_ENV: &str = "DITTO_ANTHROPIC_MODEL_CATALOG";
const PROVIDER_MODEL_CATALOG_DIR_ENV: &str = "DITTO_PROVIDER_MODEL_CATALOG_DIR";

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
    ANTHROPIC_MODEL_CATALOG.get_or_init(|| {
        let catalog_text = load_anthropic_model_catalog_text();
        toml::from_str::<AnthropicModelCatalog>(&catalog_text)
            .expect("embedded anthropic model catalog must parse")
    })
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

fn load_anthropic_model_catalog_text() -> String {
    let Some(path) = configured_anthropic_model_catalog_path() else {
        return EMBEDDED_ANTHROPIC_MODEL_CATALOG.to_string();
    };
    std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "failed to read anthropic model catalog from {}: {err}",
            path.display()
        )
    })
}

fn configured_anthropic_model_catalog_path() -> Option<PathBuf> {
    std::env::var_os(ANTHROPIC_MODEL_CATALOG_ENV)
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os(PROVIDER_MODEL_CATALOG_DIR_ENV)
                .map(PathBuf::from)
                .map(|dir| dir.join("anthropic.toml"))
        })
}
