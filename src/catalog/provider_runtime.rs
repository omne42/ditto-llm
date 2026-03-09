use std::collections::{BTreeMap, BTreeSet};

use core::fmt;

use super::{
    ApiSurfaceId, AuthMethodKind, CapabilityImplementationStatus, CapabilityStatusDescriptor,
    InvocationHints, ModelBinding, OperationKind, ProviderAuthHint, ProviderClass,
    ProviderModelDescriptor, ProviderPluginDescriptor, ResolvedInvocation, WireProtocol,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProviderId<'a>(&'a str);

impl<'a> ProviderId<'a> {
    pub const fn new(id: &'a str) -> Self {
        Self(id)
    }

    pub const fn as_str(self) -> &'a str {
        self.0
    }
}

impl fmt::Display for ProviderId<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl<'a> From<&'a str> for ProviderId<'a> {
    fn from(value: &'a str) -> Self {
        Self::new(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CapabilityKind(&'static str);

impl CapabilityKind {
    pub const fn new(id: &'static str) -> Self {
        Self(id)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }

    pub fn parse_config_token(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "llm" => Some(Self::LLM),
            "embedding" | "embeddings" => Some(Self::EMBEDDING),
            "image.generation" | "image_generation" | "image-generation" => {
                Some(Self::IMAGE_GENERATION)
            }
            "image.edit" | "image_edit" | "image-edit" => Some(Self::IMAGE_EDIT),
            "image.translation" | "image_translation" | "image-translation" => {
                Some(Self::IMAGE_TRANSLATION)
            }
            "image.question" | "image_question" | "image-question" => Some(Self::IMAGE_QUESTION),
            "video.generation" | "video_generation" | "video-generation" => {
                Some(Self::VIDEO_GENERATION)
            }
            "audio.speech" | "audio_speech" | "audio-speech" => Some(Self::AUDIO_SPEECH),
            "audio.transcription" | "audio_transcription" | "audio-transcription" => {
                Some(Self::AUDIO_TRANSCRIPTION)
            }
            "audio.translation" | "audio_translation" | "audio-translation" => {
                Some(Self::AUDIO_TRANSLATION)
            }
            "audio.voice_clone" | "audio_voice_clone" | "audio-voice-clone" => {
                Some(Self::AUDIO_VOICE_CLONE)
            }
            "audio.voice_design" | "audio_voice_design" | "audio-voice-design" => {
                Some(Self::AUDIO_VOICE_DESIGN)
            }
            "realtime" => Some(Self::REALTIME),
            "rerank" => Some(Self::RERANK),
            "classification_or_extraction" | "classification-or-extraction" => {
                Some(Self::CLASSIFICATION_OR_EXTRACTION)
            }
            "moderation" | "moderations" => Some(Self::MODERATION),
            "batch" | "batches" => Some(Self::BATCH),
            "ocr" => Some(Self::OCR),
            "model.list" | "model_list" | "model-list" | "models" => Some(Self::MODEL_LIST),
            "context.cache" | "context_cache" | "context-cache" => Some(Self::CONTEXT_CACHE),
            "music.generation" | "music_generation" | "music-generation" => {
                Some(Self::MUSIC_GENERATION)
            }
            "3d.generation" | "3d_generation" | "3d-generation" => Some(Self::THREE_D_GENERATION),
            _ => None,
        }
    }

    pub const LLM: Self = Self::new("llm");
    pub const EMBEDDING: Self = Self::new("embedding");
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
    pub const REALTIME: Self = Self::new("realtime");
    pub const RERANK: Self = Self::new("rerank");
    pub const CLASSIFICATION_OR_EXTRACTION: Self = Self::new("classification_or_extraction");
    pub const MODERATION: Self = Self::new("moderation");
    pub const BATCH: Self = Self::new("batch");
    pub const OCR: Self = Self::new("ocr");
    pub const MODEL_LIST: Self = Self::new("model.list");
    pub const CONTEXT_CACHE: Self = Self::new("context.cache");
    pub const MUSIC_GENERATION: Self = Self::new("music.generation");
    pub const THREE_D_GENERATION: Self = Self::new("3d.generation");
}

impl fmt::Display for CapabilityKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ProviderProtocolFamily {
    OpenAi,
    Anthropic,
    Google,
    Dashscope,
    Qianfan,
    Ark,
    Hunyuan,
    Minimax,
    Zhipu,
    Custom,
    Mixed,
    #[default]
    Unknown,
}

impl ProviderProtocolFamily {
    pub fn from_provider_class(class: ProviderClass) -> Self {
        match class {
            ProviderClass::GenericOpenAi | ProviderClass::OpenAiCompatible => Self::OpenAi,
            ProviderClass::NativeAnthropic => Self::Anthropic,
            ProviderClass::NativeGoogle => Self::Google,
            ProviderClass::Custom => Self::Custom,
        }
    }

    pub fn from_wire_protocol(wire_protocol: WireProtocol) -> Self {
        let wire = wire_protocol.as_str();
        if wire.starts_with("openai.") {
            Self::OpenAi
        } else if wire.starts_with("anthropic.") {
            Self::Anthropic
        } else if wire.starts_with("google.") {
            Self::Google
        } else if wire.starts_with("dashscope.") {
            Self::Dashscope
        } else if wire.starts_with("qianfan.") {
            Self::Qianfan
        } else if wire.starts_with("ark.") {
            Self::Ark
        } else if wire.starts_with("hunyuan.") {
            Self::Hunyuan
        } else if wire.starts_with("minimax.") {
            Self::Minimax
        } else if wire.starts_with("zhipu.") {
            Self::Zhipu
        } else {
            Self::Unknown
        }
    }

    pub fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::Unknown, family) | (family, Self::Unknown) => family,
            (left, right) if left == right => left,
            _ => Self::Mixed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProviderCapabilitySet(BTreeSet<CapabilityKind>);

impl ProviderCapabilitySet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, capability: CapabilityKind) -> bool {
        self.0.insert(capability)
    }

    pub fn contains(&self, capability: CapabilityKind) -> bool {
        self.0.contains(&capability)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = CapabilityKind> + '_ {
        self.0.iter().copied()
    }

    pub fn extend(&mut self, other: &Self) {
        self.0.extend(other.iter());
    }

    pub fn intersection(&self, other: &Self) -> Self {
        self.iter()
            .filter(|capability| other.contains(*capability))
            .collect()
    }

    pub fn difference(&self, other: &Self) -> Self {
        self.iter()
            .filter(|capability| !other.contains(*capability))
            .collect()
    }

    pub fn from_operations(operations: &[OperationKind]) -> Self {
        let mut out = Self::default();
        for &operation in operations {
            if let Some(capability) = capability_for_operation(operation) {
                out.insert(capability);
            }
        }
        out
    }

    pub fn from_bindings(bindings: &[ModelBinding]) -> Self {
        let mut out = Self::default();
        for binding in bindings {
            if let Some(capability) = capability_for_operation(binding.operation) {
                out.insert(capability);
            }
        }
        out
    }

    pub fn from_models(models: &[ProviderModelDescriptor]) -> Self {
        let mut out = Self::default();
        for model in models {
            out.extend(&model.capability_set());
        }
        out
    }
}

