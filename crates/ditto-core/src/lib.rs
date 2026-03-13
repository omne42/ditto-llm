#![allow(unexpected_cfgs)]

pub mod capabilities;
pub mod catalog;
#[cfg_attr(
    not(any(
        feature = "anthropic",
        feature = "bedrock",
        feature = "cohere",
        feature = "google",
        feature = "openai",
        feature = "openai-compatible",
        feature = "vertex"
    )),
    allow(dead_code)
)]
pub mod config;
pub mod contracts;
pub mod foundation;
pub mod object;
pub mod provider_options;
#[cfg_attr(
    not(any(
        feature = "anthropic",
        feature = "bedrock",
        feature = "cohere",
        feature = "gateway",
        feature = "google",
        feature = "openai",
        feature = "openai-compatible",
        feature = "vertex"
    )),
    allow(dead_code)
)]
pub mod provider_transport;
pub mod runtime;
#[cfg_attr(
    not(any(feature = "openai", feature = "openai-compatible")),
    allow(dead_code)
)]
pub mod runtime_registry;
#[cfg_attr(not(feature = "streaming"), allow(dead_code))]
pub mod session_transport;

pub mod llm_core;
#[cfg_attr(
    not(any(feature = "openai", feature = "openai-compatible")),
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
