mod auth;
mod config;
mod env;
mod http;
mod openai_compatible;
mod openai_models;

#[cfg(test)]
mod tests;

pub use auth::{resolve_auth_token, resolve_auth_token_with_default_keys};
pub use config::{
    ModelConfig, ProviderAuth, ProviderCapabilities, ProviderConfig, ThinkingIntensity,
    filter_models_whitelist, normalize_string_list, select_model_config,
};
pub use env::{Env, parse_dotenv};
pub use openai_compatible::OpenAiCompatibleClient;
pub use openai_models::{OpenAiModelsProvider, Provider, list_available_models};

#[cfg(any(
    feature = "anthropic",
    feature = "bedrock",
    feature = "cohere",
    feature = "google",
    feature = "openai",
    feature = "openai-compatible",
    feature = "vertex",
))]
pub(crate) use auth::{HttpAuth, RequestAuth, resolve_request_auth_with_default_keys};
#[cfg(any(
    feature = "anthropic",
    feature = "bedrock",
    feature = "cohere",
    feature = "google",
    feature = "openai",
    feature = "openai-compatible",
    feature = "vertex",
))]
pub(crate) use http::{apply_http_query_params, build_http_client};
