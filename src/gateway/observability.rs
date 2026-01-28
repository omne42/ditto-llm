use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ObservabilitySnapshot {
    pub requests: u64,
    pub cache_hits: u64,
    pub rate_limited: u64,
    pub guardrail_blocked: u64,
    pub budget_exceeded: u64,
    pub backend_calls: u64,
}

#[derive(Debug, Default)]
pub struct Observability {
    snapshot: ObservabilitySnapshot,
}

impl Observability {
    pub fn record_request(&mut self) {
        self.snapshot.requests = self.snapshot.requests.saturating_add(1);
    }

    pub fn record_cache_hit(&mut self) {
        self.snapshot.cache_hits = self.snapshot.cache_hits.saturating_add(1);
    }

    pub fn record_rate_limited(&mut self) {
        self.snapshot.rate_limited = self.snapshot.rate_limited.saturating_add(1);
    }

    pub fn record_guardrail_blocked(&mut self) {
        self.snapshot.guardrail_blocked = self.snapshot.guardrail_blocked.saturating_add(1);
    }

    pub fn record_budget_exceeded(&mut self) {
        self.snapshot.budget_exceeded = self.snapshot.budget_exceeded.saturating_add(1);
    }

    pub fn record_backend_call(&mut self) {
        self.snapshot.backend_calls = self.snapshot.backend_calls.saturating_add(1);
    }

    pub fn snapshot(&self) -> ObservabilitySnapshot {
        self.snapshot.clone()
    }
}
