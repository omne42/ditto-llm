//! Compatibility shim for the legacy `gateway::limits` namespace.
//!
//! Prefer `crate::gateway::domain::limits` for new code.

pub use crate::gateway::domain::limits::*;
