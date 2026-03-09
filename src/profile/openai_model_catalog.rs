use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use super::ProviderAuth;
use super::generated_catalogs::generated_openai_model_catalog;

static OPENAI_MODEL_CATALOG: OnceLock<OpenAiModelCatalog> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct OpenAiModelCatalog {
    pub provider: OpenAiCatalogProvider,
    #[serde(default)]
    pub models: BTreeMap<String, OpenAiModelCatalogEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct OpenAiCatalogProvider {
    pub id: String,
    pub display_name: String,
    pub base_url: String,
    pub protocol: String,
    pub source_url: String,
    pub auth: ProviderAuth,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct OpenAiModelCatalogEntry {
    pub source_url: String,
    #[serde(default)]
    pub availability_status: OpenAiAvailabilityStatus,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tagline: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub performance: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge_cutoff: Option<String>,
    #[serde(default)]
    pub modalities: BTreeMap<String, OpenAiModalitySupport>,
    #[serde(default)]
    pub features: BTreeMap<String, bool>,
    #[serde(default)]
    pub tools: BTreeMap<String, bool>,
    #[serde(default)]
    pub revisions: OpenAiModelRevisions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenAiModalitySupport {
    InputOnly,
    OutputOnly,
    InputAndOutput,
    NotSupported,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenAiAvailabilityStatus {
    #[default]
    Unverified,
    Available,
    CacheQuestionable,
    AvailabilityQuestionable,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct OpenAiModelRevisions {
    #[serde(default)]
    pub snapshots: Vec<String>,
}

pub fn openai_model_catalog() -> &'static OpenAiModelCatalog {
    OPENAI_MODEL_CATALOG.get_or_init(generated_openai_model_catalog)
}

pub fn openai_model_catalog_entry(model: &str) -> Option<&'static OpenAiModelCatalogEntry> {
    openai_model_catalog().models.get(model)
}
