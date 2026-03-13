//! Compatibility shim for the legacy `gateway::cache` namespace.
//!
//! Prefer `crate::gateway::domain::cache` for new code.

pub use crate::gateway::domain::cache::*;
