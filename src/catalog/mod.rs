mod builtin;
mod generated;
mod provider_runtime;
mod reference_schema;
mod resolver;

use core::fmt;

pub use builtin::{builtin_provider_plugins, builtin_registry};
pub use provider_runtime::{
    CapabilityKind, ModelCapabilityDescriptor, ProviderCapabilityBinding,
    ProviderCapabilityResolution, ProviderCapabilitySet, ProviderId, ProviderProtocolFamily,
    ProviderRuntimeSpec, capability_for_operation,
};
pub use reference_schema::{
    ReferenceCatalogExpectation, ReferenceCatalogExpectationIssue,
    ReferenceCatalogExpectationReport, ReferenceCatalogLoadError, ReferenceCatalogRole,
    ReferenceCatalogValidationIssue, ReferenceCatalogValidationReport,
    ReferenceModelCapabilityProfile, ReferenceModelEntry, ReferenceModelRecord,
    ReferenceProviderAuth, ReferenceProviderCapabilityProfile, ReferenceProviderDescriptor,
    ReferenceProviderModelCatalog, core_provider_reference_catalog_expectations,
};
pub use resolver::{RuntimeProviderApi, RuntimeProviderHints, RuntimeRoute, RuntimeRouteRequest};

macro_rules! static_id_type {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(&'static str);

        impl $name {
            pub const fn new(id: &'static str) -> Self {
                Self(id)
            }

            pub const fn as_str(self) -> &'static str {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.0)
            }
        }
    };
}

static_id_type!(OperationKind);
static_id_type!(ApiSurfaceId);
static_id_type!(WireProtocol);
static_id_type!(ContextCacheModeId);

impl OperationKind {
    pub const CHAT_COMPLETION: Self = Self::new("chat.completion");
    pub const RESPONSE: Self = Self::new("response");
    pub const TEXT_COMPLETION: Self = Self::new("text.completion");
    pub const EMBEDDING: Self = Self::new("embedding");
    pub const MULTIMODAL_EMBEDDING: Self = Self::new("embedding.multimodal");
    pub const IMAGE_GENERATION: Self = Self::new("image.generation");
    pub const IMAGE_EDIT: Self = Self::new("image.edit");
    pub const IMAGE_TRANSLATION: Self = Self::new("image.translation");
    pub const IMAGE_QUESTION: Self = Self::new("image.question");
    pub const VIDEO_GENERATION: Self = Self::new("video.generation");
    pub const AUDIO_SPEECH: Self = Self::new("audio.speech");
    pub const AUDIO_TRANSCRIPTION: Self = Self::new("audio.transcription");
    pub const AUDIO_TRANSLATION: Self = Self::new("audio.translation");
    pub const AUDIO_VOICE_CLONE: Self = Self::new("audio.voice_clone");
    pub const AUDIO_VOICE_DESIGN: Self = Self::new("audio.voice_design");
    pub const REALTIME_SESSION: Self = Self::new("realtime.session");
    pub const RERANK: Self = Self::new("rerank");
    pub const CLASSIFICATION_OR_EXTRACTION: Self = Self::new("classification_or_extraction");
    pub const MODERATION: Self = Self::new("moderation");
    pub const BATCH: Self = Self::new("batch");
    pub const OCR: Self = Self::new("ocr");
    pub const MODEL_LIST: Self = Self::new("model.list");
    pub const CONTEXT_CACHE: Self = Self::new("context.cache");
    pub const THREAD_RUN: Self = Self::new("thread.run");
    pub const GROUP_CHAT_COMPLETION: Self = Self::new("group.chat.completion");
    pub const CHAT_TRANSLATION: Self = Self::new("chat.translation");
    pub const MUSIC_GENERATION: Self = Self::new("music.generation");
    pub const THREE_D_GENERATION: Self = Self::new("3d.generation");
}

impl ApiSurfaceId {
    pub const OPENAI_CHAT_COMPLETIONS: Self = Self::new("chat.completion");
    pub const OPENAI_RESPONSES: Self = Self::new("responses");
    pub const OPENAI_TEXT_COMPLETIONS: Self = Self::new("completion.legacy");
    pub const OPENAI_EMBEDDINGS: Self = Self::new("embedding");
    pub const OPENAI_IMAGES_GENERATIONS: Self = Self::new("image.generation");
    pub const OPENAI_IMAGES_EDITS: Self = Self::new("image.edit");
    pub const OPENAI_VIDEOS: Self = Self::new("video.generation.async");
    pub const OPENAI_AUDIO_SPEECH: Self = Self::new("audio.speech");
    pub const OPENAI_AUDIO_TRANSCRIPTIONS: Self = Self::new("audio.transcription");
    pub const OPENAI_AUDIO_TRANSLATIONS: Self = Self::new("audio.translation");
    pub const OPENAI_MODERATIONS: Self = Self::new("moderation");
    pub const OPENAI_BATCHES: Self = Self::new("batch");
    pub const OPENAI_REALTIME: Self = Self::new("realtime.websocket");
    pub const ANTHROPIC_MESSAGES: Self = Self::new("anthropic.messages");
    pub const GOOGLE_GENERATE_CONTENT: Self = Self::new("generate.content");
    pub const GOOGLE_STREAM_GENERATE_CONTENT: Self = Self::new("generate.content.stream");
    pub const GOOGLE_BATCH_GENERATE_CONTENT: Self = Self::new("generate.content.batch");
    pub const GOOGLE_EMBED_CONTENT: Self = Self::new("embedding");
    pub const GOOGLE_BATCH_EMBED_CONTENT: Self = Self::new("embedding.batch");
    pub const GOOGLE_LIVE: Self = Self::new("realtime.websocket");
    pub const GOOGLE_PREDICT: Self = Self::new("image.generation");
    pub const GOOGLE_PREDICT_LONG_RUNNING: Self = Self::new("video.generation");
}

