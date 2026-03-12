use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProviderResolutionError {
    #[error("runtime route provider must be non-empty")]
    RuntimeRouteProviderMissing,
    #[error("catalog provider not found: {provider}")]
    CatalogProviderNotFound { provider: String },
    #[error("catalog route not found for provider={provider} model={model} operation={operation}")]
    CatalogRouteNotFound {
        provider: String,
        model: String,
        operation: String,
    },
    #[error("runtime route model is not set (provide model or provider_config.default_model)")]
    RuntimeRouteModelMissing,
    #[error(
        "runtime route base_url is not set (missing endpoint override, provider_config.base_url, and plugin default_base_url)"
    )]
    RuntimeRouteBaseUrlMissing,
    #[error("provider base_url is missing")]
    ProviderBaseUrlMissing,
    #[error(
        "provider hint {provider_hint:?} resolved to non-openai-compatible catalog plugin {resolved_provider} ({resolved_class})"
    )]
    UnsupportedProviderClass {
        provider_hint: String,
        resolved_provider: String,
        resolved_class: String,
    },
    #[error("generic openai-compatible catalog plugin is not available in this build")]
    GenericOpenAiCompatiblePluginUnavailable,
    #[error(
        "provider capabilities require llm support, but catalog resolved non-llm scope: {scope}"
    )]
    ProviderCapabilitiesRequireLlm { scope: String },
    #[error("configured provider not found in this build: {provider}")]
    ConfiguredProviderNotFound { provider: String },
    #[error("configured capability is unknown: {capability}")]
    ConfiguredCapabilityUnknown { capability: String },
    #[error("configured capability unsupported for provider={provider} capability={capability}")]
    ConfiguredCapabilityUnsupported {
        provider: String,
        capability: String,
    },
    #[error(
        "runtime route capability unsupported for provider={provider} model={model} capability={capability}"
    )]
    RuntimeRouteCapabilityUnsupported {
        provider: String,
        model: String,
        capability: String,
    },
}

#[derive(Debug, Error)]
pub enum DittoError {
    #[error("api error ({status}): {body}")]
    Api {
        status: reqwest::StatusCode,
        body: String,
    },
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid response: {0}")]
    InvalidResponse(String),
    #[error(transparent)]
    ProviderResolution(#[from] ProviderResolutionError),
    #[error("failed to run auth command: {0}")]
    AuthCommand(String),
    #[error("config error: {0}")]
    Config(String),
    #[error("failed to parse json: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, DittoError>;
