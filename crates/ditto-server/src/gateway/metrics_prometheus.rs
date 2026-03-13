//! Compatibility shim for the legacy `gateway::metrics_prometheus` namespace.
//!
//! Prefer `crate::gateway::adapters::telemetry::prometheus` for new code.

pub use crate::gateway::adapters::telemetry::prometheus::*;
