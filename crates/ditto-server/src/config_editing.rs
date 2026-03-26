//! Side-effecting config editing workflows.
//!
//! This module owns file mutation and interactive prompting.
//! Network model discovery is resolved by outer tools before calling into this
//! module so config edits stay deterministic. Pure config
//! schema/defaults/resolution stay in `crate::config`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use text_assets_kit::DataRootScope;
use toml_edit::{Array, DocumentMut, Item, Table, Value as TomlValue, value};

use crate::data_root::{bootstrap_server_data_root_with_options, data_root_options};
use ditto_core::config::{
    ProviderApi, ProviderCapabilities, ProviderConfig, normalize_string_list,
};
use ditto_core::contracts::{AuthMethodKind, CapabilityKind};
use ditto_core::error::{DittoError, Result};
use ditto_core::runtime_registry::{
    BuiltinProviderPreset, ResolvedProviderConfigSemantics, builtin_runtime_registry_catalog,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConfigScope {
    #[default]
    Auto,
    Workspace,
    Global,
}

impl ConfigScope {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Workspace => "workspace",
            Self::Global => "global",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderNamespace {
    Openai,
    Google,
    Gemini,
    Claude,
    Anthropic,
}

impl ProviderNamespace {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Openai => "openai",
            Self::Google => "google",
            Self::Gemini => "gemini",
            Self::Claude => "claude",
            Self::Anthropic => "anthropic",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAuthType {
    ApiKeyEnv,
    QueryParamEnv,
    HttpHeaderEnv,
    Command,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderUpsertRequest {
    pub name: String,
    pub config_path: Option<PathBuf>,
    pub root: Option<PathBuf>,
    pub scope: ConfigScope,
    pub namespace: ProviderNamespace,
    pub provider: Option<String>,
    pub enabled_capabilities: Vec<String>,
    pub base_url: Option<String>,
    pub default_model: Option<String>,
    pub upstream_api: Option<ProviderApi>,
    pub normalize_to: Option<ProviderApi>,
    pub normalize_endpoint: Option<String>,
    pub auth_type: ProviderAuthType,
    pub auth_keys: Vec<String>,
    pub auth_param: Option<String>,
    pub auth_header: Option<String>,
    pub auth_prefix: Option<String>,
    pub auth_command: Vec<String>,
    pub set_default: bool,
    pub set_default_model: bool,
    pub tools: Option<bool>,
    pub vision: Option<bool>,
    pub reasoning: Option<bool>,
    pub json_schema: Option<bool>,
    pub streaming: Option<bool>,
    pub prompt_cache: Option<bool>,
    pub discover_models: bool,
    pub discovery_api_key: Option<String>,
    pub model_whitelist: Vec<String>,
    pub register_models: bool,
    pub model_limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelUpsertRequest {
    pub name: String,
    pub config_path: Option<PathBuf>,
    pub root: Option<PathBuf>,
    pub scope: ConfigScope,
    pub provider: Option<String>,
    pub fallback_providers: Vec<String>,
    pub set_default: bool,
    pub thinking: Option<String>,
    pub context_window: Option<u64>,
    pub auto_compact_token_limit: Option<u64>,
    pub prompt_cache: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderUpsertReport {
    pub scope: ConfigScope,
    pub config_path: PathBuf,
    pub provider_ref: String,
    pub discovered_models: usize,
    pub updated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelUpsertReport {
    pub scope: ConfigScope,
    pub config_path: PathBuf,
    pub model: String,
    pub updated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderListRequest {
    pub config_path: Option<PathBuf>,
    pub root: Option<PathBuf>,
    pub scope: ConfigScope,
    pub namespace: Option<ProviderNamespace>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderSummary {
    pub provider_ref: String,
    pub namespace: String,
    pub name: String,
    pub provider: Option<String>,
    pub enabled_capabilities: Vec<String>,
    pub base_url: Option<String>,
    pub upstream_api: Option<String>,
    pub default_model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderListReport {
    pub scope: ConfigScope,
    pub config_path: PathBuf,
    pub providers: Vec<ProviderSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderShowRequest {
    pub name: String,
    pub config_path: Option<PathBuf>,
    pub root: Option<PathBuf>,
    pub scope: ConfigScope,
    pub namespace: ProviderNamespace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderShowReport {
    pub scope: ConfigScope,
    pub config_path: PathBuf,
    pub provider_ref: String,
    pub exists: bool,
    pub provider: Option<JsonValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderDeleteRequest {
    pub name: String,
    pub config_path: Option<PathBuf>,
    pub root: Option<PathBuf>,
    pub scope: ConfigScope,
    pub namespace: ProviderNamespace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderDeleteReport {
    pub scope: ConfigScope,
    pub config_path: PathBuf,
    pub provider_ref: String,
    pub deleted: bool,
    pub updated: bool,
    pub cleared_references: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelListRequest {
    pub config_path: Option<PathBuf>,
    pub root: Option<PathBuf>,
    pub scope: ConfigScope,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelSummary {
    pub name: String,
    pub thinking: Option<String>,
    pub context_window: Option<u64>,
    pub auto_compact_token_limit: Option<u64>,
    pub prompt_cache: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelListReport {
    pub scope: ConfigScope,
    pub config_path: PathBuf,
    pub models: Vec<ModelSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelShowRequest {
    pub name: String,
    pub config_path: Option<PathBuf>,
    pub root: Option<PathBuf>,
    pub scope: ConfigScope,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelShowReport {
    pub scope: ConfigScope,
    pub config_path: PathBuf,
    pub model: String,
    pub exists: bool,
    pub is_default: bool,
    pub config: Option<JsonValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDeleteRequest {
    pub name: String,
    pub config_path: Option<PathBuf>,
    pub root: Option<PathBuf>,
    pub scope: ConfigScope,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDeleteReport {
    pub scope: ConfigScope,
    pub config_path: PathBuf,
    pub model: String,
    pub deleted: bool,
    pub updated: bool,
    pub cleared_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConfigTarget {
    config_path: PathBuf,
    scope: ConfigScope,
}

pub async fn upsert_provider_config(
    mut req: ProviderUpsertRequest,
) -> Result<ProviderUpsertReport> {
    let provider_name = non_empty_trimmed(&req.name).ok_or_else(|| {
        ditto_core::config_error!(
            "error_detail.config.field_must_not_be_empty",
            "field" => "provider name"
        )
    })?;

    let target =
        resolve_config_target(req.config_path.clone(), req.root.clone(), req.scope).await?;
    let mut doc = load_or_create_document(&target.config_path).await?;
    ensure_project_config_enabled_if_missing(&mut doc);

    let namespace = req.namespace.as_str();
    let canonical_provider_ref = format!("{namespace}.providers.{provider_name}");
    let fallback_api = default_api_for_namespace(req.namespace);
    let resolved_upstream = req.upstream_api.unwrap_or(fallback_api);
    apply_provider_registry_defaults(&mut req, resolved_upstream);
    apply_builtin_provider_defaults(&mut req, resolved_upstream);
    let provider_semantics = resolve_provider_request_semantics(&req, resolved_upstream)?;

    {
        let provider_table =
            ensure_table_path(&mut doc, &[namespace, "providers", provider_name.as_str()]);

        let base_url = clean_option_string(req.base_url.clone())
            .or_else(|| existing_string_value(provider_table.get("base_url")))
            .or_else(|| infer_default_base_url(&req));

        if let Some(base_url) = base_url {
            provider_table["base_url"] = value(base_url);
        }

        provider_table["provider"] = value(provider_semantics.provider);
        let enabled_capabilities = provider_semantics
            .enabled_capabilities
            .iter()
            .map(|capability| capability.as_str().to_string())
            .collect::<Vec<_>>();
        provider_table["enabled_capabilities"] = string_array_item(enabled_capabilities.iter());

        if let Some(default_model) = clean_option_string(req.default_model.clone()) {
            provider_table["default_model"] = value(default_model);
        }

        provider_table["upstream_api"] = value(provider_api_to_toml(resolved_upstream));
        if let Some(normalize_to) = req.normalize_to {
            provider_table["normalize_to"] = value(provider_api_to_toml(normalize_to));
        }
        if let Some(normalize_endpoint) = clean_option_string(req.normalize_endpoint.clone()) {
            provider_table["normalize_endpoint"] = value(normalize_endpoint);
        }

        apply_provider_auth_table(provider_table, &req)?;
        sync_provider_capabilities_table(
            provider_table,
            &req,
            provider_semantics
                .enabled_capabilities
                .contains(&CapabilityKind::LLM),
        );
    }

    if req.set_default {
        let openai_table = ensure_table_path(&mut doc, &["openai"]);
        openai_table["provider"] = value(canonical_provider_ref.clone());
    }
    if req.set_default_model {
        if let Some(default_model) = clean_option_string(req.default_model.clone()) {
            let openai_table = ensure_table_path(&mut doc, &["openai"]);
            openai_table["model"] = value(default_model);
        }
    }

    let mut discovered_models = normalize_string_list(req.model_whitelist.clone());
    if let Some(limit) = req.model_limit {
        discovered_models.truncate(limit);
    }
    // Model discovery is resolved outside `config_editing`; this layer only
    // applies the caller-provided whitelist as a deterministic document edit.
    if req.discover_models && discovered_models.is_empty() {
        return Err(ditto_core::config_error!(
            "error_detail.config.discover_models_whitelist_required"
        ));
    }
    if !discovered_models.is_empty() {
        let provider_table =
            ensure_table_path(&mut doc, &[namespace, "providers", provider_name.as_str()]);
        provider_table["model_whitelist"] = string_array_item(discovered_models.iter());
        if req.register_models {
            let models_table = ensure_table_path(&mut doc, &["openai", "models"]);
            for model in &discovered_models {
                if !models_table.contains_key(model) {
                    models_table.insert(model, Item::Table(Table::new()));
                }
            }
        }
    }

    let updated = write_document_if_changed(&target.config_path, &doc).await?;
    let discovered_model_count = if req.discover_models {
        discovered_models.len()
    } else {
        0
    };

    Ok(ProviderUpsertReport {
        scope: target.scope,
        config_path: target.config_path,
        provider_ref: canonical_provider_ref,
        discovered_models: discovered_model_count,
        updated,
    })
}

pub async fn upsert_model_config(req: ModelUpsertRequest) -> Result<ModelUpsertReport> {
    let model_name = non_empty_trimmed(&req.name).ok_or_else(|| {
        ditto_core::config_error!(
            "error_detail.config.field_must_not_be_empty",
            "field" => "model name"
        )
    })?;

    let target =
        resolve_config_target(req.config_path.clone(), req.root.clone(), req.scope).await?;
    let mut doc = load_or_create_document(&target.config_path).await?;
    ensure_project_config_enabled_if_missing(&mut doc);

    {
        let model_table = ensure_table_path(&mut doc, &["openai", "models", model_name.as_str()]);

        if let Some(thinking) = clean_option_string(req.thinking.clone()) {
            model_table["thinking"] = value(thinking);
        }
        if let Some(context_window) = req.context_window {
            model_table["context_window"] = value(context_window as i64);
        }
        if let Some(auto_compact_token_limit) = req.auto_compact_token_limit {
            model_table["auto_compact_token_limit"] = value(auto_compact_token_limit as i64);
        }
        if let Some(prompt_cache) = req.prompt_cache {
            model_table["prompt_cache"] = value(prompt_cache);
        }
    }

    let openai_table = ensure_table_path(&mut doc, &["openai"]);
    if let Some(provider) = clean_option_string(req.provider.clone()) {
        openai_table["provider"] = value(provider);
    }
    let fallback_providers = clean_string_list(req.fallback_providers.clone());
    if !fallback_providers.is_empty() {
        openai_table["fallback_providers"] = string_array_item(fallback_providers.iter());
    }
    if req.set_default {
        openai_table["model"] = value(model_name.clone());
    }

    let updated = write_document_if_changed(&target.config_path, &doc).await?;

    Ok(ModelUpsertReport {
        scope: target.scope,
        config_path: target.config_path,
        model: model_name,
        updated,
    })
}

pub async fn list_provider_configs(req: ProviderListRequest) -> Result<ProviderListReport> {
    let target =
        resolve_config_target_for_read(req.config_path.clone(), req.root.clone(), req.scope)
            .await?;
    let doc = load_or_create_document_readonly(&target.config_path).await?;

    let namespaces = match req.namespace {
        Some(namespace) => vec![namespace],
        None => vec![
            ProviderNamespace::Openai,
            ProviderNamespace::Google,
            ProviderNamespace::Gemini,
            ProviderNamespace::Claude,
            ProviderNamespace::Anthropic,
        ],
    };

    let mut providers = Vec::<ProviderSummary>::new();
    for namespace in namespaces {
        let namespace_key = namespace.as_str();
        let Some(provider_table) = get_table_path(doc.as_table(), &[namespace_key, "providers"])
        else {
            continue;
        };
        for (name, item) in provider_table.iter() {
            let Some(table) = item.as_table() else {
                continue;
            };
            let provider_ref = format!("{namespace_key}.providers.{name}");
            providers.push(ProviderSummary {
                provider_ref,
                namespace: namespace_key.to_string(),
                name: name.to_string(),
                provider: existing_string_value(table.get("provider")),
                enabled_capabilities: existing_string_list(table.get("enabled_capabilities")),
                base_url: existing_string_value(table.get("base_url")),
                upstream_api: existing_string_value(table.get("upstream_api")),
                default_model: existing_string_value(table.get("default_model")),
            });
        }
    }
    providers.sort_by(|a, b| a.provider_ref.cmp(&b.provider_ref));

    Ok(ProviderListReport {
        scope: target.scope,
        config_path: target.config_path,
        providers,
    })
}

pub async fn show_provider_config(req: ProviderShowRequest) -> Result<ProviderShowReport> {
    let name = non_empty_trimmed(&req.name).ok_or_else(|| {
        ditto_core::config_error!(
            "error_detail.config.field_must_not_be_empty",
            "field" => "provider name"
        )
    })?;

    let target =
        resolve_config_target_for_read(req.config_path.clone(), req.root.clone(), req.scope)
            .await?;
    let doc = load_or_create_document_readonly(&target.config_path).await?;

    let namespace = req.namespace.as_str();
    let provider_ref = format!("{namespace}.providers.{name}");
    let provider = get_item_path(doc.as_table(), &[namespace, "providers", name.as_str()])
        .map(item_to_json)
        .transpose()?;

    Ok(ProviderShowReport {
        scope: target.scope,
        config_path: target.config_path,
        provider_ref,
        exists: provider.is_some(),
        provider,
    })
}

pub async fn delete_provider_config(req: ProviderDeleteRequest) -> Result<ProviderDeleteReport> {
    let provider_name = non_empty_trimmed(&req.name).ok_or_else(|| {
        ditto_core::config_error!(
            "error_detail.config.field_must_not_be_empty",
            "field" => "provider name"
        )
    })?;

    let target =
        resolve_config_target(req.config_path.clone(), req.root.clone(), req.scope).await?;
    let mut doc = load_or_create_document(&target.config_path).await?;

    let namespace = req.namespace.as_str();
    let provider_ref = format!("{namespace}.providers.{provider_name}");
    let mut deleted = false;
    let mut cleared_references = 0usize;

    if let Some(providers_table) = get_table_path_mut(doc.as_table_mut(), &[namespace, "providers"])
    {
        deleted = providers_table.remove(provider_name.as_str()).is_some();
    }

    if deleted {
        if let Some(openai_table) = get_table_path_mut(doc.as_table_mut(), &["openai"]) {
            let is_current_default = existing_string_value(openai_table.get("provider"))
                .is_some_and(|value| value == provider_ref);
            if is_current_default {
                openai_table.remove("provider");
                cleared_references = cleared_references.saturating_add(1);
            }
            if let Some(item) = openai_table.get_mut("fallback_providers") {
                cleared_references = cleared_references
                    .saturating_add(remove_string_from_array_item(item, provider_ref.as_str()));
            }
        }

        if let Some(models_table) = get_table_path_mut(doc.as_table_mut(), &["openai", "models"]) {
            for (_model_name, item) in models_table.iter_mut() {
                let Some(model_table) = item.as_table_mut() else {
                    continue;
                };
                let model_provider = existing_string_value(model_table.get("provider"));
                if model_provider.is_some_and(|value| value == provider_ref) {
                    model_table.remove("provider");
                    cleared_references = cleared_references.saturating_add(1);
                }
                if let Some(item) = model_table.get_mut("fallback_providers") {
                    cleared_references = cleared_references
                        .saturating_add(remove_string_from_array_item(item, provider_ref.as_str()));
                }
            }
        }
    }

    let updated = if deleted {
        write_document_if_changed(&target.config_path, &doc).await?
    } else {
        false
    };

    Ok(ProviderDeleteReport {
        scope: target.scope,
        config_path: target.config_path,
        provider_ref,
        deleted,
        updated,
        cleared_references,
    })
}

pub async fn list_model_configs(req: ModelListRequest) -> Result<ModelListReport> {
    let target =
        resolve_config_target_for_read(req.config_path.clone(), req.root.clone(), req.scope)
            .await?;
    let doc = load_or_create_document_readonly(&target.config_path).await?;

    let mut models = Vec::<ModelSummary>::new();
    if let Some(models_table) = get_table_path(doc.as_table(), &["openai", "models"]) {
        for (name, item) in models_table.iter() {
            let Some(table) = item.as_table() else {
                continue;
            };
            models.push(ModelSummary {
                name: name.to_string(),
                thinking: existing_string_value(table.get("thinking")),
                context_window: existing_integer_value_u64(table.get("context_window")),
                auto_compact_token_limit: existing_integer_value_u64(
                    table.get("auto_compact_token_limit"),
                ),
                prompt_cache: existing_bool_value(table.get("prompt_cache")),
            });
        }
    }
    models.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(ModelListReport {
        scope: target.scope,
        config_path: target.config_path,
        models,
    })
}

pub async fn show_model_config(req: ModelShowRequest) -> Result<ModelShowReport> {
    let model_name = non_empty_trimmed(&req.name).ok_or_else(|| {
        ditto_core::config_error!(
            "error_detail.config.field_must_not_be_empty",
            "field" => "model name"
        )
    })?;

    let target =
        resolve_config_target_for_read(req.config_path.clone(), req.root.clone(), req.scope)
            .await?;
    let doc = load_or_create_document_readonly(&target.config_path).await?;

    let is_default = get_item_path(doc.as_table(), &["openai", "model"])
        .and_then(item_as_string)
        .is_some_and(|value| value == model_name);
    let config = get_item_path(doc.as_table(), &["openai", "models", model_name.as_str()])
        .map(item_to_json)
        .transpose()?;

    Ok(ModelShowReport {
        scope: target.scope,
        config_path: target.config_path,
        model: model_name,
        exists: config.is_some(),
        is_default,
        config,
    })
}

pub async fn delete_model_config(req: ModelDeleteRequest) -> Result<ModelDeleteReport> {
    let model_name = non_empty_trimmed(&req.name).ok_or_else(|| {
        ditto_core::config_error!(
            "error_detail.config.field_must_not_be_empty",
            "field" => "model name"
        )
    })?;

    let target =
        resolve_config_target(req.config_path.clone(), req.root.clone(), req.scope).await?;
    let mut doc = load_or_create_document(&target.config_path).await?;

    let mut deleted = false;
    if let Some(models_table) = get_table_path_mut(doc.as_table_mut(), &["openai", "models"]) {
        deleted = models_table.remove(model_name.as_str()).is_some();
    }

    let mut cleared_default = false;
    if deleted {
        if let Some(openai_table) = get_table_path_mut(doc.as_table_mut(), &["openai"]) {
            let is_default = existing_string_value(openai_table.get("model"))
                .is_some_and(|value| value == model_name);
            if is_default {
                openai_table.remove("model");
                cleared_default = true;
            }
        }
    }

    let updated = if deleted {
        write_document_if_changed(&target.config_path, &doc).await?
    } else {
        false
    };

    Ok(ModelDeleteReport {
        scope: target.scope,
        config_path: target.config_path,
        model: model_name,
        deleted,
        updated,
        cleared_default,
    })
}

#[cfg(feature = "config-interactive")]
pub fn complete_provider_upsert_request_interactive(
    mut req: ProviderUpsertRequest,
) -> Result<ProviderUpsertRequest> {
    ensure_interactive_stdio()?;

    eprintln!("interactive provider add: press Enter to keep current value");

    req.name = prompt_required_string("provider name", non_empty_trimmed(&req.name))?;
    req.namespace = prompt_provider_namespace(req.namespace)?;

    let default_upstream = req
        .upstream_api
        .unwrap_or_else(|| default_api_for_namespace(req.namespace));
    req.upstream_api = Some(prompt_provider_api("upstream_api", default_upstream)?);
    let resolved_upstream = req.upstream_api.unwrap_or(default_upstream);

    apply_provider_registry_defaults(&mut req, resolved_upstream);
    apply_builtin_provider_defaults(&mut req, resolved_upstream);
    req.provider = prompt_optional_string("provider", clean_option_string(req.provider.clone()))?;
    apply_provider_registry_defaults(&mut req, resolved_upstream);
    apply_builtin_provider_defaults(&mut req, resolved_upstream);
    req.enabled_capabilities = prompt_csv_list("enabled_capabilities", req.enabled_capabilities)?;

    let inferred_base = infer_default_base_url(&req);
    req.base_url = prompt_optional_string(
        "base_url",
        clean_option_string(req.base_url.clone()).or(inferred_base),
    )?;
    let default_model_label =
        provider_default_model_prompt_label_for_request(&req, resolved_upstream);
    req.default_model = prompt_optional_string(
        default_model_label.as_str(),
        clean_option_string(req.default_model.clone()),
    )?;
    req.normalize_to = prompt_optional_provider_api("normalize_to", req.normalize_to)?;
    req.normalize_endpoint = prompt_optional_string(
        "normalize_endpoint",
        clean_option_string(req.normalize_endpoint.clone()),
    )?;

    req.auth_type = prompt_provider_auth_type(req.auth_type)?;
    match req.auth_type {
        ProviderAuthType::ApiKeyEnv => {
            req.auth_keys = prompt_csv_list("auth_keys (env names)", req.auth_keys)?;
            req.auth_param = None;
            req.auth_header = None;
            req.auth_prefix = None;
            req.auth_command = Vec::new();
        }
        ProviderAuthType::QueryParamEnv => {
            req.auth_keys = prompt_csv_list("auth_keys (env names)", req.auth_keys)?;
            req.auth_param = prompt_optional_string("auth_param", req.auth_param)?;
            req.auth_prefix = prompt_optional_string("auth_prefix", req.auth_prefix)?;
            req.auth_header = None;
            req.auth_command = Vec::new();
        }
        ProviderAuthType::HttpHeaderEnv => {
            req.auth_keys = prompt_csv_list("auth_keys (env names)", req.auth_keys)?;
            req.auth_header = prompt_optional_string("auth_header", req.auth_header)?;
            req.auth_prefix = prompt_optional_string("auth_prefix", req.auth_prefix)?;
            req.auth_param = None;
            req.auth_command = Vec::new();
        }
        ProviderAuthType::Command => {
            req.auth_command = prompt_csv_list("auth_command (argv csv)", req.auth_command)?;
            req.auth_keys = Vec::new();
            req.auth_param = None;
            req.auth_header = None;
            req.auth_prefix = None;
        }
    }

    req.set_default = prompt_bool("set_default", req.set_default)?;
    req.set_default_model = prompt_bool("set_default_model", req.set_default_model)?;
    req.discover_models = prompt_bool("discover_models", req.discover_models)?;
    if req.discover_models {
        req.discovery_api_key =
            prompt_optional_string("api_key_for_discovery (optional)", req.discovery_api_key)?;
        req.register_models = prompt_bool("register_models", req.register_models)?;
        req.model_limit = prompt_optional_u64("model_limit", req.model_limit.map(|v| v as u64))?
            .map(|v| v as usize);
    }

    Ok(req)
}

#[cfg(feature = "config-interactive")]
pub fn complete_model_upsert_request_interactive(
    mut req: ModelUpsertRequest,
) -> Result<ModelUpsertRequest> {
    ensure_interactive_stdio()?;

    eprintln!("interactive model add: press Enter to keep current value");

    req.name = prompt_required_string("model name", non_empty_trimmed(&req.name))?;
    let provider_label = model_provider_prompt_label(req.name.as_str(), "provider");
    req.provider = prompt_optional_string(provider_label.as_str(), req.provider)?;
    let fallback_label = model_provider_prompt_label(req.name.as_str(), "fallback_providers (csv)");
    req.fallback_providers = prompt_csv_list(fallback_label.as_str(), req.fallback_providers)?;
    req.set_default = prompt_bool("set_default", req.set_default)?;
    req.thinking = prompt_optional_string("thinking", req.thinking)?;
    req.context_window = prompt_optional_u64("context_window", req.context_window)?;
    req.auto_compact_token_limit =
        prompt_optional_u64("auto_compact_token_limit", req.auto_compact_token_limit)?;
    req.prompt_cache = prompt_optional_bool("prompt_cache", req.prompt_cache)?;

    Ok(req)
}

fn default_api_for_namespace(namespace: ProviderNamespace) -> ProviderApi {
    match namespace {
        ProviderNamespace::Openai => ProviderApi::OpenaiChatCompletions,
        ProviderNamespace::Google | ProviderNamespace::Gemini => ProviderApi::GeminiGenerateContent,
        ProviderNamespace::Claude | ProviderNamespace::Anthropic => ProviderApi::AnthropicMessages,
    }
}

fn provider_api_to_toml(api: ProviderApi) -> &'static str {
    match api {
        ProviderApi::OpenaiChatCompletions => "openai_chat_completions",
        ProviderApi::OpenaiResponses => "openai_responses",
        ProviderApi::GeminiGenerateContent => "gemini_generate_content",
        ProviderApi::AnthropicMessages => "anthropic_messages",
    }
}

#[cfg(feature = "config-interactive")]
fn provider_auth_type_to_toml(auth_type: ProviderAuthType) -> &'static str {
    match auth_type {
        ProviderAuthType::ApiKeyEnv => "api_key_env",
        ProviderAuthType::QueryParamEnv => "query_param_env",
        ProviderAuthType::HttpHeaderEnv => "http_header_env",
        ProviderAuthType::Command => "command",
    }
}

#[cfg(feature = "config-interactive")]
fn parse_provider_namespace_token(raw: &str) -> Option<ProviderNamespace> {
    match raw.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "openai" => Some(ProviderNamespace::Openai),
        "google" => Some(ProviderNamespace::Google),
        "gemini" => Some(ProviderNamespace::Gemini),
        "claude" => Some(ProviderNamespace::Claude),
        "anthropic" => Some(ProviderNamespace::Anthropic),
        _ => None,
    }
}

#[cfg(feature = "config-interactive")]
fn parse_provider_api_token(raw: &str) -> Option<ProviderApi> {
    match raw.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "openai_chat_completions" | "chat_completions" => Some(ProviderApi::OpenaiChatCompletions),
        "openai_responses" | "responses" => Some(ProviderApi::OpenaiResponses),
        "gemini_generate_content" | "generate_content" | "generatecontent" => {
            Some(ProviderApi::GeminiGenerateContent)
        }
        "anthropic_messages" | "messages" => Some(ProviderApi::AnthropicMessages),
        _ => None,
    }
}

#[cfg(feature = "config-interactive")]
fn parse_provider_auth_type_token(raw: &str) -> Option<ProviderAuthType> {
    match raw.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "api_key_env" => Some(ProviderAuthType::ApiKeyEnv),
        "query_param_env" => Some(ProviderAuthType::QueryParamEnv),
        "http_header_env" => Some(ProviderAuthType::HttpHeaderEnv),
        "command" => Some(ProviderAuthType::Command),
        _ => None,
    }
}

#[cfg(feature = "config-interactive")]
fn parse_bool_token(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "t" | "yes" | "y" | "on" => Some(true),
        "0" | "false" | "f" | "no" | "n" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(feature = "config-interactive")]
fn ensure_interactive_stdio() -> Result<()> {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err(ditto_core::config_error!(
            "error_detail.config.interactive_tty_required"
        ));
    }
    Ok(())
}

#[cfg(feature = "config-interactive")]
fn prompt_line(label: &str) -> Result<String> {
    use std::io::Write;
    print!("{label}");
    std::io::stdout().flush().map_err(DittoError::Io)?;
    let mut buf = String::new();
    std::io::stdin()
        .read_line(&mut buf)
        .map_err(DittoError::Io)?;
    Ok(buf.trim().to_string())
}

#[cfg(feature = "config-interactive")]
fn prompt_required_string(label: &str, current: Option<String>) -> Result<String> {
    loop {
        let suffix = current
            .as_deref()
            .map(|v| format!(" [{v}]"))
            .unwrap_or_default();
        let input = prompt_line(format!("{label}{suffix}: ").as_str())?;
        if input.is_empty() {
            if let Some(current) = current.as_deref() {
                return Ok(current.to_string());
            }
            eprintln!("value is required");
            continue;
        }
        return Ok(input);
    }
}

#[cfg(feature = "config-interactive")]
fn prompt_optional_string(label: &str, current: Option<String>) -> Result<Option<String>> {
    let suffix = current
        .as_deref()
        .map(|v| format!(" [{v}]"))
        .unwrap_or_default();
    let input = prompt_line(format!("{label}{suffix} (Enter keep, '-' clear): ").as_str())?;
    if input.is_empty() {
        return Ok(current);
    }
    if input == "-" {
        return Ok(None);
    }
    Ok(Some(input))
}

#[cfg(feature = "config-interactive")]
fn prompt_bool(label: &str, current: bool) -> Result<bool> {
    loop {
        let input = prompt_line(format!("{label} [{current}] (true/false): ").as_str())?;
        if input.is_empty() {
            return Ok(current);
        }
        if let Some(parsed) = parse_bool_token(&input) {
            return Ok(parsed);
        }
        eprintln!("invalid bool; use true/false");
    }
}

#[cfg(feature = "config-interactive")]
fn prompt_optional_bool(label: &str, current: Option<bool>) -> Result<Option<bool>> {
    loop {
        let suffix = current
            .map(|v| v.to_string())
            .map(|v| format!(" [{v}]"))
            .unwrap_or_default();
        let input =
            prompt_line(format!("{label}{suffix} (true/false, Enter keep, '-' clear): ").as_str())?;
        if input.is_empty() {
            return Ok(current);
        }
        if input == "-" {
            return Ok(None);
        }
        if let Some(parsed) = parse_bool_token(&input) {
            return Ok(Some(parsed));
        }
        eprintln!("invalid bool; use true/false");
    }
}

#[cfg(feature = "config-interactive")]
fn prompt_optional_u64(label: &str, current: Option<u64>) -> Result<Option<u64>> {
    loop {
        let suffix = current.map(|v| format!(" [{v}]")).unwrap_or_default();
        let input =
            prompt_line(format!("{label}{suffix} (number, Enter keep, '-' clear): ").as_str())?;
        if input.is_empty() {
            return Ok(current);
        }
        if input == "-" {
            return Ok(None);
        }
        match input.parse::<u64>() {
            Ok(value) => return Ok(Some(value)),
            Err(_) => eprintln!("invalid number"),
        }
    }
}

#[cfg(feature = "config-interactive")]
fn prompt_provider_namespace(current: ProviderNamespace) -> Result<ProviderNamespace> {
    loop {
        let input = prompt_line(
            format!(
                "namespace [{}] (openai/google/gemini/claude/anthropic): ",
                current.as_str()
            )
            .as_str(),
        )?;
        if input.is_empty() {
            return Ok(current);
        }
        if let Some(parsed) = parse_provider_namespace_token(&input) {
            return Ok(parsed);
        }
        eprintln!("invalid namespace");
    }
}

#[cfg(feature = "config-interactive")]
fn prompt_provider_api(label: &str, current: ProviderApi) -> Result<ProviderApi> {
    loop {
        let input = prompt_line(
            format!(
                "{label} [{}] (openai_chat_completions/openai_responses/gemini_generate_content/anthropic_messages): ",
                provider_api_to_toml(current)
            )
            .as_str(),
        )?;
        if input.is_empty() {
            return Ok(current);
        }
        if let Some(parsed) = parse_provider_api_token(&input) {
            return Ok(parsed);
        }
        eprintln!("invalid provider api");
    }
}

#[cfg(feature = "config-interactive")]
fn prompt_optional_provider_api(
    label: &str,
    current: Option<ProviderApi>,
) -> Result<Option<ProviderApi>> {
    loop {
        let suffix = current
            .map(provider_api_to_toml)
            .map(|v| format!(" [{v}]"))
            .unwrap_or_default();
        let input = prompt_line(
            format!(
                "{label}{suffix} (Enter keep, '-' clear, openai_chat_completions/openai_responses/gemini_generate_content/anthropic_messages): "
            )
            .as_str(),
        )?;
        if input.is_empty() {
            return Ok(current);
        }
        if input == "-" {
            return Ok(None);
        }
        if let Some(parsed) = parse_provider_api_token(&input) {
            return Ok(Some(parsed));
        }
        eprintln!("invalid provider api");
    }
}

#[cfg(feature = "config-interactive")]
fn prompt_provider_auth_type(current: ProviderAuthType) -> Result<ProviderAuthType> {
    loop {
        let input = prompt_line(
            format!(
                "auth_type [{}] (api_key_env/query_param_env/http_header_env/command): ",
                provider_auth_type_to_toml(current)
            )
            .as_str(),
        )?;
        if input.is_empty() {
            return Ok(current);
        }
        if let Some(parsed) = parse_provider_auth_type_token(&input) {
            return Ok(parsed);
        }
        eprintln!("invalid auth_type");
    }
}

#[cfg(feature = "config-interactive")]
fn prompt_csv_list(label: &str, current: Vec<String>) -> Result<Vec<String>> {
    let default = if current.is_empty() {
        String::new()
    } else {
        current.join(",")
    };
    let suffix = if default.is_empty() {
        String::new()
    } else {
        format!(" [{default}]")
    };
    let input = prompt_line(
        format!("{label}{suffix} (comma separated, Enter keep, '-' clear): ").as_str(),
    )?;
    if input.is_empty() {
        return Ok(current);
    }
    if input == "-" {
        return Ok(Vec::new());
    }
    Ok(input
        .split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .collect())
}

fn apply_builtin_provider_defaults(
    req: &mut ProviderUpsertRequest,
    resolved_upstream: ProviderApi,
) {
    let auth_is_unset = req.auth_keys.is_empty()
        && clean_option_string(req.auth_param.clone()).is_none()
        && clean_option_string(req.auth_header.clone()).is_none()
        && clean_option_string(req.auth_prefix.clone()).is_none()
        && req.auth_command.is_empty();

    if clean_option_string(req.base_url.clone()).is_none() {
        let registry = builtin_runtime_registry_catalog();
        if let Some(base_url) = registry
            .provider_preset(req.name.as_str())
            .and_then(|preset| preset.default_base_url)
        {
            req.base_url = Some(base_url.to_string());
        }
    }

    let Some(preset) = resolved_provider_editor_preset(req, resolved_upstream) else {
        return;
    };

    if let Some(auth_hint) = preset.auth_hint {
        if auth_is_unset {
            req.auth_type = provider_auth_type_from_hint(
                auth_hint.method,
                auth_hint.header_name,
                auth_hint.prefix,
            );
        }
        if req.auth_keys.is_empty() {
            req.auth_keys = auth_hint
                .env_keys
                .iter()
                .map(|key| (*key).to_string())
                .collect();
        }
        if req.auth_param.is_none() {
            req.auth_param = auth_hint.query_param.map(|value| value.to_string());
        }
        if req.auth_header.is_none() {
            req.auth_header = auth_hint.header_name.map(|value| value.to_string());
        }
        if req.auth_prefix.is_none() {
            req.auth_prefix = auth_hint.prefix.map(|value| value.to_string());
        }
    }
}

fn provider_auth_type_from_hint(
    method: AuthMethodKind,
    header_name: Option<&'static str>,
    prefix: Option<&'static str>,
) -> ProviderAuthType {
    match method {
        AuthMethodKind::ApiKeyQuery => ProviderAuthType::QueryParamEnv,
        AuthMethodKind::ApiKeyHeader => {
            let uses_default_bearer = header_name
                .map(|value| value.eq_ignore_ascii_case("authorization"))
                .unwrap_or(true)
                && matches!(prefix, None | Some("Bearer "));
            if uses_default_bearer {
                ProviderAuthType::ApiKeyEnv
            } else {
                ProviderAuthType::HttpHeaderEnv
            }
        }
        AuthMethodKind::CommandToken => ProviderAuthType::Command,
        _ => ProviderAuthType::ApiKeyEnv,
    }
}

#[cfg(feature = "config-interactive")]
fn provider_default_model_prompt_label(provider_name: &str) -> String {
    let registry = builtin_runtime_registry_catalog();
    let examples: Vec<&str> = registry
        .models_for_provider(provider_name)
        .into_iter()
        .take(3)
        .map(|candidate| candidate.model)
        .collect();
    let summary = registry.provider_capability_summary(provider_name);
    let capability_hint = summary
        .as_ref()
        .map(|summary| {
            let mut capabilities = summary
                .capabilities
                .iter()
                .map(|capability| capability.to_string())
                .collect::<Vec<_>>();
            capabilities.sort();
            capabilities.truncate(3);
            if capabilities.is_empty() {
                String::new()
            } else {
                format!(" [caps: {}]", capabilities.join(", "))
            }
        })
        .unwrap_or_default();
    if examples.is_empty() {
        if summary.as_ref().is_some_and(|summary| {
            summary.model_count == 0 && summary.capabilities.contains(&CapabilityKind::LLM)
        }) {
            return format!(
                "default_model (required; registry has no builtin model list){capability_hint}"
            );
        }
        return format!("default_model{capability_hint}");
    }
    format!(
        "default_model (e.g. {}){capability_hint}",
        examples.join(", ")
    )
}

#[cfg(feature = "config-interactive")]
fn provider_default_model_prompt_label_for_request(
    req: &ProviderUpsertRequest,
    resolved_upstream: ProviderApi,
) -> String {
    let provider_name = resolved_provider_editor_hint(req, resolved_upstream)
        .unwrap_or_else(|| req.name.trim().to_string());
    provider_default_model_prompt_label(provider_name.as_str())
}

#[cfg(feature = "config-interactive")]
fn model_provider_prompt_label(model_name: &str, base: &str) -> String {
    let registry = builtin_runtime_registry_catalog();
    let mut providers = BTreeSet::new();
    for candidate in registry
        .provider_candidates_for_model(model_name)
        .into_iter()
        .take(6)
    {
        providers.insert(candidate.provider);
    }
    if providers.is_empty() {
        return base.to_string();
    }
    format!(
        "{base} [suggest: {}]",
        providers.into_iter().collect::<Vec<_>>().join(", ")
    )
}

fn apply_provider_registry_defaults(
    req: &mut ProviderUpsertRequest,
    resolved_upstream: ProviderApi,
) {
    if clean_option_string(req.provider.clone()).is_none() {
        req.provider = infer_default_runtime_provider(req.name.as_str(), resolved_upstream);
    }
    if req.enabled_capabilities.is_empty() {
        req.enabled_capabilities = infer_default_enabled_capability_names(resolved_upstream);
    }
}

fn resolved_provider_editor_hint(
    req: &ProviderUpsertRequest,
    resolved_upstream: ProviderApi,
) -> Option<String> {
    clean_option_string(req.provider.clone()).or_else(|| {
        let provider_name = non_empty_trimmed(&req.name)?;
        builtin_runtime_registry_catalog()
            .provider_preset(&provider_name)
            .map(|preset| preset.provider.to_string())
            .or_else(|| infer_default_runtime_provider(&provider_name, resolved_upstream))
    })
}

fn resolved_provider_editor_preset(
    req: &ProviderUpsertRequest,
    resolved_upstream: ProviderApi,
) -> Option<BuiltinProviderPreset> {
    let provider_name = resolved_provider_editor_hint(req, resolved_upstream)?;
    builtin_runtime_registry_catalog().provider_preset(provider_name.as_str())
}

fn infer_default_runtime_provider(
    provider_name: &str,
    resolved_upstream: ProviderApi,
) -> Option<String> {
    if let Some(preset) = builtin_runtime_registry_catalog().provider_preset(provider_name) {
        return Some(preset.provider.to_string());
    }

    let fallback = match resolved_upstream {
        ProviderApi::OpenaiChatCompletions => "openai-compatible",
        ProviderApi::OpenaiResponses => "openai",
        ProviderApi::GeminiGenerateContent => "google",
        ProviderApi::AnthropicMessages => "anthropic",
    };
    Some(fallback.to_string())
}

fn infer_default_enabled_capability_names(resolved_upstream: ProviderApi) -> Vec<String> {
    let capability = match resolved_upstream {
        ProviderApi::OpenaiChatCompletions
        | ProviderApi::OpenaiResponses
        | ProviderApi::GeminiGenerateContent
        | ProviderApi::AnthropicMessages => CapabilityKind::LLM,
    };
    vec![capability.to_string()]
}

fn resolve_provider_request_semantics(
    req: &ProviderUpsertRequest,
    resolved_upstream: ProviderApi,
) -> Result<ResolvedProviderConfigSemantics> {
    let provider_config = ProviderConfig {
        provider: clean_option_string(req.provider.clone()),
        enabled_capabilities: clean_string_list(req.enabled_capabilities.clone()),
        default_model: clean_option_string(req.default_model.clone()),
        capabilities: provider_capabilities_from_request(req),
        upstream_api: Some(resolved_upstream),
        ..ProviderConfig::default()
    };
    builtin_runtime_registry_catalog()
        .resolve_provider_config_semantics(req.name.as_str(), &provider_config)
}

fn provider_capabilities_from_request(req: &ProviderUpsertRequest) -> Option<ProviderCapabilities> {
    let has_llm_capability_flags = req.tools.is_some()
        || req.vision.is_some()
        || req.reasoning.is_some()
        || req.json_schema.is_some()
        || req.streaming.is_some()
        || req.prompt_cache.is_some();
    if !has_llm_capability_flags {
        return None;
    }
    Some(ProviderCapabilities {
        tools: req.tools.unwrap_or(false),
        vision: req.vision.unwrap_or(false),
        reasoning: req.reasoning.unwrap_or(false),
        json_schema: req.json_schema.unwrap_or(false),
        streaming: req.streaming.unwrap_or(false),
        prompt_cache: req.prompt_cache.unwrap_or(true),
    })
}

fn infer_default_base_url(req: &ProviderUpsertRequest) -> Option<String> {
    let provider_name = non_empty_trimmed(&req.name)?;
    builtin_runtime_registry_catalog()
        .provider_preset(&provider_name)
        .and_then(|preset| preset.default_base_url.map(|value| value.to_string()))
}

fn apply_provider_auth_table(
    provider_table: &mut Table,
    req: &ProviderUpsertRequest,
) -> Result<()> {
    let auth_table = ensure_table_in_table(provider_table, "auth");
    auth_table.clear();

    match req.auth_type {
        ProviderAuthType::ApiKeyEnv => {
            auth_table["type"] = value("api_key_env");
            let keys = clean_string_list(req.auth_keys.clone());
            if !keys.is_empty() {
                auth_table["keys"] = string_array_item(keys.iter());
            }
        }
        ProviderAuthType::QueryParamEnv => {
            auth_table["type"] = value("query_param_env");
            let param =
                clean_option_string(req.auth_param.clone()).unwrap_or_else(|| "key".to_string());
            auth_table["param"] = value(param);
            let keys = clean_string_list(req.auth_keys.clone());
            if !keys.is_empty() {
                auth_table["keys"] = string_array_item(keys.iter());
            }
            if let Some(prefix) = normalize_auth_prefix(req.auth_prefix.clone()) {
                auth_table["prefix"] = value(prefix);
            }
        }
        ProviderAuthType::HttpHeaderEnv => {
            auth_table["type"] = value("http_header_env");
            let header = clean_option_string(req.auth_header.clone())
                .unwrap_or_else(|| "Authorization".to_string());
            auth_table["header"] = value(header);
            let keys = clean_string_list(req.auth_keys.clone());
            if !keys.is_empty() {
                auth_table["keys"] = string_array_item(keys.iter());
            }
            if let Some(prefix) = normalize_auth_prefix(req.auth_prefix.clone()) {
                auth_table["prefix"] = value(prefix);
            }
        }
        ProviderAuthType::Command => {
            let command = clean_string_list(req.auth_command.clone());
            if command.is_empty() {
                return Err(ditto_core::config_error!(
                    "error_detail.config.auth_command_required"
                ));
            }
            auth_table["type"] = value("command");
            auth_table["command"] = string_array_item(command.iter());
        }
    }

    Ok(())
}

fn sync_provider_capabilities_table(
    provider_table: &mut Table,
    req: &ProviderUpsertRequest,
    llm_enabled: bool,
) {
    if !llm_enabled {
        provider_table.remove("capabilities");
        return;
    }

    let has_capability_update = req.tools.is_some()
        || req.vision.is_some()
        || req.reasoning.is_some()
        || req.json_schema.is_some()
        || req.streaming.is_some()
        || req.prompt_cache.is_some();

    if !has_capability_update {
        return;
    }

    let capabilities = ensure_table_in_table(provider_table, "capabilities");
    if let Some(v) = req.tools {
        capabilities["tools"] = value(v);
    }
    if let Some(v) = req.vision {
        capabilities["vision"] = value(v);
    }
    if let Some(v) = req.reasoning {
        capabilities["reasoning"] = value(v);
    }
    if let Some(v) = req.json_schema {
        capabilities["json_schema"] = value(v);
    }
    if let Some(v) = req.streaming {
        capabilities["streaming"] = value(v);
    }
    if let Some(v) = req.prompt_cache {
        capabilities["prompt_cache"] = value(v);
    }
}

async fn resolve_config_target_for_read(
    config_path: Option<PathBuf>,
    root: Option<PathBuf>,
    scope: ConfigScope,
) -> Result<ConfigTarget> {
    if let Some(path) = config_path {
        return Ok(ConfigTarget {
            config_path: path,
            scope,
        });
    }

    let root =
        bootstrap_server_data_root_with_options(&data_root_options(root, data_root_scope(scope)))?
            .data_root;

    let local_path = root.join("config_local.toml");
    let shared_path = root.join("config.toml");

    let config_path = match scope {
        ConfigScope::Workspace => {
            if try_exists(&local_path).await {
                local_path
            } else if try_exists(&shared_path).await {
                shared_path
            } else {
                local_path
            }
        }
        ConfigScope::Global => {
            if try_exists(&shared_path).await {
                shared_path
            } else if try_exists(&local_path).await {
                local_path
            } else {
                shared_path
            }
        }
        ConfigScope::Auto => {
            if try_exists(&local_path).await {
                local_path
            } else if try_exists(&shared_path).await {
                shared_path
            } else {
                local_path
            }
        }
    };

    Ok(ConfigTarget { config_path, scope })
}

async fn resolve_config_target(
    config_path: Option<PathBuf>,
    root: Option<PathBuf>,
    scope: ConfigScope,
) -> Result<ConfigTarget> {
    if let Some(path) = config_path {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(DittoError::Io)?;
        }
        return Ok(ConfigTarget {
            config_path: path,
            scope,
        });
    }

    let root =
        bootstrap_server_data_root_with_options(&data_root_options(root, data_root_scope(scope)))?
            .data_root;

    let local_path = root.join("config_local.toml");
    let shared_path = root.join("config.toml");

    let config_path = match scope {
        ConfigScope::Workspace => {
            if try_exists(&local_path).await {
                local_path
            } else if try_exists(&shared_path).await {
                shared_path
            } else {
                local_path
            }
        }
        ConfigScope::Global => {
            if try_exists(&shared_path).await {
                shared_path
            } else if try_exists(&local_path).await {
                local_path
            } else {
                shared_path
            }
        }
        ConfigScope::Auto => {
            if try_exists(&local_path).await {
                local_path
            } else if try_exists(&shared_path).await {
                shared_path
            } else {
                local_path
            }
        }
    };

    Ok(ConfigTarget { config_path, scope })
}

async fn try_exists(path: &Path) -> bool {
    tokio::fs::try_exists(path).await.unwrap_or(false)
}

fn data_root_scope(scope: ConfigScope) -> DataRootScope {
    match scope {
        ConfigScope::Auto => DataRootScope::Auto,
        ConfigScope::Workspace => DataRootScope::Workspace,
        ConfigScope::Global => DataRootScope::Global,
    }
}

async fn load_or_create_document(path: &PathBuf) -> Result<DocumentMut> {
    if try_exists(path.as_path()).await {
        let raw = tokio::fs::read_to_string(path)
            .await
            .map_err(DittoError::Io)?;
        return raw.parse::<DocumentMut>().map_err(|err| {
            ditto_core::config_error!(
                "error_detail.config.parse_toml",
                "path" => path.display().to_string(),
                "error" => err.to_string()
            )
        });
    }

    Ok(DocumentMut::new())
}

async fn load_or_create_document_readonly(path: &PathBuf) -> Result<DocumentMut> {
    if try_exists(path.as_path()).await {
        let raw = tokio::fs::read_to_string(path)
            .await
            .map_err(DittoError::Io)?;
        return raw.parse::<DocumentMut>().map_err(|err| {
            ditto_core::config_error!(
                "error_detail.config.parse_toml",
                "path" => path.display().to_string(),
                "error" => err.to_string()
            )
        });
    }
    Ok(DocumentMut::new())
}

fn ensure_project_config_enabled_if_missing(doc: &mut DocumentMut) {
    let project_config = ensure_table_path(doc, &["project_config"]);
    if !project_config.contains_key("enabled") {
        project_config["enabled"] = value(true);
    }
}

async fn write_document_if_changed(path: &PathBuf, doc: &DocumentMut) -> Result<bool> {
    let mut out = doc.to_string();
    if !out.ends_with('\n') {
        out.push('\n');
    }

    let old = tokio::fs::read_to_string(path).await.ok();
    if old.as_deref() == Some(out.as_str()) {
        return Ok(false);
    }

    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(DittoError::Io)?;
    }
    tokio::fs::write(path, out).await.map_err(DittoError::Io)?;
    Ok(true)
}

fn ensure_table_path<'a>(doc: &'a mut DocumentMut, path: &[&str]) -> &'a mut Table {
    let mut table = doc.as_table_mut();
    for key in path {
        if !table.contains_key(key) {
            table.insert(key, Item::Table(Table::new()));
        }
        let item = table
            .get_mut(key)
            .expect("table key exists after insertion in ensure_table_path");
        if !item.is_table() {
            *item = Item::Table(Table::new());
        }
        table = item
            .as_table_mut()
            .expect("item turned into table in ensure_table_path");
    }
    table
}

fn ensure_table_in_table<'a>(table: &'a mut Table, key: &str) -> &'a mut Table {
    if !table.contains_key(key) {
        table.insert(key, Item::Table(Table::new()));
    }
    let item = table
        .get_mut(key)
        .expect("table key exists after insertion in ensure_table_in_table");
    if !item.is_table() {
        *item = Item::Table(Table::new());
    }
    item.as_table_mut()
        .expect("item turned into table in ensure_table_in_table")
}

fn get_table_path<'a>(table: &'a Table, path: &[&str]) -> Option<&'a Table> {
    let mut table = table;
    for key in path {
        let item = table.get(key)?;
        table = item.as_table()?;
    }
    Some(table)
}

fn get_table_path_mut<'a>(table: &'a mut Table, path: &[&str]) -> Option<&'a mut Table> {
    let mut table = table;
    for key in path {
        let item = table.get_mut(key)?;
        table = item.as_table_mut()?;
    }
    Some(table)
}

fn get_item_path<'a>(table: &'a Table, path: &[&str]) -> Option<&'a Item> {
    let (last, prefix) = path.split_last()?;
    let table = get_table_path(table, prefix)?;
    table.get(last)
}

fn item_to_json(item: &Item) -> Result<JsonValue> {
    if !item.is_table() {
        return Err(ditto_core::config_error!(
            "error_detail.config.item_to_json_requires_table"
        ));
    }
    let raw = item.to_string();
    let parsed = toml::from_str::<toml::Value>(&raw).map_err(|err| {
        ditto_core::config_error!(
            "error_detail.config.parse_table_item_for_json",
            "error" => err.to_string()
        )
    })?;
    serde_json::to_value(parsed).map_err(|err| {
        ditto_core::config_error!(
            "error_detail.config.serialize_table_item_to_json",
            "error" => err.to_string()
        )
    })
}

fn item_as_string(item: &Item) -> Option<String> {
    item.as_value()
        .and_then(TomlValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn existing_string_value(item: Option<&Item>) -> Option<String> {
    item.and_then(Item::as_value)
        .and_then(TomlValue::as_str)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn existing_string_list(item: Option<&Item>) -> Vec<String> {
    let Some(value) = item.and_then(Item::as_value) else {
        return Vec::new();
    };
    let Some(array) = value.as_array() else {
        return Vec::new();
    };
    array
        .iter()
        .filter_map(TomlValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn existing_integer_value_u64(item: Option<&Item>) -> Option<u64> {
    let value = item
        .and_then(Item::as_value)
        .and_then(TomlValue::as_integer)?;
    u64::try_from(value).ok()
}

fn existing_bool_value(item: Option<&Item>) -> Option<bool> {
    item.and_then(Item::as_value).and_then(TomlValue::as_bool)
}

fn clean_option_string(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim();
        if value.is_empty() {
            None
        } else {
            Some(value.to_string())
        }
    })
}

fn normalize_auth_prefix(value: Option<String>) -> Option<String> {
    let value = value?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.eq_ignore_ascii_case("bearer") {
        return Some("Bearer ".to_string());
    }
    Some(trimmed.to_string())
}

fn non_empty_trimmed(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn clean_string_list(values: Vec<String>) -> Vec<String> {
    let mut out = Vec::<String>::new();
    let mut seen = BTreeSet::<String>::new();
    for value in values {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        let value = value.to_string();
        if seen.insert(value.clone()) {
            out.push(value);
        }
    }
    out
}

fn remove_string_from_array_item(item: &mut Item, target: &str) -> usize {
    let Some(value) = item.as_value_mut() else {
        return 0;
    };
    let Some(array) = value.as_array_mut() else {
        return 0;
    };
    let mut removed = 0usize;
    array.retain(|entry| {
        let keep = entry.as_str().is_none_or(|value| value.trim() != target);
        if !keep {
            removed = removed.saturating_add(1);
        }
        keep
    });
    removed
}

fn string_array_item<'a>(values: impl Iterator<Item = &'a String>) -> Item {
    let mut out = Array::default();
    for value in values {
        out.push(value.as_str());
    }
    Item::Value(TomlValue::Array(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_auth_prefix_adds_bearer_space() {
        assert_eq!(
            normalize_auth_prefix(Some("Bearer".to_string())),
            Some("Bearer ".to_string())
        );
        assert_eq!(
            normalize_auth_prefix(Some(" bearer ".to_string())),
            Some("Bearer ".to_string())
        );
        assert_eq!(
            normalize_auth_prefix(Some("Token".to_string())),
            Some("Token".to_string())
        );
    }

    #[cfg(feature = "config-interactive")]
    #[test]
    fn parse_provider_api_token_accepts_aliases() {
        assert_eq!(
            parse_provider_api_token("chat_completions"),
            Some(ProviderApi::OpenaiChatCompletions)
        );
        assert_eq!(
            parse_provider_api_token("generate-content"),
            Some(ProviderApi::GeminiGenerateContent)
        );
        assert_eq!(
            parse_provider_api_token("messages"),
            Some(ProviderApi::AnthropicMessages)
        );
    }

    #[cfg(feature = "config-interactive")]
    #[test]
    fn parse_bool_token_accepts_common_values() {
        assert_eq!(parse_bool_token("yes"), Some(true));
        assert_eq!(parse_bool_token("0"), Some(false));
        assert_eq!(parse_bool_token("unknown"), None);
    }

    #[cfg(feature = "provider-google")]
    #[test]
    fn apply_builtin_provider_defaults_uses_google_catalog_hint() {
        let mut req = ProviderUpsertRequest {
            name: "google".to_string(),
            config_path: None,
            root: None,
            scope: ConfigScope::Workspace,
            namespace: ProviderNamespace::Google,
            provider: None,
            enabled_capabilities: Vec::new(),
            base_url: None,
            default_model: None,
            upstream_api: None,
            normalize_to: None,
            normalize_endpoint: None,
            auth_type: ProviderAuthType::ApiKeyEnv,
            auth_keys: Vec::new(),
            auth_param: None,
            auth_header: None,
            auth_prefix: None,
            auth_command: Vec::new(),
            set_default: false,
            set_default_model: false,
            tools: None,
            vision: None,
            reasoning: None,
            json_schema: None,
            streaming: None,
            prompt_cache: None,
            discover_models: false,
            discovery_api_key: None,
            model_whitelist: Vec::new(),
            register_models: false,
            model_limit: None,
        };

        apply_builtin_provider_defaults(&mut req, ProviderApi::GeminiGenerateContent);

        assert_eq!(
            req.base_url.as_deref(),
            Some("https://generativelanguage.googleapis.com/v1beta")
        );
        assert_eq!(req.auth_type, ProviderAuthType::HttpHeaderEnv);
        assert_eq!(req.auth_header.as_deref(), Some("x-goog-api-key"));
        assert_eq!(
            req.auth_keys,
            vec!["GOOGLE_API_KEY".to_string(), "GEMINI_API_KEY".to_string()]
        );
    }

    #[test]
    fn apply_builtin_provider_defaults_uses_registry_for_custom_openai_node() {
        let mut req = ProviderUpsertRequest {
            name: "my-proxy".to_string(),
            config_path: None,
            root: None,
            scope: ConfigScope::Workspace,
            namespace: ProviderNamespace::Openai,
            provider: None,
            enabled_capabilities: Vec::new(),
            base_url: None,
            default_model: None,
            upstream_api: Some(ProviderApi::OpenaiChatCompletions),
            normalize_to: None,
            normalize_endpoint: None,
            auth_type: ProviderAuthType::ApiKeyEnv,
            auth_keys: Vec::new(),
            auth_param: None,
            auth_header: None,
            auth_prefix: None,
            auth_command: Vec::new(),
            set_default: false,
            set_default_model: false,
            tools: None,
            vision: None,
            reasoning: None,
            json_schema: None,
            streaming: None,
            prompt_cache: None,
            discover_models: false,
            discovery_api_key: None,
            model_whitelist: Vec::new(),
            register_models: false,
            model_limit: None,
        };

        apply_provider_registry_defaults(&mut req, ProviderApi::OpenaiChatCompletions);
        apply_builtin_provider_defaults(&mut req, ProviderApi::OpenaiChatCompletions);

        assert_eq!(req.provider.as_deref(), Some("openai-compatible"));
        assert_eq!(req.base_url, None);
        assert_eq!(
            req.auth_keys,
            vec![
                "OPENAI_COMPAT_API_KEY".to_string(),
                "OPENAI_API_KEY".to_string(),
            ]
        );
    }

    fn unique_test_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock before epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "ditto-config-editor-test-{tag}-{}-{nanos}",
            std::process::id()
        ))
    }

    #[tokio::test]
    async fn provider_upsert_merges_without_overwriting_other_sections() -> Result<()> {
        let root = unique_test_dir("provider");
        tokio::fs::create_dir_all(&root)
            .await
            .map_err(DittoError::Io)?;
        let config_path = root.join("config_local.toml");
        tokio::fs::write(
            &config_path,
            "[project_config]\nenabled = true\n\n[ui]\nshow_thinking = true\n",
        )
        .await
        .map_err(DittoError::Io)?;

        let report = upsert_provider_config(ProviderUpsertRequest {
            name: "openrouter".to_string(),
            config_path: Some(config_path.clone()),
            root: None,
            scope: ConfigScope::Workspace,
            namespace: ProviderNamespace::Google,
            provider: None,
            enabled_capabilities: Vec::new(),
            base_url: Some("https://openrouter.ai/api/v1".to_string()),
            default_model: Some("google/gemini-3.1-pro-preview".to_string()),
            upstream_api: Some(ProviderApi::OpenaiChatCompletions),
            normalize_to: Some(ProviderApi::OpenaiChatCompletions),
            normalize_endpoint: Some("/v1/chat/completions".to_string()),
            auth_type: ProviderAuthType::ApiKeyEnv,
            auth_keys: vec!["OPENROUTER_API_KEY".to_string()],
            auth_param: None,
            auth_header: None,
            auth_prefix: None,
            auth_command: Vec::new(),
            set_default: false,
            set_default_model: false,
            tools: Some(true),
            vision: Some(true),
            reasoning: Some(false),
            json_schema: Some(false),
            streaming: Some(true),
            prompt_cache: Some(true),
            discover_models: false,
            discovery_api_key: None,
            model_whitelist: Vec::new(),
            register_models: false,
            model_limit: None,
        })
        .await?;

        assert!(report.updated);
        let parsed = toml::from_str::<toml::Value>(
            &tokio::fs::read_to_string(&config_path)
                .await
                .map_err(DittoError::Io)?,
        )
        .map_err(|err| {
            ditto_core::config_error!(
                "error_detail.config.parse_toml",
                "path" => "test toml",
                "error" => err.to_string()
            )
        })?;
        assert_eq!(
            parsed
                .get("ui")
                .and_then(|v| v.get("show_thinking"))
                .and_then(toml::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            parsed
                .get("google")
                .and_then(|v| v.get("providers"))
                .and_then(|v| v.get("openrouter"))
                .and_then(|v| v.get("base_url"))
                .and_then(toml::Value::as_str),
            Some("https://openrouter.ai/api/v1")
        );

        let _ = tokio::fs::remove_dir_all(&root).await;
        Ok(())
    }

    #[tokio::test]
    async fn model_upsert_keeps_existing_provider_pointer() -> Result<()> {
        let root = unique_test_dir("model");
        tokio::fs::create_dir_all(&root)
            .await
            .map_err(DittoError::Io)?;
        let config_path = root.join("config_local.toml");
        tokio::fs::write(
            &config_path,
            "[project_config]\nenabled = true\n\n[openai]\nprovider = \"google.providers.openrouter\"\n",
        )
        .await
        .map_err(DittoError::Io)?;

        let report = upsert_model_config(ModelUpsertRequest {
            name: "google/gemini-3.1-pro-preview".to_string(),
            config_path: Some(config_path.clone()),
            root: None,
            scope: ConfigScope::Workspace,
            provider: None,
            fallback_providers: Vec::new(),
            set_default: true,
            thinking: Some("high".to_string()),
            context_window: Some(1_000_000),
            auto_compact_token_limit: Some(900_000),
            prompt_cache: Some(true),
        })
        .await?;

        assert!(report.updated);
        let parsed = toml::from_str::<toml::Value>(
            &tokio::fs::read_to_string(&config_path)
                .await
                .map_err(DittoError::Io)?,
        )
        .map_err(|err| {
            ditto_core::config_error!(
                "error_detail.config.parse_toml",
                "path" => "test toml",
                "error" => err.to_string()
            )
        })?;
        assert_eq!(
            parsed
                .get("openai")
                .and_then(|v| v.get("provider"))
                .and_then(toml::Value::as_str),
            Some("google.providers.openrouter")
        );
        assert_eq!(
            parsed
                .get("openai")
                .and_then(|v| v.get("model"))
                .and_then(toml::Value::as_str),
            Some("google/gemini-3.1-pro-preview")
        );

        let _ = tokio::fs::remove_dir_all(&root).await;
        Ok(())
    }

    #[tokio::test]
    async fn provider_list_show_delete_flow() -> Result<()> {
        let root = unique_test_dir("provider-list-delete");
        tokio::fs::create_dir_all(&root)
            .await
            .map_err(DittoError::Io)?;
        let config_path = root.join("config_local.toml");
        tokio::fs::write(
            &config_path,
            r#"[project_config]
enabled = true

[openai]
provider = "google.providers.openrouter"
fallback_providers = ["google.providers.openrouter", "openai.providers.backup"]

[openai.models."gemini-3.1-pro"]
provider = "google.providers.openrouter"
fallback_providers = ["google.providers.openrouter", "openai.providers.backup"]

[google.providers.openrouter]
base_url = "https://openrouter.ai/api/v1"
upstream_api = "gemini_generate_content"

[google.providers.openrouter.auth]
type = "api_key_env"
keys = ["OPENROUTER_API_KEY"]
"#,
        )
        .await
        .map_err(DittoError::Io)?;

        let listed = list_provider_configs(ProviderListRequest {
            config_path: Some(config_path.clone()),
            root: None,
            scope: ConfigScope::Workspace,
            namespace: Some(ProviderNamespace::Google),
        })
        .await?;
        assert_eq!(listed.providers.len(), 1);
        assert_eq!(
            listed.providers[0].provider_ref,
            "google.providers.openrouter"
        );

        let shown = show_provider_config(ProviderShowRequest {
            name: "openrouter".to_string(),
            config_path: Some(config_path.clone()),
            root: None,
            scope: ConfigScope::Workspace,
            namespace: ProviderNamespace::Google,
        })
        .await?;
        assert!(shown.exists);

        let deleted = delete_provider_config(ProviderDeleteRequest {
            name: "openrouter".to_string(),
            config_path: Some(config_path.clone()),
            root: None,
            scope: ConfigScope::Workspace,
            namespace: ProviderNamespace::Google,
        })
        .await?;
        assert!(deleted.deleted);
        assert!(deleted.cleared_references >= 3);

        let parsed = toml::from_str::<toml::Value>(
            &tokio::fs::read_to_string(&config_path)
                .await
                .map_err(DittoError::Io)?,
        )
        .map_err(|err| {
            ditto_core::config_error!(
                "error_detail.config.parse_toml",
                "path" => "test toml",
                "error" => err.to_string()
            )
        })?;
        assert!(
            parsed
                .get("google")
                .and_then(|v| v.get("providers"))
                .and_then(|v| v.get("openrouter"))
                .is_none()
        );
        assert_ne!(
            parsed
                .get("openai")
                .and_then(|v| v.get("provider"))
                .and_then(toml::Value::as_str),
            Some("google.providers.openrouter")
        );

        let _ = tokio::fs::remove_dir_all(&root).await;
        Ok(())
    }

    #[tokio::test]
    async fn model_list_show_delete_flow() -> Result<()> {
        let root = unique_test_dir("model-list-delete");
        tokio::fs::create_dir_all(&root)
            .await
            .map_err(DittoError::Io)?;
        let config_path = root.join("config_local.toml");
        tokio::fs::write(
            &config_path,
            r#"[project_config]
enabled = true

[openai]
model = "gemini-3.1-pro"

[openai.models."gemini-3.1-pro"]
thinking = "high"
context_window = 1000000
auto_compact_token_limit = 900000
prompt_cache = true
"#,
        )
        .await
        .map_err(DittoError::Io)?;

        let listed = list_model_configs(ModelListRequest {
            config_path: Some(config_path.clone()),
            root: None,
            scope: ConfigScope::Workspace,
        })
        .await?;
        assert_eq!(listed.models.len(), 1);
        assert_eq!(listed.models[0].name, "gemini-3.1-pro");
        assert_eq!(listed.models[0].prompt_cache, Some(true));

        let shown = show_model_config(ModelShowRequest {
            name: "gemini-3.1-pro".to_string(),
            config_path: Some(config_path.clone()),
            root: None,
            scope: ConfigScope::Workspace,
        })
        .await?;
        assert!(shown.exists);
        assert!(shown.is_default);

        let deleted = delete_model_config(ModelDeleteRequest {
            name: "gemini-3.1-pro".to_string(),
            config_path: Some(config_path.clone()),
            root: None,
            scope: ConfigScope::Workspace,
        })
        .await?;
        assert!(deleted.deleted);
        assert!(deleted.cleared_default);

        let parsed = toml::from_str::<toml::Value>(
            &tokio::fs::read_to_string(&config_path)
                .await
                .map_err(DittoError::Io)?,
        )
        .map_err(|err| {
            ditto_core::config_error!(
                "error_detail.config.parse_toml",
                "path" => "test toml",
                "error" => err.to_string()
            )
        })?;
        assert!(
            parsed
                .get("openai")
                .and_then(|v| v.get("models"))
                .and_then(|v| v.get("gemini-3.1-pro"))
                .is_none()
        );
        assert!(parsed.get("openai").and_then(|v| v.get("model")).is_none());

        let _ = tokio::fs::remove_dir_all(&root).await;
        Ok(())
    }
}
