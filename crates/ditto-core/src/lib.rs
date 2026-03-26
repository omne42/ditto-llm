pub mod resources;

pub mod capabilities;
pub mod catalog;
#[cfg_attr(
    not(any(
        feature = "provider-anthropic",
        feature = "provider-bedrock",
        feature = "provider-cohere",
        feature = "provider-google",
        feature = "provider-openai",
        feature = "provider-openai-compatible",
        feature = "provider-vertex"
    )),
    allow(dead_code)
)]
pub mod config;
pub mod contracts;
pub mod error;
pub mod object;
pub mod provider_options;
#[cfg_attr(
    not(any(
        feature = "provider-anthropic",
        feature = "provider-bedrock",
        feature = "provider-cohere",
        feature = "gateway",
        feature = "provider-google",
        feature = "provider-openai",
        feature = "provider-openai-compatible",
        feature = "provider-vertex"
    )),
    allow(dead_code)
)]
pub mod provider_transport;
pub mod runtime;
#[cfg_attr(
    not(any(feature = "provider-openai", feature = "provider-openai-compatible")),
    allow(dead_code)
)]
pub mod runtime_registry;
#[cfg_attr(not(feature = "cap-llm-streaming"), allow(dead_code))]
pub mod session_transport;

pub mod llm_core;
#[cfg_attr(
    not(any(feature = "provider-openai", feature = "provider-openai-compatible")),
    allow(dead_code)
)]
pub mod providers;
pub mod types;
pub mod utils;

#[cfg(feature = "agent")]
pub mod agent;
#[cfg(feature = "auth")]
pub mod auth;
#[cfg(feature = "sdk")]
pub mod sdk;
