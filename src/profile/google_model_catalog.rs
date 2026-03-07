use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use super::ProviderAuth;

const EMBEDDED_GOOGLE_MODEL_CATALOG: &str =
    include_str!("../../catalog/provider_models/google.toml");
const GOOGLE_MODEL_CATALOG_ENV: &str = "DITTO_GOOGLE_MODEL_CATALOG";
const PROVIDER_MODEL_CATALOG_DIR_ENV: &str = "DITTO_PROVIDER_MODEL_CATALOG_DIR";

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
    GOOGLE_MODEL_CATALOG.get_or_init(|| {
        let catalog_text = load_google_model_catalog_text();
        toml::from_str::<GoogleModelCatalog>(&catalog_text)
            .expect("embedded google model catalog must parse")
    })
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

fn load_google_model_catalog_text() -> String {
    let Some(path) = configured_google_model_catalog_path() else {
        return EMBEDDED_GOOGLE_MODEL_CATALOG.to_string();
    };
    std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "failed to read google model catalog from {}: {err}",
            path.display()
        )
    })
}

fn configured_google_model_catalog_path() -> Option<PathBuf> {
    std::env::var_os(GOOGLE_MODEL_CATALOG_ENV)
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os(PROVIDER_MODEL_CATALOG_DIR_ENV)
                .map(PathBuf::from)
                .map(|dir| dir.join("google.toml"))
        })
}
