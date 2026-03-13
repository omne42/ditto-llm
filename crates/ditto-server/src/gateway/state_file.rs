//! Compatibility shim for the legacy `gateway::state_file` namespace.
//!
//! Prefer `crate::gateway::adapters::state::file` for new code.

pub use crate::gateway::adapters::state::file::*;
