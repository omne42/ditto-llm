//! Configuration layer.
//!
//! This namespace is the source of truth for dynamic user-controlled configuration.
//! Legacy `profile` paths remain available only as compatibility shims that re-export
//! the concrete implementations defined here.

pub(crate) mod auth;
pub(crate) mod editor;
pub(crate) mod env;
pub(crate) mod http;
pub(crate) mod provider_config;
pub(crate) mod routing_policy;

pub use auth::{resolve_auth_token, resolve_auth_token_with_default_keys};
pub use editor::{
    ConfigScope, ModelDeleteReport, ModelDeleteRequest, ModelListReport, ModelListRequest,
    ModelShowReport, ModelShowRequest, ModelSummary, ModelUpsertReport, ModelUpsertRequest,
    ProviderAuthType, ProviderDeleteReport, ProviderDeleteRequest, ProviderListReport,
    ProviderListRequest, ProviderNamespace, ProviderShowReport, ProviderShowRequest,
    ProviderSummary, ProviderUpsertReport, ProviderUpsertRequest,
    complete_model_upsert_request_interactive, complete_provider_upsert_request_interactive,
    delete_model_config, delete_provider_config, list_model_configs, list_provider_configs,
    show_model_config, show_provider_config, upsert_model_config, upsert_provider_config,
};
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
