//! Compatibility shim for the legacy `gateway::budget` namespace.
//!
//! Prefer `crate::gateway::domain::budget` for new code.

pub use crate::gateway::domain::budget::*;
