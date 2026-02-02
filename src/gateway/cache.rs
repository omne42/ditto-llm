use std::collections::{HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};

use super::GatewayResponse;

fn default_cache_max_entries() -> usize {
    1024
}

fn default_cache_max_body_bytes() -> usize {
    1024 * 1024
}

fn default_cache_max_total_body_bytes() -> usize {
    64 * 1024 * 1024
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CacheConfig {
    pub enabled: bool,
    pub ttl_seconds: Option<u64>,
    #[serde(default = "default_cache_max_entries")]
    pub max_entries: usize,
    #[serde(default = "default_cache_max_body_bytes")]
    pub max_body_bytes: usize,
    #[serde(default = "default_cache_max_total_body_bytes")]
    pub max_total_body_bytes: usize,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            ttl_seconds: None,
            max_entries: default_cache_max_entries(),
            max_body_bytes: default_cache_max_body_bytes(),
            max_total_body_bytes: default_cache_max_total_body_bytes(),
        }
    }
}

#[derive(Clone, Debug)]
struct CacheEntry {
    response: GatewayResponse,
    expires_at: Option<u64>,
    body_bytes: usize,
}

#[derive(Debug, Default)]
struct ScopedCache {
    entries: HashMap<String, CacheEntry>,
    order: VecDeque<String>,
    total_body_bytes: usize,
}

#[derive(Debug, Default)]
pub struct ResponseCache {
    scopes: HashMap<String, ScopedCache>,
}

impl ResponseCache {
    pub fn get(&mut self, scope: &str, key: &str, now: u64) -> Option<GatewayResponse> {
        let cache = self.scopes.get_mut(scope)?;
        let expires_at = cache.entries.get(key)?.expires_at;

        if expires_at.is_some_and(|expires_at| now >= expires_at) {
            if let Some(entry) = cache.entries.remove(key) {
                cache.total_body_bytes = cache.total_body_bytes.saturating_sub(entry.body_bytes);
            }
            cache.order.retain(|candidate| candidate != key);
            if cache.entries.is_empty() {
                self.scopes.remove(scope);
            }
            return None;
        }

        let response = cache.entries.get(key)?.response.clone();
        cache.order.retain(|candidate| candidate != key);
        cache.order.push_back(key.to_string());
        Some(response)
    }

    pub fn insert(
        &mut self,
        scope: &str,
        key: String,
        response: GatewayResponse,
        ttl_seconds: Option<u64>,
        max_entries: usize,
        max_body_bytes: usize,
        max_total_body_bytes: usize,
        now: u64,
    ) {
        if ttl_seconds.is_some_and(|ttl| ttl == 0)
            || max_entries == 0
            || max_body_bytes == 0
            || max_total_body_bytes == 0
        {
            return;
        }

        let body_bytes = response.content.len();
        if body_bytes > max_body_bytes || body_bytes > max_total_body_bytes {
            if let Some(cache) = self.scopes.get_mut(scope) {
                if let Some(entry) = cache.entries.remove(&key) {
                    cache.total_body_bytes =
                        cache.total_body_bytes.saturating_sub(entry.body_bytes);
                }
                cache.order.retain(|candidate| candidate != &key);
                if cache.entries.is_empty() {
                    self.scopes.remove(scope);
                }
            }
            return;
        }

        let expires_at = ttl_seconds.map(|ttl| now.saturating_add(ttl));
        let entry = CacheEntry {
            response,
            expires_at,
            body_bytes,
        };

        let cache = self.scopes.entry(scope.to_string()).or_default();
        use std::collections::hash_map::Entry;

        match cache.entries.entry(key.clone()) {
            Entry::Occupied(mut occupied) => {
                let old_body_bytes = occupied.get().body_bytes;
                cache.total_body_bytes = cache.total_body_bytes.saturating_sub(old_body_bytes);
                occupied.insert(entry);
                cache.total_body_bytes = cache.total_body_bytes.saturating_add(body_bytes);
                cache.order.retain(|candidate| candidate != &key);
                cache.order.push_back(key);
                return;
            }
            Entry::Vacant(vacant) => {
                vacant.insert(entry);
            }
        }

        cache.total_body_bytes = cache.total_body_bytes.saturating_add(body_bytes);
        cache.order.retain(|candidate| candidate != &key);
        cache.order.push_back(key);

        while let Some(candidate) = cache.order.front().cloned() {
            let expired = match cache
                .entries
                .get(&candidate)
                .and_then(|entry| entry.expires_at)
            {
                Some(expires_at) => now >= expires_at,
                None => false,
            };
            if !expired {
                break;
            }
            cache.order.pop_front();
            if let Some(entry) = cache.entries.remove(&candidate) {
                cache.total_body_bytes = cache.total_body_bytes.saturating_sub(entry.body_bytes);
            }
        }

        while cache.entries.len() > max_entries || cache.total_body_bytes > max_total_body_bytes {
            let Some(candidate) = cache.order.pop_front() else {
                break;
            };
            if let Some(entry) = cache.entries.remove(&candidate) {
                cache.total_body_bytes = cache.total_body_bytes.saturating_sub(entry.body_bytes);
            }
        }

        if cache.entries.is_empty() {
            self.scopes.remove(scope);
        }
    }

