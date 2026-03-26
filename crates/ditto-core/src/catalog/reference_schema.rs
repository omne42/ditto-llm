use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use config_kit::{ConfigFormat, ConfigFormatSet, ConfigLoadOptions, load_typed_config_file};
use serde::Deserialize;
use serde_json::{Map as JsonMap, Value as JsonValue};

use super::{OperationKind, ProviderCapabilitySet, capability_for_operation};
use crate::error::ReferenceCatalogLoadError;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ReferenceProviderModelCatalog {
    pub provider: ReferenceProviderDescriptor,
    #[serde(default)]
    pub models: BTreeMap<String, ReferenceModelEntry>,
}

impl ReferenceProviderModelCatalog {
    pub fn from_json_str(input: &str) -> Result<Self, ReferenceCatalogLoadError> {
        ConfigFormat::Json
            .parse(input)
            .map_err(ReferenceCatalogLoadError::from)
    }

    pub fn from_toml_str(input: &str) -> Result<Self, ReferenceCatalogLoadError> {
        ConfigFormat::Toml
            .parse(input)
            .map_err(ReferenceCatalogLoadError::from)
    }

    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, ReferenceCatalogLoadError> {
        load_typed_config_file(path, ConfigLoadOptions::new(), ConfigFormatSet::JSON_TOML)
            .map_err(ReferenceCatalogLoadError::from)
    }

    pub fn validate(&self, expected_provider_id: Option<&str>) -> ReferenceCatalogValidationReport {
        let mut report = ReferenceCatalogValidationReport::default();

        if let Some(expected) = expected_provider_id {
            if self.provider.id != expected {
                report
                    .issues
                    .push(ReferenceCatalogValidationIssue::ProviderIdMismatch {
                        expected: expected.to_string(),
                        actual: self.provider.id.clone(),
                    });
            }
        }

        for (model_id, model) in &self.models {
            if model.api_surfaces.is_empty() {
                report
                    .issues
                    .push(ReferenceCatalogValidationIssue::ModelMissingApiSurfaces {
                        model_id: model_id.clone(),
                    });
            }

            let declared_surfaces = model.api_surface_set();
            let record_surfaces = model.record_surface_set();
            let undeclared_record_surfaces: BTreeSet<String> = record_surfaces
                .difference(&declared_surfaces)
                .filter(|surface| !is_global_reference_surface(surface))
                .cloned()
                .collect();

            if !undeclared_record_surfaces.is_empty() {
                report.issues.push(
                    ReferenceCatalogValidationIssue::ModelRecordSurfaceUndeclared {
                        model_id: model_id.clone(),
                        declared: declared_surfaces,
                        recorded: undeclared_record_surfaces,
                    },
                );
            }

            for record in model
                .records
                .iter()
                .filter(|record| record.describes_api_surface())
            {
                if record.requires_explicit_api_surface() && record.api_surface.is_empty() {
                    report
                        .issues
                        .push(ReferenceCatalogValidationIssue::RecordMissingApiSurface {
                            model_id: model_id.clone(),
                            source_url: record.source_url.clone(),
                        });
                }
                if record.requires_endpoint() && record.endpoint.is_empty() {
                    report
                        .issues
                        .push(ReferenceCatalogValidationIssue::RecordMissingEndpoint {
                            model_id: model_id.clone(),
                            api_surface: record.api_surface.clone(),
                        });
                }
            }
        }

        report
    }

    pub fn canonical_json_value(&self) -> JsonValue {
        let mut root = JsonMap::new();
        root.insert("provider".to_string(), self.provider.canonical_json_value());

        let mut models = JsonMap::new();
        for (model_id, model) in &self.models {
            models.insert(model_id.clone(), model.canonical_json_value());
        }
        root.insert("models".to_string(), JsonValue::Object(models));

        JsonValue::Object(root)
    }

    pub fn catalog_api_surface_set(&self) -> BTreeSet<String> {
        self.models
            .values()
            .flat_map(|model| model.api_surfaces.iter().cloned())
            .collect()
    }

    pub fn capability_profile(&self) -> ReferenceProviderCapabilityProfile {
        let mut capabilities = ProviderCapabilitySet::new();
        let mut unmapped_api_surfaces = BTreeSet::new();

        for model in self.models.values() {
            let profile = model.capability_profile();
            capabilities.extend(&profile.capabilities);
            unmapped_api_surfaces.extend(profile.unmapped_api_surfaces);
        }

        ReferenceProviderCapabilityProfile {
            provider_id: self.provider.id.clone(),
            capabilities,
            unmapped_api_surfaces,
        }
    }

