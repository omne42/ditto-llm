//! Compatibility shim for the legacy `gateway::http_backend` namespace.
//!
//! Prefer `crate::gateway::adapters::backend::http` for new code.

pub use crate::gateway::adapters::backend::http::*;
