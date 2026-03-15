use crate::catalog::generated::providers::{OPENAI_BEHAVIORS, OPENAI_MODELS};
use crate::catalog::{
    ApiSurfaceId, AuthMethodKind, EndpointQueryParam, EndpointTemplate, EvidenceLevel, EvidenceRef,
    HttpMethod, ModelBinding, ModelSelector, OperationKind, ProviderAuthHint, ProviderClass,
    ProviderPluginDescriptor, TransportKind, VerificationStatus, WireProtocol,
};

const OPENAI_AUTH_METHODS: &[AuthMethodKind] = &[
    AuthMethodKind::ApiKeyHeader,
    AuthMethodKind::CommandToken,
    AuthMethodKind::StaticBearer,
];

const OPENAI_AUTH_HINT: ProviderAuthHint = ProviderAuthHint {
    method: AuthMethodKind::ApiKeyHeader,
    env_keys: &["OPENAI_API_KEY"],
    query_param: None,
    header_name: Some("authorization"),
    prefix: Some("Bearer "),
};

const OPENAI_EVIDENCE: &[EvidenceRef] = &[EvidenceRef {
    level: EvidenceLevel::OfficialDocs,
    source_url: "https://platform.openai.com/docs/api-reference",
    note: Some("Generic OpenAI API surface built into Ditto core."),
}];

const OPENAI_BINDINGS: &[ModelBinding] = &[
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
        evidence: OPENAI_EVIDENCE,
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
        evidence: OPENAI_EVIDENCE,
    },
    #[cfg(feature = "cap-embedding")]
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
        evidence: OPENAI_EVIDENCE,
    },
    #[cfg(any(feature = "cap-image-generation", feature = "cap-image-edit"))]
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
        evidence: OPENAI_EVIDENCE,
    },
    #[cfg(any(feature = "cap-image-generation", feature = "cap-image-edit"))]
    ModelBinding {
        operation: OperationKind::IMAGE_EDIT,
        selector: ModelSelector::Any,
        surface: ApiSurfaceId::OPENAI_IMAGES_EDITS,
        wire_protocol: WireProtocol::OPENAI_IMAGES,
        endpoint: EndpointTemplate {
            transport: TransportKind::Http,
            http_method: Some(HttpMethod::Post),
            base_url_override: None,
            path_template: "/v1/images/edits",
            query_params: &[],
        },
        quirks: None,
        streaming: None,
        async_job: None,
        verification: VerificationStatus::Explicit,
        evidence: OPENAI_EVIDENCE,
    },
    #[cfg(feature = "cap-video-generation")]
    ModelBinding {
        operation: OperationKind::VIDEO_GENERATION,
        selector: ModelSelector::Exact(&[
            "sora-2",
            "sora-2-2025-10-06",
            "sora-2-2025-12-08",
            "sora-2-pro",
            "sora-2-pro-2025-10-06",
        ]),
        surface: ApiSurfaceId::OPENAI_VIDEOS,
        wire_protocol: WireProtocol::OPENAI_VIDEOS,
        endpoint: EndpointTemplate {
            transport: TransportKind::Http,
            http_method: Some(HttpMethod::Post),
            base_url_override: None,
            path_template: "/v1/videos",
            query_params: &[],
        },
        quirks: None,
        streaming: None,
        async_job: Some(true),
        verification: VerificationStatus::Explicit,
        evidence: OPENAI_EVIDENCE,
    },
    #[cfg(any(feature = "cap-audio-transcription", feature = "cap-audio-speech"))]
    ModelBinding {
        operation: OperationKind::AUDIO_SPEECH,
        selector: ModelSelector::Any,
        surface: ApiSurfaceId::OPENAI_AUDIO_SPEECH,
        wire_protocol: WireProtocol::OPENAI_AUDIO,
        endpoint: EndpointTemplate {
            transport: TransportKind::Http,
            http_method: Some(HttpMethod::Post),
            base_url_override: None,
            path_template: "/v1/audio/speech",
            query_params: &[],
        },
        quirks: None,
        streaming: None,
        async_job: None,
        verification: VerificationStatus::Explicit,
        evidence: OPENAI_EVIDENCE,
    },
    #[cfg(any(feature = "cap-audio-transcription", feature = "cap-audio-speech"))]
    ModelBinding {
        operation: OperationKind::AUDIO_TRANSCRIPTION,
        selector: ModelSelector::Any,
        surface: ApiSurfaceId::OPENAI_AUDIO_TRANSCRIPTIONS,
        wire_protocol: WireProtocol::OPENAI_AUDIO,
        endpoint: EndpointTemplate {
            transport: TransportKind::Http,
            http_method: Some(HttpMethod::Post),
            base_url_override: None,
            path_template: "/v1/audio/transcriptions",
            query_params: &[],
        },
        quirks: None,
        streaming: None,
        async_job: None,
        verification: VerificationStatus::Explicit,
        evidence: OPENAI_EVIDENCE,
    },
    #[cfg(feature = "cap-moderation")]
    ModelBinding {
        operation: OperationKind::MODERATION,
        selector: ModelSelector::Any,
        surface: ApiSurfaceId::OPENAI_MODERATIONS,
        wire_protocol: WireProtocol::OPENAI_MODERATIONS,
        endpoint: EndpointTemplate {
            transport: TransportKind::Http,
            http_method: Some(HttpMethod::Post),
            base_url_override: None,
            path_template: "/v1/moderations",
            query_params: &[],
        },
        quirks: None,
        streaming: None,
        async_job: None,
        verification: VerificationStatus::Explicit,
        evidence: OPENAI_EVIDENCE,
    },
    #[cfg(feature = "cap-batch")]
    ModelBinding {
        operation: OperationKind::BATCH,
        selector: ModelSelector::Any,
        surface: ApiSurfaceId::OPENAI_BATCHES,
        wire_protocol: WireProtocol::OPENAI_BATCHES,
        endpoint: EndpointTemplate {
            transport: TransportKind::Http,
            http_method: Some(HttpMethod::Post),
            base_url_override: None,
            path_template: "/v1/batches",
            query_params: &[],
        },
        quirks: None,
        streaming: None,
        async_job: None,
        verification: VerificationStatus::Explicit,
        evidence: OPENAI_EVIDENCE,
    },
    #[cfg(feature = "cap-realtime")]
    ModelBinding {
        operation: OperationKind::REALTIME_SESSION,
        selector: ModelSelector::Any,
        surface: ApiSurfaceId::OPENAI_REALTIME,
        wire_protocol: WireProtocol::OPENAI_REALTIME,
        endpoint: EndpointTemplate {
            transport: TransportKind::WebSocket,
            http_method: None,
            base_url_override: None,
            path_template: "/v1/realtime",
            query_params: &[EndpointQueryParam {
                name: "model",
                value_template: "{model}",
            }],
        },
        quirks: None,
        streaming: Some(true),
        async_job: None,
        verification: VerificationStatus::Explicit,
        evidence: OPENAI_EVIDENCE,
    },
];

pub const GENERIC_OPENAI_PLUGIN: ProviderPluginDescriptor = ProviderPluginDescriptor {
    id: "openai",
    display_name: "Generic OpenAI API",
    class: ProviderClass::GenericOpenAi,
    default_base_url: Some("https://api.openai.com/v1"),
    supported_auth: OPENAI_AUTH_METHODS,
    auth_hint: Some(OPENAI_AUTH_HINT),
    models: OPENAI_MODELS,
    bindings: OPENAI_BINDINGS,
    behaviors: OPENAI_BEHAVIORS,
    capability_statuses: &[],
};
