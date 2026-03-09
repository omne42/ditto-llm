//! Compatibility shim for the legacy `gateway::postgres_store` namespace.
//!
//! Prefer `crate::gateway::adapters::store::postgres` for new code.

pub use crate::gateway::adapters::store::postgres::*;
