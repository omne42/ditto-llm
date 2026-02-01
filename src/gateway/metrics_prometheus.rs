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

    proxy_responses_by_status: HashMap<u16, u64>,
    proxy_responses_by_path_status: HashMap<String, HashMap<u16, u64>>,
}

impl PrometheusMetrics {
    pub fn new(config: PrometheusMetricsConfig) -> Self {
        Self {
            config,
            proxy_requests_total: 0,
            proxy_requests_by_key: HashMap::new(),
            proxy_requests_by_model: HashMap::new(),
            proxy_requests_by_path: HashMap::new(),
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
            proxy_responses_by_status: HashMap::new(),
            proxy_responses_by_path_status: HashMap::new(),
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
        let backend = limit_label(
            backend,
            &mut self.proxy_backend_in_flight,
            self.config.max_backend_series,
        );
        *self.proxy_backend_in_flight.entry(backend).or_default() += 1;
    }

    pub fn record_proxy_backend_in_flight_dec(&mut self, backend: &str) {
        let backend = limit_label(
            backend,
            &mut self.proxy_backend_in_flight,
            self.config.max_backend_series,
        );
        let entry = self.proxy_backend_in_flight.entry(backend).or_default();
        *entry = entry.saturating_sub(1);
    }

    pub fn observe_proxy_backend_request_duration(&mut self, backend: &str, duration: Duration) {
        let backend = limit_label(
            backend,
            &mut self.proxy_backend_request_duration_seconds,
            self.config.max_backend_series,
        );
        self.proxy_backend_request_duration_seconds
            .entry(backend)
            .or_default()
            .observe(duration);
    }

    pub fn observe_proxy_request_duration(&mut self, path: &str, duration: Duration) {
        let path = limit_label(
            path,
            &mut self.proxy_request_duration_seconds,
            self.config.max_path_series,
        );
        self.proxy_request_duration_seconds
            .entry(path)
            .or_default()
            .observe(duration);
    }

    pub fn record_proxy_response_status(&mut self, status: u16) {
        *self.proxy_responses_by_status.entry(status).or_default() += 1;
    }

    pub fn record_proxy_response_status_by_path(&mut self, path: &str, status: u16) {
        self.record_proxy_response_status(status);
        let path = limit_label(
            path,
            &mut self.proxy_responses_by_path_status,
            self.config.max_path_series,
        );
        *self
            .proxy_responses_by_path_status
            .entry(path)
            .or_default()
            .entry(status)
            .or_default() += 1;
    }

    pub fn render(&self) -> String {
        let mut out = String::new();

        out.push_str("# HELP ditto_gateway_proxy_requests_total Total proxy requests.\n");
        out.push_str("# TYPE ditto_gateway_proxy_requests_total counter\n");
        out.push_str(&format!(
            "ditto_gateway_proxy_requests_total {}\n",
            self.proxy_requests_total
        ));

        write_counter_map(
            &mut out,
            "ditto_gateway_proxy_requests_by_key_total",
            "Proxy requests grouped by virtual key id.",
            "virtual_key_id",
            &self.proxy_requests_by_key,
        );

        write_counter_map(
            &mut out,
            "ditto_gateway_proxy_requests_by_model_total",
            "Proxy requests grouped by model.",
            "model",
            &self.proxy_requests_by_model,
        );

        write_counter_map(
            &mut out,
            "ditto_gateway_proxy_requests_by_path_total",
            "Proxy requests grouped by OpenAI path.",
            "path",
            &self.proxy_requests_by_path,
        );

        out.push_str("# HELP ditto_gateway_proxy_cache_lookups_total Total proxy cache lookups.\n");
        out.push_str("# TYPE ditto_gateway_proxy_cache_lookups_total counter\n");
        out.push_str(&format!(
            "ditto_gateway_proxy_cache_lookups_total {}\n",
            self.proxy_cache_lookups_total
        ));

        write_counter_map(
            &mut out,
            "ditto_gateway_proxy_cache_lookups_by_path_total",
            "Proxy cache lookups grouped by OpenAI path.",
            "path",
            &self.proxy_cache_lookups_by_path,
        );

        out.push_str("# HELP ditto_gateway_proxy_cache_hits_total Total proxy cache hits.\n");
        out.push_str("# TYPE ditto_gateway_proxy_cache_hits_total counter\n");
        out.push_str(&format!(
            "ditto_gateway_proxy_cache_hits_total {}\n",
            self.proxy_cache_hits_total
        ));

        write_counter_map(
            &mut out,
            "ditto_gateway_proxy_cache_hits_by_source_total",
            "Proxy cache hits grouped by cache source.",
            "source",
            &self.proxy_cache_hits_by_source,
        );

        write_counter_map(
            &mut out,
            "ditto_gateway_proxy_cache_hits_by_path_total",
            "Proxy cache hits grouped by OpenAI path.",
            "path",
            &self.proxy_cache_hits_by_path,
        );

        out.push_str("# HELP ditto_gateway_proxy_cache_misses_total Total proxy cache misses.\n");
        out.push_str("# TYPE ditto_gateway_proxy_cache_misses_total counter\n");
        out.push_str(&format!(
            "ditto_gateway_proxy_cache_misses_total {}\n",
            self.proxy_cache_misses_total
        ));

        write_counter_map(
            &mut out,
            "ditto_gateway_proxy_cache_misses_by_path_total",
            "Proxy cache misses grouped by OpenAI path.",
            "path",
            &self.proxy_cache_misses_by_path,
        );

        write_counter_map(
            &mut out,
            "ditto_gateway_proxy_cache_stores_total",
            "Total proxy cache stores.",
            "target",
            &self.proxy_cache_stores_by_target,
        );

        write_counter_map(
            &mut out,
            "ditto_gateway_proxy_cache_store_errors_total",
            "Total proxy cache store errors.",
            "target",
            &self.proxy_cache_store_errors_by_target,
        );

        write_counter_map(
            &mut out,
            "ditto_gateway_proxy_cache_purges_total",
            "Total proxy cache purges.",
            "scope",
            &self.proxy_cache_purges_by_scope,
        );

        write_counter_map(
            &mut out,
            "ditto_gateway_proxy_backend_attempts_total",
            "Total proxy backend attempts.",
            "backend",
            &self.proxy_backend_attempts_total,
        );
        write_counter_map(
            &mut out,
            "ditto_gateway_proxy_backend_success_total",
            "Total proxy backend success responses.",
            "backend",
            &self.proxy_backend_success_total,
        );
        write_counter_map(
            &mut out,
            "ditto_gateway_proxy_backend_failures_total",
            "Total proxy backend failures (network/retryable status).",
            "backend",
            &self.proxy_backend_failures_total,
        );

        write_gauge_map(
            &mut out,
            "ditto_gateway_proxy_backend_in_flight",
            "In-flight proxy backend requests.",
            "backend",
            &self.proxy_backend_in_flight,
        );

        write_histogram_map(
            &mut out,
            "ditto_gateway_proxy_backend_request_duration_seconds",
            "Proxy backend request duration in seconds.",
            "backend",
            &self.proxy_backend_request_duration_seconds,
        );

        write_histogram_map(
            &mut out,
            "ditto_gateway_proxy_request_duration_seconds",
            "Proxy request duration in seconds.",
            "path",
            &self.proxy_request_duration_seconds,
        );

        out.push_str(
            "# HELP ditto_gateway_proxy_responses_total Total proxy responses by HTTP status.\n",
        );
        out.push_str("# TYPE ditto_gateway_proxy_responses_total counter\n");
        let mut statuses: Vec<(u16, u64)> = self
            .proxy_responses_by_status
            .iter()
            .map(|(k, v)| (*k, *v))
            .collect();
        statuses.sort_by_key(|(k, _)| *k);
        for (status, value) in statuses {
            out.push_str(&format!(
                "ditto_gateway_proxy_responses_total{{status=\"{}\"}} {}\n",
                status, value
            ));
        }

        out.push_str(
            "# HELP ditto_gateway_proxy_responses_by_path_status_total Total proxy responses grouped by path and status.\n",
        );
        out.push_str("# TYPE ditto_gateway_proxy_responses_by_path_status_total counter\n");
        let mut path_entries: Vec<_> = self.proxy_responses_by_path_status.iter().collect();
        path_entries.sort_by(|(a, _), (b, _)| a.cmp(b));
        for (path, status_map) in path_entries {
            let mut status_entries: Vec<_> = status_map.iter().collect();
            status_entries.sort_by_key(|(status, _)| *status);
            for (status, count) in status_entries {
                out.push_str(&format!(
                    "ditto_gateway_proxy_responses_by_path_status_total{{path=\"{}\",status=\"{}\"}} {count}\n",
                    escape_label_value(path),
                    status
                ));
            }
        }

        out
    }
}

pub(crate) fn normalize_proxy_path_label(path_and_query: &str) -> String {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    let path = path.strip_suffix('/').unwrap_or(path);

    match path {
        "/v1/chat/completions"
        | "/v1/completions"
        | "/v1/embeddings"
        | "/v1/moderations"
        | "/v1/images/generations"
        | "/v1/audio/transcriptions"
        | "/v1/audio/translations"
        | "/v1/audio/speech"
        | "/v1/files"
        | "/v1/rerank"
        | "/v1/batches"
        | "/v1/models"
        | "/v1/responses"
        | "/v1/responses/compact" => path.to_string(),
        _ => {
            if path.starts_with("/v1/models/") {
                return "/v1/models/*".to_string();
            }
            if path.starts_with("/v1/batches/") {
                if path.ends_with("/cancel") {
                    return "/v1/batches/*/cancel".to_string();
                }
                return "/v1/batches/*".to_string();
            }
            if path.starts_with("/v1/responses/") {
                return "/v1/responses/*".to_string();
            }

            "/v1/*".to_string()
        }
    }
}

fn bump_limited(map: &mut HashMap<String, u64>, key: &str, max_series: usize) {
    let key = if map.contains_key(key) || map.len() < max_series {
        key.to_string()
    } else {
        "__overflow__".to_string()
    };
    *map.entry(key).or_default() += 1;
}

fn limit_label<T>(key: &str, map: &mut HashMap<String, T>, max_series: usize) -> String {
    if map.contains_key(key) || map.len() < max_series {
        key.to_string()
    } else {
        "__overflow__".to_string()
    }
}

fn write_counter_map(
    out: &mut String,
    metric: &str,
    help: &str,
    label: &str,
    map: &HashMap<String, u64>,
) {
    out.push_str(&format!("# HELP {metric} {help}\n"));
    out.push_str(&format!("# TYPE {metric} counter\n"));

    let mut entries: Vec<(&String, &u64)> = map.iter().collect();
    entries.sort_by(|(a, _), (b, _)| a.cmp(b));
    for (value, count) in entries {
        out.push_str(&format!(
            "{metric}{{{label}=\"{}\"}} {count}\n",
            escape_label_value(value)
        ));
    }
}

fn write_gauge_map(
    out: &mut String,
    metric: &str,
    help: &str,
    label: &str,
    map: &HashMap<String, u64>,
) {
    out.push_str(&format!("# HELP {metric} {help}\n"));
    out.push_str(&format!("# TYPE {metric} gauge\n"));

    let mut entries: Vec<(&String, &u64)> = map.iter().collect();
    entries.sort_by(|(a, _), (b, _)| a.cmp(b));
    for (value, count) in entries {
        out.push_str(&format!(
            "{metric}{{{label}=\"{}\"}} {count}\n",
            escape_label_value(value)
        ));
    }
}

fn escape_label_value(value: &str) -> String {
    let mut out = String::new();
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '"' => out.push_str("\\\""),
            _ => out.push(c),
        }
    }
    out
}

