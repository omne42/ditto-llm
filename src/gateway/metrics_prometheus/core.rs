use std::collections::HashMap;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct PrometheusMetricsConfig {
    pub max_key_series: usize,
    pub max_model_series: usize,
    pub max_backend_series: usize,
    pub max_path_series: usize,
}

impl Default for PrometheusMetricsConfig {
    fn default() -> Self {
        Self {
            max_key_series: 1024,
            max_model_series: 1024,
            max_backend_series: 128,
            max_path_series: 128,
        }
    }
}

#[derive(Debug)]
pub struct PrometheusMetrics {
    config: PrometheusMetricsConfig,

    proxy_requests_total: u64,
    proxy_requests_by_key: HashMap<String, u64>,
    proxy_requests_by_model: HashMap<String, u64>,
    proxy_requests_by_path: HashMap<String, u64>,

    proxy_rate_limited_total: u64,
    proxy_rate_limited_by_key: HashMap<String, u64>,
    proxy_rate_limited_by_model: HashMap<String, u64>,
    proxy_rate_limited_by_path: HashMap<String, u64>,

    proxy_guardrail_blocked_total: u64,
    proxy_guardrail_blocked_by_key: HashMap<String, u64>,
    proxy_guardrail_blocked_by_model: HashMap<String, u64>,
    proxy_guardrail_blocked_by_path: HashMap<String, u64>,

    proxy_budget_exceeded_total: u64,
    proxy_budget_exceeded_by_key: HashMap<String, u64>,
    proxy_budget_exceeded_by_model: HashMap<String, u64>,
    proxy_budget_exceeded_by_path: HashMap<String, u64>,

    proxy_cache_lookups_total: u64,
    proxy_cache_lookups_by_path: HashMap<String, u64>,

    proxy_cache_hits_total: u64,
    proxy_cache_hits_by_source: HashMap<String, u64>,
    proxy_cache_hits_by_path: HashMap<String, u64>,

    proxy_cache_misses_total: u64,
    proxy_cache_misses_by_path: HashMap<String, u64>,

    proxy_cache_stores_by_target: HashMap<String, u64>,
    proxy_cache_store_errors_by_target: HashMap<String, u64>,
    proxy_cache_purges_by_scope: HashMap<String, u64>,

    proxy_backend_attempts_total: HashMap<String, u64>,
    proxy_backend_success_total: HashMap<String, u64>,
    proxy_backend_failures_total: HashMap<String, u64>,
    proxy_backend_in_flight: HashMap<String, u64>,
    proxy_backend_request_duration_seconds: HashMap<String, DurationHistogram>,
    proxy_request_duration_seconds: HashMap<String, DurationHistogram>,
    proxy_request_duration_seconds_by_model: HashMap<String, DurationHistogram>,

    proxy_stream_connections: u64,
    proxy_stream_connections_by_backend: HashMap<String, u64>,
    proxy_stream_connections_by_path: HashMap<String, u64>,

    proxy_stream_bytes_total: u64,
    proxy_stream_bytes_by_backend: HashMap<String, u64>,
    proxy_stream_bytes_by_path: HashMap<String, u64>,

    proxy_stream_completed_total: u64,
    proxy_stream_completed_by_backend: HashMap<String, u64>,
    proxy_stream_completed_by_path: HashMap<String, u64>,

    proxy_stream_errors_total: u64,
    proxy_stream_errors_by_backend: HashMap<String, u64>,
    proxy_stream_errors_by_path: HashMap<String, u64>,

    proxy_stream_aborted_total: u64,
    proxy_stream_aborted_by_backend: HashMap<String, u64>,
    proxy_stream_aborted_by_path: HashMap<String, u64>,

    proxy_responses_by_status: HashMap<u16, u64>,
    proxy_responses_by_path_status: HashMap<String, HashMap<u16, u64>>,
    proxy_responses_by_backend_status: HashMap<String, HashMap<u16, u64>>,
    proxy_responses_by_model_status: HashMap<String, HashMap<u16, u64>>,
}

