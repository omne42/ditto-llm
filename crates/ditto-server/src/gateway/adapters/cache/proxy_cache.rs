use std::collections::{HashMap, VecDeque};

use axum::http::{HeaderMap, Method};
use bytes::Bytes;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug)]
pub struct ProxyCacheConfig {
    pub ttl_seconds: u64,
    pub max_entries: usize,
    pub max_body_bytes: usize,
    pub max_total_body_bytes: usize,
    pub streaming_enabled: bool,
    pub max_stream_body_bytes: usize,
}

impl Default for ProxyCacheConfig {
    fn default() -> Self {
        Self {
            ttl_seconds: 60,
            max_entries: 1024,
            max_body_bytes: 1024 * 1024,
            max_total_body_bytes: 64 * 1024 * 1024,
            streaming_enabled: false,
            max_stream_body_bytes: 1024 * 1024,
        }
    }
}

impl ProxyCacheConfig {
    pub(crate) fn streaming_cache_enabled(&self) -> bool {
        self.streaming_enabled && self.max_stream_body_bytes > 0
    }

    pub(crate) fn max_body_bytes_for_headers(&self, headers: &HeaderMap) -> usize {
        if is_sse_content_type(headers) {
            if self.streaming_cache_enabled() {
                self.max_stream_body_bytes
            } else {
                0
            }
        } else {
            self.max_body_bytes
        }
    }

    pub(crate) fn stream_recorder(&self) -> Option<ProxyCacheStreamRecorder> {
        self.streaming_cache_enabled()
            .then(|| ProxyCacheStreamRecorder::new(self.max_stream_body_bytes))
    }
}

#[derive(Debug)]
pub(crate) struct ProxyCacheStreamRecorder {
    max_body_bytes: usize,
    buffer: bytes::BytesMut,
    overflowed: bool,
}

impl ProxyCacheStreamRecorder {
    pub(crate) fn new(max_body_bytes: usize) -> Self {
        Self {
            max_body_bytes,
            buffer: bytes::BytesMut::new(),
            overflowed: max_body_bytes == 0,
        }
    }

    pub(crate) fn ingest(&mut self, chunk: &Bytes) {
        if self.overflowed {
            return;
        }

        let next_len = self.buffer.len().saturating_add(chunk.len());
        if next_len > self.max_body_bytes {
            self.buffer.clear();
            self.overflowed = true;
            return;
        }

        self.buffer.extend_from_slice(chunk.as_ref());
    }

    pub(crate) fn finish(self) -> Option<Bytes> {
        if self.overflowed {
            None
        } else {
            Some(self.buffer.freeze())
        }
    }
}

