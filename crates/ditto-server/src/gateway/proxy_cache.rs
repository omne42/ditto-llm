//! Compatibility shim for the legacy `gateway::proxy_cache` namespace.
//!
//! Prefer `crate::gateway::adapters::cache::proxy_cache` for new code.

pub use crate::gateway::adapters::cache::proxy_cache::*;
