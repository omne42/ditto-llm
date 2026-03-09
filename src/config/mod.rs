//! Configuration layer.
//!
//! This namespace is the source of truth for dynamic user-controlled configuration.
//! Interactive editors and file mutation workflows live in `crate::apps` so the
//! configuration module stays focused on schema, defaults, and resolution helpers.

pub(crate) mod auth;
pub(crate) mod env;
pub(crate) mod http;
pub(crate) mod provider_config;
pub(crate) mod routing_policy;

pub use auth::{resolve_auth_token, resolve_auth_token_with_default_keys};
pub use env::{Env, parse_dotenv};
pub use provider_config::{
    ModelConfig, ProviderApi, ProviderAuth, ProviderCapabilities, ProviderConfig,
    ThinkingIntensity, filter_models_whitelist, merge_provider_config, normalize_string_list,
    select_model_config,
};
pub use routing_policy::{
    ProviderRoutingConfig, ResolvedRoutingPlan, ResolvedRoutingTarget, RoutingConfigFormat,
    RoutingContext, RoutingOverride, RoutingPhase, RoutingPolicy, RoutingPolicySource,
    RoutingProviderProfile, RoutingStagePolicy, RoutingTarget,
};

#[cfg(any(
    feature = "anthropic",
    feature = "cohere",
    feature = "google",
    feature = "openai",
    feature = "openai-compatible",
    feature = "vertex",
))]
pub(crate) use auth::HttpAuth;
#[cfg(any(
    feature = "anthropic",
    feature = "cohere",
    feature = "google",
    feature = "openai",
    feature = "openai-compatible",
))]
pub(crate) use auth::resolve_provider_request_auth_required;
#[cfg(any(
    feature = "anthropic",
    feature = "cohere",
    feature = "google",
    feature = "openai",
    feature = "openai-compatible",
))]
pub(crate) use auth::{RequestAuth, resolve_provider_request_auth_optional};
#[cfg(any(
    feature = "anthropic",
    feature = "cohere",
    feature = "google",
    feature = "bedrock",
    feature = "vertex",
))]
pub(crate) use http::{DEFAULT_HTTP_TIMEOUT, build_http_client};
#[cfg(any(
    feature = "anthropic",
    feature = "cohere",
    feature = "google",
    feature = "openai",
    feature = "openai-compatible",
))]
pub(crate) use http::{apply_http_query_params, default_http_client, resolve_http_provider_config};