#[derive(Clone, Debug)]
pub struct CachedProxyResponse {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Bytes,
    pub backend: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProxyCacheEntryMetadata {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub scope: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub method: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_partition: Option<String>,
}

impl ProxyCacheEntryMetadata {
    pub fn new(
        scope: impl Into<String>,
        method: &Method,
        path_and_query: &str,
        model: Option<&str>,
        route_partition: Option<&str>,
    ) -> Self {
        Self {
            scope: normalize_required_string(scope.into()),
            method: method.as_str().to_ascii_uppercase(),
            path: normalize_path_string(path_and_query).unwrap_or_default(),
            model: normalize_optional_string(model),
            route_partition: normalize_optional_string(route_partition),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProxyCachePurgeSelector {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl ProxyCachePurgeSelector {
    pub fn into_normalized(mut self) -> Self {
        self.cache_key = normalize_optional_owned(self.cache_key);
        self.scope = normalize_optional_owned(self.scope);
        self.method = normalize_method_string(self.method);
        self.path = normalize_path_owned(self.path);
        self.model = normalize_optional_owned(self.model);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.cache_key.is_none()
            && self.scope.is_none()
            && self.method.is_none()
            && self.path.is_none()
            && self.model.is_none()
    }

    pub fn as_exact_cache_key(&self) -> Option<&str> {
        if self.scope.is_none()
            && self.method.is_none()
            && self.path.is_none()
            && self.model.is_none()
        {
            self.cache_key.as_deref()
        } else {
            None
        }
    }

    pub fn kind_label(&self) -> &'static str {
        if self.as_exact_cache_key().is_some() {
            "key"
        } else {
            "selector"
        }
    }

    pub fn matches(&self, cache_key: &str, metadata: &ProxyCacheEntryMetadata) -> bool {
        if let Some(expected) = self.cache_key.as_deref()
            && expected != cache_key
        {
            return false;
        }
        if let Some(expected) = self.scope.as_deref()
            && expected != metadata.scope
        {
            return false;
        }
        if let Some(expected) = self.method.as_deref()
            && expected != metadata.method
        {
            return false;
        }
        if let Some(expected) = self.path.as_deref()
            && expected != metadata.path
        {
            return false;
        }
        if let Some(expected) = self.model.as_deref()
            && metadata.model.as_deref() != Some(expected)
        {
            return false;
        }
        true
    }
}

#[derive(Clone, Debug)]
pub struct ProxyCacheStoredResponse {
    pub response: CachedProxyResponse,
    pub metadata: ProxyCacheEntryMetadata,
}

#[derive(Clone, Debug)]
struct CacheEntry {
    response: CachedProxyResponse,
    metadata: ProxyCacheEntryMetadata,
    expires_at: u64,
}

#[derive(Debug)]
pub struct ProxyResponseCache {
    config: ProxyCacheConfig,
    entries: HashMap<String, CacheEntry>,
    order: VecDeque<String>,
    total_body_bytes: usize,
    last_prune_at: Option<u64>,
    last_write_prune_at: Option<u64>,
}

impl ProxyResponseCache {
    fn remove_key_from_order(&mut self, key: &str) {
        if self.order.front().is_some_and(|candidate| candidate == key) {
            self.order.pop_front();
            return;
        }
        if self.order.back().is_some_and(|candidate| candidate == key) {
            self.order.pop_back();
            return;
        }
        if let Some(index) = self.order.iter().position(|candidate| candidate == key) {
            self.order.remove(index);
        }
    }

    fn move_key_to_back(&mut self, key: &str) {
        if self.order.back().is_some_and(|candidate| candidate == key) {
            return;
        }
        if let Some(index) = self.order.iter().position(|candidate| candidate == key) {
            if let Some(existing) = self.order.remove(index) {
                self.order.push_back(existing);
            }
            return;
        }
        self.order.push_back(key.to_string());
    }

    fn prune_expired(&mut self, now: u64) {
        if self.order.is_empty() {
            return;
        }

        let mut keep = VecDeque::with_capacity(self.order.len());
        while let Some(candidate) = self.order.pop_front() {
            match self.entries.get(&candidate) {
                Some(entry) if now >= entry.expires_at => {
                    if let Some(entry) = self.entries.remove(&candidate) {
                        self.total_body_bytes = self
                            .total_body_bytes
                            .saturating_sub(entry.response.body.len());
                    }
                }
                Some(_) => keep.push_back(candidate),
                None => {}
            }
        }
        self.order = keep;
    }

    pub fn new(config: ProxyCacheConfig) -> Self {
        Self {
            config,
            entries: HashMap::new(),
            order: VecDeque::new(),
            total_body_bytes: 0,
            last_prune_at: None,
            last_write_prune_at: None,
        }
    }

    fn maybe_prune_expired_on_read(&mut self, now: u64) {
        if self.last_prune_at == Some(now) {
            return;
        }
        self.prune_expired(now);
        self.last_prune_at = Some(now);
    }

    fn maybe_prune_expired_on_write(&mut self, now: u64) {
        if self.last_write_prune_at == Some(now) {
            return;
        }
        self.prune_expired(now);
        self.last_write_prune_at = Some(now);
    }

    pub fn get(&mut self, key: &str, now: u64) -> Option<CachedProxyResponse> {
        self.maybe_prune_expired_on_read(now);

        let (expired, response) = {
            let entry = self.entries.get(key)?;
            if now >= entry.expires_at {
                (true, None)
            } else {
                (false, Some(entry.response.clone()))
            }
        };
        if expired {
            let _ = self.remove(key);
            return None;
        }
        let response = response?;
        self.move_key_to_back(key);
        Some(response)
    }

    pub fn insert(&mut self, key: String, response: CachedProxyResponse, now: u64) {
        self.insert_with_metadata(key, response, ProxyCacheEntryMetadata::default(), now);
    }

    pub fn insert_with_metadata(
        &mut self,
        key: String,
        response: CachedProxyResponse,
        metadata: ProxyCacheEntryMetadata,
        now: u64,
    ) {
        if self.config.ttl_seconds == 0
            || self.config.max_entries == 0
            || self.config.max_body_bytes == 0
            || self.config.max_total_body_bytes == 0
        {
            return;
        }

        let body_len = response.body.len();
        let max_body_bytes = self.config.max_body_bytes_for_headers(&response.headers);
        if max_body_bytes == 0
            || body_len > max_body_bytes
            || body_len > self.config.max_total_body_bytes
        {
            let _ = self.remove(&key);
            return;
        }

        let expires_at = now.saturating_add(self.config.ttl_seconds);
        let entry = CacheEntry {
            response,
            metadata,
            expires_at,
        };

        let (was_present, old_body_len) = if let Some(existing) = self.entries.get_mut(&key) {
            let old_body_len = existing.response.body.len();
            *existing = entry;
            (true, Some(old_body_len))
        } else {
            self.entries.insert(key.clone(), entry);
            (false, None)
        };

        if let Some(old_body_len) = old_body_len {
            self.total_body_bytes = self.total_body_bytes.saturating_sub(old_body_len);
        }

        self.total_body_bytes = self.total_body_bytes.saturating_add(body_len);
        if was_present {
            self.move_key_to_back(&key);
        } else {
            self.order.push_back(key);
        }
        self.maybe_prune_expired_on_write(now);

        while self.entries.len() > self.config.max_entries
            || self.total_body_bytes > self.config.max_total_body_bytes
        {
            let Some(candidate) = self.order.pop_front() else {
                break;
            };
            if let Some(entry) = self.entries.remove(&candidate) {
                self.total_body_bytes = self
                    .total_body_bytes
                    .saturating_sub(entry.response.body.len());
            }
        }
    }

    pub fn remove(&mut self, key: &str) -> bool {
        let existed = if let Some(entry) = self.entries.remove(key) {
            self.total_body_bytes = self
                .total_body_bytes
                .saturating_sub(entry.response.body.len());
            true
        } else {
            false
        };
        if existed {
            self.remove_key_from_order(key);
        }
        existed
    }

    pub fn purge_matching(&mut self, selector: &ProxyCachePurgeSelector) -> u64 {
        if selector.is_empty() {
            return 0;
        }

        if let Some(cache_key) = selector.as_exact_cache_key() {
            return u64::from(self.remove(cache_key));
        }

        let keys: Vec<String> = self
            .entries
            .iter()
            .filter(|(cache_key, entry)| selector.matches(cache_key, &entry.metadata))
            .map(|(cache_key, _entry)| cache_key.clone())
            .collect();

        let deleted = keys.len() as u64;
        for cache_key in keys {
            let _ = self.remove(&cache_key);
        }
        deleted
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
        self.total_body_bytes = 0;
        self.last_prune_at = None;
        self.last_write_prune_at = None;
    }
}

fn is_sse_content_type(headers: &HeaderMap) -> bool {
    headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().starts_with("text/event-stream"))
}

fn normalize_optional_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn normalize_optional_owned(value: Option<String>) -> Option<String> {
    normalize_optional_string(value.as_deref())
}

fn normalize_required_string(value: String) -> String {
    value.trim().to_string()
}

fn normalize_method_string(value: Option<String>) -> Option<String> {
    normalize_optional_owned(value).map(|value| value.to_ascii_uppercase())
}

fn normalize_path_owned(value: Option<String>) -> Option<String> {
    value.as_deref().and_then(normalize_path_string)
}

fn normalize_path_string(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let path = value.split_once('?').map_or(value, |(path, _)| path).trim();
    if path.is_empty() {
        None
    } else {
        Some(path.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cached(body: &'static [u8]) -> CachedProxyResponse {
        CachedProxyResponse {
            status: 200,
            headers: HeaderMap::new(),
            body: Bytes::from_static(body),
            backend: "b".to_string(),
        }
    }

    fn metadata(path: &str, model: Option<&str>) -> ProxyCacheEntryMetadata {
        ProxyCacheEntryMetadata::new("vk:key-1", &Method::POST, path, model, None)
    }

    #[test]
    fn cache_enforces_ttl() {
        let mut cache = ProxyResponseCache::new(ProxyCacheConfig {
            ttl_seconds: 1,
            max_entries: 10,
            max_body_bytes: 1024,
            max_total_body_bytes: 1024,
            ..Default::default()
        });
        cache.insert("k".to_string(), cached(b"ok"), 10);

        assert!(cache.get("k", 10).is_some());
        assert!(cache.get("k", 11).is_none());
    }

    #[test]
    fn cache_does_not_retain_expired_entries_in_order() {
        let mut cache = ProxyResponseCache::new(ProxyCacheConfig {
            ttl_seconds: 1,
            max_entries: 10,
            max_body_bytes: 1024,
            max_total_body_bytes: 1024,
            ..Default::default()
        });
        cache.insert("k".to_string(), cached(b"ok"), 10);

        assert_eq!(cache.order.len(), 1);
        assert!(cache.get("k", 11).is_none());
        assert!(cache.order.is_empty());
    }

    #[test]
    fn cache_evicts_when_over_capacity() {
        let mut cache = ProxyResponseCache::new(ProxyCacheConfig {
            ttl_seconds: 60,
            max_entries: 2,
            max_body_bytes: 1024,
            max_total_body_bytes: 1024,
            ..Default::default()
        });

        for key in ["a", "b", "c"] {
            cache.insert(key.to_string(), cached(b"ok"), 0);
        }

        assert!(cache.get("a", 0).is_none());
        assert!(cache.get("b", 0).is_some());
        assert!(cache.get("c", 0).is_some());
    }

    #[test]
    fn cache_skips_entries_larger_than_max_body_bytes() {
        let mut cache = ProxyResponseCache::new(ProxyCacheConfig {
            ttl_seconds: 60,
            max_entries: 10,
            max_body_bytes: 2,
            max_total_body_bytes: 100,
            ..Default::default()
        });
        cache.insert("k".to_string(), cached(b"too big"), 0);
        assert!(cache.get("k", 0).is_none());
    }

    #[test]
    fn cache_evicts_until_under_total_body_bytes_budget() {
        let mut cache = ProxyResponseCache::new(ProxyCacheConfig {
            ttl_seconds: 60,
            max_entries: 10,
            max_body_bytes: 10,
            max_total_body_bytes: 3,
            ..Default::default()
        });

        cache.insert("a".to_string(), cached(b"aa"), 0);
        cache.insert("b".to_string(), cached(b"bb"), 0);

        assert!(cache.get("a", 0).is_none());
        assert!(cache.get("b", 0).is_some());
    }

    #[test]
    fn cache_replacing_entry_enforces_total_body_bytes() {
        let mut cache = ProxyResponseCache::new(ProxyCacheConfig {
            ttl_seconds: 60,
            max_entries: 10,
            max_body_bytes: 10,
            max_total_body_bytes: 3,
            ..Default::default()
        });
        cache.insert("a".to_string(), cached(b"aa"), 0);
        cache.insert("b".to_string(), cached(b"b"), 0);
        cache.insert("a".to_string(), cached(b"aaa"), 0);

        assert!(cache.get("a", 0).is_some());
        assert!(cache.get("b", 0).is_none());
    }

    #[test]
    fn cache_get_promotes_recency_for_eviction() {
        let mut cache = ProxyResponseCache::new(ProxyCacheConfig {
            ttl_seconds: 60,
            max_entries: 2,
            max_body_bytes: 10,
            max_total_body_bytes: 100,
            ..Default::default()
        });
        for key in ["a", "b"] {
            cache.insert(key.to_string(), cached(b"ok"), 0);
        }

        assert!(cache.get("a", 0).is_some());
        cache.insert("c".to_string(), cached(b"ok"), 0);

        assert!(cache.get("a", 0).is_some());
        assert!(cache.get("b", 0).is_none());
        assert!(cache.get("c", 0).is_some());
    }

    #[test]
    fn cache_insert_prunes_expired_before_capacity_eviction() {
        let mut cache = ProxyResponseCache::new(ProxyCacheConfig {
            ttl_seconds: 2,
            max_entries: 2,
            max_body_bytes: 10,
            max_total_body_bytes: 100,
            ..Default::default()
        });
        cache.insert("a".to_string(), cached(b"aa"), 0);
        cache.insert("b".to_string(), cached(b"bb"), 1);
        assert!(cache.get("a", 1).is_some());
        cache.insert("c".to_string(), cached(b"cc"), 2);

        assert!(cache.get("a", 2).is_none());
        assert!(cache.get("b", 2).is_some());
        assert!(cache.get("c", 2).is_some());
    }

    #[test]
    fn cache_oversized_replacement_clears_existing_entry() {
        let mut cache = ProxyResponseCache::new(ProxyCacheConfig {
            ttl_seconds: 60,
            max_entries: 10,
            max_body_bytes: 3,
            max_total_body_bytes: 10,
            ..Default::default()
        });

        cache.insert("k".to_string(), cached(b"ok"), 0);
        assert!(cache.get("k", 0).is_some());

        cache.insert("k".to_string(), cached(b"toolarge"), 0);

        assert!(cache.get("k", 0).is_none());
        assert_eq!(cache.total_body_bytes, 0);
    }

    #[test]
    fn cache_get_prunes_expired_entries_even_on_miss() {
        let mut cache = ProxyResponseCache::new(ProxyCacheConfig {
            ttl_seconds: 1,
            max_entries: 10,
            max_body_bytes: 10,
            max_total_body_bytes: 100,
            ..Default::default()
        });

        cache.insert("a".to_string(), cached(b"aa"), 0);
        cache.insert("b".to_string(), cached(b"bb"), 0);

        assert!(cache.get("missing", 2).is_none());
        assert!(cache.entries.is_empty());
        assert!(cache.order.is_empty());
        assert_eq!(cache.total_body_bytes, 0);
    }

    #[test]
    fn cache_get_miss_still_prunes_when_write_happened_same_second() {
        let mut cache = ProxyResponseCache::new(ProxyCacheConfig {
            ttl_seconds: 1,
            max_entries: 10,
            max_body_bytes: 10,
            max_total_body_bytes: 100,
            ..Default::default()
        });

        cache.insert("a".to_string(), cached(b"aa"), 0);
        cache.insert("b".to_string(), cached(b"bb"), 1);

        assert!(cache.get("missing", 1).is_none());
        assert!(!cache.entries.contains_key("a"));
        assert!(cache.entries.contains_key("b"));
    }

    #[test]
    fn cache_purge_matching_uses_and_semantics_for_selector_fields() {
        let mut cache = ProxyResponseCache::new(ProxyCacheConfig {
            ttl_seconds: 60,
            max_entries: 10,
            max_body_bytes: 128,
            max_total_body_bytes: 1024,
            ..Default::default()
        });

        cache.insert_with_metadata(
            "cache-a".to_string(),
            cached(b"a"),
            metadata("/v1/responses?stream=false", Some("gpt-4o-mini")),
            0,
        );
        cache.insert_with_metadata(
            "cache-b".to_string(),
            cached(b"b"),
            metadata("/v1/responses", Some("gpt-4o")),
            0,
        );

        let deleted = cache.purge_matching(
            &ProxyCachePurgeSelector {
                path: Some("/v1/responses".to_string()),
                model: Some("gpt-4o-mini".to_string()),
                ..Default::default()
            }
            .into_normalized(),
        );

        assert_eq!(deleted, 1);
        assert!(cache.get("cache-a", 0).is_none());
        assert!(cache.get("cache-b", 0).is_some());
    }

    #[test]
    fn proxy_cache_metadata_and_selector_normalize_inputs() {
        let metadata = ProxyCacheEntryMetadata::new(
            "  vk:key-1  ",
            &Method::POST,
            " /v1/responses?foo=bar ",
            Some("  gpt-4o-mini  "),
            Some("  route:abc  "),
        );
        assert_eq!(metadata.scope, "vk:key-1");
        assert_eq!(metadata.method, "POST");
        assert_eq!(metadata.path, "/v1/responses");
        assert_eq!(metadata.model.as_deref(), Some("gpt-4o-mini"));
        assert_eq!(metadata.route_partition.as_deref(), Some("route:abc"));

        let selector = ProxyCachePurgeSelector {
            method: Some(" post ".to_string()),
            path: Some(" /v1/responses?x=1 ".to_string()),
            model: Some(" gpt-4o-mini ".to_string()),
            ..Default::default()
        }
        .into_normalized();
        assert_eq!(selector.method.as_deref(), Some("POST"));
        assert_eq!(selector.path.as_deref(), Some("/v1/responses"));
        assert_eq!(selector.model.as_deref(), Some("gpt-4o-mini"));
    }

    #[test]
    fn stream_recorder_drops_oversized_streams() {
        let mut recorder = ProxyCacheStreamRecorder::new(4);
        recorder.ingest(&Bytes::from_static(b"ab"));
        recorder.ingest(&Bytes::from_static(b"cde"));
        assert!(recorder.finish().is_none());
    }

    #[test]
    fn streaming_cache_limit_is_separate_from_nonstream_limit() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "content-type",
            axum::http::HeaderValue::from_static("text/event-stream"),
        );

        let config = ProxyCacheConfig {
            max_body_bytes: 4,
            streaming_enabled: true,
            max_stream_body_bytes: 8,
            ..Default::default()
        };

        assert_eq!(config.max_body_bytes_for_headers(&HeaderMap::new()), 4);
        assert_eq!(config.max_body_bytes_for_headers(&headers), 8);
    }
}
