//! Compatibility shim for the legacy `gateway::proxy_backend` namespace.
//!
//! Prefer `crate::gateway::adapters::backend::proxy` for new code.

pub use crate::gateway::adapters::backend::proxy::*;