    pub fn validate_expectation(
        &self,
        expectation: ReferenceCatalogExpectation,
    ) -> ReferenceCatalogExpectationReport {
        let mut report = ReferenceCatalogExpectationReport::default();

        if self.provider.id != expectation.provider_id {
            report
                .issues
                .push(ReferenceCatalogExpectationIssue::ProviderFieldMismatch {
                    field: "provider.id",
                    expected: expectation.provider_id.to_string(),
                    actual: Some(self.provider.id.clone()),
                });
        }

        if let Some(expected) = expectation.display_name {
            let actual = self.provider.display_name.clone();
            if actual.as_deref() != Some(expected) {
                report
                    .issues
                    .push(ReferenceCatalogExpectationIssue::ProviderFieldMismatch {
                        field: "provider.display_name",
                        expected: expected.to_string(),
                        actual,
                    });
            }
        }

        if let Some(expected) = expectation.base_url {
            let actual = self.provider.base_url.clone();
            if actual.as_deref() != Some(expected) {
                report
                    .issues
                    .push(ReferenceCatalogExpectationIssue::ProviderFieldMismatch {
                        field: "provider.base_url",
                        expected: expected.to_string(),
                        actual,
                    });
            }
        }

        if let Some(expected) = expectation.protocol {
            let actual = self.provider.protocol.clone();
            if actual.as_deref() != Some(expected) {
                report
                    .issues
                    .push(ReferenceCatalogExpectationIssue::ProviderFieldMismatch {
                        field: "provider.protocol",
                        expected: expected.to_string(),
                        actual,
                    });
            }
        }

        if let Some(expected) = expectation.auth_type {
            let actual = Some(self.provider.auth.auth_type.clone());
            if actual.as_deref() != Some(expected) {
                report
                    .issues
                    .push(ReferenceCatalogExpectationIssue::ProviderFieldMismatch {
                        field: "provider.auth.type",
                        expected: expected.to_string(),
                        actual,
                    });
            }
        }

        if let Some(expected_prefix) = expectation.source_url_prefix {
            let actual = self.provider.source_url.clone();
            if !actual
                .as_deref()
                .is_some_and(|actual| actual.starts_with(expected_prefix))
            {
                report.issues.push(
                    ReferenceCatalogExpectationIssue::ProviderSourceUrlMismatch {
                        expected_prefix: expected_prefix.to_string(),
                        actual,
                    },
                );
            }
        }

        if self.models.len() < expectation.min_model_count {
            report
                .issues
                .push(ReferenceCatalogExpectationIssue::ModelCountTooSmall {
                    expected_at_least: expectation.min_model_count,
                    actual: self.models.len(),
                });
        }

        let actual_surfaces = self.catalog_api_surface_set();
        for required_surface in expectation.required_api_surfaces {
            if !actual_surfaces.contains(*required_surface) {
                report.issues.push(
                    ReferenceCatalogExpectationIssue::MissingRequiredApiSurface {
                        api_surface: (*required_surface).to_string(),
                    },
                );
            }
        }

        report
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ReferenceProviderDescriptor {
    pub id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default)]
    pub source_url: Option<String>,
    pub auth: ReferenceProviderAuth,
    #[serde(flatten)]
    pub vendor_metadata: BTreeMap<String, JsonValue>,
}

