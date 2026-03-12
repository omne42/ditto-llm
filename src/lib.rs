pub mod capabilities;
pub mod catalog;
pub mod config;
#[cfg(feature = "config-editing")]
pub mod config_editing;
pub mod contracts;
pub mod foundation;
pub mod object;
pub mod provider_options;
pub mod provider_transport;
pub mod runtime;
pub mod runtime_registry;
pub mod session_transport;

pub mod llm_core;
pub mod providers;
pub mod types;
pub mod utils;

#[cfg(feature = "agent")]
pub mod agent;
#[cfg(feature = "auth")]
pub mod auth;
#[cfg(feature = "gateway")]
pub mod gateway;
#[cfg(feature = "sdk")]
pub mod sdk;

// ROOT-NO-LOWLEVEL-ALIASES: keep crate root as a module index only. Low-level
// L0 owners stay under their explicit namespaces, including internal call sites.
