//! Compatibility shim for the legacy `gateway::mysql_store` namespace.
//!
//! Prefer `crate::gateway::adapters::store::mysql` for new code.

pub use crate::gateway::adapters::store::mysql::*;