impl ReferenceProviderDescriptor {
    fn canonical_json_value(&self) -> JsonValue {
        let mut out = JsonMap::new();
        out.insert("id".to_string(), JsonValue::String(self.id.clone()));
        insert_optional_string(&mut out, "display_name", &self.display_name);
        insert_optional_string(&mut out, "base_url", &self.base_url);
        insert_optional_string(&mut out, "protocol", &self.protocol);
        insert_optional_string(&mut out, "source_url", &self.source_url);
        out.insert("auth".to_string(), self.auth.canonical_json_value());
        insert_vendor_metadata(&mut out, &self.vendor_metadata);
        JsonValue::Object(out)
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ReferenceProviderAuth {
    #[serde(rename = "type")]
    pub auth_type: String,
    #[serde(default)]
    pub keys: Vec<String>,
    #[serde(flatten)]
    pub vendor_metadata: BTreeMap<String, JsonValue>,
}

impl ReferenceProviderAuth {
    fn canonical_json_value(&self) -> JsonValue {
        let mut out = JsonMap::new();
        out.insert(
            "type".to_string(),
            JsonValue::String(self.auth_type.clone()),
        );
        out.insert(
            "keys".to_string(),
            JsonValue::Array(self.keys.iter().cloned().map(JsonValue::String).collect()),
        );
        insert_vendor_metadata(&mut out, &self.vendor_metadata);
        JsonValue::Object(out)
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ReferenceModelEntry {
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub api_surfaces: Vec<String>,
    #[serde(default)]
    pub records: Vec<ReferenceModelRecord>,
    #[serde(flatten)]
    pub vendor_metadata: BTreeMap<String, JsonValue>,
}

impl ReferenceModelEntry {
    fn api_surface_set(&self) -> BTreeSet<String> {
        self.api_surfaces.iter().cloned().collect()
    }

    pub fn capability_profile(&self) -> ReferenceModelCapabilityProfile {
        let api_surfaces = self.api_surface_set();
        let mut capabilities = ProviderCapabilitySet::new();
        let mut unmapped_api_surfaces = BTreeSet::new();

        for surface in &api_surfaces {
            let Some(operation) = operation_for_api_surface(surface) else {
                unmapped_api_surfaces.insert(surface.clone());
                continue;
            };
            if let Some(capability) = capability_for_operation(operation) {
                capabilities.insert(capability);
            } else {
                unmapped_api_surfaces.insert(surface.clone());
            }
        }

        ReferenceModelCapabilityProfile {
            api_surfaces,
            capabilities,
            unmapped_api_surfaces,
        }
    }

    fn record_surface_set(&self) -> BTreeSet<String> {
        self.records
            .iter()
            .filter(|record| record.describes_api_surface())
            .map(|record| record.api_surface.clone())
            .filter(|surface| !surface.is_empty())
            .collect()
    }

    fn canonical_json_value(&self) -> JsonValue {
        let mut out = JsonMap::new();
        insert_optional_string(&mut out, "display_name", &self.display_name);
        insert_optional_string(&mut out, "source_url", &self.source_url);

        let mut api_surfaces = self.api_surfaces.clone();
        api_surfaces.sort();
        out.insert(
            "api_surfaces".to_string(),
            JsonValue::Array(api_surfaces.into_iter().map(JsonValue::String).collect()),
        );

        let mut records: Vec<JsonValue> = self
            .records
            .iter()
            .map(ReferenceModelRecord::canonical_json_value)
            .collect();
        records.sort_by_key(canonical_value_key);
        out.insert("records".to_string(), JsonValue::Array(records));

        insert_vendor_metadata(&mut out, &self.vendor_metadata);
        JsonValue::Object(out)
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ReferenceModelRecord {
    #[serde(default)]
    pub table_kind: Option<String>,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub source_page: Option<String>,
    #[serde(default)]
    pub section: Option<String>,
    #[serde(default)]
    pub api_surface: String,
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(flatten)]
    pub vendor_metadata: BTreeMap<String, JsonValue>,
}

impl ReferenceModelRecord {
    fn describes_api_surface(&self) -> bool {
        self.table_kind.as_deref() == Some("api_reference")
            || !self.api_surface.is_empty()
            || !self.endpoint.is_empty()
    }

    fn requires_explicit_api_surface(&self) -> bool {
        self.table_kind.as_deref() == Some("api_reference")
    }

    fn requires_endpoint(&self) -> bool {
        self.table_kind.as_deref() == Some("api_reference")
    }

    fn canonical_json_value(&self) -> JsonValue {
        let mut out = JsonMap::new();
        insert_optional_string(&mut out, "table_kind", &self.table_kind);
        insert_optional_string(&mut out, "source_url", &self.source_url);
        insert_optional_string(&mut out, "source_page", &self.source_page);
        insert_optional_string(&mut out, "section", &self.section);
        out.insert(
            "api_surface".to_string(),
            JsonValue::String(self.api_surface.clone()),
        );
        out.insert(
            "endpoint".to_string(),
            JsonValue::String(self.endpoint.clone()),
        );
        insert_optional_string(&mut out, "notes", &self.notes);
        insert_vendor_metadata(&mut out, &self.vendor_metadata);
        JsonValue::Object(out)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceModelCapabilityProfile {
    pub api_surfaces: BTreeSet<String>,
    pub capabilities: ProviderCapabilitySet,
    pub unmapped_api_surfaces: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceProviderCapabilityProfile {
    pub provider_id: String,
    pub capabilities: ProviderCapabilitySet,
    pub unmapped_api_surfaces: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReferenceCatalogValidationReport {
    pub issues: Vec<ReferenceCatalogValidationIssue>,
}

impl ReferenceCatalogValidationReport {
    pub fn is_clean(&self) -> bool {
        self.issues.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceCatalogRole {
    CompleteProviderDirectory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReferenceCatalogExpectation {
    pub stem: &'static str,
    pub role: ReferenceCatalogRole,
    pub provider_id: &'static str,
    pub display_name: Option<&'static str>,
    pub base_url: Option<&'static str>,
    pub protocol: Option<&'static str>,
    pub auth_type: Option<&'static str>,
    pub source_url_prefix: Option<&'static str>,
    pub min_model_count: usize,
    pub required_api_surfaces: &'static [&'static str],
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReferenceCatalogExpectationReport {
    pub issues: Vec<ReferenceCatalogExpectationIssue>,
}

impl ReferenceCatalogExpectationReport {
    pub fn is_clean(&self) -> bool {
        self.issues.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReferenceCatalogExpectationIssue {
    ProviderFieldMismatch {
        field: &'static str,
        expected: String,
        actual: Option<String>,
    },
    ProviderSourceUrlMismatch {
        expected_prefix: String,
        actual: Option<String>,
    },
    ModelCountTooSmall {
        expected_at_least: usize,
        actual: usize,
    },
    MissingRequiredApiSurface {
        api_surface: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReferenceCatalogValidationIssue {
    ProviderIdMismatch {
        expected: String,
        actual: String,
    },
    ModelMissingApiSurfaces {
        model_id: String,
    },
    ModelRecordSurfaceUndeclared {
        model_id: String,
        declared: BTreeSet<String>,
        recorded: BTreeSet<String>,
    },
    RecordMissingApiSurface {
        model_id: String,
        source_url: Option<String>,
    },
    RecordMissingEndpoint {
        model_id: String,
        api_surface: String,
    },
}

fn insert_optional_string(
    target: &mut JsonMap<String, JsonValue>,
    key: &str,
    value: &Option<String>,
) {
    if let Some(value) = value {
        target.insert(key.to_string(), JsonValue::String(value.clone()));
    }
}

fn insert_vendor_metadata(
    target: &mut JsonMap<String, JsonValue>,
    vendor_metadata: &BTreeMap<String, JsonValue>,
) {
    for (key, value) in vendor_metadata {
        target.insert(key.clone(), value.clone());
    }
}

fn canonical_value_key(value: &JsonValue) -> String {
    serde_json::to_string(value).expect("reference catalog values should serialize")
}

const OPENAI_REFERENCE_REQUIRED_SURFACES: &[&str] = &[
    "responses",
    "chat.completion",
    "completion.legacy",
    "embedding",
    "image.generation",
    "image.edit",
    "audio.speech",
    "audio.transcription",
    "moderation",
    "realtime.websocket",
];
const DEEPSEEK_REFERENCE_REQUIRED_SURFACES: &[&str] =
    &["chat.completion", "completion.fim.beta", "context.cache"];
const GOOGLE_REFERENCE_REQUIRED_SURFACES: &[&str] = &[
    "generate.content",
    "embedding",
    "image.generation",
    "video.generation",
    "realtime.websocket",
];
const ANTHROPIC_REFERENCE_REQUIRED_SURFACES: &[&str] = &["anthropic.messages"];

const CORE_PROVIDER_REFERENCE_CATALOG_EXPECTATIONS: &[ReferenceCatalogExpectation] = &[
    ReferenceCatalogExpectation {
        stem: "openai",
        role: ReferenceCatalogRole::CompleteProviderDirectory,
        provider_id: "openai",
        display_name: Some("OpenAI"),
        base_url: Some("https://api.openai.com/v1"),
        protocol: Some("openai"),
        auth_type: Some("api_key_env"),
        source_url_prefix: Some("https://developers.openai.com/"),
        min_model_count: 20,
        required_api_surfaces: OPENAI_REFERENCE_REQUIRED_SURFACES,
    },
    ReferenceCatalogExpectation {
        stem: "deepseek",
        role: ReferenceCatalogRole::CompleteProviderDirectory,
        provider_id: "deepseek",
        display_name: Some("DeepSeek API"),
        base_url: Some("https://api.deepseek.com"),
        protocol: Some("openai"),
        auth_type: Some("api_key_env"),
        source_url_prefix: Some("https://api-docs.deepseek.com/"),
        min_model_count: 2,
        required_api_surfaces: DEEPSEEK_REFERENCE_REQUIRED_SURFACES,
    },
    ReferenceCatalogExpectation {
        stem: "google",
        role: ReferenceCatalogRole::CompleteProviderDirectory,
        provider_id: "google",
        display_name: Some("Google AI for Developers"),
        base_url: Some("https://generativelanguage.googleapis.com/v1beta"),
        protocol: Some("gemini_generate_content"),
        auth_type: Some("query_param_env"),
        source_url_prefix: Some("https://ai.google.dev/"),
        min_model_count: 5,
        required_api_surfaces: GOOGLE_REFERENCE_REQUIRED_SURFACES,
    },
    ReferenceCatalogExpectation {
        stem: "anthropic",
        role: ReferenceCatalogRole::CompleteProviderDirectory,
        provider_id: "anthropic",
        display_name: Some("Anthropic"),
        base_url: Some("https://api.anthropic.com/v1"),
        protocol: Some("anthropic_messages"),
        auth_type: Some("api_key_env"),
        source_url_prefix: Some("https://docs.anthropic.com/"),
        min_model_count: 5,
        required_api_surfaces: ANTHROPIC_REFERENCE_REQUIRED_SURFACES,
    },
];

pub fn core_provider_reference_catalog_expectations() -> &'static [ReferenceCatalogExpectation] {
    CORE_PROVIDER_REFERENCE_CATALOG_EXPECTATIONS
}

fn is_global_reference_surface(surface: &str) -> bool {
    matches!(surface, "model.list")
}

fn operation_for_api_surface(surface: &str) -> Option<OperationKind> {
    Some(match surface {
        "responses" | "response.create.beta" => OperationKind::RESPONSE,
        "chat.completion"
        | "chat.completion.async"
        | "anthropic.messages"
        | "generate.content"
        | "generate.content.stream"
        | "generate.content.batch"
        | "minimax.chatcompletion_v2" => OperationKind::CHAT_COMPLETION,
        "group.chat.completion" => OperationKind::GROUP_CHAT_COMPLETION,
        "thread.run" => OperationKind::THREAD_RUN,
        "chat.translation" => OperationKind::CHAT_TRANSLATION,
        "completion.legacy" | "completion.fim.beta" => OperationKind::TEXT_COMPLETION,
        "embedding" => OperationKind::EMBEDDING,
        "embedding.multimodal" => OperationKind::MULTIMODAL_EMBEDDING,
        "image.generation" | "image.generation.async" => OperationKind::IMAGE_GENERATION,
        "image.edit" => OperationKind::IMAGE_EDIT,
        "image.translation" => OperationKind::IMAGE_TRANSLATION,
        "image.question" => OperationKind::IMAGE_QUESTION,
        "video.generation" | "video.generation.async" => OperationKind::VIDEO_GENERATION,
        "audio.speech" | "audio.speech.async" => OperationKind::AUDIO_SPEECH,
        "audio.transcription" | "audio.transcription.realtime" => {
            OperationKind::AUDIO_TRANSCRIPTION
        }
        "audio.translation" => OperationKind::AUDIO_TRANSLATION,
        "audio.voice_clone" | "audio.voice_cloning" => OperationKind::AUDIO_VOICE_CLONE,
        "audio.voice_design" => OperationKind::AUDIO_VOICE_DESIGN,
        "realtime.websocket" => OperationKind::REALTIME_SESSION,
        "rerank" => OperationKind::RERANK,
        "classification_or_extraction" => OperationKind::CLASSIFICATION_OR_EXTRACTION,
        "moderation" => OperationKind::MODERATION,
        "batch" => OperationKind::BATCH,
        "ocr" => OperationKind::OCR,
        "model.list" => OperationKind::MODEL_LIST,
        "context.cache" => OperationKind::CONTEXT_CACHE,
        "music.generation" => OperationKind::MUSIC_GENERATION,
        "3d.generation" => OperationKind::THREE_D_GENERATION,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn provider_models_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("catalog")
            .join("provider_models")
    }

    fn pair_paths(stem: &str) -> (PathBuf, PathBuf) {
        let dir = provider_models_dir();
        (
            dir.join(format!("{stem}.json")),
            dir.join(format!("{stem}.toml")),
        )
    }

    #[test]
    fn provider_model_reference_capability_profiles_are_stable() {
        let dir = provider_models_dir();
        for entry in fs::read_dir(&dir).expect("provider_models dir should exist") {
            let entry = entry.expect("dir entry should be readable");
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }

            let catalog = ReferenceProviderModelCatalog::from_path(&path)
                .unwrap_or_else(|error| panic!("failed to load {}: {error}", path.display()));
            let provider_profile = catalog.capability_profile();
            assert!(
                provider_profile.unmapped_api_surfaces.is_empty(),
                "provider capability profile has unmapped api surfaces for {}: {:?}",
                path.display(),
                provider_profile.unmapped_api_surfaces
            );
            assert!(
                !provider_profile.capabilities.is_empty(),
                "provider capability profile is empty for {}",
                path.display()
            );

            for (model_id, model) in &catalog.models {
                let profile = model.capability_profile();
                assert_eq!(profile.api_surfaces, model.api_surface_set());
                assert!(
                    profile.unmapped_api_surfaces.is_empty(),
                    "model capability profile has unmapped api surfaces for {} / {}: {:?}",
                    path.display(),
                    model_id,
                    profile.unmapped_api_surfaces
                );
                assert!(
                    !profile.capabilities.is_empty(),
                    "model capability profile is empty for {} / {}",
                    path.display(),
                    model_id
                );
                for capability in profile.capabilities.iter() {
                    assert!(
                        provider_profile.capabilities.contains(capability),
                        "provider capability profile missing model capability {} for {} / {}",
                        capability,
                        path.display(),
                        model_id
                    );
                }
            }
        }
    }

    #[test]
    fn core_provider_reference_catalog_expectations_hold() {
        for expectation in core_provider_reference_catalog_expectations() {
            let (json_path, toml_path) = pair_paths(expectation.stem);
            for path in [&json_path, &toml_path] {
                let catalog = ReferenceProviderModelCatalog::from_path(path)
                    .unwrap_or_else(|error| panic!("failed to load {}: {error}", path.display()));
                let report = catalog.validate_expectation(*expectation);
                assert!(
                    report.is_clean(),
                    "reference expectation validation failed for {}: {:?}",
                    path.display(),
                    report.issues
                );
            }
        }
    }

    #[test]
    fn provider_model_reference_pairs_are_canonical_equivalent() {
        let dir = provider_models_dir();
        let mut stems = BTreeSet::new();

        for entry in fs::read_dir(&dir).expect("provider_models dir should exist") {
            let entry = entry.expect("dir entry should be readable");
            let path = entry.path();
            let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
                continue;
            };
            stems.insert(stem.to_string());
        }

        for stem in stems {
            let (json_path, toml_path) = pair_paths(&stem);
            if !json_path.exists() || !toml_path.exists() {
                continue;
            }

            let json_catalog = ReferenceProviderModelCatalog::from_path(&json_path)
                .unwrap_or_else(|error| panic!("failed to load {}: {error}", json_path.display()));
            let toml_catalog = ReferenceProviderModelCatalog::from_path(&toml_path)
                .unwrap_or_else(|error| panic!("failed to load {}: {error}", toml_path.display()));

            assert_eq!(
                json_catalog.canonical_json_value(),
                toml_catalog.canonical_json_value(),
                "reference pair drift detected for provider {stem}"
            );

            let json_report = json_catalog.validate(Some(&stem));
            assert!(
                json_report.is_clean(),
                "json reference validation failed for provider {stem}: {:?}",
                json_report.issues
            );

            let toml_report = toml_catalog.validate(Some(&stem));
            assert!(
                toml_report.is_clean(),
                "toml reference validation failed for provider {stem}: {:?}",
                toml_report.issues
            );
        }
    }

    #[test]
    fn provider_model_reference_rejects_yaml_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("provider.yaml");
        fs::write(
            &path,
            r#"
provider:
  id: example
models: {}
"#,
        )
        .expect("write yaml");

        let err =
            ReferenceProviderModelCatalog::from_path(&path).expect_err("yaml should be rejected");
        assert!(err.to_string().contains("reference catalog load failed"));
        assert!(err.to_string().contains("expected json or toml"));
    }
}
