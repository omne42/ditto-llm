//! Configuration layer.
//!
//! This namespace is the source of truth for dynamic user-controlled configuration.
//! Side-effecting config editing and file mutation workflows live in
//! `crate::config_editing` behind the `config-editing` feature so this module
//! stays focused on schema, defaults, and resolution helpers.

pub(crate) mod auth;
pub(crate) mod env;
pub(crate) mod provider_config;
pub(crate) mod routing_policy;

pub use auth::{resolve_auth_token, resolve_auth_token_with_default_keys};
pub use env::{Env, parse_dotenv};
pub use provider_config::{
    ModelConfig, OpenAiCompatibleConfig, ProviderApi, ProviderAuth, ProviderCapabilities,
    ProviderConfig, ThinkingIntensity, filter_models_whitelist, merge_provider_config,
    normalize_string_list, select_model_config,
};
pub use routing_policy::{
    ProviderRoutingConfig, ResolvedRoutingPlan, ResolvedRoutingTarget, RoutingConfigFormat,
    RoutingContext, RoutingOverride, RoutingPhase, RoutingPolicy, RoutingPolicySource,
    RoutingProviderProfile, RoutingStagePolicy, RoutingTarget,
};

#[cfg(any(
    feature = "provider-anthropic",
    feature = "provider-cohere",
    feature = "provider-google",
    feature = "provider-openai",
    feature = "provider-openai-compatible",
    feature = "provider-vertex",
))]
pub(crate) use auth::HttpAuth;
#[cfg(any(
    feature = "provider-anthropic",
    feature = "provider-cohere",
    feature = "provider-google",
    feature = "provider-openai",
    feature = "provider-openai-compatible",
))]
pub(crate) use auth::resolve_provider_request_auth_required;
#[cfg(any(
    feature = "provider-anthropic",
    feature = "provider-cohere",
    feature = "provider-google",
    feature = "provider-openai",
    feature = "provider-openai-compatible",
))]
pub(crate) use auth::{RequestAuth, resolve_provider_request_auth_optional};
