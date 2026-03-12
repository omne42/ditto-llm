use serde::Serialize;

use crate::catalog::{CatalogRegistry, ProviderPluginDescriptor};
use crate::contracts::{
    AuthMethodKind, CapabilityKind, HttpMethod, OperationKind, ProviderAuthHint, ProviderClass,
    ProviderProtocolFamily, VerificationStatus,
};

/// Machine-readable runtime_registry snapshot derived from the static catalog.
///
/// `catalog` remains the source of truth for provider/model metadata.
/// `runtime_registry` exposes that truth as a stable snapshot that AI callers
/// and higher layers can inspect without re-implementing provider assembly rules.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeRegistrySnapshot {
    providers: Vec<RuntimeRegistryProviderEntry>,
}

impl RuntimeRegistrySnapshot {
    pub fn from_registry(registry: CatalogRegistry) -> Self {
        let mut providers: Vec<_> = registry
            .plugins()
            .iter()
            .map(RuntimeRegistryProviderEntry::from_plugin)
            .collect();
        providers.sort_by(|left, right| left.provider.cmp(&right.provider));
        Self { providers }
    }

    pub fn providers(&self) -> &[RuntimeRegistryProviderEntry] {
        &self.providers
    }

    pub fn provider(&self, provider: &str) -> Option<&RuntimeRegistryProviderEntry> {
        self.providers.iter().find(|entry| {
            entry.provider == provider || entry.display_name.eq_ignore_ascii_case(provider.trim())
        })
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeRegistryProviderEntry {
    pub provider: &'static str,
    pub display_name: &'static str,
    pub class: &'static str,
    pub protocol_family: &'static str,
    pub default_base_url: Option<&'static str>,
    pub supported_auth: Vec<&'static str>,
    pub auth_hint: Option<RuntimeAuthHintEntry>,
    pub capabilities: Vec<&'static str>,
    pub capability_statuses: Vec<RuntimeCapabilityStatusEntry>,
    pub capability_bindings: Vec<RuntimeCapabilityBindingEntry>,
    pub models: Vec<RuntimeRegistryModelEntry>,
}

impl RuntimeRegistryProviderEntry {
    fn from_plugin(plugin: &ProviderPluginDescriptor) -> Self {
        let runtime_spec = plugin.runtime_spec();
        let mut capabilities: Vec<_> = runtime_spec
            .capabilities
            .iter()
            .map(CapabilityKind::as_str)
            .collect();
        capabilities.sort_unstable();

        let mut capability_statuses: Vec<_> = runtime_spec
            .capability_statuses
            .into_iter()
            .map(RuntimeCapabilityStatusEntry::from_descriptor)
            .collect();
        capability_statuses.sort_by(|left, right| left.capability.cmp(right.capability));

        let mut capability_bindings: Vec<_> = runtime_spec
            .capability_bindings
            .into_iter()
            .map(RuntimeCapabilityBindingEntry::from_binding)
            .collect();
        capability_bindings.sort_by(|left, right| left.capability.cmp(right.capability));

        let mut models: Vec<_> = plugin
            .models()
            .iter()
            .map(|model| RuntimeRegistryModelEntry::from_model(plugin, model))
            .collect();
        models.sort_by(|left, right| left.model.cmp(right.model));

        Self {
            provider: plugin.id,
            display_name: plugin.display_name,
            class: provider_class_name(plugin.class),
            protocol_family: provider_protocol_family_name(plugin.protocol_family()),
            default_base_url: plugin.default_base_url,
            supported_auth: plugin
                .supported_auth
                .iter()
                .copied()
                .map(auth_method_name)
                .collect(),
            auth_hint: plugin.auth_hint.map(RuntimeAuthHintEntry::from_hint),
            capabilities,
            capability_statuses,
            capability_bindings,
            models,
        }
    }

    pub fn provider(&self) -> &'static str {
        self.provider
    }

    pub fn model(&self, model: &str) -> Option<&RuntimeRegistryModelEntry> {
        self.models
            .iter()
            .find(|entry| entry.model == model || entry.aliases.iter().any(|alias| *alias == model))
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeRegistryModelEntry {
    pub provider: &'static str,
    pub model: &'static str,
    pub display_name: &'static str,
    pub aliases: &'static [&'static str],
    pub brand: Option<&'static str>,
    pub family: Option<&'static str>,
    pub summary: Option<&'static str>,
    pub supported_operations: Vec<&'static str>,
    pub capabilities: Vec<&'static str>,
    pub capability_statuses: Vec<RuntimeCapabilityStatusEntry>,
}

impl RuntimeRegistryModelEntry {
    pub fn provider(&self) -> &'static str {
        self.provider
    }

    pub fn model(&self) -> &'static str {
        self.model
    }

