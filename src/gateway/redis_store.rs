//! Compatibility shim for the legacy `gateway::redis_store` namespace.
//!
//! Prefer `crate::gateway::adapters::store::redis` for new code.

pub use crate::gateway::adapters::store::redis::*;
