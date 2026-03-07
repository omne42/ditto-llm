#[cfg(not(feature = "openai"))]
use crate::catalog::ProviderModelDescriptor;
#[cfg(feature = "openai")]
use crate::catalog::generated::OPENAI_MODELS;
use crate::catalog::{
    ApiSurfaceId, AuthMethodKind, EndpointTemplate, EvidenceLevel, EvidenceRef, HttpMethod,
    ModelBinding, ModelSelector, OperationKind, ProviderAuthHint, ProviderClass,
    ProviderPluginDescriptor, TransportKind, VerificationStatus, WireProtocol,
};

#[cfg(not(feature = "openai"))]
const OPENAI_MODELS: &[ProviderModelDescriptor] = &[];

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
        evidence: OPENAI_EVIDENCE,
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
        evidence: OPENAI_EVIDENCE,
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
        evidence: OPENAI_EVIDENCE,
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
        evidence: OPENAI_EVIDENCE,
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
        evidence: OPENAI_EVIDENCE,
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
        evidence: OPENAI_EVIDENCE,
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
};
