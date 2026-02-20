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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    last_write_prune_at: Option<u64>,
}

impl ScopedCache {
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
                Some(entry) if entry.expires_at.is_some_and(|expires_at| now >= expires_at) => {
                    if let Some(entry) = self.entries.remove(&candidate) {
                        self.total_body_bytes =
                            self.total_body_bytes.saturating_sub(entry.body_bytes);
                    }
                }
                Some(_) => keep.push_back(candidate),
                None => {}
            }
        }
        self.order = keep;
    }

    fn maybe_prune_expired_on_write(&mut self, now: u64) {
        // Insert-heavy traffic can repeatedly touch the same scope in one second.
        // Throttle full-scope prune scans to once per second per scope.
        if self.last_write_prune_at == Some(now) {
            return;
        }
        self.prune_expired(now);
        self.last_write_prune_at = Some(now);
    }
}

#[derive(Debug, Default)]
pub struct ResponseCache {
    scopes: HashMap<String, ScopedCache>,
    last_prune_at: Option<u64>,
}

impl ResponseCache {
    fn maybe_prune_expired_on_read(&mut self, now: u64) {
        // Read-heavy traffic can keep expired entries resident for a long time.
        // Prune at most once per second to bound overhead while reclaiming memory.
        if self.last_prune_at == Some(now) {
            return;
        }
        self.scopes.retain(|_, cache| {
            cache.prune_expired(now);
            !cache.entries.is_empty()
        });
        self.last_prune_at = Some(now);
    }

    pub fn get(&mut self, scope: &str, key: &str, now: u64) -> Option<GatewayResponse> {
        self.maybe_prune_expired_on_read(now);

        let cache = self.scopes.get_mut(scope)?;
        let (expired, response) = {
            let entry = cache.entries.get(key)?;
            if entry.expires_at.is_some_and(|expires_at| now >= expires_at) {
                (true, None)
            } else {
                (false, Some(entry.response.clone()))
            }
        };
        if expired {
            if let Some(entry) = cache.entries.remove(key) {
                cache.total_body_bytes = cache.total_body_bytes.saturating_sub(entry.body_bytes);
            }
            cache.remove_key_from_order(key);
            if cache.entries.is_empty() {
                self.scopes.remove(scope);
            }
            return None;
        }

        let response = response?;
        cache.move_key_to_back(key);
        Some(response)
    }