    pub fn retain_scopes(&mut self, scopes: &HashSet<String>) {
        self.scopes.retain(|scope, _| scopes.contains(scope));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_entries_larger_than_max_body_bytes() {
        let mut cache = ResponseCache::default();
        cache.insert(
            "scope",
            "k".to_string(),
            GatewayResponse {
                content: "x".repeat(8),
                output_tokens: 0,
                backend: "b".to_string(),
                cached: false,
            },
            Some(60),
            10,
            4,
            100,
            0,
        );
        assert!(cache.get("scope", "k", 0).is_none());
    }

    #[test]
    fn evicts_until_under_total_body_bytes_budget() {
        let mut cache = ResponseCache::default();
        for key in ["a", "b", "c"] {
            cache.insert(
                "scope",
                key.to_string(),
                GatewayResponse {
                    content: key.repeat(2),
                    output_tokens: 0,
                    backend: "b".to_string(),
                    cached: false,
                },
                Some(60),
                10,
                10,
                4,
                0,
            );
        }

        assert!(cache.get("scope", "a", 0).is_none());
        assert!(cache.get("scope", "b", 0).is_some());
        assert!(cache.get("scope", "c", 0).is_some());
    }

    #[test]
    fn overwriting_entry_updates_total_body_bytes() {
        let mut cache = ResponseCache::default();
        cache.insert(
            "scope",
            "k".to_string(),
            GatewayResponse {
                content: "aa".to_string(),
                output_tokens: 0,
                backend: "b".to_string(),
                cached: false,
            },
            Some(60),
            10,
            10,
            100,
            0,
        );

        cache.insert(
            "scope",
            "k".to_string(),
            GatewayResponse {
                content: "aaaa".to_string(),
                output_tokens: 0,
                backend: "b".to_string(),
                cached: false,
            },
            Some(60),
            10,
            10,
            3,
            0,
        );

        assert!(cache.get("scope", "k", 0).is_none());
    }

    #[test]
    fn expired_entries_release_total_body_bytes() {
        let mut cache = ResponseCache::default();
        cache.insert(
            "scope",
            "k".to_string(),
            GatewayResponse {
                content: "aa".to_string(),
                output_tokens: 0,
                backend: "b".to_string(),
                cached: false,
            },
            Some(1),
            10,
            10,
            100,
            0,
        );

        assert!(cache.get("scope", "k", 1).is_none());

        cache.insert(
            "scope",
            "k2".to_string(),
            GatewayResponse {
                content: "aa".to_string(),
                output_tokens: 0,
                backend: "b".to_string(),
                cached: false,
            },
            Some(60),
            10,
            10,
            2,
            1,
        );

        assert!(cache.get("scope", "k2", 1).is_some());
    }
}
