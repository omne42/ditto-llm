use crate::catalog::{
    ApiSurfaceId, AuthMethodKind, EndpointTemplate, EvidenceLevel, EvidenceRef, HttpMethod,
    ModelBinding, ModelSelector, OperationKind, ProviderAuthHint, ProviderClass,
    ProviderPluginDescriptor, TransportKind, VerificationStatus, WireProtocol,
};

const OPENAI_COMPAT_AUTH_METHODS: &[AuthMethodKind] = &[
    AuthMethodKind::ApiKeyHeader,
    AuthMethodKind::CommandToken,
    AuthMethodKind::StaticBearer,
];

const OPENAI_COMPAT_AUTH_HINT: ProviderAuthHint = ProviderAuthHint {
    method: AuthMethodKind::ApiKeyHeader,
    env_keys: &["OPENAI_COMPAT_API_KEY", "OPENAI_API_KEY"],
    query_param: None,
    header_name: Some("authorization"),
    prefix: Some("Bearer "),
};

const OPENAI_COMPAT_EVIDENCE: &[EvidenceRef] = &[EvidenceRef {
    level: EvidenceLevel::OfficialDocs,
    source_url: "https://platform.openai.com/docs/api-reference/chat/create",
    note: Some("Generic OpenAI-compatible API surface built into Ditto core."),
}];

const OPENAI_COMPAT_BINDINGS: &[ModelBinding] = &[
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
        evidence: OPENAI_COMPAT_EVIDENCE,
    },
    #[cfg(feature = "embeddings")]
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
        evidence: OPENAI_COMPAT_EVIDENCE,
    },
    #[cfg(feature = "images")]
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
        evidence: OPENAI_COMPAT_EVIDENCE,
    },
    #[cfg(feature = "images")]
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
        evidence: OPENAI_COMPAT_EVIDENCE,
    },
    #[cfg(feature = "audio")]
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
        evidence: OPENAI_COMPAT_EVIDENCE,
    },
    #[cfg(feature = "audio")]
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
        evidence: OPENAI_COMPAT_EVIDENCE,
    },
    #[cfg(feature = "moderations")]
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
        evidence: OPENAI_COMPAT_EVIDENCE,
    },
    #[cfg(feature = "batches")]
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
        evidence: OPENAI_COMPAT_EVIDENCE,
    },
];

pub const GENERIC_OPENAI_COMPATIBLE_PLUGIN: ProviderPluginDescriptor = ProviderPluginDescriptor {
    id: "openai-compatible",
    display_name: "Generic OpenAI-Compatible API",
    class: ProviderClass::OpenAiCompatible,
    default_base_url: None,
    supported_auth: OPENAI_COMPAT_AUTH_METHODS,
    auth_hint: Some(OPENAI_COMPAT_AUTH_HINT),
    models: &[],
    bindings: OPENAI_COMPAT_BINDINGS,
    behaviors: &[],
    capability_statuses: &[],
};
