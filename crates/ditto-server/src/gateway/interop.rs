//! Compatibility shim for the legacy `gateway::interop` namespace.
//!
//! Prefer `crate::gateway::application::interop` for new code.

pub(crate) use crate::gateway::application::interop::*;