impl PrometheusMetrics {
    pub fn new(config: PrometheusMetricsConfig) -> Self {
        Self {
            config,
            proxy_requests_total: 0,
            proxy_requests_by_key: HashMap::new(),
            proxy_requests_by_model: HashMap::new(),
            proxy_requests_by_path: HashMap::new(),
            proxy_rate_limited_total: 0,
            proxy_rate_limited_by_key: HashMap::new(),
            proxy_rate_limited_by_model: HashMap::new(),
            proxy_rate_limited_by_path: HashMap::new(),
            proxy_guardrail_blocked_total: 0,
            proxy_guardrail_blocked_by_key: HashMap::new(),
            proxy_guardrail_blocked_by_model: HashMap::new(),
            proxy_guardrail_blocked_by_path: HashMap::new(),
            proxy_budget_exceeded_total: 0,
            proxy_budget_exceeded_by_key: HashMap::new(),
            proxy_budget_exceeded_by_model: HashMap::new(),
            proxy_budget_exceeded_by_path: HashMap::new(),
            proxy_cache_lookups_total: 0,
            proxy_cache_lookups_by_path: HashMap::new(),
            proxy_cache_hits_total: 0,
            proxy_cache_hits_by_source: HashMap::new(),
            proxy_cache_hits_by_path: HashMap::new(),
            proxy_cache_misses_total: 0,
            proxy_cache_misses_by_path: HashMap::new(),
            proxy_cache_stores_by_target: HashMap::new(),
            proxy_cache_store_errors_by_target: HashMap::new(),
            proxy_cache_purges_by_scope: HashMap::new(),
            proxy_backend_attempts_total: HashMap::new(),
            proxy_backend_success_total: HashMap::new(),
            proxy_backend_failures_total: HashMap::new(),
            proxy_backend_in_flight: HashMap::new(),
            proxy_backend_request_duration_seconds: HashMap::new(),
            proxy_request_duration_seconds: HashMap::new(),
            proxy_request_duration_seconds_by_model: HashMap::new(),
            proxy_stream_connections: 0,
            proxy_stream_connections_by_backend: HashMap::new(),
            proxy_stream_connections_by_path: HashMap::new(),
            proxy_stream_bytes_total: 0,
            proxy_stream_bytes_by_backend: HashMap::new(),
            proxy_stream_bytes_by_path: HashMap::new(),
            proxy_stream_completed_total: 0,
            proxy_stream_completed_by_backend: HashMap::new(),
            proxy_stream_completed_by_path: HashMap::new(),
            proxy_stream_errors_total: 0,
            proxy_stream_errors_by_backend: HashMap::new(),
            proxy_stream_errors_by_path: HashMap::new(),
            proxy_stream_aborted_total: 0,
            proxy_stream_aborted_by_backend: HashMap::new(),
            proxy_stream_aborted_by_path: HashMap::new(),
            proxy_responses_by_status: HashMap::new(),
            proxy_responses_by_path_status: HashMap::new(),
            proxy_responses_by_backend_status: HashMap::new(),
            proxy_responses_by_model_status: HashMap::new(),
        }
    }

    pub fn record_proxy_request(
        &mut self,
        virtual_key_id: Option<&str>,
        model: Option<&str>,
        path: &str,
    ) {
        self.proxy_requests_total = self.proxy_requests_total.saturating_add(1);
        bump_limited(
            &mut self.proxy_requests_by_key,
            virtual_key_id.unwrap_or("public"),
            self.config.max_key_series,
        );
        if let Some(model) = model {
            bump_limited(
                &mut self.proxy_requests_by_model,
                model,
                self.config.max_model_series,
            );
        }
        bump_limited(
            &mut self.proxy_requests_by_path,
            path,
            self.config.max_path_series,
        );
    }

    pub fn record_proxy_rate_limited(
        &mut self,
        virtual_key_id: Option<&str>,
        model: Option<&str>,
        path: &str,
    ) {
        self.proxy_rate_limited_total = self.proxy_rate_limited_total.saturating_add(1);
        bump_limited(
            &mut self.proxy_rate_limited_by_key,
            virtual_key_id.unwrap_or("public"),
            self.config.max_key_series,
        );
        if let Some(model) = model {
            bump_limited(
                &mut self.proxy_rate_limited_by_model,
                model,
                self.config.max_model_series,
            );
        }
        bump_limited(
            &mut self.proxy_rate_limited_by_path,
            path,
            self.config.max_path_series,
        );
    }

    pub fn record_proxy_guardrail_blocked(
        &mut self,
        virtual_key_id: Option<&str>,
        model: Option<&str>,
        path: &str,
    ) {
        self.proxy_guardrail_blocked_total = self.proxy_guardrail_blocked_total.saturating_add(1);
        bump_limited(
            &mut self.proxy_guardrail_blocked_by_key,
            virtual_key_id.unwrap_or("public"),
            self.config.max_key_series,
        );
        if let Some(model) = model {
            bump_limited(
                &mut self.proxy_guardrail_blocked_by_model,
                model,
                self.config.max_model_series,
            );
        }
        bump_limited(
            &mut self.proxy_guardrail_blocked_by_path,
            path,
            self.config.max_path_series,
        );
    }