    pub fn insert(
        &mut self,
        scope: &str,
        key: String,
        response: GatewayResponse,
        config: &CacheConfig,
        now: u64,
    ) {
        if !config.enabled
            || config.ttl_seconds.is_some_and(|ttl| ttl == 0)
            || config.max_entries == 0
            || config.max_body_bytes == 0
            || config.max_total_body_bytes == 0
        {
            return;
        }

        let body_bytes = response.content.len();
        if body_bytes > config.max_body_bytes || body_bytes > config.max_total_body_bytes {
            if let Some(cache) = self.scopes.get_mut(scope) {
                if let Some(entry) = cache.entries.remove(&key) {
                    cache.total_body_bytes =
                        cache.total_body_bytes.saturating_sub(entry.body_bytes);
                }
                cache.remove_key_from_order(&key);
                if cache.entries.is_empty() {
                    self.scopes.remove(scope);
                }
            }
            return;
        }

        let expires_at = config.ttl_seconds.map(|ttl| now.saturating_add(ttl));
        let entry = CacheEntry {
            response,
            expires_at,
            body_bytes,
        };

        if !self.scopes.contains_key(scope) {
            self.scopes
                .insert(scope.to_string(), ScopedCache::default());
        }
        let cache = self
            .scopes
            .get_mut(scope)
            .expect("scope cache must exist after insert");

        let old_body_bytes = if let Some(existing) = cache.entries.get_mut(&key) {
            let old_body_bytes = existing.body_bytes;
            *existing = entry;
            Some(old_body_bytes)
        } else {
            cache.entries.insert(key.clone(), entry);
            None
        };

        if let Some(old_body_bytes) = old_body_bytes {
            cache.total_body_bytes = cache.total_body_bytes.saturating_sub(old_body_bytes);
        }

        cache.total_body_bytes = cache.total_body_bytes.saturating_add(body_bytes);
        cache.move_key_to_back(&key);

        cache.maybe_prune_expired_on_write(now);

        while cache.entries.len() > config.max_entries
            || cache.total_body_bytes > config.max_total_body_bytes
        {
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

    pub fn remove_scope(&mut self, scope: &str) {
        self.scopes.remove(scope);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_entries_larger_than_max_body_bytes() {
        let mut cache = ResponseCache::default();
        let config = CacheConfig {
            enabled: true,
            ttl_seconds: Some(60),
            max_entries: 10,
            max_body_bytes: 4,
            max_total_body_bytes: 100,
        };
        cache.insert(
            "scope",
            "k".to_string(),
            GatewayResponse {
                content: "x".repeat(8),
                output_tokens: 0,
                backend: "b".to_string(),
                cached: false,
            },
            &config,
            0,
        );
        assert!(cache.get("scope", "k", 0).is_none());
    }

    #[test]
    fn evicts_until_under_total_body_bytes_budget() {
        let mut cache = ResponseCache::default();
        let config = CacheConfig {
            enabled: true,
            ttl_seconds: Some(60),
            max_entries: 10,
            max_body_bytes: 10,
            max_total_body_bytes: 4,
        };
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
                &config,
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
        let config = CacheConfig {
            enabled: true,
            ttl_seconds: Some(60),
            max_entries: 10,
            max_body_bytes: 10,
            max_total_body_bytes: 100,
        };
        cache.insert(
            "scope",
            "k".to_string(),
            GatewayResponse {
                content: "aa".to_string(),
                output_tokens: 0,
                backend: "b".to_string(),
                cached: false,
            },
            &config,
            0,
        );

        let config = CacheConfig {
            enabled: true,
            ttl_seconds: Some(60),
            max_entries: 10,
            max_body_bytes: 10,
            max_total_body_bytes: 3,
        };
        cache.insert(
            "scope",
            "k".to_string(),
            GatewayResponse {
                content: "aaaa".to_string(),
                output_tokens: 0,
                backend: "b".to_string(),
                cached: false,
            },
            &config,
            0,
        );

        assert!(cache.get("scope", "k", 0).is_none());
    }

    #[test]
    fn expired_entries_release_total_body_bytes() {
        let mut cache = ResponseCache::default();
        let config = CacheConfig {
            enabled: true,
            ttl_seconds: Some(1),
            max_entries: 10,
            max_body_bytes: 10,
            max_total_body_bytes: 100,
        };
        cache.insert(
            "scope",
            "k".to_string(),
            GatewayResponse {
                content: "aa".to_string(),
                output_tokens: 0,
                backend: "b".to_string(),
                cached: false,
            },
            &config,
            0,
        );

        assert!(cache.get("scope", "k", 1).is_none());

        let config = CacheConfig {
            enabled: true,
            ttl_seconds: Some(60),
            max_entries: 10,
            max_body_bytes: 10,
            max_total_body_bytes: 2,
        };
        cache.insert(
            "scope",
            "k2".to_string(),
            GatewayResponse {
                content: "aa".to_string(),
                output_tokens: 0,
                backend: "b".to_string(),
                cached: false,
            },
            &config,
            1,
        );

        assert!(cache.get("scope", "k2", 1).is_some());
    }

    #[test]
    fn insert_prunes_expired_entries_before_capacity_eviction() {
        let mut cache = ResponseCache::default();
        let long_ttl = CacheConfig {
            enabled: true,
            ttl_seconds: Some(60),
            max_entries: 2,
            max_body_bytes: 10,
            max_total_body_bytes: 100,
        };
        let short_ttl = CacheConfig {
            enabled: true,
            ttl_seconds: Some(1),
            max_entries: 2,
            max_body_bytes: 10,
            max_total_body_bytes: 100,
        };

        cache.insert(
            "scope",
            "a".to_string(),
            GatewayResponse {
                content: "aa".to_string(),
                output_tokens: 0,
                backend: "b".to_string(),
                cached: false,
            },
            &long_ttl,
            0,
        );
        cache.insert(
            "scope",
            "b".to_string(),
            GatewayResponse {
                content: "bb".to_string(),
                output_tokens: 0,
                backend: "b".to_string(),
                cached: false,
            },
            &short_ttl,
            0,
        );
        cache.insert(
            "scope",
            "c".to_string(),
            GatewayResponse {
                content: "cc".to_string(),
                output_tokens: 0,
                backend: "b".to_string(),
                cached: false,
            },
            &long_ttl,
            2,
        );

        assert!(cache.get("scope", "a", 2).is_some());
        assert!(cache.get("scope", "b", 2).is_none());
        assert!(cache.get("scope", "c", 2).is_some());
    }

    #[test]
    fn get_prunes_expired_entries_even_on_miss() {
        let mut cache = ResponseCache::default();
        let short_ttl = CacheConfig {
            enabled: true,
            ttl_seconds: Some(1),
            max_entries: 10,
            max_body_bytes: 10,
            max_total_body_bytes: 100,
        };
        let long_ttl = CacheConfig {
            enabled: true,
            ttl_seconds: Some(60),
            max_entries: 10,
            max_body_bytes: 10,
            max_total_body_bytes: 100,
        };

        cache.insert(
            "scope",
            "a".to_string(),
            GatewayResponse {
                content: "aa".to_string(),
                output_tokens: 0,
                backend: "b".to_string(),
                cached: false,
            },
            &short_ttl,
            0,
        );
        cache.insert(
            "scope",
            "b".to_string(),
            GatewayResponse {
                content: "bb".to_string(),
                output_tokens: 0,
                backend: "b".to_string(),
                cached: false,
            },
            &long_ttl,
            1,
        );

        assert!(cache.get("scope", "missing", 1).is_none());

        let scoped = cache.scopes.get("scope").expect("scope");
        assert!(!scoped.entries.contains_key("a"));
        assert!(scoped.entries.contains_key("b"));
        assert_eq!(scoped.order.len(), 1);
        assert_eq!(scoped.order.front().map(String::as_str), Some("b"));
        assert_eq!(scoped.total_body_bytes, 2);
    }

    #[test]
    fn replacing_entry_still_enforces_total_body_budget() {
        let mut cache = ResponseCache::default();
        let config = CacheConfig {
            enabled: true,
            ttl_seconds: Some(60),
            max_entries: 10,
            max_body_bytes: 10,
            max_total_body_bytes: 4,
        };

        cache.insert(
            "scope",
            "a".to_string(),
            GatewayResponse {
                content: "aa".to_string(),
                output_tokens: 0,
                backend: "b".to_string(),
                cached: false,
            },
            &config,
            0,
        );
        cache.insert(
            "scope",
            "b".to_string(),
            GatewayResponse {
                content: "bb".to_string(),
                output_tokens: 0,
                backend: "b".to_string(),
                cached: false,
            },
            &config,
            0,
        );
        cache.insert(
            "scope",
            "a".to_string(),
            GatewayResponse {
                content: "aaa".to_string(),
                output_tokens: 0,
                backend: "b".to_string(),
                cached: false,
            },
            &config,
            0,
        );

        assert!(cache.get("scope", "a", 0).is_some());
        assert!(cache.get("scope", "b", 0).is_none());
    }

    #[test]
    fn repeated_hot_get_does_not_grow_lru_queue() {
        let mut cache = ResponseCache::default();
        let config = CacheConfig {
            enabled: true,
            ttl_seconds: Some(60),
            max_entries: 10,
            max_body_bytes: 10,
            max_total_body_bytes: 100,
        };

        cache.insert(
            "scope",
            "a".to_string(),
            GatewayResponse {
                content: "aa".to_string(),
                output_tokens: 0,
                backend: "b".to_string(),
                cached: false,
            },
            &config,
            0,
        );

        for _ in 0..5 {
            assert!(cache.get("scope", "a", 0).is_some());
        }

        let scoped = cache.scopes.get("scope").expect("scope");
        assert_eq!(scoped.order.len(), 1);
        assert_eq!(scoped.order.front().map(String::as_str), Some("a"));
    }

    #[test]
    fn remove_scope_drops_all_entries_for_scope() {
        let mut cache = ResponseCache::default();
        let config = CacheConfig {
            enabled: true,
            ttl_seconds: Some(60),
            max_entries: 10,
            max_body_bytes: 10,
            max_total_body_bytes: 100,
        };

        cache.insert(
            "scope-a",
            "a".to_string(),
            GatewayResponse {
                content: "aa".to_string(),
                output_tokens: 0,
                backend: "b".to_string(),
                cached: false,
            },
            &config,
            0,
        );
        cache.insert(
            "scope-b",
            "b".to_string(),
            GatewayResponse {
                content: "bb".to_string(),
                output_tokens: 0,
                backend: "b".to_string(),
                cached: false,
            },
            &config,
            0,
        );

        cache.remove_scope("scope-a");

        assert!(cache.get("scope-a", "a", 0).is_none());
        assert!(cache.get("scope-b", "b", 0).is_some());
    }
}
