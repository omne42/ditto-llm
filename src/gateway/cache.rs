use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};

use super::GatewayResponse;

fn default_cache_max_entries() -> usize {
    1024
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CacheConfig {
    pub enabled: bool,
    pub ttl_seconds: Option<u64>,
    #[serde(default = "default_cache_max_entries")]
    pub max_entries: usize,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            ttl_seconds: None,
            max_entries: default_cache_max_entries(),
        }
    }
}

#[derive(Clone, Debug)]
struct CacheEntry {
    response: GatewayResponse,
    expires_at: Option<u64>,
}

#[derive(Debug, Default)]
struct ScopedCache {
    entries: HashMap<String, CacheEntry>,
    order: VecDeque<String>,
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
            cache.entries.remove(key);
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
        now: u64,
    ) {
        if ttl_seconds.is_some_and(|ttl| ttl == 0) || max_entries == 0 {
            return;
        }

        let expires_at = ttl_seconds.map(|ttl| now.saturating_add(ttl));
        let entry = CacheEntry {
            response,
            expires_at,
        };

        let cache = self.scopes.entry(scope.to_string()).or_default();
        cache.entries.insert(key.clone(), entry);
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
            cache.entries.remove(&candidate);
        }

        while cache.entries.len() > max_entries {
            let Some(candidate) = cache.order.pop_front() else {
                break;
            };
            cache.entries.remove(&candidate);
        }

        if cache.entries.is_empty() {
            self.scopes.remove(scope);
        }
    }
}
