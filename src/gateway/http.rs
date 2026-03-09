//! Compatibility shim for the legacy `gateway::http` namespace.
//!
//! Prefer `crate::gateway::transport::http` for new code.

pub use crate::gateway::transport::http::*;