impl FromIterator<CapabilityKind> for ProviderCapabilitySet {
    fn from_iter<T: IntoIterator<Item = CapabilityKind>>(iter: T) -> Self {
        let mut out = Self::default();
        for capability in iter {
            out.insert(capability);
        }
        out
    }
}

type CapabilityStatusMap = BTreeMap<CapabilityKind, CapabilityImplementationStatus>;

fn capability_status_map_from_operations(operations: &[OperationKind]) -> CapabilityStatusMap {
    let mut out = CapabilityStatusMap::new();
    for &operation in operations {
        if let Some(capability) = capability_for_operation(operation) {
            out.insert(capability, CapabilityImplementationStatus::Implemented);
        }
    }
    out
}

fn capability_status_map_from_bindings(bindings: &[ModelBinding]) -> CapabilityStatusMap {
    let mut out = CapabilityStatusMap::new();
    for binding in bindings {
        if let Some(capability) = capability_for_operation(binding.operation) {
            out.insert(capability, CapabilityImplementationStatus::Implemented);
        }
    }
    out
}

fn merge_capability_status_descriptors(
    target: &mut CapabilityStatusMap,
    descriptors: &[CapabilityStatusDescriptor],
) {
    for descriptor in descriptors {
        target.insert(descriptor.capability, descriptor.status);
    }
}

fn capability_status_descriptors_from_map(
    statuses: &CapabilityStatusMap,
) -> Vec<CapabilityStatusDescriptor> {
    statuses
        .iter()
        .map(|(capability, status)| CapabilityStatusDescriptor {
            capability: *capability,
            status: *status,
        })
        .collect()
}

