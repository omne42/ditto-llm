//! Gateway telemetry adapters.

#[cfg(feature = "gateway-metrics-prometheus")]
pub mod prometheus;
#[cfg(feature = "gateway-metrics-prometheus")]
pub use prometheus::{PrometheusMetrics, PrometheusMetricsConfig};