    pub fn record_proxy_budget_exceeded(
        &mut self,
        virtual_key_id: Option<&str>,
        model: Option<&str>,
        path: &str,
    ) {
        self.proxy_budget_exceeded_total = self.proxy_budget_exceeded_total.saturating_add(1);
        bump_limited(
            &mut self.proxy_budget_exceeded_by_key,
            virtual_key_id.unwrap_or("public"),
            self.config.max_key_series,
        );
        if let Some(model) = model {
            bump_limited(
                &mut self.proxy_budget_exceeded_by_model,
                model,
                self.config.max_model_series,
            );
        }
        bump_limited(
            &mut self.proxy_budget_exceeded_by_path,
            path,
            self.config.max_path_series,
        );
    }

    pub fn record_proxy_cache_lookup(&mut self, path: &str) {
        self.proxy_cache_lookups_total = self.proxy_cache_lookups_total.saturating_add(1);
        bump_limited(
            &mut self.proxy_cache_lookups_by_path,
            path,
            self.config.max_path_series,
        );
    }

    pub fn record_proxy_cache_hit(&mut self) {
        self.proxy_cache_hits_total = self.proxy_cache_hits_total.saturating_add(1);
    }

    pub fn record_proxy_cache_hit_by_source(&mut self, source: &str) {
        bump_limited(&mut self.proxy_cache_hits_by_source, source, 8);
    }

    pub fn record_proxy_cache_hit_by_path(&mut self, path: &str) {
        bump_limited(
            &mut self.proxy_cache_hits_by_path,
            path,
            self.config.max_path_series,
        );
    }

    pub fn record_proxy_cache_miss(&mut self, path: &str) {
        self.proxy_cache_misses_total = self.proxy_cache_misses_total.saturating_add(1);
        bump_limited(
            &mut self.proxy_cache_misses_by_path,
            path,
            self.config.max_path_series,
        );
    }

    pub fn record_proxy_cache_store(&mut self, target: &str) {
        bump_limited(&mut self.proxy_cache_stores_by_target, target, 8);
    }

    pub fn record_proxy_cache_store_error(&mut self, target: &str) {
        bump_limited(&mut self.proxy_cache_store_errors_by_target, target, 8);
    }

    pub fn record_proxy_cache_purge(&mut self, scope: &str) {
        bump_limited(&mut self.proxy_cache_purges_by_scope, scope, 8);
    }

    pub fn record_proxy_backend_attempt(&mut self, backend: &str) {
        bump_limited(
            &mut self.proxy_backend_attempts_total,
            backend,
            self.config.max_backend_series,
        );
    }

    pub fn record_proxy_backend_success(&mut self, backend: &str) {
        bump_limited(
            &mut self.proxy_backend_success_total,
            backend,
            self.config.max_backend_series,
        );
    }

    pub fn record_proxy_backend_failure(&mut self, backend: &str) {
        bump_limited(
            &mut self.proxy_backend_failures_total,
            backend,
            self.config.max_backend_series,
        );
    }

    pub fn record_proxy_backend_in_flight_inc(&mut self, backend: &str) {
        if let Some(entry) = entry_limited(
            &mut self.proxy_backend_in_flight,
            backend,
            self.config.max_backend_series,
        ) {
            *entry = entry.saturating_add(1);
        }
    }

    pub fn record_proxy_backend_in_flight_dec(&mut self, backend: &str) {
        if let Some(entry) = entry_limited(
            &mut self.proxy_backend_in_flight,
            backend,
            self.config.max_backend_series,
        ) {
            *entry = entry.saturating_sub(1);
        }
    }

    pub fn observe_proxy_backend_request_duration(&mut self, backend: &str, duration: Duration) {
        if let Some(histogram) = entry_limited(
            &mut self.proxy_backend_request_duration_seconds,
            backend,
            self.config.max_backend_series,
        ) {
            histogram.observe(duration);
        }
    }

    pub fn observe_proxy_request_duration(&mut self, path: &str, duration: Duration) {
        if let Some(histogram) = entry_limited(
            &mut self.proxy_request_duration_seconds,
            path,
            self.config.max_path_series,
        ) {
            histogram.observe(duration);
        }
    }

