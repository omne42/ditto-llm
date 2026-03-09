//! Gateway adapters layer.

pub mod backend;
pub mod cache;
pub mod state;
pub mod store;
#[cfg(feature = "gateway-metrics-prometheus")]
pub mod telemetry;
