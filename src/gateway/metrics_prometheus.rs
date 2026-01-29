use std::collections::HashMap;

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