impl WireProtocol {
    pub const OPENAI_CHAT_COMPLETIONS: Self = Self::new("openai.chat_completions");
    pub const OPENAI_RESPONSES: Self = Self::new("openai.responses");
    pub const OPENAI_TEXT_COMPLETIONS: Self = Self::new("openai.text_completions");
    pub const OPENAI_EMBEDDINGS: Self = Self::new("openai.embeddings");
    pub const OPENAI_IMAGES: Self = Self::new("openai.images");
    pub const OPENAI_VIDEOS: Self = Self::new("openai.videos");
    pub const OPENAI_AUDIO: Self = Self::new("openai.audio");
    pub const OPENAI_MODERATIONS: Self = Self::new("openai.moderations");
    pub const OPENAI_BATCHES: Self = Self::new("openai.batches");
    pub const OPENAI_REALTIME: Self = Self::new("openai.realtime");
    pub const ANTHROPIC_MESSAGES: Self = Self::new("anthropic.messages");
    pub const GOOGLE_GENERATE_CONTENT: Self = Self::new("google.generate_content");
    pub const GOOGLE_EMBED_CONTENT: Self = Self::new("google.embed_content");
    pub const GOOGLE_LIVE: Self = Self::new("google.live");
    pub const GOOGLE_PREDICT: Self = Self::new("google.predict");
    pub const GOOGLE_PREDICT_LONG_RUNNING: Self = Self::new("google.predict_long_running");
    pub const DASHSCOPE_NATIVE: Self = Self::new("dashscope.native");
    pub const DASHSCOPE_INFERENCE_WS: Self = Self::new("dashscope.inference_ws");
    pub const DASHSCOPE_REALTIME_WS: Self = Self::new("dashscope.realtime_ws");
    pub const QIANFAN_NATIVE: Self = Self::new("qianfan.native");
    pub const ARK_NATIVE: Self = Self::new("ark.native");
    pub const HUNYUAN_NATIVE: Self = Self::new("hunyuan.native");
    pub const MINIMAX_NATIVE: Self = Self::new("minimax.native");
    pub const ZHIPU_NATIVE: Self = Self::new("zhipu.native");
}

impl ContextCacheModeId {
    pub const PASSIVE: Self = Self::new("passive");
    pub const PROMPT_CACHE_KEY: Self = Self::new("prompt_cache_key");
    pub const ANTHROPIC_COMPATIBLE: Self = Self::new("anthropic_compatible");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransportKind {
    Http,
    WebSocket,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuthMethodKind {
    ApiKeyHeader,
    ApiKeyQuery,
    CommandToken,
    StaticBearer,
    SigV4,
    OAuthClientCredentials,
    OAuthDeviceCode,
    OAuthBrowserPkce,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VerificationStatus {
    Explicit,
    FamilyInferred,
    DocsOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EvidenceLevel {
    OfficialDocs,
    OfficialSdk,
    OfficialDemo,
    CommunityRepo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderClass {
    GenericOpenAi,
    NativeAnthropic,
    NativeGoogle,
    OpenAiCompatible,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderAuthHint {
    pub method: AuthMethodKind,
    pub env_keys: &'static [&'static str],
    pub query_param: Option<&'static str>,
    pub header_name: Option<&'static str>,
    pub prefix: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvidenceRef {
    pub level: EvidenceLevel,
    pub source_url: &'static str,
    pub note: Option<&'static str>,
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndpointQueryParam {
    pub name: &'static str,
    pub value_template: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolQuirks {
    pub require_model_prefix: bool,
    pub supports_system_role: bool,
    pub force_stream_options: bool,
}

impl ProtocolQuirks {
    pub const NONE: Self = Self {
        require_model_prefix: false,
        supports_system_role: true,
        force_stream_options: false,
    };
}

impl Default for ProtocolQuirks {
    fn default() -> Self {
        Self::NONE
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndpointTemplate {
    pub transport: TransportKind,
    pub http_method: Option<HttpMethod>,
    pub base_url_override: Option<&'static str>,
    pub path_template: &'static str,
    pub query_params: &'static [EndpointQueryParam],
}

impl EndpointTemplate {
    pub fn render(self, model: &str) -> ResolvedEndpoint {
        ResolvedEndpoint {
            transport: self.transport,
            http_method: self.http_method,
            base_url_override: self
                .base_url_override
                .map(|base_url| render_template(base_url, model)),
            path: render_template(self.path_template, model),
            query_params: self
                .query_params
                .iter()
                .map(|param| {
                    (
                        param.name.to_string(),
                        render_template(param.value_template, model),
                    )
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedEndpoint {
    pub transport: TransportKind,
    pub http_method: Option<HttpMethod>,
    pub base_url_override: Option<String>,
    pub path: String,
    pub query_params: Vec<(String, String)>,
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

    fn match_score(
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

fn render_template(template: &str, model: &str) -> String {
    template.replace("{model}", model)
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
