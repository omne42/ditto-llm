use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::GatewayResponse;

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct CacheConfig {
    pub enabled: bool,
    pub ttl_seconds: Option<u64>,
}

#[derive(Clone, Debug)]
struct CacheEntry {
    response: GatewayResponse,
    expires_at: Option<u64>,
}

#[derive(Debug, Default)]
pub struct ResponseCache {
    entries: HashMap<String, CacheEntry>,
}

impl ResponseCache {
    pub fn get(&mut self, key: &str, now: u64) -> Option<GatewayResponse> {
        let entry = self.entries.get(key)?;
        if let Some(expires_at) = entry.expires_at {
            if now >= expires_at {
                self.entries.remove(key);
                return None;
            }
        }
        Some(entry.response.clone())
    }

    pub fn insert(
        &mut self,
        key: String,
        response: GatewayResponse,
        ttl_seconds: Option<u64>,
        now: u64,
    ) {
        let expires_at = ttl_seconds.map(|ttl| now.saturating_add(ttl));
        let entry = CacheEntry {
            response,
            expires_at,
        };
        self.entries.insert(key, entry);
    }
}
