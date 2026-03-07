use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use super::ProviderAuth;

const EMBEDDED_OPENAI_MODEL_CATALOG: &str =
    include_str!("../../catalog/provider_models/openai.toml");
const OPENAI_MODEL_CATALOG_ENV: &str = "DITTO_OPENAI_MODEL_CATALOG";
const PROVIDER_MODEL_CATALOG_DIR_ENV: &str = "DITTO_PROVIDER_MODEL_CATALOG_DIR";

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

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct OpenAiModelRevisions {
    #[serde(default)]
    pub snapshots: Vec<String>,
}

pub fn openai_model_catalog() -> &'static OpenAiModelCatalog {
    OPENAI_MODEL_CATALOG.get_or_init(|| {
        let catalog_text = load_openai_model_catalog_text();
        toml::from_str::<OpenAiModelCatalog>(&catalog_text)
            .expect("embedded openai model catalog must parse")
    })
}

pub fn openai_model_catalog_entry(model: &str) -> Option<&'static OpenAiModelCatalogEntry> {
    openai_model_catalog().models.get(model)
}

fn load_openai_model_catalog_text() -> String {
    let Some(path) = configured_openai_model_catalog_path() else {
        return EMBEDDED_OPENAI_MODEL_CATALOG.to_string();
    };
    std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "failed to read openai model catalog from {}: {err}",
            path.display()
        )
    })
}

fn configured_openai_model_catalog_path() -> Option<PathBuf> {
    std::env::var_os(OPENAI_MODEL_CATALOG_ENV)
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os(PROVIDER_MODEL_CATALOG_DIR_ENV)
                .map(PathBuf::from)
                .map(|dir| dir.join("openai.toml"))
        })
}
