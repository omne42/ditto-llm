use core::fmt;
use std::collections::BTreeMap;

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
    pub fn transport(&self) -> TransportKind {
        self.invocation.endpoint.transport
    }

    pub fn http_method(&self) -> Option<HttpMethod> {
        self.invocation.endpoint.http_method
    }

    pub fn path(&self) -> &str {
        self.invocation.endpoint.path.as_str()
    }
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

fn render_template(template: &str, model: &str) -> String {
    template.replace("{model}", model)
}
