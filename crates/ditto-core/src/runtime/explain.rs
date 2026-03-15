use serde::Serialize;

use crate::catalog::CatalogRegistry;
use crate::contracts::{
    CapabilityKind, HttpMethod, InvocationHints, RuntimeProviderApi, RuntimeRoute,
    RuntimeRouteRequest, TransportKind, VerificationStatus,
};
use crate::error::Result;

use super::builtin::builtin_runtime_assembly;
use super::route::{
    RuntimeResolvedBaseUrlSource as InternalBaseUrlSource,
    RuntimeResolvedModelSource as InternalModelSource,
    RuntimeResolvedProviderSource as InternalProviderSource, resolve_runtime_route_plan,
};

/// Why the runtime ended up selecting a given provider plugin.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeProviderSelectionSource {
    RequestProvider,
    ConfiguredProvider,
    UpstreamApiFallback,
}

/// Where the final model value came from.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeModelSelectionSource {
    RequestModel,
    ProviderDefaultModel,
}

/// Where the final base URL came from.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeBaseUrlSelectionSource {
    EndpointOverride,
    ProviderConfig,
    PluginDefault,
    InferredFromUpstreamApi,
}

/// Minimal machine-readable explanation for a runtime route decision.
///
/// This keeps route selection explainable for AI clients without leaking
/// provider-specific implementation details into higher layers.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeRouteExplain {
    pub provider_hint: String,
    pub resolved_provider: &'static str,
    pub provider_source: RuntimeProviderSelectionSource,
    pub requested_model: Option<String>,
    pub resolved_model: String,
    pub model_source: RuntimeModelSelectionSource,
    pub operation: &'static str,
    pub required_capability: Option<&'static str>,
    pub upstream_api: Option<&'static str>,
    pub normalized_hints: RuntimeInvocationHintsEntry,
    pub capability_resolution: RuntimeCapabilityResolutionExplain,
    pub base_url_source: RuntimeBaseUrlSelectionSource,
    pub route: RuntimeResolvedRouteEntry,
}

