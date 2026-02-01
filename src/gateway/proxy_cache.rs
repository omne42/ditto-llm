use std::collections::{HashMap, VecDeque};

use axum::http::HeaderMap;
use bytes::Bytes;

#[derive(Clone, Debug)]
pub struct ProxyCacheConfig {
    pub ttl_seconds: u64,
    pub max_entries: usize,
}

impl Default for ProxyCacheConfig {
    fn default() -> Self {
        Self {
            ttl_seconds: 60,
            max_entries: 1024,
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

#[derive(Clone, Debug)]
struct CacheEntry {
    response: CachedProxyResponse,
    expires_at: u64,
}

#[derive(Debug)]
pub struct ProxyResponseCache {
    config: ProxyCacheConfig,
    entries: HashMap<String, CacheEntry>,
    order: VecDeque<String>,
}

impl ProxyResponseCache {
    pub fn new(config: ProxyCacheConfig) -> Self {
        Self {
            config,
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    pub fn get(&mut self, key: &str, now: u64) -> Option<CachedProxyResponse> {
        let expires_at = self.entries.get(key)?.expires_at;
        if now >= expires_at {
            self.entries.remove(key);
            self.order.retain(|candidate| candidate != key);
            return None;
        }
        Some(self.entries.get(key)?.response.clone())
    }

    pub fn insert(&mut self, key: String, response: CachedProxyResponse, now: u64) {
        if self.config.ttl_seconds == 0 || self.config.max_entries == 0 {
            return;
        }

        use std::collections::hash_map::Entry;

        let expires_at = now.saturating_add(self.config.ttl_seconds);
        let entry = CacheEntry {
            response,
            expires_at,
        };

        match self.entries.entry(key.clone()) {
            Entry::Occupied(mut occupied) => {
                occupied.insert(entry);
                self.order.retain(|candidate| candidate != &key);
                self.order.push_back(key);
                return;
            }
            Entry::Vacant(vacant) => {
                vacant.insert(entry);
            }
        }

        self.order.push_back(key);

        while self.entries.len() > self.config.max_entries {
            let Some(candidate) = self.order.pop_front() else {
                break;
            };
            self.entries.remove(&candidate);
        }
    }

    pub fn remove(&mut self, key: &str) -> bool {
        let existed = self.entries.remove(key).is_some();
        if existed {
            self.order.retain(|candidate| candidate != key);
        }
        existed
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_enforces_ttl() {
        let mut cache = ProxyResponseCache::new(ProxyCacheConfig {
            ttl_seconds: 1,
            max_entries: 10,
        });
        cache.insert(
            "k".to_string(),
            CachedProxyResponse {
                status: 200,
                headers: HeaderMap::new(),
                body: Bytes::from_static(b"ok"),
                backend: "b".to_string(),
            },
            10,
        );

        assert!(cache.get("k", 10).is_some());
        assert!(cache.get("k", 11).is_none());
    }

    #[test]
    fn cache_does_not_retain_expired_entries_in_order() {
        let mut cache = ProxyResponseCache::new(ProxyCacheConfig {
            ttl_seconds: 1,
            max_entries: 10,
        });
        cache.insert(
            "k".to_string(),
            CachedProxyResponse {
                status: 200,
                headers: HeaderMap::new(),
                body: Bytes::from_static(b"ok"),
                backend: "b".to_string(),
            },
            10,
        );

        assert_eq!(cache.order.len(), 1);
        assert!(cache.get("k", 11).is_none());
        assert!(cache.order.is_empty());
    }

    #[test]
    fn cache_evicts_when_over_capacity() {
        let mut cache = ProxyResponseCache::new(ProxyCacheConfig {
            ttl_seconds: 60,
            max_entries: 2,
        });

        for key in ["a", "b", "c"] {
            cache.insert(
                key.to_string(),
                CachedProxyResponse {
                    status: 200,
                    headers: HeaderMap::new(),
                    body: Bytes::from_static(b"ok"),
                    backend: "b".to_string(),
                },
                0,
            );
        }

        assert!(cache.get("a", 0).is_none());
        assert!(cache.get("b", 0).is_some());
        assert!(cache.get("c", 0).is_some());
    }
}
