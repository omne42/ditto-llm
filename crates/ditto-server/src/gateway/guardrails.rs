//! Compatibility shim for the legacy `gateway::guardrails` namespace.
//!
//! Prefer `crate::gateway::domain::guardrails` for new code.

pub use crate::gateway::domain::guardrails::*;