impl RuntimeRouteExplain {
    pub fn route(&self) -> &RuntimeResolvedRouteEntry {
        &self.route
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeInvocationHintsEntry {
    pub streaming: Option<bool>,
    pub async_job: Option<bool>,
    pub preferred_surface: Option<&'static str>,
    pub preferred_wire_protocol: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeCapabilityResolutionExplain {
    pub provider: &'static str,
    pub requested_model: Option<String>,
    pub resolved_model: Option<RuntimeResolvedModelCapabilityExplain>,
    pub provider_capabilities: Vec<&'static str>,
    pub model_capabilities: Vec<&'static str>,
    pub effective_capabilities: Vec<&'static str>,
    pub provider_only_capabilities: Vec<&'static str>,
    pub model_only_capabilities: Vec<&'static str>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeResolvedModelCapabilityExplain {
    pub model: &'static str,
    pub display_name: &'static str,
    pub capabilities: Vec<&'static str>,
    pub supported_operations: Vec<&'static str>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeResolvedRouteEntry {
    pub provider: &'static str,
    pub surface: &'static str,
    pub wire_protocol: &'static str,
    pub transport: &'static str,
    pub http_method: Option<&'static str>,
    pub path: String,
    pub base_url: String,
    pub url: String,
    pub query_params: Vec<(String, String)>,
    pub streaming: Option<bool>,
    pub async_job: Option<bool>,
    pub verification: &'static str,
}

impl From<InternalProviderSource> for RuntimeProviderSelectionSource {
    fn from(value: InternalProviderSource) -> Self {
        match value {
            InternalProviderSource::RequestProvider => Self::RequestProvider,
            InternalProviderSource::ConfiguredProvider => Self::ConfiguredProvider,
            InternalProviderSource::UpstreamApiFallback => Self::UpstreamApiFallback,
        }
    }
}

impl From<InternalModelSource> for RuntimeModelSelectionSource {
    fn from(value: InternalModelSource) -> Self {
        match value {
            InternalModelSource::RequestModel => Self::RequestModel,
            InternalModelSource::ProviderDefaultModel => Self::ProviderDefaultModel,
        }
    }
}

impl From<InternalBaseUrlSource> for RuntimeBaseUrlSelectionSource {
    fn from(value: InternalBaseUrlSource) -> Self {
        match value {
            InternalBaseUrlSource::EndpointOverride => Self::EndpointOverride,
            InternalBaseUrlSource::ProviderConfig => Self::ProviderConfig,
            InternalBaseUrlSource::PluginDefault => Self::PluginDefault,
            InternalBaseUrlSource::InferredFromUpstreamApi => Self::InferredFromUpstreamApi,
        }
    }
}

pub(crate) fn explain_runtime_route(
    registry: CatalogRegistry,
    request: RuntimeRouteRequest<'_>,
) -> Result<RuntimeRouteExplain> {
    let plan = resolve_runtime_route_plan(&registry, request)?;

    Ok(RuntimeRouteExplain {
        provider_hint: plan.provider_hint,
        resolved_provider: plan.resolved_provider,
        provider_source: plan.provider_source.into(),
        requested_model: plan.requested_model,
        resolved_model: plan.resolved_model,
        model_source: plan.model_source.into(),
        operation: plan.operation.as_str(),
        required_capability: plan.required_capability.map(CapabilityKind::as_str),
        upstream_api: plan.upstream_api.map(runtime_provider_api_name),
        normalized_hints: RuntimeInvocationHintsEntry::from_hints(plan.normalized_hints),
        capability_resolution: RuntimeCapabilityResolutionExplain::from_resolution(
            plan.capability_resolution,
        ),
        base_url_source: plan.base_url_source.into(),
        route: RuntimeResolvedRouteEntry::from_route(plan.route),
    })
}

pub fn explain_builtin_runtime_route(
    request: RuntimeRouteRequest<'_>,
) -> Result<RuntimeRouteExplain> {
    let runtime = builtin_runtime_assembly();
    explain_runtime_route(runtime.catalog(), request)
}

impl RuntimeInvocationHintsEntry {
    fn from_hints(hints: InvocationHints) -> Self {
        Self {
            streaming: hints.streaming,
            async_job: hints.async_job,
            preferred_surface: hints.preferred_surface.map(|surface| surface.as_str()),
            preferred_wire_protocol: hints
                .preferred_wire_protocol
                .map(|protocol| protocol.as_str()),
        }
    }
}

impl RuntimeCapabilityResolutionExplain {
    fn from_resolution(resolution: crate::catalog::ProviderCapabilityResolution) -> Self {
        let resolved_model = resolution
            .resolved_model
            .map(RuntimeResolvedModelCapabilityExplain::from_descriptor);

        Self {
            provider: resolution.provider.as_str(),
            requested_model: resolution.requested_model,
            resolved_model,
            provider_capabilities: sorted_capabilities(&resolution.provider_capabilities),
            model_capabilities: sorted_capabilities(&resolution.model_capabilities),
            effective_capabilities: sorted_capabilities(&resolution.effective_capabilities),
            provider_only_capabilities: sorted_capabilities(&resolution.provider_only_capabilities),
            model_only_capabilities: sorted_capabilities(&resolution.model_only_capabilities),
        }
    }
}

impl RuntimeResolvedModelCapabilityExplain {
    fn from_descriptor(descriptor: crate::catalog::ModelCapabilityDescriptor) -> Self {
        Self {
            model: descriptor.model,
            display_name: descriptor.display_name,
            capabilities: sorted_capabilities(&descriptor.capabilities),
            supported_operations: descriptor
                .operations
                .iter()
                .map(|operation| operation.as_str())
                .collect(),
        }
    }
}

impl RuntimeResolvedRouteEntry {
    fn from_route(route: RuntimeRoute) -> Self {
        Self {
            provider: route.invocation.provider,
            surface: route.invocation.surface.as_str(),
            wire_protocol: route.invocation.wire_protocol.as_str(),
            transport: transport_name(route.invocation.endpoint.transport),
            http_method: route.invocation.endpoint.http_method.map(http_method_name),
            path: route.invocation.endpoint.path,
            base_url: route.base_url,
            url: route.url,
            query_params: route.query_params,
            streaming: route.invocation.streaming,
            async_job: route.invocation.async_job,
            verification: verification_status_name(route.invocation.verification),
        }
    }
}

fn sorted_capabilities(capabilities: &crate::catalog::ProviderCapabilitySet) -> Vec<&'static str> {
    let mut out: Vec<_> = capabilities.iter().map(CapabilityKind::as_str).collect();
    out.sort_unstable();
    out
}

fn runtime_provider_api_name(api: RuntimeProviderApi) -> &'static str {
    match api {
        RuntimeProviderApi::OpenaiChatCompletions => "openai_chat_completions",
        RuntimeProviderApi::OpenaiResponses => "openai_responses",
        RuntimeProviderApi::GeminiGenerateContent => "gemini_generate_content",
        RuntimeProviderApi::AnthropicMessages => "anthropic_messages",
    }
}

pub(super) fn transport_name(transport: TransportKind) -> &'static str {
    match transport {
        TransportKind::Http => "http",
        TransportKind::WebSocket => "websocket",
    }
}

pub(super) fn http_method_name(method: HttpMethod) -> &'static str {
    match method {
        HttpMethod::Get => "GET",
        HttpMethod::Post => "POST",
        HttpMethod::Put => "PUT",
        HttpMethod::Delete => "DELETE",
    }
}

fn verification_status_name(status: VerificationStatus) -> &'static str {
    match status {
        VerificationStatus::Explicit => "explicit",
        VerificationStatus::FamilyInferred => "family_inferred",
        VerificationStatus::DocsOnly => "docs_only",
    }
}
