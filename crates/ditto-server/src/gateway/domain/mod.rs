//! Gateway domain layer.

pub mod budget;
pub mod cache;
pub mod guardrails;
pub mod limits;
pub mod router;
pub mod store_ports;
pub mod store_types;

use super::{VirtualKeyConfig, hash64_fnv1a};

pub use super::{GatewayError, GatewayRequest, GatewayResponse};
pub use budget::{BudgetConfig, BudgetTracker};
pub use cache::{CacheConfig, ResponseCache};
pub use guardrails::GuardrailsConfig;
pub use limits::{LimitsConfig, RateLimiter};
pub use router::{RouteBackend, RouteRule, Router, RouterConfig};
pub use store_ports::{ProxyRequestIdempotencyStore, ProxyRequestIdempotencyStoreError};
pub use store_types::{
    AuditLogRecord, BudgetLedgerRecord, CostLedgerRecord, ProxyRequestFingerprint,
    ProxyRequestIdempotencyBeginOutcome, ProxyRequestIdempotencyRecord,
    ProxyRequestIdempotencyState, ProxyRequestReplayError, ProxyRequestReplayOutcome,
    ProxyRequestReplayResponse, StoredHttpHeader,
};
