use std::collections::BTreeMap;

use super::endpoint::{EndpointTemplate, ProtocolQuirks, ResolvedEndpoint};
use super::ids::{ApiSurfaceId, CapabilityKind, OperationKind, ProviderId, WireProtocol};
use super::provider::{EvidenceRef, VerificationStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelSelector {
    Any,
    Exact(&'static [&'static str]),
    Prefix(&'static [&'static str]),
}

impl ModelSelector {
    pub fn matches(self, model: &str) -> bool {
        match self {
            Self::Any => true,
            Self::Exact(items) => items.contains(&model),
            Self::Prefix(prefixes) => prefixes.iter().any(|prefix| model.starts_with(prefix)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InvocationHints {
    pub streaming: Option<bool>,
    pub async_job: Option<bool>,
    pub preferred_surface: Option<ApiSurfaceId>,
    pub preferred_wire_protocol: Option<WireProtocol>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelBinding {
    pub operation: OperationKind,
    pub selector: ModelSelector,
    pub surface: ApiSurfaceId,
    pub wire_protocol: WireProtocol,
    pub endpoint: EndpointTemplate,
    pub quirks: Option<ProtocolQuirks>,
    pub streaming: Option<bool>,
    pub async_job: Option<bool>,
    pub verification: VerificationStatus,
    pub evidence: &'static [EvidenceRef],
}

impl ModelBinding {
    pub fn matches(&self, model: &str, operation: OperationKind, hints: InvocationHints) -> bool {
        self.match_score(model, operation, hints).is_some()
    }

    pub fn match_score(
        &self,
        model: &str,
        operation: OperationKind,
        hints: InvocationHints,
    ) -> Option<u32> {
        if self.operation != operation || !self.selector.matches(model) {
            return None;
        }

        let mut score = 0_u32;

        if let Some(surface) = hints.preferred_surface {
            if self.surface != surface {
                return None;
            }
            score += 64;
        }

        if let Some(protocol) = hints.preferred_wire_protocol {
            if self.wire_protocol != protocol {
                return None;
            }
            score += 32;
        }

        if let Some(expected_streaming) = self.streaming {
            let desired_streaming = hints.streaming.unwrap_or(false);
            if expected_streaming != desired_streaming {
                return None;
            }
            score += 16;
        }

        if let Some(desired_async) = hints.async_job {
            let expected_async = self.async_job.unwrap_or(false);
            if expected_async != desired_async {
                return None;
            }
            score += 8;
        }

        Some(score)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedInvocation {
    pub provider: &'static str,
    pub model: String,
    pub operation: OperationKind,
    pub surface: ApiSurfaceId,
    pub wire_protocol: WireProtocol,
    pub endpoint: ResolvedEndpoint,
    pub quirks: ProtocolQuirks,
    pub streaming: Option<bool>,
    pub async_job: Option<bool>,
    pub verification: VerificationStatus,
    pub evidence: &'static [EvidenceRef],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeProviderApi {
    OpenaiChatCompletions,
    OpenaiResponses,
    GeminiGenerateContent,
    AnthropicMessages,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RuntimeProviderHints<'a> {
    pub configured_provider: Option<&'a str>,
    pub base_url: Option<&'a str>,
    pub default_model: Option<&'a str>,
    pub enabled_capabilities: &'a [String],
    pub http_query_params: Option<&'a BTreeMap<String, String>>,
    pub upstream_api: Option<RuntimeProviderApi>,
}

#[derive(Debug, Clone, Copy)]
pub struct RuntimeRouteRequest<'a> {
    pub provider: &'a str,
    pub model: Option<&'a str>,
    pub operation: OperationKind,
    pub provider_hints: RuntimeProviderHints<'a>,
    pub hints: InvocationHints,
    pub required_capability: Option<CapabilityKind>,
}

impl<'a> RuntimeRouteRequest<'a> {
    pub fn new(provider: &'a str, model: Option<&'a str>, operation: OperationKind) -> Self {
        Self {
            provider,
            model,
            operation,
            provider_hints: RuntimeProviderHints::default(),
            hints: InvocationHints::default(),
            required_capability: None,
        }
    }

    pub fn with_runtime_hints(mut self, provider_hints: RuntimeProviderHints<'a>) -> Self {
        self.provider_hints = provider_hints;
        self
    }

    pub fn with_hints(mut self, hints: InvocationHints) -> Self {
        self.hints = hints;
        self
    }

    pub fn with_required_capability(mut self, capability: CapabilityKind) -> Self {
        self.required_capability = Some(capability);
        self
    }

    pub fn provider_id(self) -> ProviderId<'a> {
        ProviderId::new(self.provider)
    }
}

impl Default for RuntimeRouteRequest<'_> {
    fn default() -> Self {
        Self::new("", None, OperationKind::CHAT_COMPLETION)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeRoute {
    pub invocation: ResolvedInvocation,
    pub base_url: String,
    pub url: String,
    pub query_params: Vec<(String, String)>,
}

impl RuntimeRoute {
    pub fn transport(&self) -> super::endpoint::TransportKind {
        self.invocation.endpoint.transport
    }

    pub fn http_method(&self) -> Option<super::endpoint::HttpMethod> {
        self.invocation.endpoint.http_method
    }

    pub fn path(&self) -> &str {
        self.invocation.endpoint.path.as_str()
    }
}
