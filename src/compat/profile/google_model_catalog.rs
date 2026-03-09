use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use super::ProviderAuth;
use super::google_catalog_data::generated_google_model_catalog;

static GOOGLE_MODEL_CATALOG: OnceLock<GoogleModelCatalog> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct GoogleModelCatalog {
    pub provider: GoogleCatalogProvider,
    #[serde(default)]
    pub models: BTreeMap<String, GoogleModelCatalogEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct GoogleCatalogProvider {
    pub id: String,
    pub display_name: String,
    pub base_url: String,
    pub protocol: String,
    pub source_url: String,
    pub auth: ProviderAuth,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct GoogleModelCatalogEntry {
    pub source_url: String,
    pub display_name: String,
    pub model_code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_update: Option<String>,
    #[serde(default)]
    pub supported_data_types: GoogleSupportedDataTypes,
    #[serde(default)]
    pub limits: BTreeMap<String, String>,
    #[serde(default)]
    pub capabilities: BTreeMap<String, String>,
    #[serde(default)]
    pub versions: Vec<GoogleModelVersion>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct GoogleSupportedDataTypes {
    #[serde(default)]
    pub input: Vec<String>,
    #[serde(default)]
    pub output: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct GoogleModelVersion {
    pub channel: String,
    pub model: String,
}

pub fn google_model_catalog() -> &'static GoogleModelCatalog {
    GOOGLE_MODEL_CATALOG.get_or_init(generated_google_model_catalog)
}

pub fn google_model_catalog_entry(doc_slug: &str) -> Option<&'static GoogleModelCatalogEntry> {
    google_model_catalog().models.get(doc_slug)
}

pub fn google_model_catalog_entry_by_model(
    model: &str,
) -> Option<&'static GoogleModelCatalogEntry> {
    google_model_catalog().models.values().find(|entry| {
        entry.model_code == model || entry.versions.iter().any(|version| version.model == model)
    })
}
