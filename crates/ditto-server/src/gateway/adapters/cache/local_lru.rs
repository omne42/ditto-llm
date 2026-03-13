use std::collections::{HashMap, VecDeque};

// GATEWAY-LOCAL-LRU-CACHE: small in-process cache adapter shared by gateway
// application code that needs bounded recency storage without owning cache
// mechanics inline.
#[derive(Debug)]
pub(crate) struct LocalLruCache<V> {
    entries: HashMap<String, V>,
    order: VecDeque<String>,
}

impl<V> Default for LocalLruCache<V> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }
}

impl<V: Clone> LocalLruCache<V> {
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

    pub(crate) fn get(&mut self, key: &str) -> Option<V> {
        let value = self.entries.get(key).cloned()?;
        self.move_key_to_back(key);
        Some(value)
    }

    pub(crate) fn insert(&mut self, key: String, value: V, max_entries: usize) {
        if max_entries == 0 {
            return;
        }

        let replaced = self.entries.insert(key.clone(), value).is_some();
        if replaced {
            self.move_key_to_back(&key);
        } else {
            self.order.push_back(key);
        }

        while self.entries.len() > max_entries {
            let Some(candidate) = self.order.pop_front() else {
                break;
            };
            self.entries.remove(&candidate);
        }
    }

    #[cfg(feature = "gateway-translation")]
    pub(crate) fn remove(&mut self, key: &str) -> Option<V> {
        let value = self.entries.remove(key)?;
        if let Some(index) = self.order.iter().position(|candidate| candidate == key) {
            let _ = self.order.remove(index);
        }
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::LocalLruCache;

    #[test]
    fn get_promotes_recency() {
        let mut cache = LocalLruCache::default();
        cache.insert("a".to_string(), 1, 2);
        cache.insert("b".to_string(), 2, 2);

        assert_eq!(cache.get("a"), Some(1));
        cache.insert("c".to_string(), 3, 2);

        assert_eq!(cache.get("a"), Some(1));
        assert_eq!(cache.get("b"), None);
        assert_eq!(cache.get("c"), Some(3));
    }

    #[test]
    fn hot_get_does_not_grow_order() {
        let mut cache = LocalLruCache::default();
        cache.insert("a".to_string(), 1, 10);

        for _ in 0..5 {
            assert_eq!(cache.get("a"), Some(1));
        }

        assert_eq!(cache.order.len(), 1);
        assert_eq!(cache.order.front().map(String::as_str), Some("a"));
    }
}