    fn from_model(
        plugin: &ProviderPluginDescriptor,
        model: &crate::catalog::ProviderModelDescriptor,
    ) -> Self {
        let descriptor = model.capability_descriptor(plugin.provider_id());
        let mut capabilities: Vec<_> = descriptor
            .capabilities
            .iter()
            .map(CapabilityKind::as_str)
            .collect();
        capabilities.sort_unstable();

        let mut capability_statuses: Vec<_> = descriptor
            .capability_statuses
            .into_iter()
            .map(RuntimeCapabilityStatusEntry::from_descriptor)
            .collect();
        capability_statuses.sort_by(|left, right| left.capability.cmp(right.capability));

        Self {
            provider: plugin.id,
            model: model.id,
            display_name: model.display_name,
            aliases: model.aliases,
            brand: model.brand,
            family: model.family,
            summary: model.summary,
            supported_operations: descriptor
                .operations
                .iter()
                .map(|operation| operation.as_str())
                .collect(),
            capabilities,
            capability_statuses,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeCapabilityStatusEntry {
    pub capability: &'static str,
    pub status: &'static str,
}

impl RuntimeCapabilityStatusEntry {
    fn from_descriptor(descriptor: crate::catalog::CapabilityStatusDescriptor) -> Self {
        Self {
            capability: descriptor.capability.as_str(),
            status: capability_implementation_status_name(descriptor.status),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeCapabilityBindingEntry {
    pub capability: &'static str,
    pub adapter_family: &'static str,
    pub operations: Vec<&'static str>,
    pub surfaces: Vec<&'static str>,
    pub wire_protocols: Vec<&'static str>,
}

impl RuntimeCapabilityBindingEntry {
    fn from_binding(binding: crate::catalog::ProviderCapabilityBinding) -> Self {
        Self {
            capability: binding.capability.as_str(),
            adapter_family: provider_protocol_family_name(binding.adapter_family),
            operations: binding
                .operations
                .into_iter()
                .map(OperationKind::as_str)
                .collect(),
            surfaces: binding
                .surfaces
                .into_iter()
                .map(|surface| surface.as_str())
                .collect(),
            wire_protocols: binding
                .wire_protocols
                .into_iter()
                .map(|protocol| protocol.as_str())
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeAuthHintEntry {
    pub method: &'static str,
    pub env_keys: &'static [&'static str],
    pub query_param: Option<&'static str>,
    pub header_name: Option<&'static str>,
    pub prefix: Option<&'static str>,
}

impl RuntimeAuthHintEntry {
    fn from_hint(hint: ProviderAuthHint) -> Self {
        Self {
            method: auth_method_name(hint.method),
            env_keys: hint.env_keys,
            query_param: hint.query_param,
            header_name: hint.header_name,
            prefix: hint.prefix,
        }
    }
}

pub fn builtin_runtime_registry() -> RuntimeRegistrySnapshot {
    RuntimeRegistrySnapshot::from_registry(crate::catalog::builtin_registry())
}

fn auth_method_name(method: AuthMethodKind) -> &'static str {
    match method {
        AuthMethodKind::ApiKeyHeader => "api_key_header",
        AuthMethodKind::ApiKeyQuery => "api_key_query",
        AuthMethodKind::CommandToken => "command_token",
        AuthMethodKind::StaticBearer => "static_bearer",
        AuthMethodKind::SigV4 => "sigv4",
        AuthMethodKind::OAuthClientCredentials => "oauth_client_credentials",
        AuthMethodKind::OAuthDeviceCode => "oauth_device_code",
        AuthMethodKind::OAuthBrowserPkce => "oauth_browser_pkce",
    }
}

fn provider_class_name(class: ProviderClass) -> &'static str {
    match class {
        ProviderClass::GenericOpenAi => "generic_openai",
        ProviderClass::NativeAnthropic => "native_anthropic",
        ProviderClass::NativeGoogle => "native_google",
        ProviderClass::OpenAiCompatible => "openai_compatible",
        ProviderClass::Custom => "custom",
    }
}

fn provider_protocol_family_name(family: ProviderProtocolFamily) -> &'static str {
    match family {
        ProviderProtocolFamily::OpenAi => "openai",
        ProviderProtocolFamily::Anthropic => "anthropic",
        ProviderProtocolFamily::Google => "google",
        ProviderProtocolFamily::Dashscope => "dashscope",
        ProviderProtocolFamily::Qianfan => "qianfan",
        ProviderProtocolFamily::Ark => "ark",
        ProviderProtocolFamily::Hunyuan => "hunyuan",
        ProviderProtocolFamily::Minimax => "minimax",
        ProviderProtocolFamily::Zhipu => "zhipu",
        ProviderProtocolFamily::Custom => "custom",
        ProviderProtocolFamily::Mixed => "mixed",
        ProviderProtocolFamily::Unknown => "unknown",
    }
}

fn capability_implementation_status_name(
    status: crate::catalog::CapabilityImplementationStatus,
) -> &'static str {
    match status {
        crate::catalog::CapabilityImplementationStatus::Implemented => "implemented",
        crate::catalog::CapabilityImplementationStatus::Planned => "planned",
        crate::catalog::CapabilityImplementationStatus::Blocked => "blocked",
    }
}

#[allow(dead_code)]
fn verification_status_name(status: VerificationStatus) -> &'static str {
    match status {
        VerificationStatus::Explicit => "explicit",
        VerificationStatus::FamilyInferred => "family_inferred",
        VerificationStatus::DocsOnly => "docs_only",
    }
}

#[allow(dead_code)]
fn http_method_name(method: HttpMethod) -> &'static str {
    match method {
        HttpMethod::Get => "GET",
        HttpMethod::Post => "POST",
        HttpMethod::Put => "PUT",
        HttpMethod::Delete => "DELETE",
    }
}
