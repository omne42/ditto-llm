//! Canonical provider-options helper boundary.
//!
//! This module exposes a small canonical provider-options schema core plus thin
//! adapters for request DTOs and provider warning surfaces. It is intentionally
//! not a standalone request owner; `contracts` continues to own request/response
//! DTOs while this module keeps provider passthrough discoverable and explicit.

mod core;
mod envelope;
mod request;
mod support;

pub use core::{
    JsonSchemaFormat, ProviderOptions, ReasoningEffort, ReasoningSummary, ResponseFormat,
};

pub use envelope::ProviderOptionsEnvelope;
#[cfg(any(
    feature = "provider-anthropic",
    feature = "provider-bedrock",
    feature = "provider-cohere",
    feature = "provider-google",
    feature = "provider-openai",
    feature = "provider-openai-compatible",
    feature = "provider-vertex",
))]
pub(crate) use envelope::merge_provider_options_into_body;
#[allow(unused_imports)]
pub(crate) use envelope::select_provider_options_value;
pub use request::{
    request_parsed_provider_options, request_parsed_provider_options_for,
    request_provider_options_for, request_provider_options_value_for,
    request_with_provider_options, request_with_provider_response_format,
};
#[cfg(any(
    feature = "provider-anthropic",
    feature = "provider-bedrock",
    feature = "provider-google",
    feature = "provider-vertex",
))]
#[allow(unused_imports)]
pub(crate) use support::{ProviderOptionsSupport, warn_unsupported_provider_options};