#[derive(Clone, Debug)]
struct DurationHistogram {
    buckets: [f64; 11],
    bucket_counts: [u64; 11],
    sum_seconds: f64,
    count: u64,
}

impl Default for DurationHistogram {
    fn default() -> Self {
        Self::new()
    }
}

impl DurationHistogram {
    fn new() -> Self {
        Self {
            buckets: [
                0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
            ],
            bucket_counts: [0; 11],
            sum_seconds: 0.0,
            count: 0,
        }
    }

    fn observe(&mut self, duration: Duration) {
        let seconds = duration.as_secs_f64();
        self.sum_seconds += seconds;
        self.count = self.count.saturating_add(1);
        for (idx, bound) in self.buckets.iter().enumerate() {
            if seconds <= *bound {
                self.bucket_counts[idx] = self.bucket_counts[idx].saturating_add(1);
            }
        }
    }
}

fn write_histogram_map(
    out: &mut String,
    metric: &str,
    help: &str,
    label: &str,
    map: &HashMap<String, DurationHistogram>,
) {
    out.push_str(&format!("# HELP {metric} {help}\n"));
    out.push_str(&format!("# TYPE {metric} histogram\n"));

    let mut entries: Vec<(&String, &DurationHistogram)> = map.iter().collect();
    entries.sort_by(|(a, _), (b, _)| a.cmp(b));
    for (value, hist) in entries {
        let value = escape_label_value(value);
        for (idx, bound) in hist.buckets.iter().enumerate() {
            out.push_str(&format!(
                "{metric}_bucket{{{label}=\"{value}\",le=\"{bound}\"}} {}\n",
                hist.bucket_counts[idx]
            ));
        }
        out.push_str(&format!(
            "{metric}_bucket{{{label}=\"{value}\",le=\"+Inf\"}} {}\n",
            hist.count
        ));
        out.push_str(&format!(
            "{metric}_sum{{{label}=\"{value}\"}} {}\n",
            hist.sum_seconds
        ));
        out.push_str(&format!(
            "{metric}_count{{{label}=\"{value}\"}} {}\n",
            hist.count
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_label_values() {
        assert_eq!(escape_label_value("a"), "a");
        assert_eq!(escape_label_value("a\"b"), "a\\\"b");
        assert_eq!(escape_label_value("a\\b"), "a\\\\b");
        assert_eq!(escape_label_value("a\nb"), "a\\nb");
    }
}