fn implemented_capability_set_from_status_map(
    statuses: &CapabilityStatusMap,
) -> ProviderCapabilitySet {
    statuses
        .iter()
        .filter_map(|(capability, status)| {
            (*status == CapabilityImplementationStatus::Implemented).then_some(*capability)
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCapabilityBinding {
    pub provider: ProviderId<'static>,
    pub capability: CapabilityKind,
    pub adapter_family: ProviderProtocolFamily,
    pub operations: Vec<OperationKind>,
    pub surfaces: Vec<ApiSurfaceId>,
    pub wire_protocols: Vec<WireProtocol>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRuntimeSpec {
    pub provider: ProviderId<'static>,
    pub display_name: &'static str,
    pub class: ProviderClass,
    pub protocol_family: ProviderProtocolFamily,
    pub default_base_url: Option<&'static str>,
    pub supported_auth: &'static [AuthMethodKind],
    pub auth_hint: Option<ProviderAuthHint>,
    pub capabilities: ProviderCapabilitySet,
    pub capability_statuses: Vec<CapabilityStatusDescriptor>,
    pub capability_bindings: Vec<ProviderCapabilityBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCapabilityResolution {
    pub provider: ProviderId<'static>,
    pub requested_model: Option<String>,
    pub resolved_model: Option<ModelCapabilityDescriptor>,
    pub provider_capabilities: ProviderCapabilitySet,
    pub model_capabilities: ProviderCapabilitySet,
    pub effective_capabilities: ProviderCapabilitySet,
    pub provider_only_capabilities: ProviderCapabilitySet,
    pub model_only_capabilities: ProviderCapabilitySet,
}

impl ProviderCapabilityResolution {
    pub fn model_is_catalog_known(&self) -> bool {
        self.resolved_model.is_some()
    }

    pub fn provider_supports(&self, capability: CapabilityKind) -> bool {
        self.provider_capabilities.contains(capability)
    }

    pub fn model_supports(&self, capability: CapabilityKind) -> bool {
        self.model_capabilities.contains(capability)
    }

    pub fn effective_supports(&self, capability: CapabilityKind) -> bool {
        self.effective_capabilities.contains(capability)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CapabilityScopeSetResolution {
    model_capabilities: ProviderCapabilitySet,
    effective_capabilities: ProviderCapabilitySet,
    provider_only_capabilities: ProviderCapabilitySet,
    model_only_capabilities: ProviderCapabilitySet,
}

impl CapabilityScopeSetResolution {
    fn for_model(
        provider_capabilities: &ProviderCapabilitySet,
        model_capabilities: ProviderCapabilitySet,
    ) -> Self {
        Self {
            effective_capabilities: provider_capabilities.intersection(&model_capabilities),
            provider_only_capabilities: provider_capabilities.difference(&model_capabilities),
            model_only_capabilities: model_capabilities.difference(provider_capabilities),
            model_capabilities,
        }
    }

    fn provider_scope(provider_capabilities: &ProviderCapabilitySet) -> Self {
        Self {
            model_capabilities: ProviderCapabilitySet::new(),
            effective_capabilities: provider_capabilities.clone(),
            provider_only_capabilities: ProviderCapabilitySet::new(),
            model_only_capabilities: ProviderCapabilitySet::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCapabilityDescriptor {
    pub provider: ProviderId<'static>,
    pub model: &'static str,
    pub display_name: &'static str,
    pub capabilities: ProviderCapabilitySet,
    pub capability_statuses: Vec<CapabilityStatusDescriptor>,
    pub operations: &'static [OperationKind],
}

pub fn capability_for_operation(operation: OperationKind) -> Option<CapabilityKind> {
    Some(match operation {
        OperationKind::CHAT_COMPLETION
        | OperationKind::RESPONSE
        | OperationKind::TEXT_COMPLETION
        | OperationKind::THREAD_RUN
        | OperationKind::GROUP_CHAT_COMPLETION
        | OperationKind::CHAT_TRANSLATION => CapabilityKind::LLM,
        OperationKind::EMBEDDING | OperationKind::MULTIMODAL_EMBEDDING => CapabilityKind::EMBEDDING,
        OperationKind::IMAGE_GENERATION => CapabilityKind::IMAGE_GENERATION,
        OperationKind::IMAGE_EDIT => CapabilityKind::IMAGE_EDIT,
        OperationKind::IMAGE_TRANSLATION => CapabilityKind::IMAGE_TRANSLATION,
        OperationKind::IMAGE_QUESTION => CapabilityKind::IMAGE_QUESTION,
        OperationKind::VIDEO_GENERATION => CapabilityKind::VIDEO_GENERATION,
        OperationKind::AUDIO_SPEECH => CapabilityKind::AUDIO_SPEECH,
        OperationKind::AUDIO_TRANSCRIPTION => CapabilityKind::AUDIO_TRANSCRIPTION,
        OperationKind::AUDIO_TRANSLATION => CapabilityKind::AUDIO_TRANSLATION,
        OperationKind::AUDIO_VOICE_CLONE => CapabilityKind::AUDIO_VOICE_CLONE,
        OperationKind::AUDIO_VOICE_DESIGN => CapabilityKind::AUDIO_VOICE_DESIGN,
        OperationKind::REALTIME_SESSION => CapabilityKind::REALTIME,
        OperationKind::RERANK => CapabilityKind::RERANK,
        OperationKind::CLASSIFICATION_OR_EXTRACTION => CapabilityKind::CLASSIFICATION_OR_EXTRACTION,
        OperationKind::MODERATION => CapabilityKind::MODERATION,
        OperationKind::BATCH => CapabilityKind::BATCH,
        OperationKind::OCR => CapabilityKind::OCR,
        OperationKind::MODEL_LIST => CapabilityKind::MODEL_LIST,
        OperationKind::CONTEXT_CACHE => CapabilityKind::CONTEXT_CACHE,
        OperationKind::MUSIC_GENERATION => CapabilityKind::MUSIC_GENERATION,
        OperationKind::THREE_D_GENERATION => CapabilityKind::THREE_D_GENERATION,
        _ => return None,
    })
}

impl ProviderModelDescriptor {
    pub fn capability_status_map(&self) -> CapabilityStatusMap {
        let mut statuses = capability_status_map_from_operations(self.supported_operations);
        merge_capability_status_descriptors(&mut statuses, self.capability_statuses);
        statuses
    }

    pub fn capability_statuses(&self) -> Vec<CapabilityStatusDescriptor> {
        capability_status_descriptors_from_map(&self.capability_status_map())
    }

    pub fn capability_status(
        &self,
        capability: CapabilityKind,
    ) -> Option<CapabilityImplementationStatus> {
        self.capability_status_map().get(&capability).copied()
    }

    pub fn capability_set(&self) -> ProviderCapabilitySet {
        implemented_capability_set_from_status_map(&self.capability_status_map())
    }

    pub fn supports_capability(&self, capability: CapabilityKind) -> bool {
        self.capability_set().contains(capability)
    }

    pub fn capability_descriptor(
        &self,
        provider: ProviderId<'static>,
    ) -> ModelCapabilityDescriptor {
        ModelCapabilityDescriptor {
            provider,
            model: self.id,
            display_name: self.display_name,
            capabilities: self.capability_set(),
            capability_statuses: self.capability_statuses(),
            operations: self.supported_operations,
        }
    }
}

impl ProviderPluginDescriptor {
    pub const fn provider_id(&self) -> ProviderId<'static> {
        ProviderId::new(self.id)
    }

    pub fn protocol_family(&self) -> ProviderProtocolFamily {
        let family = self
            .bindings
            .iter()
            .fold(ProviderProtocolFamily::Unknown, |acc, binding| {
                acc.merge(ProviderProtocolFamily::from_wire_protocol(
                    binding.wire_protocol,
                ))
            });
        family.merge(ProviderProtocolFamily::from_provider_class(self.class))
    }

    pub fn capability_status_map(&self) -> CapabilityStatusMap {
        let mut statuses = capability_status_map_from_bindings(self.bindings);
        merge_capability_status_descriptors(&mut statuses, self.capability_statuses);
        statuses
    }

    pub fn capability_statuses(&self) -> Vec<CapabilityStatusDescriptor> {
        capability_status_descriptors_from_map(&self.capability_status_map())
    }

    pub fn capability_status(
        &self,
        capability: CapabilityKind,
    ) -> Option<CapabilityImplementationStatus> {
        self.capability_status_map().get(&capability).copied()
    }

    pub fn capability_set(&self) -> ProviderCapabilitySet {
        implemented_capability_set_from_status_map(&self.capability_status_map())
    }

    pub fn supports_capability(&self, capability: CapabilityKind) -> bool {
        self.capability_set().contains(capability)
    }

    pub fn capability_bindings(&self) -> Vec<ProviderCapabilityBinding> {
        #[derive(Default)]
        struct AggregatedBinding {
            operations: BTreeSet<OperationKind>,
            surfaces: BTreeSet<ApiSurfaceId>,
            wire_protocols: BTreeSet<WireProtocol>,
            adapter_family: ProviderProtocolFamily,
        }

        let implemented_capabilities = self.capability_set();
        let mut grouped = BTreeMap::<CapabilityKind, AggregatedBinding>::new();
        for binding in self.bindings {
            let Some(capability) = capability_for_operation(binding.operation) else {
                continue;
            };
            if !implemented_capabilities.contains(capability) {
                continue;
            }
            let entry = grouped.entry(capability).or_default();
            entry.operations.insert(binding.operation);
            entry.surfaces.insert(binding.surface);
            entry.wire_protocols.insert(binding.wire_protocol);
            entry.adapter_family =
                entry
                    .adapter_family
                    .merge(ProviderProtocolFamily::from_wire_protocol(
                        binding.wire_protocol,
                    ));
        }

        grouped
            .into_iter()
            .map(|(capability, aggregated)| ProviderCapabilityBinding {
                provider: self.provider_id(),
                capability,
                adapter_family: aggregated
                    .adapter_family
                    .merge(ProviderProtocolFamily::from_provider_class(self.class)),
                operations: aggregated.operations.into_iter().collect(),
                surfaces: aggregated.surfaces.into_iter().collect(),
                wire_protocols: aggregated.wire_protocols.into_iter().collect(),
            })
            .collect()
    }

    pub fn runtime_spec(&self) -> ProviderRuntimeSpec {
        ProviderRuntimeSpec {
            provider: self.provider_id(),
            display_name: self.display_name,
            class: self.class,
            protocol_family: self.protocol_family(),
            default_base_url: self.default_base_url,
            supported_auth: self.supported_auth,
            auth_hint: self.auth_hint,
            capabilities: self.capability_set(),
            capability_statuses: self.capability_statuses(),
            capability_bindings: self.capability_bindings(),
        }
    }

    pub fn capability_resolution(&self, model: Option<&str>) -> ProviderCapabilityResolution {
        let provider_capabilities = self.capability_set();
        let requested_model = model
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        let resolved_model = requested_model
            .as_deref()
            .and_then(|model_name| self.model(model_name))
            .map(|model| model.capability_descriptor(self.provider_id()));

        let scope_sets = if let Some(model_descriptor) = &resolved_model {
            CapabilityScopeSetResolution::for_model(
                &provider_capabilities,
                model_descriptor.capabilities.clone(),
            )
        } else {
            CapabilityScopeSetResolution::provider_scope(&provider_capabilities)
        };

        ProviderCapabilityResolution {
            provider: self.provider_id(),
            requested_model,
            resolved_model,
            provider_capabilities,
            model_capabilities: scope_sets.model_capabilities,
            effective_capabilities: scope_sets.effective_capabilities,
            provider_only_capabilities: scope_sets.provider_only_capabilities,
            model_only_capabilities: scope_sets.model_only_capabilities,
        }
    }
}

fn normalize_provider_token(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}

fn provider_match_score(plugin: &ProviderPluginDescriptor, normalized_hint: &str) -> Option<usize> {
    let provider = normalize_provider_token(plugin.id);
    let display = normalize_provider_token(plugin.display_name);

    if normalized_hint == provider || normalized_hint == display {
        return Some(100);
    }
    if normalized_hint.starts_with(&provider) || normalized_hint.contains(&provider) {
        return Some(80 + provider.len());
    }
    if normalized_hint.starts_with(&display) || normalized_hint.contains(&display) {
        return Some(60 + display.len());
    }
    None
}

impl super::CatalogRegistry {
    pub fn plugin_by_id(
        &self,
        provider: ProviderId<'_>,
    ) -> Option<&'static ProviderPluginDescriptor> {
        self.plugins()
            .iter()
            .find(|plugin| plugin.provider_id().as_str() == provider.as_str())
    }

    pub fn plugin_by_hint(
        &self,
        provider_name_hint: &str,
    ) -> Option<&'static ProviderPluginDescriptor> {
        let hint = normalize_provider_token(provider_name_hint);
        if hint.is_empty() {
            return None;
        }

        self.plugins()
            .iter()
            .filter_map(|plugin| provider_match_score(plugin, &hint).map(|score| (plugin, score)))
            .max_by_key(|(_, score)| *score)
            .map(|(plugin, _)| plugin)
    }

    pub fn models_by_provider_id(
        &self,
        provider: ProviderId<'_>,
    ) -> Option<&'static [ProviderModelDescriptor]> {
        Some(self.plugin_by_id(provider)?.models())
    }

    pub fn resolve_for_provider(
        &self,
        provider: ProviderId<'_>,
        model: &str,
        operation: OperationKind,
    ) -> Option<ResolvedInvocation> {
        self.plugin_by_id(provider)?.resolve(model, operation)
    }

    pub fn resolve_with_hints_for_provider(
        &self,
        provider: ProviderId<'_>,
        model: &str,
        operation: OperationKind,
        hints: InvocationHints,
    ) -> Option<ResolvedInvocation> {
        self.plugin_by_id(provider)?
            .resolve_with_hints(model, operation, hints)
    }

    pub fn provider_runtime_spec(&self, provider: ProviderId<'_>) -> Option<ProviderRuntimeSpec> {
        Some(self.plugin_by_id(provider)?.runtime_spec())
    }

    pub fn provider_runtime_spec_by_hint(
        &self,
        provider_name_hint: &str,
    ) -> Option<ProviderRuntimeSpec> {
        Some(self.plugin_by_hint(provider_name_hint)?.runtime_spec())
    }

    pub fn provider_supports_capability(
        &self,
        provider: ProviderId<'_>,
        capability: CapabilityKind,
    ) -> Option<bool> {
        Some(self.plugin_by_id(provider)?.supports_capability(capability))
    }

    pub fn provider_supports_capability_by_hint(
        &self,
        provider_name_hint: &str,
        capability: CapabilityKind,
    ) -> Option<bool> {
        Some(
            self.plugin_by_hint(provider_name_hint)?
                .supports_capability(capability),
        )
    }

    pub fn model_capability_descriptor(
        &self,
        provider: ProviderId<'_>,
        model: &str,
    ) -> Option<ModelCapabilityDescriptor> {
        let plugin = self.plugin_by_id(provider)?;
        Some(
            plugin
                .model(model)?
                .capability_descriptor(plugin.provider_id()),
        )
    }

    pub fn provider_capability_resolution(
        &self,
        provider: ProviderId<'_>,
        model: Option<&str>,
    ) -> Option<ProviderCapabilityResolution> {
        Some(self.plugin_by_id(provider)?.capability_resolution(model))
    }

    pub fn provider_capability_resolution_by_hint(
        &self,
        provider_name_hint: &str,
        model: Option<&str>,
    ) -> Option<ProviderCapabilityResolution> {
        Some(
            self.plugin_by_hint(provider_name_hint)?
                .capability_resolution(model),
        )
    }

    pub fn provider_model_supports_capability(
        &self,
        provider: ProviderId<'_>,
        model: &str,
        capability: CapabilityKind,
    ) -> Option<bool> {
        let plugin = self.plugin_by_id(provider)?;
        let model = plugin.model(model)?;
        Some(plugin.supports_capability(capability) && model.supports_capability(capability))
    }

    pub fn provider_model_supports_capability_by_hint(
        &self,
        provider_name_hint: &str,
        model: &str,
        capability: CapabilityKind,
    ) -> Option<bool> {
        let plugin = self.plugin_by_hint(provider_name_hint)?;
        let model = plugin.model(model)?;
        Some(plugin.supports_capability(capability) && model.supports_capability(capability))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{ApiSurfaceId, TransportKind, WireProtocol};
    use crate::catalog::{
        EndpointTemplate, EvidenceRef, HttpMethod, ModelSelector, VerificationStatus,
    };

    const EMPTY_EVIDENCE: &[EvidenceRef] = &[];

    #[test]
    fn capability_for_operation_maps_core_categories() {
        assert_eq!(
            capability_for_operation(OperationKind::CHAT_COMPLETION),
            Some(CapabilityKind::LLM)
        );
        assert_eq!(
            capability_for_operation(OperationKind::RESPONSE),
            Some(CapabilityKind::LLM)
        );
        assert_eq!(
            capability_for_operation(OperationKind::EMBEDDING),
            Some(CapabilityKind::EMBEDDING)
        );
        assert_eq!(
            capability_for_operation(OperationKind::REALTIME_SESSION),
            Some(CapabilityKind::REALTIME)
        );
        assert_eq!(
            capability_for_operation(OperationKind::IMAGE_EDIT),
            Some(CapabilityKind::IMAGE_EDIT)
        );
    }

    #[test]
    fn provider_capability_set_dedupes_operations() {
        let set = ProviderCapabilitySet::from_operations(&[
            OperationKind::CHAT_COMPLETION,
            OperationKind::RESPONSE,
            OperationKind::EMBEDDING,
            OperationKind::CHAT_COMPLETION,
        ]);
        assert!(set.contains(CapabilityKind::LLM));
        assert!(set.contains(CapabilityKind::EMBEDDING));
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn model_capability_descriptor_uses_supported_operations() {
        const MODEL: ProviderModelDescriptor = ProviderModelDescriptor {
            id: "demo-model",
            display_name: "Demo Model",
            aliases: &[],
            brand: None,
            family: None,
            summary: None,
            supported_operations: &[
                OperationKind::CHAT_COMPLETION,
                OperationKind::RESPONSE,
                OperationKind::EMBEDDING,
            ],
            capability_statuses: &[],
        };

        let descriptor = MODEL.capability_descriptor(ProviderId::new("demo-provider"));
        assert_eq!(descriptor.provider, ProviderId::new("demo-provider"));
        assert!(descriptor.capabilities.contains(CapabilityKind::LLM));
        assert!(descriptor.capabilities.contains(CapabilityKind::EMBEDDING));
        assert_eq!(descriptor.capabilities.len(), 2);
    }

    #[test]
    fn capability_statuses_filter_non_implemented_entries() {
        const BINDINGS: &[ModelBinding] = &[
            ModelBinding {
                operation: OperationKind::CHAT_COMPLETION,
                selector: ModelSelector::Any,
                surface: ApiSurfaceId::OPENAI_CHAT_COMPLETIONS,
                wire_protocol: WireProtocol::OPENAI_CHAT_COMPLETIONS,
                endpoint: EndpointTemplate {
                    transport: TransportKind::Http,
                    http_method: Some(HttpMethod::Post),
                    base_url_override: None,
                    path_template: "/v1/chat/completions",
                    query_params: &[],
                },
                quirks: None,
                streaming: None,
                async_job: None,
                verification: VerificationStatus::Explicit,
                evidence: EMPTY_EVIDENCE,
            },
            ModelBinding {
                operation: OperationKind::EMBEDDING,
                selector: ModelSelector::Any,
                surface: ApiSurfaceId::OPENAI_EMBEDDINGS,
                wire_protocol: WireProtocol::OPENAI_EMBEDDINGS,
                endpoint: EndpointTemplate {
                    transport: TransportKind::Http,
                    http_method: Some(HttpMethod::Post),
                    base_url_override: None,
                    path_template: "/v1/embeddings",
                    query_params: &[],
                },
                quirks: None,
                streaming: None,
                async_job: None,
                verification: VerificationStatus::Explicit,
                evidence: EMPTY_EVIDENCE,
            },
            ModelBinding {
                operation: OperationKind::IMAGE_GENERATION,
                selector: ModelSelector::Any,
                surface: ApiSurfaceId::OPENAI_IMAGES_GENERATIONS,
                wire_protocol: WireProtocol::OPENAI_IMAGES,
                endpoint: EndpointTemplate {
                    transport: TransportKind::Http,
                    http_method: Some(HttpMethod::Post),
                    base_url_override: None,
                    path_template: "/v1/images/generations",
                    query_params: &[],
                },
                quirks: None,
                streaming: None,
                async_job: None,
                verification: VerificationStatus::Explicit,
                evidence: EMPTY_EVIDENCE,
            },
        ];
        const MODELS: &[ProviderModelDescriptor] = &[ProviderModelDescriptor {
            id: "chat-lite",
            display_name: "Chat Lite",
            aliases: &[],
            brand: None,
            family: None,
            summary: None,
            supported_operations: &[OperationKind::CHAT_COMPLETION, OperationKind::EMBEDDING],
            capability_statuses: &[
                CapabilityStatusDescriptor::blocked(CapabilityKind::EMBEDDING),
                CapabilityStatusDescriptor::planned(CapabilityKind::IMAGE_GENERATION),
            ],
        }];
        const PLUGIN: ProviderPluginDescriptor = ProviderPluginDescriptor {
            id: "status-provider",
            display_name: "Status Provider",
            class: ProviderClass::OpenAiCompatible,
            default_base_url: Some("https://example.com/v1"),
            supported_auth: &[AuthMethodKind::ApiKeyHeader],
            auth_hint: None,
            models: MODELS,
            bindings: BINDINGS,
            behaviors: &[],
            capability_statuses: &[CapabilityStatusDescriptor::planned(
                CapabilityKind::IMAGE_GENERATION,
            )],
        };

        let model = &MODELS[0];
        assert_eq!(
            model.capability_status(CapabilityKind::EMBEDDING),
            Some(CapabilityImplementationStatus::Blocked)
        );
        assert!(model.capability_set().contains(CapabilityKind::LLM));
        assert!(!model.capability_set().contains(CapabilityKind::EMBEDDING));

        let runtime_spec = PLUGIN.runtime_spec();
        assert!(runtime_spec.capabilities.contains(CapabilityKind::LLM));
        assert!(
            runtime_spec
                .capabilities
                .contains(CapabilityKind::EMBEDDING)
        );
        assert!(
            !runtime_spec
                .capabilities
                .contains(CapabilityKind::IMAGE_GENERATION)
        );
        assert!(
            !runtime_spec
                .capability_bindings
                .iter()
                .any(|binding| binding.capability == CapabilityKind::IMAGE_GENERATION)
        );
        assert_eq!(
            runtime_spec
                .capability_statuses
                .iter()
                .find(|descriptor| descriptor.capability == CapabilityKind::IMAGE_GENERATION)
                .map(|descriptor| descriptor.status),
            Some(CapabilityImplementationStatus::Planned)
        );

        let resolution = PLUGIN.capability_resolution(Some("chat-lite"));
        assert!(resolution.provider_supports(CapabilityKind::EMBEDDING));
        assert!(!resolution.model_supports(CapabilityKind::EMBEDDING));
        assert!(resolution.effective_supports(CapabilityKind::LLM));
        assert!(!resolution.effective_supports(CapabilityKind::EMBEDDING));
        assert!(!resolution.effective_supports(CapabilityKind::IMAGE_GENERATION));
    }

    #[test]
    fn plugin_runtime_spec_groups_bindings_by_capability() {
        const BINDINGS: &[ModelBinding] = &[
            ModelBinding {
                operation: OperationKind::CHAT_COMPLETION,
                selector: ModelSelector::Any,
                surface: ApiSurfaceId::OPENAI_CHAT_COMPLETIONS,
                wire_protocol: WireProtocol::OPENAI_CHAT_COMPLETIONS,
                endpoint: EndpointTemplate {
                    transport: TransportKind::Http,
                    http_method: Some(HttpMethod::Post),
                    base_url_override: None,
                    path_template: "/v1/chat/completions",
                    query_params: &[],
                },
                quirks: None,
                streaming: Some(true),
                async_job: None,
                verification: VerificationStatus::Explicit,
                evidence: EMPTY_EVIDENCE,
            },
            ModelBinding {
                operation: OperationKind::RESPONSE,
                selector: ModelSelector::Any,
                surface: ApiSurfaceId::OPENAI_RESPONSES,
                wire_protocol: WireProtocol::OPENAI_RESPONSES,
                endpoint: EndpointTemplate {
                    transport: TransportKind::Http,
                    http_method: Some(HttpMethod::Post),
                    base_url_override: None,
                    path_template: "/v1/responses",
                    query_params: &[],
                },
                quirks: None,
                streaming: None,
                async_job: None,
                verification: VerificationStatus::Explicit,
                evidence: EMPTY_EVIDENCE,
            },
            ModelBinding {
                operation: OperationKind::EMBEDDING,
                selector: ModelSelector::Any,
                surface: ApiSurfaceId::OPENAI_EMBEDDINGS,
                wire_protocol: WireProtocol::OPENAI_EMBEDDINGS,
                endpoint: EndpointTemplate {
                    transport: TransportKind::Http,
                    http_method: Some(HttpMethod::Post),
                    base_url_override: None,
                    path_template: "/v1/embeddings",
                    query_params: &[],
                },
                quirks: None,
                streaming: None,
                async_job: None,
                verification: VerificationStatus::Explicit,
                evidence: EMPTY_EVIDENCE,
            },
        ];
        const MODELS: &[ProviderModelDescriptor] = &[ProviderModelDescriptor {
            id: "demo-model",
            display_name: "Demo Model",
            aliases: &[],
            brand: None,
            family: None,
            summary: None,
            supported_operations: &[OperationKind::CHAT_COMPLETION, OperationKind::EMBEDDING],
            capability_statuses: &[],
        }];
        const PLUGIN: ProviderPluginDescriptor = ProviderPluginDescriptor {
            id: "demo-provider",
            display_name: "Demo Provider",
            class: ProviderClass::OpenAiCompatible,
            default_base_url: Some("https://example.com/v1"),
            supported_auth: &[AuthMethodKind::ApiKeyHeader],
            auth_hint: Some(ProviderAuthHint {
                method: AuthMethodKind::ApiKeyHeader,
                env_keys: &["DEMO_API_KEY"],
                query_param: None,
                header_name: Some("authorization"),
                prefix: Some("Bearer "),
            }),
            models: MODELS,
            bindings: BINDINGS,
            behaviors: &[],
            capability_statuses: &[],
        };

        let spec = PLUGIN.runtime_spec();
        assert_eq!(spec.provider, ProviderId::new("demo-provider"));
        assert_eq!(spec.protocol_family, ProviderProtocolFamily::OpenAi);
        assert!(spec.capabilities.contains(CapabilityKind::LLM));
        assert!(spec.capabilities.contains(CapabilityKind::EMBEDDING));
        assert_eq!(spec.capability_bindings.len(), 2);

        let llm_binding = spec
            .capability_bindings
            .iter()
            .find(|binding| binding.capability == CapabilityKind::LLM)
            .expect("llm binding should exist");
        assert_eq!(llm_binding.adapter_family, ProviderProtocolFamily::OpenAi);
        assert_eq!(
            llm_binding.operations,
            vec![OperationKind::CHAT_COMPLETION, OperationKind::RESPONSE]
        );
    }

    #[test]
    fn registry_supports_typed_provider_lookup() {
        const BINDINGS: &[ModelBinding] = &[ModelBinding {
            operation: OperationKind::CHAT_COMPLETION,
            selector: ModelSelector::Any,
            surface: ApiSurfaceId::OPENAI_CHAT_COMPLETIONS,
            wire_protocol: WireProtocol::OPENAI_CHAT_COMPLETIONS,
            endpoint: EndpointTemplate {
                transport: TransportKind::Http,
                http_method: Some(HttpMethod::Post),
                base_url_override: None,
                path_template: "/v1/chat/completions",
                query_params: &[],
            },
            quirks: None,
            streaming: None,
            async_job: None,
            verification: VerificationStatus::Explicit,
            evidence: EMPTY_EVIDENCE,
        }];
        const PLUGIN: ProviderPluginDescriptor = ProviderPluginDescriptor {
            id: "typed-provider",
            display_name: "Typed Provider",
            class: ProviderClass::GenericOpenAi,
            default_base_url: Some("https://example.com/v1"),
            supported_auth: &[AuthMethodKind::ApiKeyHeader],
            auth_hint: None,
            models: &[],
            bindings: BINDINGS,
            behaviors: &[],
            capability_statuses: &[],
        };
        let registry = super::super::CatalogRegistry::new(&[PLUGIN]);
        let provider = ProviderId::new("typed-provider");

        assert!(registry.plugin_by_id(provider).is_some());
        assert!(
            registry
                .resolve_for_provider(provider, "demo-model", OperationKind::CHAT_COMPLETION)
                .is_some()
        );
        assert_eq!(
            registry
                .provider_runtime_spec(provider)
                .expect("runtime spec should exist")
                .provider,
            provider
        );
    }

    #[test]
    fn capability_resolution_intersects_provider_and_model_capabilities() {
        const BINDINGS: &[ModelBinding] = &[
            ModelBinding {
                operation: OperationKind::CHAT_COMPLETION,
                selector: ModelSelector::Any,
                surface: ApiSurfaceId::OPENAI_CHAT_COMPLETIONS,
                wire_protocol: WireProtocol::OPENAI_CHAT_COMPLETIONS,
                endpoint: EndpointTemplate {
                    transport: TransportKind::Http,
                    http_method: Some(HttpMethod::Post),
                    base_url_override: None,
                    path_template: "/v1/chat/completions",
                    query_params: &[],
                },
                quirks: None,
                streaming: None,
                async_job: None,
                verification: VerificationStatus::Explicit,
                evidence: EMPTY_EVIDENCE,
            },
            ModelBinding {
                operation: OperationKind::EMBEDDING,
                selector: ModelSelector::Any,
                surface: ApiSurfaceId::OPENAI_EMBEDDINGS,
                wire_protocol: WireProtocol::OPENAI_EMBEDDINGS,
                endpoint: EndpointTemplate {
                    transport: TransportKind::Http,
                    http_method: Some(HttpMethod::Post),
                    base_url_override: None,
                    path_template: "/v1/embeddings",
                    query_params: &[],
                },
                quirks: None,
                streaming: None,
                async_job: None,
                verification: VerificationStatus::Explicit,
                evidence: EMPTY_EVIDENCE,
            },
        ];
        const MODELS: &[ProviderModelDescriptor] = &[ProviderModelDescriptor {
            id: "chat-only",
            display_name: "Chat Only",
            aliases: &[],
            brand: None,
            family: None,
            summary: None,
            supported_operations: &[OperationKind::CHAT_COMPLETION],
            capability_statuses: &[],
        }];
        const PLUGIN: ProviderPluginDescriptor = ProviderPluginDescriptor {
            id: "demo-provider",
            display_name: "Demo Provider",
            class: ProviderClass::OpenAiCompatible,
            default_base_url: None,
            supported_auth: &[AuthMethodKind::ApiKeyHeader],
            auth_hint: None,
            models: MODELS,
            bindings: BINDINGS,
            behaviors: &[],
            capability_statuses: &[],
        };

        let resolution = PLUGIN.capability_resolution(Some("chat-only"));
        assert!(resolution.model_is_catalog_known());
        assert!(resolution.provider_supports(CapabilityKind::EMBEDDING));
        assert!(resolution.model_supports(CapabilityKind::LLM));
        assert!(resolution.effective_supports(CapabilityKind::LLM));
        assert!(!resolution.effective_supports(CapabilityKind::EMBEDDING));
        assert!(
            resolution
                .provider_only_capabilities
                .contains(CapabilityKind::EMBEDDING)
        );
        assert!(resolution.model_only_capabilities.is_empty());
    }

    #[test]
    fn capability_resolution_falls_back_to_provider_scope_for_unknown_model() {
        const BINDINGS: &[ModelBinding] = &[ModelBinding {
            operation: OperationKind::CHAT_COMPLETION,
            selector: ModelSelector::Any,
            surface: ApiSurfaceId::OPENAI_CHAT_COMPLETIONS,
            wire_protocol: WireProtocol::OPENAI_CHAT_COMPLETIONS,
            endpoint: EndpointTemplate {
                transport: TransportKind::Http,
                http_method: Some(HttpMethod::Post),
                base_url_override: None,
                path_template: "/v1/chat/completions",
                query_params: &[],
            },
            quirks: None,
            streaming: None,
            async_job: None,
            verification: VerificationStatus::Explicit,
            evidence: EMPTY_EVIDENCE,
        }];
        const PLUGIN: ProviderPluginDescriptor = ProviderPluginDescriptor {
            id: "typed-provider",
            display_name: "Typed Provider",
            class: ProviderClass::GenericOpenAi,
            default_base_url: Some("https://example.com/v1"),
            supported_auth: &[AuthMethodKind::ApiKeyHeader],
            auth_hint: None,
            models: &[],
            bindings: BINDINGS,
            behaviors: &[],
            capability_statuses: &[],
        };

        let resolution = PLUGIN.capability_resolution(Some("future-model"));
        assert_eq!(resolution.requested_model.as_deref(), Some("future-model"));
        assert!(!resolution.model_is_catalog_known());
        assert!(resolution.model_capabilities.is_empty());
        assert_eq!(
            resolution.provider_capabilities,
            resolution.effective_capabilities
        );
    }

    #[test]
    fn registry_answers_provider_and_model_capability_queries() {
        const BINDINGS: &[ModelBinding] = &[
            ModelBinding {
                operation: OperationKind::CHAT_COMPLETION,
                selector: ModelSelector::Any,
                surface: ApiSurfaceId::OPENAI_CHAT_COMPLETIONS,
                wire_protocol: WireProtocol::OPENAI_CHAT_COMPLETIONS,
                endpoint: EndpointTemplate {
                    transport: TransportKind::Http,
                    http_method: Some(HttpMethod::Post),
                    base_url_override: None,
                    path_template: "/v1/chat/completions",
                    query_params: &[],
                },
                quirks: None,
                streaming: None,
                async_job: None,
                verification: VerificationStatus::Explicit,
                evidence: EMPTY_EVIDENCE,
            },
            ModelBinding {
                operation: OperationKind::EMBEDDING,
                selector: ModelSelector::Any,
                surface: ApiSurfaceId::OPENAI_EMBEDDINGS,
                wire_protocol: WireProtocol::OPENAI_EMBEDDINGS,
                endpoint: EndpointTemplate {
                    transport: TransportKind::Http,
                    http_method: Some(HttpMethod::Post),
                    base_url_override: None,
                    path_template: "/v1/embeddings",
                    query_params: &[],
                },
                quirks: None,
                streaming: None,
                async_job: None,
                verification: VerificationStatus::Explicit,
                evidence: EMPTY_EVIDENCE,
            },
        ];
        const MODELS: &[ProviderModelDescriptor] = &[
            ProviderModelDescriptor {
                id: "chat-only",
                display_name: "Chat Only",
                aliases: &[],
                brand: None,
                family: None,
                summary: None,
                supported_operations: &[OperationKind::CHAT_COMPLETION],
                capability_statuses: &[],
            },
            ProviderModelDescriptor {
                id: "embed-only",
                display_name: "Embed Only",
                aliases: &[],
                brand: None,
                family: None,
                summary: None,
                supported_operations: &[OperationKind::EMBEDDING],
                capability_statuses: &[],
            },
        ];
        const PLUGIN: ProviderPluginDescriptor = ProviderPluginDescriptor {
            id: "typed-provider",
            display_name: "Typed Provider",
            class: ProviderClass::OpenAiCompatible,
            default_base_url: Some("https://example.com/v1"),
            supported_auth: &[AuthMethodKind::ApiKeyHeader],
            auth_hint: None,
            models: MODELS,
            bindings: BINDINGS,
            behaviors: &[],
            capability_statuses: &[],
        };
        let registry = super::super::CatalogRegistry::new(&[PLUGIN]);

        assert_eq!(
            registry.provider_supports_capability(
                ProviderId::new("typed-provider"),
                CapabilityKind::LLM
            ),
            Some(true)
        );
        assert_eq!(
            registry
                .provider_supports_capability_by_hint("typed-provider", CapabilityKind::EMBEDDING),
            Some(true)
        );
        assert_eq!(
            registry.provider_supports_capability(
                ProviderId::new("typed-provider"),
                CapabilityKind::IMAGE_GENERATION
            ),
            Some(false)
        );
        assert_eq!(
            registry.provider_model_supports_capability(
                ProviderId::new("typed-provider"),
                "chat-only",
                CapabilityKind::LLM
            ),
            Some(true)
        );
        assert_eq!(
            registry.provider_model_supports_capability(
                ProviderId::new("typed-provider"),
                "chat-only",
                CapabilityKind::EMBEDDING
            ),
            Some(false)
        );
        assert_eq!(
            registry.provider_model_supports_capability_by_hint(
                "typed-provider",
                "embed-only",
                CapabilityKind::EMBEDDING
            ),
            Some(true)
        );
        assert_eq!(
            registry.provider_model_supports_capability(
                ProviderId::new("typed-provider"),
                "missing",
                CapabilityKind::LLM
            ),
            None
        );
    }

    #[test]
    fn registry_can_match_provider_by_hint() {
        const BINDINGS: &[ModelBinding] = &[ModelBinding {
            operation: OperationKind::CHAT_COMPLETION,
            selector: ModelSelector::Any,
            surface: ApiSurfaceId::OPENAI_CHAT_COMPLETIONS,
            wire_protocol: WireProtocol::OPENAI_CHAT_COMPLETIONS,
            endpoint: EndpointTemplate {
                transport: TransportKind::Http,
                http_method: Some(HttpMethod::Post),
                base_url_override: None,
                path_template: "/v1/chat/completions",
                query_params: &[],
            },
            quirks: None,
            streaming: None,
            async_job: None,
            verification: VerificationStatus::Explicit,
            evidence: EMPTY_EVIDENCE,
        }];
        const PLUGIN: ProviderPluginDescriptor = ProviderPluginDescriptor {
            id: "openrouter",
            display_name: "OpenRouter API",
            class: ProviderClass::OpenAiCompatible,
            default_base_url: Some("https://openrouter.ai/api/v1"),
            supported_auth: &[AuthMethodKind::ApiKeyHeader],
            auth_hint: None,
            models: &[],
            bindings: BINDINGS,
            behaviors: &[],
            capability_statuses: &[],
        };
        let registry = super::super::CatalogRegistry::new(&[PLUGIN]);

        let resolved = registry
            .plugin_by_hint("openrouter-gemini31")
            .expect("provider hint should match openrouter");
        assert_eq!(resolved.id, "openrouter");
        assert_eq!(
            registry
                .provider_runtime_spec_by_hint("openrouter-gemini31")
                .expect("runtime spec should resolve by hint")
                .provider,
            ProviderId::new("openrouter")
        );
    }
}