    pub fn observe_proxy_request_duration_by_model(&mut self, model: &str, duration: Duration) {
        if let Some(histogram) = entry_limited(
            &mut self.proxy_request_duration_seconds_by_model,
            model,
            self.config.max_model_series,
        ) {
            histogram.observe(duration);
        }
    }

    pub fn record_proxy_stream_open(&mut self, backend: &str, path: &str) {
        self.proxy_stream_connections = self.proxy_stream_connections.saturating_add(1);

        if let Some(entry) = entry_limited(
            &mut self.proxy_stream_connections_by_backend,
            backend,
            self.config.max_backend_series,
        ) {
            *entry = entry.saturating_add(1);
        }

        if let Some(entry) = entry_limited(
            &mut self.proxy_stream_connections_by_path,
            path,
            self.config.max_path_series,
        ) {
            *entry = entry.saturating_add(1);
        }
    }

    pub fn record_proxy_stream_close(&mut self, backend: &str, path: &str) {
        self.proxy_stream_connections = self.proxy_stream_connections.saturating_sub(1);

        if let Some(entry) = entry_limited(
            &mut self.proxy_stream_connections_by_backend,
            backend,
            self.config.max_backend_series,
        ) {
            *entry = entry.saturating_sub(1);
        }

        if let Some(entry) = entry_limited(
            &mut self.proxy_stream_connections_by_path,
            path,
            self.config.max_path_series,
        ) {
            *entry = entry.saturating_sub(1);
        }
    }

    pub fn record_proxy_stream_bytes(&mut self, backend: &str, path: &str, bytes: u64) {
        self.proxy_stream_bytes_total = self.proxy_stream_bytes_total.saturating_add(bytes);
        add_limited(
            &mut self.proxy_stream_bytes_by_backend,
            backend,
            self.config.max_backend_series,
            bytes,
        );
        add_limited(
            &mut self.proxy_stream_bytes_by_path,
            path,
            self.config.max_path_series,
            bytes,
        );
    }

    pub fn record_proxy_stream_completed(&mut self, backend: &str, path: &str) {
        self.proxy_stream_completed_total = self.proxy_stream_completed_total.saturating_add(1);
        bump_limited(
            &mut self.proxy_stream_completed_by_backend,
            backend,
            self.config.max_backend_series,
        );
        bump_limited(
            &mut self.proxy_stream_completed_by_path,
            path,
            self.config.max_path_series,
        );
    }

    pub fn record_proxy_stream_error(&mut self, backend: &str, path: &str) {
        self.proxy_stream_errors_total = self.proxy_stream_errors_total.saturating_add(1);
        bump_limited(
            &mut self.proxy_stream_errors_by_backend,
            backend,
            self.config.max_backend_series,
        );
        bump_limited(
            &mut self.proxy_stream_errors_by_path,
            path,
            self.config.max_path_series,
        );
    }

    pub fn record_proxy_stream_aborted(&mut self, backend: &str, path: &str) {
        self.proxy_stream_aborted_total = self.proxy_stream_aborted_total.saturating_add(1);
        bump_limited(
            &mut self.proxy_stream_aborted_by_backend,
            backend,
            self.config.max_backend_series,
        );
        bump_limited(
            &mut self.proxy_stream_aborted_by_path,
            path,
            self.config.max_path_series,
        );
    }

    pub fn record_proxy_response_status(&mut self, status: u16) {
        *self.proxy_responses_by_status.entry(status).or_default() += 1;
    }

    pub fn record_proxy_response_status_by_path(&mut self, path: &str, status: u16) {
        self.record_proxy_response_status(status);
        if let Some(statuses) = entry_limited(
            &mut self.proxy_responses_by_path_status,
            path,
            self.config.max_path_series,
        ) {
            *statuses.entry(status).or_default() += 1;
        }
    }

    pub fn record_proxy_response_status_by_backend(&mut self, backend: &str, status: u16) {
        if let Some(statuses) = entry_limited(
            &mut self.proxy_responses_by_backend_status,
            backend,
            self.config.max_backend_series,
        ) {
            *statuses.entry(status).or_default() += 1;
        }
    }

    pub fn record_proxy_response_status_by_model(&mut self, model: &str, status: u16) {
        if let Some(statuses) = entry_limited(
            &mut self.proxy_responses_by_model_status,
            model,
            self.config.max_model_series,
        ) {
            *statuses.entry(status).or_default() += 1;
        }
    }

}
