//! Compatibility shim for the legacy `gateway::sqlite_store` namespace.
//!
//! Prefer `crate::gateway::adapters::store::sqlite` for new code.

pub use crate::gateway::adapters::store::sqlite::*;
