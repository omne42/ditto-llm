use std::collections::HashMap;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct PrometheusMetricsConfig {
    pub max_key_series: usize,
    pub max_model_series: usize,
    pub max_backend_series: usize,
}

impl Default for PrometheusMetricsConfig {
    fn default() -> Self {
        Self {
            max_key_series: 1024,
            max_model_series: 1024,
            max_backend_series: 128,
        }
    }
}

#[derive(Debug)]
pub struct PrometheusMetrics {
    config: PrometheusMetricsConfig,

    proxy_requests_total: u64,
    proxy_requests_by_key: HashMap<String, u64>,
    proxy_requests_by_model: HashMap<String, u64>,

    proxy_cache_hits_total: u64,

    proxy_backend_attempts_total: HashMap<String, u64>,
    proxy_backend_success_total: HashMap<String, u64>,
    proxy_backend_failures_total: HashMap<String, u64>,
    proxy_backend_in_flight: HashMap<String, u64>,
    proxy_backend_request_duration_seconds: HashMap<String, DurationHistogram>,

    proxy_responses_by_status: HashMap<u16, u64>,
}

impl PrometheusMetrics {
    pub fn new(config: PrometheusMetricsConfig) -> Self {
        Self {
            config,
            proxy_requests_total: 0,
            proxy_requests_by_key: HashMap::new(),
            proxy_requests_by_model: HashMap::new(),
            proxy_cache_hits_total: 0,
            proxy_backend_attempts_total: HashMap::new(),
            proxy_backend_success_total: HashMap::new(),
            proxy_backend_failures_total: HashMap::new(),
            proxy_backend_in_flight: HashMap::new(),
            proxy_backend_request_duration_seconds: HashMap::new(),
            proxy_responses_by_status: HashMap::new(),
        }
    }

    pub fn record_proxy_request(&mut self, virtual_key_id: Option<&str>, model: Option<&str>) {
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
    }

    pub fn record_proxy_cache_hit(&mut self) {
        self.proxy_cache_hits_total = self.proxy_cache_hits_total.saturating_add(1);
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
            .or_insert_with(DurationHistogram::new)
            .observe(duration);
    }

    pub fn record_proxy_response_status(&mut self, status: u16) {
        *self.proxy_responses_by_status.entry(status).or_default() += 1;
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

        out.push_str("# HELP ditto_gateway_proxy_cache_hits_total Total proxy cache hits.\n");
        out.push_str("# TYPE ditto_gateway_proxy_cache_hits_total counter\n");
        out.push_str(&format!(
            "ditto_gateway_proxy_cache_hits_total {}\n",
            self.proxy_cache_hits_total
        ));

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

        out
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

#[derive(Clone, Debug, Default)]
struct DurationHistogram {
    buckets: [f64; 11],
    bucket_counts: [u64; 11],
    sum_seconds: f64,
    count: u64,
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
