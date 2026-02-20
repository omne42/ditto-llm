impl PrometheusMetrics {
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

        out.push_str(
            "# HELP ditto_gateway_proxy_rate_limited_total Total proxy rate limited responses.\n",
        );
        out.push_str("# TYPE ditto_gateway_proxy_rate_limited_total counter\n");
        out.push_str(&format!(
            "ditto_gateway_proxy_rate_limited_total {}\n",
            self.proxy_rate_limited_total
        ));

        write_counter_map(
            &mut out,
            "ditto_gateway_proxy_rate_limited_by_key_total",
            "Proxy rate limited responses grouped by virtual key id.",
            "virtual_key_id",
            &self.proxy_rate_limited_by_key,
        );

        write_counter_map(
            &mut out,
            "ditto_gateway_proxy_rate_limited_by_model_total",
            "Proxy rate limited responses grouped by model.",
            "model",
            &self.proxy_rate_limited_by_model,
        );

        write_counter_map(
            &mut out,
            "ditto_gateway_proxy_rate_limited_by_path_total",
            "Proxy rate limited responses grouped by OpenAI path.",
            "path",
            &self.proxy_rate_limited_by_path,
        );

        out.push_str(
            "# HELP ditto_gateway_proxy_guardrail_blocked_total Total proxy guardrail blocked responses.\n",
        );
        out.push_str("# TYPE ditto_gateway_proxy_guardrail_blocked_total counter\n");
        out.push_str(&format!(
            "ditto_gateway_proxy_guardrail_blocked_total {}\n",
            self.proxy_guardrail_blocked_total
        ));

        write_counter_map(
            &mut out,
            "ditto_gateway_proxy_guardrail_blocked_by_key_total",
            "Proxy guardrail blocked responses grouped by virtual key id.",
            "virtual_key_id",
            &self.proxy_guardrail_blocked_by_key,
        );

        write_counter_map(
            &mut out,
            "ditto_gateway_proxy_guardrail_blocked_by_model_total",
            "Proxy guardrail blocked responses grouped by model.",
            "model",
            &self.proxy_guardrail_blocked_by_model,
        );

        write_counter_map(
            &mut out,
            "ditto_gateway_proxy_guardrail_blocked_by_path_total",
            "Proxy guardrail blocked responses grouped by OpenAI path.",
            "path",
            &self.proxy_guardrail_blocked_by_path,
        );

        out.push_str(
            "# HELP ditto_gateway_proxy_budget_exceeded_total Total proxy budget exceeded responses.\n",
        );
        out.push_str("# TYPE ditto_gateway_proxy_budget_exceeded_total counter\n");
        out.push_str(&format!(
            "ditto_gateway_proxy_budget_exceeded_total {}\n",
            self.proxy_budget_exceeded_total
        ));

        write_counter_map(
            &mut out,
            "ditto_gateway_proxy_budget_exceeded_by_key_total",
            "Proxy budget exceeded responses grouped by virtual key id.",
            "virtual_key_id",
            &self.proxy_budget_exceeded_by_key,
        );

        write_counter_map(
            &mut out,
            "ditto_gateway_proxy_budget_exceeded_by_model_total",
            "Proxy budget exceeded responses grouped by model.",
            "model",
            &self.proxy_budget_exceeded_by_model,
        );

        write_counter_map(
            &mut out,
            "ditto_gateway_proxy_budget_exceeded_by_path_total",
            "Proxy budget exceeded responses grouped by OpenAI path.",
            "path",
            &self.proxy_budget_exceeded_by_path,
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

        write_histogram_map(
            &mut out,
            "ditto_gateway_proxy_request_duration_seconds_by_model",
            "Proxy request duration in seconds grouped by model.",
            "model",
            &self.proxy_request_duration_seconds_by_model,
        );

        out.push_str("# HELP ditto_gateway_proxy_stream_connections Current active SSE streams.\n");
        out.push_str("# TYPE ditto_gateway_proxy_stream_connections gauge\n");
        out.push_str(&format!(
            "ditto_gateway_proxy_stream_connections {}\n",
            self.proxy_stream_connections
        ));

        write_gauge_map(
            &mut out,
            "ditto_gateway_proxy_stream_connections_by_backend",
            "Current active SSE streams grouped by backend.",
            "backend",
            &self.proxy_stream_connections_by_backend,
        );

        write_gauge_map(
            &mut out,
            "ditto_gateway_proxy_stream_connections_by_path",
            "Current active SSE streams grouped by OpenAI path.",
            "path",
            &self.proxy_stream_connections_by_path,
        );

        out.push_str("# HELP ditto_gateway_proxy_stream_bytes_total Total SSE bytes streamed.\n");
        out.push_str("# TYPE ditto_gateway_proxy_stream_bytes_total counter\n");
        out.push_str(&format!(
            "ditto_gateway_proxy_stream_bytes_total {}\n",
            self.proxy_stream_bytes_total
        ));

        write_counter_map(
            &mut out,
            "ditto_gateway_proxy_stream_bytes_by_backend_total",
            "SSE bytes streamed grouped by backend.",
            "backend",
            &self.proxy_stream_bytes_by_backend,
        );

        write_counter_map(
            &mut out,
            "ditto_gateway_proxy_stream_bytes_by_path_total",
            "SSE bytes streamed grouped by OpenAI path.",
            "path",
            &self.proxy_stream_bytes_by_path,
        );

        out.push_str(
            "# HELP ditto_gateway_proxy_stream_completed_total Total completed SSE streams.\n",
        );
        out.push_str("# TYPE ditto_gateway_proxy_stream_completed_total counter\n");
        out.push_str(&format!(
            "ditto_gateway_proxy_stream_completed_total {}\n",
            self.proxy_stream_completed_total
        ));

        write_counter_map(
            &mut out,
            "ditto_gateway_proxy_stream_completed_by_backend_total",
            "Completed SSE streams grouped by backend.",
            "backend",
            &self.proxy_stream_completed_by_backend,
        );

        write_counter_map(
            &mut out,
            "ditto_gateway_proxy_stream_completed_by_path_total",
            "Completed SSE streams grouped by OpenAI path.",
            "path",
            &self.proxy_stream_completed_by_path,
        );

        out.push_str("# HELP ditto_gateway_proxy_stream_errors_total Total errored SSE streams.\n");
        out.push_str("# TYPE ditto_gateway_proxy_stream_errors_total counter\n");
        out.push_str(&format!(
            "ditto_gateway_proxy_stream_errors_total {}\n",
            self.proxy_stream_errors_total
        ));

        write_counter_map(
            &mut out,
            "ditto_gateway_proxy_stream_errors_by_backend_total",
            "Errored SSE streams grouped by backend.",
            "backend",
            &self.proxy_stream_errors_by_backend,
        );

        write_counter_map(
            &mut out,
            "ditto_gateway_proxy_stream_errors_by_path_total",
            "Errored SSE streams grouped by OpenAI path.",
            "path",
            &self.proxy_stream_errors_by_path,
        );

        out.push_str(
            "# HELP ditto_gateway_proxy_stream_aborted_total Total aborted SSE streams.\n",
        );
        out.push_str("# TYPE ditto_gateway_proxy_stream_aborted_total counter\n");
        out.push_str(&format!(
            "ditto_gateway_proxy_stream_aborted_total {}\n",
            self.proxy_stream_aborted_total
        ));

        write_counter_map(
            &mut out,
            "ditto_gateway_proxy_stream_aborted_by_backend_total",
            "Aborted SSE streams grouped by backend.",
            "backend",
            &self.proxy_stream_aborted_by_backend,
        );

        write_counter_map(
            &mut out,
            "ditto_gateway_proxy_stream_aborted_by_path_total",
            "Aborted SSE streams grouped by OpenAI path.",
            "path",
            &self.proxy_stream_aborted_by_path,
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

        out.push_str(
            "# HELP ditto_gateway_proxy_responses_by_backend_status_total Total proxy responses grouped by backend and status.\n",
        );
        out.push_str("# TYPE ditto_gateway_proxy_responses_by_backend_status_total counter\n");
        let mut backend_entries: Vec<_> = self.proxy_responses_by_backend_status.iter().collect();
        backend_entries.sort_by(|(a, _), (b, _)| a.cmp(b));
        for (backend, status_map) in backend_entries {
            let mut status_entries: Vec<_> = status_map.iter().collect();
            status_entries.sort_by_key(|(status, _)| *status);
            for (status, count) in status_entries {
                out.push_str(&format!(
                    "ditto_gateway_proxy_responses_by_backend_status_total{{backend=\"{}\",status=\"{}\"}} {count}\n",
                    escape_label_value(backend),
                    status
                ));
            }
        }

        out.push_str(
            "# HELP ditto_gateway_proxy_responses_by_model_status_total Total proxy responses grouped by model and status.\n",
        );
        out.push_str("# TYPE ditto_gateway_proxy_responses_by_model_status_total counter\n");
        let mut model_entries: Vec<_> = self.proxy_responses_by_model_status.iter().collect();
        model_entries.sort_by(|(a, _), (b, _)| a.cmp(b));
        for (model, status_map) in model_entries {
            let mut status_entries: Vec<_> = status_map.iter().collect();
            status_entries.sort_by_key(|(status, _)| *status);
            for (status, count) in status_entries {
                out.push_str(&format!(
                    "ditto_gateway_proxy_responses_by_model_status_total{{model=\"{}\",status=\"{}\"}} {count}\n",
                    escape_label_value(model),
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
            if path.starts_with("/v1/files/") {
                if path.ends_with("/content") {
                    return "/v1/files/*/content".to_string();
                }
                return "/v1/files/*".to_string();
            }
            if path.starts_with("/v1/responses/") {
                return "/v1/responses/*".to_string();
            }

            "/v1/*".to_string()
        }
    }
}

const OVERFLOW_SERIES_LABEL: &str = "__overflow__";

fn entry_limited<'a, T: Default>(
    map: &'a mut HashMap<String, T>,
    key: &str,
    max_series: usize,
) -> Option<&'a mut T> {
    if max_series == 0 {
        return None;
    }

    if map.contains_key(key) {
        return map.get_mut(key);
    }

    if map.len() < max_series {
        return Some(map.entry(key.to_string()).or_default());
    }

    if map.contains_key(OVERFLOW_SERIES_LABEL) {
        return map.get_mut(OVERFLOW_SERIES_LABEL);
    }

    Some(map.entry(OVERFLOW_SERIES_LABEL.to_string()).or_default())
}

fn bump_limited(map: &mut HashMap<String, u64>, key: &str, max_series: usize) {
    if let Some(entry) = entry_limited(map, key, max_series) {
        *entry = entry.saturating_add(1);
    }
}

fn add_limited(map: &mut HashMap<String, u64>, key: &str, max_series: usize, delta: u64) {
    if let Some(entry) = entry_limited(map, key, max_series) {
        *entry = entry.saturating_add(delta);
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
    use std::time::Duration;

    #[test]
    fn escapes_label_values() {
        assert_eq!(escape_label_value("a"), "a");
        assert_eq!(escape_label_value("a\"b"), "a\\\"b");
        assert_eq!(escape_label_value("a\\b"), "a\\\\b");
        assert_eq!(escape_label_value("a\nb"), "a\\nb");
    }

    #[test]
    fn zero_series_limits_disable_labelled_metrics() {
        let mut metrics = PrometheusMetrics::new(PrometheusMetricsConfig {
            max_key_series: 0,
            max_model_series: 0,
            max_backend_series: 0,
            max_path_series: 0,
        });

        metrics.record_proxy_request(Some("vk-1"), Some("model-1"), "/v1/chat/completions");
        metrics.record_proxy_rate_limited(Some("vk-1"), Some("model-1"), "/v1/chat/completions");
        metrics.record_proxy_guardrail_blocked(
            Some("vk-1"),
            Some("model-1"),
            "/v1/chat/completions",
        );
        metrics.record_proxy_budget_exceeded(Some("vk-1"), Some("model-1"), "/v1/chat/completions");
        metrics.record_proxy_cache_lookup("/v1/chat/completions");
        metrics.record_proxy_cache_hit_by_source("memory");
        metrics.record_proxy_cache_hit_by_path("/v1/chat/completions");
        metrics.record_proxy_cache_miss("/v1/chat/completions");
        metrics.record_proxy_backend_attempt("backend-a");
        metrics.record_proxy_backend_success("backend-a");
        metrics.record_proxy_backend_failure("backend-a");
        metrics.record_proxy_backend_in_flight_inc("backend-a");
        metrics.record_proxy_backend_in_flight_dec("backend-a");
        metrics.observe_proxy_backend_request_duration("backend-a", Duration::from_millis(10));
        metrics.observe_proxy_request_duration("/v1/chat/completions", Duration::from_millis(10));
        metrics.observe_proxy_request_duration_by_model("model-1", Duration::from_millis(10));
        metrics.record_proxy_stream_open("backend-a", "/v1/chat/completions");
        metrics.record_proxy_stream_bytes("backend-a", "/v1/chat/completions", 128);
        metrics.record_proxy_stream_close("backend-a", "/v1/chat/completions");
        metrics.record_proxy_stream_completed("backend-a", "/v1/chat/completions");
        metrics.record_proxy_stream_error("backend-a", "/v1/chat/completions");
        metrics.record_proxy_stream_aborted("backend-a", "/v1/chat/completions");
        metrics.record_proxy_response_status_by_path("/v1/chat/completions", 200);
        metrics.record_proxy_response_status_by_backend("backend-a", 200);
        metrics.record_proxy_response_status_by_model("model-1", 200);

        assert_eq!(metrics.proxy_requests_total, 1);
        assert_eq!(metrics.proxy_rate_limited_total, 1);
        assert_eq!(metrics.proxy_guardrail_blocked_total, 1);
        assert_eq!(metrics.proxy_budget_exceeded_total, 1);
        assert_eq!(metrics.proxy_cache_lookups_total, 1);
        assert_eq!(metrics.proxy_stream_bytes_total, 128);
        assert_eq!(metrics.proxy_stream_connections, 0);
        assert_eq!(metrics.proxy_stream_completed_total, 1);
        assert_eq!(metrics.proxy_stream_errors_total, 1);
        assert_eq!(metrics.proxy_stream_aborted_total, 1);

        assert!(metrics.proxy_requests_by_key.is_empty());
        assert!(metrics.proxy_requests_by_model.is_empty());
        assert!(metrics.proxy_requests_by_path.is_empty());
        assert!(metrics.proxy_rate_limited_by_key.is_empty());
        assert!(metrics.proxy_rate_limited_by_model.is_empty());
        assert!(metrics.proxy_rate_limited_by_path.is_empty());
        assert!(metrics.proxy_guardrail_blocked_by_key.is_empty());
        assert!(metrics.proxy_guardrail_blocked_by_model.is_empty());
        assert!(metrics.proxy_guardrail_blocked_by_path.is_empty());
        assert!(metrics.proxy_budget_exceeded_by_key.is_empty());
        assert!(metrics.proxy_budget_exceeded_by_model.is_empty());
        assert!(metrics.proxy_budget_exceeded_by_path.is_empty());
        assert!(metrics.proxy_cache_lookups_by_path.is_empty());
        assert_eq!(metrics.proxy_cache_hits_by_source.get("memory"), Some(&1));
        assert!(metrics.proxy_cache_hits_by_path.is_empty());
        assert!(metrics.proxy_cache_misses_by_path.is_empty());
        assert!(metrics.proxy_backend_attempts_total.is_empty());
        assert!(metrics.proxy_backend_success_total.is_empty());
        assert!(metrics.proxy_backend_failures_total.is_empty());
        assert!(metrics.proxy_backend_in_flight.is_empty());
        assert!(metrics.proxy_backend_request_duration_seconds.is_empty());
        assert!(metrics.proxy_request_duration_seconds.is_empty());
        assert!(metrics.proxy_request_duration_seconds_by_model.is_empty());
        assert!(metrics.proxy_stream_connections_by_backend.is_empty());
        assert!(metrics.proxy_stream_connections_by_path.is_empty());
        assert!(metrics.proxy_stream_bytes_by_backend.is_empty());
        assert!(metrics.proxy_stream_bytes_by_path.is_empty());
        assert!(metrics.proxy_stream_completed_by_backend.is_empty());
        assert!(metrics.proxy_stream_completed_by_path.is_empty());
        assert!(metrics.proxy_stream_errors_by_backend.is_empty());
        assert!(metrics.proxy_stream_errors_by_path.is_empty());
        assert!(metrics.proxy_stream_aborted_by_backend.is_empty());
        assert!(metrics.proxy_stream_aborted_by_path.is_empty());
        assert!(metrics.proxy_responses_by_path_status.is_empty());
        assert!(metrics.proxy_responses_by_backend_status.is_empty());
        assert!(metrics.proxy_responses_by_model_status.is_empty());
    }

    #[test]
    fn overflow_series_is_reused_without_expanding_cardinality() {
        let mut map = HashMap::<String, u64>::new();
        bump_limited(&mut map, "first", 1);
        bump_limited(&mut map, "second", 1);
        bump_limited(&mut map, "third", 1);

        assert_eq!(map.len(), 2);
        assert_eq!(map.get("first"), Some(&1));
        assert_eq!(map.get(OVERFLOW_SERIES_LABEL), Some(&2));
    }
}
