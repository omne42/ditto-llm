mod builtin;
mod generated;
mod provider_runtime;
mod reference_schema;

#[allow(unused_imports)]
pub(crate) use crate::contracts::{
    ApiSurfaceId, AuthMethodKind, CapabilityKind, ContextCacheModeId, EndpointQueryParam,
    EndpointTemplate, EvidenceLevel, EvidenceRef, HttpMethod, InvocationHints, ModelBinding,
    ModelSelector, OperationKind, ProtocolQuirks, ProviderAuthHint, ProviderClass, ProviderId,
    ProviderProtocolFamily, ResolvedEndpoint, ResolvedInvocation, RuntimeProviderApi,
    RuntimeProviderHints, RuntimeRoute, RuntimeRouteRequest, TransportKind, VerificationStatus,
    WireProtocol, capability_for_operation,
};
pub use builtin::{builtin_provider_plugins, builtin_registry};
pub use provider_runtime::{
    ModelCapabilityDescriptor, ProviderCapabilityBinding, ProviderCapabilityResolution,
    ProviderCapabilitySet, ProviderRuntimeSpec,
};
pub use reference_schema::{
    ReferenceCatalogExpectation, ReferenceCatalogExpectationIssue,
    ReferenceCatalogExpectationReport, ReferenceCatalogLoadError, ReferenceCatalogRole,
    ReferenceCatalogValidationIssue, ReferenceCatalogValidationReport,
    ReferenceModelCapabilityProfile, ReferenceModelEntry, ReferenceModelRecord,
    ReferenceProviderAuth, ReferenceProviderCapabilityProfile, ReferenceProviderDescriptor,
    ReferenceProviderModelCatalog, core_provider_reference_catalog_expectations,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CapabilityImplementationStatus {
    Implemented,
    Planned,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CapabilityStatusDescriptor {
    pub capability: CapabilityKind,
    pub status: CapabilityImplementationStatus,
}

impl CapabilityStatusDescriptor {
    pub const fn implemented(capability: CapabilityKind) -> Self {
        Self {
            capability,
            status: CapabilityImplementationStatus::Implemented,
        }
    }

    pub const fn planned(capability: CapabilityKind) -> Self {
        Self {
            capability,
            status: CapabilityImplementationStatus::Planned,
        }
    }

    pub const fn blocked(capability: CapabilityKind) -> Self {
        Self {
            capability,
            status: CapabilityImplementationStatus::Blocked,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BehaviorSupport {
    Unknown,
    Unsupported,
    Supported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssistantToolFollowupRequirement {
    None,
    RequiresReasoningContent,
    RequiresThoughtSignature,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReasoningOutputMode {
    Unsupported,
    Optional,
    Always,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReasoningActivationKind {
    Unavailable,
    OpenAiReasoningEffort,
    DeepSeekThinkingTypeEnabled,
    AlwaysOn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CacheUsageReportingKind {
    Unknown,
    StandardUsage,
    DeepSeekPromptCacheHitMiss,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelBehaviorDescriptor {
    pub model: &'static str,
    pub operation: OperationKind,
    pub tool_calls: BehaviorSupport,
    pub tool_choice_required: BehaviorSupport,
    pub assistant_tool_followup: AssistantToolFollowupRequirement,
    pub reasoning_output: ReasoningOutputMode,
    pub reasoning_activation: ReasoningActivationKind,
    pub context_cache_modes: &'static [ContextCacheModeId],
    pub context_cache_default_enabled: bool,
    pub cache_usage_reporting: CacheUsageReportingKind,
    pub notes: Option<&'static str>,
}

impl ModelBehaviorDescriptor {
    pub fn matches(&self, model: &str, operation: OperationKind) -> bool {
        self.model == model && self.operation == operation
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderModelDescriptor {
    pub id: &'static str,
    pub display_name: &'static str,
    pub aliases: &'static [&'static str],
    pub brand: Option<&'static str>,
    pub family: Option<&'static str>,
    pub summary: Option<&'static str>,
    pub supported_operations: &'static [OperationKind],
    pub capability_statuses: &'static [CapabilityStatusDescriptor],
}

impl ProviderModelDescriptor {
    pub fn matches(&self, model: &str) -> bool {
        self.id == model || self.aliases.contains(&model)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderPluginDescriptor {
    pub id: &'static str,
    pub display_name: &'static str,
    pub class: ProviderClass,
    pub default_base_url: Option<&'static str>,
    pub supported_auth: &'static [AuthMethodKind],
    pub auth_hint: Option<ProviderAuthHint>,
    pub models: &'static [ProviderModelDescriptor],
    pub bindings: &'static [ModelBinding],
    pub behaviors: &'static [ModelBehaviorDescriptor],
    pub capability_statuses: &'static [CapabilityStatusDescriptor],
}

impl ProviderPluginDescriptor {
    pub fn models(&self) -> &'static [ProviderModelDescriptor] {
        self.models
    }

    pub fn model(&self, model: &str) -> Option<&'static ProviderModelDescriptor> {
        self.models.iter().find(|entry| entry.matches(model))
    }

    pub fn behaviors(&self) -> &'static [ModelBehaviorDescriptor] {
        self.behaviors
    }

    pub fn behavior(
        &self,
        model: &str,
        operation: OperationKind,
    ) -> Option<&'static ModelBehaviorDescriptor> {
        let canonical_model = self.model(model).map(|entry| entry.id).unwrap_or(model);
        self.behaviors.iter().find(|behavior| {
            behavior.matches(model, operation) || behavior.matches(canonical_model, operation)
        })
    }

    pub fn resolve(&self, model: &str, operation: OperationKind) -> Option<ResolvedInvocation> {
        self.resolve_with_hints(model, operation, InvocationHints::default())
    }

    pub fn resolve_with_hints(
        &self,
        model: &str,
        operation: OperationKind,
        hints: InvocationHints,
    ) -> Option<ResolvedInvocation> {
        let model_descriptor = self.model(model);
        if let Some(descriptor) = model_descriptor {
            if !descriptor.supported_operations.contains(&operation) {
                return None;
            }
        }

        let canonical_model = model_descriptor.map(|entry| entry.id);

        let binding = self
            .bindings
            .iter()
            .filter_map(|binding| {
                binding
                    .match_score(model, operation, hints)
                    .or_else(|| {
                        canonical_model
                            .and_then(|canonical| binding.match_score(canonical, operation, hints))
                    })
                    .map(|score| (binding, score))
            })
            .max_by_key(|(_, score)| *score)
            .map(|(binding, _)| binding)?;

        Some(ResolvedInvocation {
            provider: self.id,
            model: model.to_string(),
            operation,
            surface: binding.surface,
            wire_protocol: binding.wire_protocol,
            endpoint: binding.endpoint.render(model),
            quirks: binding.quirks.unwrap_or(ProtocolQuirks::NONE),
            streaming: binding.streaming,
            async_job: binding.async_job,
            verification: binding.verification,
            evidence: binding.evidence,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogRegistry {
    plugins: &'static [ProviderPluginDescriptor],
}

impl CatalogRegistry {
    pub const fn new(plugins: &'static [ProviderPluginDescriptor]) -> Self {
        Self { plugins }
    }

    pub fn plugins(&self) -> &'static [ProviderPluginDescriptor] {
        self.plugins
    }

    pub fn plugin(&self, provider: &str) -> Option<&'static ProviderPluginDescriptor> {
        self.plugins.iter().find(|plugin| plugin.id == provider)
    }

    pub fn models(&self, provider: &str) -> Option<&'static [ProviderModelDescriptor]> {
        Some(self.plugin(provider)?.models())
    }

    pub fn model(&self, provider: &str, model: &str) -> Option<&'static ProviderModelDescriptor> {
        self.plugin(provider)?.model(model)
    }

    pub fn behavior(
        &self,
        provider: &str,
        model: &str,
        operation: OperationKind,
    ) -> Option<&'static ModelBehaviorDescriptor> {
        self.plugin(provider)?.behavior(model, operation)
    }

    pub fn resolve(
        &self,
        provider: &str,
        model: &str,
        operation: OperationKind,
    ) -> Option<ResolvedInvocation> {
        self.plugin(provider)?.resolve(model, operation)
    }

    pub fn resolve_with_hints(
        &self,
        provider: &str,
        model: &str,
        operation: OperationKind,
        hints: InvocationHints,
    ) -> Option<ResolvedInvocation> {
        self.plugin(provider)?
            .resolve_with_hints(model, operation, hints)
    }

    pub fn supports_operation(
        &self,
        provider: &str,
        model: &str,
        operation: OperationKind,
    ) -> bool {
        let model = model.trim();
        if model.is_empty() {
            return false;
        }

        self.resolve(provider, model, operation).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_matches_exact_and_prefix() {
        assert!(ModelSelector::Any.matches("gpt-4.1"));
        assert!(ModelSelector::Exact(&["gpt-4.1"]).matches("gpt-4.1"));
        assert!(!ModelSelector::Exact(&["gpt-4.1"]).matches("gpt-4o"));
        assert!(ModelSelector::Prefix(&["gpt-"]).matches("gpt-4o"));
        assert!(!ModelSelector::Prefix(&["claude-"]).matches("gpt-4o"));
    }

    #[test]
    fn endpoint_template_renders_model_placeholder() {
        let endpoint = EndpointTemplate {
            transport: TransportKind::Http,
            http_method: Some(HttpMethod::Post),
            base_url_override: Some("https://generativelanguage.googleapis.com/v1beta"),
            path_template: "/v1/models/{model}:generateContent",
            query_params: &[EndpointQueryParam {
                name: "model",
                value_template: "{model}",
            }],
        }
        .render("gemini-3.1-pro");

        assert_eq!(
            endpoint.base_url_override.as_deref(),
            Some("https://generativelanguage.googleapis.com/v1beta")
        );
        assert_eq!(endpoint.path, "/v1/models/gemini-3.1-pro:generateContent");
        assert_eq!(
            endpoint.query_params,
            vec![("model".to_string(), "gemini-3.1-pro".to_string())]
        );
    }

    #[cfg(any(feature = "provider-openai", feature = "openai"))]
    #[test]
    fn builtin_registry_resolves_generic_openai_chat() {
        let resolved = builtin_registry()
            .resolve("openai", "gpt-4.1", OperationKind::CHAT_COMPLETION)
            .expect("generic openai chat binding should exist");

        assert_eq!(resolved.provider, "openai");
        assert_eq!(resolved.surface, ApiSurfaceId::OPENAI_CHAT_COMPLETIONS);
        assert_eq!(
            resolved.wire_protocol,
            WireProtocol::OPENAI_CHAT_COMPLETIONS
        );
        assert_eq!(resolved.endpoint.path, "/v1/chat/completions");
        assert_eq!(resolved.endpoint.transport, TransportKind::Http);
        assert_eq!(resolved.endpoint.http_method, Some(HttpMethod::Post));
        assert_eq!(resolved.quirks, ProtocolQuirks::NONE);
        assert_eq!(resolved.streaming, None);
        assert_eq!(resolved.async_job, None);
    }

    #[cfg(any(feature = "provider-openai", feature = "openai"))]
    #[test]
    fn builtin_registry_exposes_official_openai_models() {
        let registry = builtin_registry();
        let model = registry
            .model("openai", "gpt-4.1")
            .expect("openai official model should exist");

        assert_eq!(model.display_name, "GPT-4.1");
        assert!(
            model
                .supported_operations
                .contains(&OperationKind::CHAT_COMPLETION)
        );
    }

    #[cfg(any(feature = "provider-openai", feature = "openai"))]
    #[test]
    fn builtin_registry_supports_operation_queries() {
        let registry = builtin_registry();
        assert!(registry.supports_operation("openai", "gpt-4.1", OperationKind::CHAT_COMPLETION));
        assert!(!registry.supports_operation("openai", "", OperationKind::CHAT_COMPLETION));
    }

    #[cfg(all(
        feature = "embeddings",
        any(feature = "provider-openai", feature = "openai")
    ))]
    #[test]
    fn builtin_registry_resolves_generic_openai_embeddings() {
        let resolved = builtin_registry()
            .resolve("openai", "text-embedding-3-large", OperationKind::EMBEDDING)
            .expect("generic openai embedding binding should exist");

        assert_eq!(resolved.surface, ApiSurfaceId::OPENAI_EMBEDDINGS);
        assert_eq!(resolved.wire_protocol, WireProtocol::OPENAI_EMBEDDINGS);
        assert_eq!(resolved.endpoint.path, "/v1/embeddings");
    }

    #[test]
    fn binding_prefers_requested_wire_protocol() {
        const EVIDENCE: &[EvidenceRef] = &[];
        const MODELS: &[ProviderModelDescriptor] = &[ProviderModelDescriptor {
            id: "MiniMax-M2",
            display_name: "MiniMax M2",
            aliases: &[],
            brand: Some("minimax"),
            family: Some("m2"),
            summary: None,
            supported_operations: &[OperationKind::CHAT_COMPLETION],
            capability_statuses: &[],
        }];
        const BINDINGS: &[ModelBinding] = &[
            ModelBinding {
                operation: OperationKind::CHAT_COMPLETION,
                selector: ModelSelector::Exact(&["MiniMax-M2"]),
                surface: ApiSurfaceId::OPENAI_CHAT_COMPLETIONS,
                wire_protocol: WireProtocol::OPENAI_CHAT_COMPLETIONS,
                endpoint: EndpointTemplate {
                    transport: TransportKind::Http,
                    http_method: Some(HttpMethod::Post),
                    base_url_override: None,
                    path_template: "/v1/chat/completions",
                    query_params: &[],
                },
                quirks: Some(ProtocolQuirks {
                    require_model_prefix: false,
                    supports_system_role: false,
                    force_stream_options: false,
                }),
                streaming: None,
                async_job: None,
                verification: VerificationStatus::Explicit,
                evidence: EVIDENCE,
            },
            ModelBinding {
                operation: OperationKind::CHAT_COMPLETION,
                selector: ModelSelector::Exact(&["MiniMax-M2"]),
                surface: ApiSurfaceId::ANTHROPIC_MESSAGES,
                wire_protocol: WireProtocol::ANTHROPIC_MESSAGES,
                endpoint: EndpointTemplate {
                    transport: TransportKind::Http,
                    http_method: Some(HttpMethod::Post),
                    base_url_override: None,
                    path_template: "/anthropic/v1/messages",
                    query_params: &[],
                },
                quirks: None,
                streaming: None,
                async_job: None,
                verification: VerificationStatus::Explicit,
                evidence: EVIDENCE,
            },
        ];
        const PLUGIN: ProviderPluginDescriptor = ProviderPluginDescriptor {
            id: "minimax",
            display_name: "MiniMax",
            class: ProviderClass::Custom,
            default_base_url: None,
            supported_auth: &[AuthMethodKind::ApiKeyHeader],
            auth_hint: None,
            models: MODELS,
            bindings: BINDINGS,
            behaviors: &[],
            capability_statuses: &[],
        };

        let resolved = PLUGIN
            .resolve_with_hints(
                "MiniMax-M2",
                OperationKind::CHAT_COMPLETION,
                InvocationHints {
                    preferred_wire_protocol: Some(WireProtocol::ANTHROPIC_MESSAGES),
                    ..InvocationHints::default()
                },
            )
            .expect("preferred anthropic binding should resolve");

        assert_eq!(resolved.surface, ApiSurfaceId::ANTHROPIC_MESSAGES);
        assert_eq!(resolved.endpoint.path, "/anthropic/v1/messages");
        assert_eq!(resolved.quirks, ProtocolQuirks::NONE);
    }

    #[test]
    fn binding_defaults_to_non_streaming_when_not_requested() {
        const EVIDENCE: &[EvidenceRef] = &[];
        const BINDINGS: &[ModelBinding] = &[
            ModelBinding {
                operation: OperationKind::CHAT_COMPLETION,
                selector: ModelSelector::Exact(&["gemini-3.1-pro"]),
                surface: ApiSurfaceId::GOOGLE_STREAM_GENERATE_CONTENT,
                wire_protocol: WireProtocol::GOOGLE_GENERATE_CONTENT,
                endpoint: EndpointTemplate {
                    transport: TransportKind::Http,
                    http_method: Some(HttpMethod::Post),
                    base_url_override: None,
                    path_template: "/v1beta/models/{model}:streamGenerateContent",
                    query_params: &[EndpointQueryParam {
                        name: "alt",
                        value_template: "sse",
                    }],
                },
                quirks: None,
                streaming: Some(true),
                async_job: None,
                verification: VerificationStatus::Explicit,
                evidence: EVIDENCE,
            },
            ModelBinding {
                operation: OperationKind::CHAT_COMPLETION,
                selector: ModelSelector::Exact(&["gemini-3.1-pro"]),
                surface: ApiSurfaceId::GOOGLE_GENERATE_CONTENT,
                wire_protocol: WireProtocol::GOOGLE_GENERATE_CONTENT,
                endpoint: EndpointTemplate {
                    transport: TransportKind::Http,
                    http_method: Some(HttpMethod::Post),
                    base_url_override: None,
                    path_template: "/v1beta/models/{model}:generateContent",
                    query_params: &[],
                },
                quirks: None,
                streaming: Some(false),
                async_job: None,
                verification: VerificationStatus::Explicit,
                evidence: EVIDENCE,
            },
        ];
        const PLUGIN: ProviderPluginDescriptor = ProviderPluginDescriptor {
            id: "google",
            display_name: "Google",
            class: ProviderClass::NativeGoogle,
            default_base_url: None,
            supported_auth: &[AuthMethodKind::ApiKeyQuery],
            auth_hint: None,
            models: &[],
            bindings: BINDINGS,
            behaviors: &[],
            capability_statuses: &[],
        };

        let resolved = PLUGIN
            .resolve("gemini-3.1-pro", OperationKind::CHAT_COMPLETION)
            .expect("default non-streaming binding should resolve");

        assert_eq!(resolved.surface, ApiSurfaceId::GOOGLE_GENERATE_CONTENT);
        assert_eq!(resolved.streaming, Some(false));
    }

    #[cfg(feature = "provider-google")]
    #[test]
    fn builtin_registry_resolves_generated_google_bindings() {
        let registry = builtin_registry();

        let non_stream = registry
            .resolve("google", "gemini-3.1-pro", OperationKind::CHAT_COMPLETION)
            .expect("google non-streaming binding should resolve");
        assert_eq!(non_stream.surface, ApiSurfaceId::GOOGLE_GENERATE_CONTENT);
        assert_eq!(non_stream.streaming, None);
        assert!(non_stream.endpoint.path.ends_with(":generateContent"));

        let stream = registry
            .resolve_with_hints(
                "google",
                "gemini-3.1-pro",
                OperationKind::CHAT_COMPLETION,
                InvocationHints {
                    streaming: Some(true),
                    preferred_surface: Some(ApiSurfaceId::GOOGLE_STREAM_GENERATE_CONTENT),
                    ..InvocationHints::default()
                },
            )
            .expect("google streaming binding should resolve");
        assert_eq!(stream.surface, ApiSurfaceId::GOOGLE_STREAM_GENERATE_CONTENT);
        assert_eq!(stream.streaming, Some(true));
        assert_eq!(
            stream.endpoint.query_params,
            vec![("alt".to_string(), "sse".to_string())]
        );
    }

    #[cfg(feature = "provider-openrouter")]
    #[test]
    fn builtin_registry_exposes_generated_openrouter_models() {
        let registry = builtin_registry();
        let model = registry
            .model("openrouter", "google/gemini-2.5-flash-lite")
            .expect("openrouter model should exist");
        assert_eq!(model.display_name, "Google: Gemini 2.5 Flash Lite");
        assert!(
            model
                .supported_operations
                .contains(&OperationKind::CHAT_COMPLETION)
        );

        let resolved = registry
            .resolve(
                "openrouter",
                "google/gemini-2.5-flash-lite",
                OperationKind::CHAT_COMPLETION,
            )
            .expect("openrouter chat binding should resolve");
        assert_eq!(resolved.surface, ApiSurfaceId::OPENAI_CHAT_COMPLETIONS);
        assert_eq!(
            resolved.wire_protocol,
            WireProtocol::OPENAI_CHAT_COMPLETIONS
        );
        assert_eq!(resolved.endpoint.path, "/chat/completions");
    }

    #[cfg(feature = "provider-doubao")]
    #[test]
    fn builtin_registry_resolves_generated_doubao_bindings() {
        let registry = builtin_registry();

        let context = registry
            .resolve("doubao", "deepseek-r1-250528", OperationKind::CONTEXT_CACHE)
            .expect("doubao context cache binding should resolve");
        assert_eq!(context.wire_protocol, WireProtocol::ARK_NATIVE);
        assert_eq!(context.endpoint.path, "/context/create");

        let batch = registry
            .resolve("doubao", "deepseek-r1-250528", OperationKind::BATCH)
            .expect("doubao batch binding should resolve");
        assert_eq!(batch.endpoint.path, "/batch/chat/completions");

        let multimodal_embedding = registry
            .resolve(
                "doubao",
                "doubao-embedding-vision-250328",
                OperationKind::MULTIMODAL_EMBEDDING,
            )
            .expect("doubao multimodal embedding binding should resolve");
        assert_eq!(multimodal_embedding.endpoint.path, "/embeddings/multimodal");
    }

    #[cfg(feature = "provider-minimax")]
    #[test]
    fn builtin_registry_respects_generated_minimax_protocol_preferences() {
        let registry = builtin_registry();
        let resolved = registry
            .resolve_with_hints(
                "minimax",
                "MiniMax-M2",
                OperationKind::CHAT_COMPLETION,
                InvocationHints {
                    preferred_wire_protocol: Some(WireProtocol::ANTHROPIC_MESSAGES),
                    ..InvocationHints::default()
                },
            )
            .expect("minimax anthropic-compatible binding should resolve");

        assert_eq!(resolved.wire_protocol, WireProtocol::ANTHROPIC_MESSAGES);
        assert!(resolved.endpoint.path.ends_with("/messages"));
    }
}
