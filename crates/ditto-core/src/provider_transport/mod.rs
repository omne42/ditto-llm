//! L0 boundary: provider_transport.
//!
//! This module owns provider-facing HTTP transport materialization:
//! client construction, base URL/query shaping, checked request execution,
//! and bounded response-body handling. Provider adapters should depend on
//! this boundary instead of reaching into generic utility namespaces.

mod config;
mod http;
mod policy;

pub use policy::{HttpClientPolicy, HttpResponseBodyPolicy, HttpTransportPolicy};

#[cfg(any(
    feature = "provider-anthropic",
    feature = "provider-cohere",
    feature = "provider-google",
    feature = "provider-bedrock",
    feature = "provider-vertex"
))]
#[allow(unused_imports)]
pub(crate) use config::DEFAULT_HTTP_TIMEOUT;
#[cfg(any(feature = "provider-bedrock", feature = "provider-vertex"))]
#[allow(unused_imports)]
pub(crate) use config::build_http_client;
#[cfg(any(
    feature = "provider-google",
    feature = "provider-cohere",
    feature = "provider-openai",
    feature = "provider-openai-compatible",
))]
#[allow(unused_imports)]
pub(crate) use config::default_http_client;
#[allow(unused_imports)]
pub(crate) use config::{
    ResolvedHttpProviderConfig, apply_http_query_params, build_http_client_with_policy,
    header_map_from_pairs, resolve_http_provider_config, resolve_http_provider_config_with_policy,
};
#[cfg(any(
    feature = "gateway",
    feature = "provider-openai",
    feature = "provider-openai-compatible"
))]
#[allow(unused_imports)]
pub(crate) use http::read_reqwest_body_bytes_bounded_with_content_length;
pub use http::response_text_truncated;
#[allow(unused_imports)]
pub(crate) use http::{
    send_checked, send_checked_bytes, send_checked_bytes_with_policy, send_checked_json,
    send_checked_json_with_policy, send_checked_with_policy,
};
