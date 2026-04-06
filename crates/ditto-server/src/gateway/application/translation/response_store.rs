use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::Mutex;

const DEFAULT_TRANSLATION_RESPONSE_STORE_MAX_ENTRIES: usize = 128;
const DEFAULT_TRANSLATION_RESPONSE_STORE_MAX_TOTAL_BYTES: usize = 1024 * 1024;
const TRANSLATION_RESPONSE_HANDLE_PREFIX: &str = "resp_ditto_";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TranslationResponseOwner {
    pub(crate) virtual_key_id: Option<String>,
    pub(crate) tenant_id: Option<String>,
    pub(crate) project_id: Option<String>,
    pub(crate) user_id: Option<String>,
}

impl TranslationResponseOwner {
    pub(crate) fn matches(&self, requester: &Self) -> bool {
        [
            (&self.virtual_key_id, &requester.virtual_key_id),
            (&self.tenant_id, &requester.tenant_id),
            (&self.project_id, &requester.project_id),
            (&self.user_id, &requester.user_id),
        ]
        .into_iter()
        .all(|(stored, actual)| {
            stored
                .as_ref()
                .is_none_or(|stored| actual.as_ref() == Some(stored))
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct StoredTranslationResponse {
    pub(crate) owner: TranslationResponseOwner,
    pub(crate) response: Value,
    pub(crate) input_items: Vec<Value>,
}

#[derive(Debug)]
struct StoredTranslationResponseEntry {
    stored: StoredTranslationResponse,
    bytes: usize,
}

#[derive(Debug)]
struct TranslationResponseStoreState {
    entries: HashMap<String, StoredTranslationResponseEntry>,
    order: VecDeque<String>,
    total_bytes: usize,
    max_entries: usize,
    max_total_bytes: usize,
}

impl TranslationResponseStoreState {
    fn new(max_entries: usize, max_total_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            total_bytes: 0,
            max_entries,
            max_total_bytes,
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

    fn remove_entry(&mut self, key: &str) -> Option<StoredTranslationResponse> {
        let entry = self.entries.remove(key)?;
        self.total_bytes = self.total_bytes.saturating_sub(entry.bytes);
        if let Some(index) = self.order.iter().position(|candidate| candidate == key) {
            let _ = self.order.remove(index);
        }
        Some(entry.stored)
    }

    fn store(&mut self, response_id: String, stored: StoredTranslationResponse, bytes: usize) {
        if self.max_entries == 0 || self.max_total_bytes == 0 || bytes > self.max_total_bytes {
            return;
        }

        let _ = self.remove_entry(&response_id);
        self.total_bytes = self.total_bytes.saturating_add(bytes);
        self.entries.insert(
            response_id.clone(),
            StoredTranslationResponseEntry { stored, bytes },
        );
        self.order.push_back(response_id);

        while self.entries.len() > self.max_entries || self.total_bytes > self.max_total_bytes {
            let Some(candidate) = self.order.pop_front() else {
                break;
            };
            if let Some(entry) = self.entries.remove(&candidate) {
                self.total_bytes = self.total_bytes.saturating_sub(entry.bytes);
            }
        }
    }

    fn get(&mut self, response_id: &str) -> Option<StoredTranslationResponse> {
        let stored = self.entries.get(response_id)?.stored.clone();
        self.move_key_to_back(response_id);
        Some(stored)
    }
}

#[derive(Clone)]
pub(super) struct TranslationResponseStore {
    state: Arc<Mutex<TranslationResponseStoreState>>,
}

impl Default for TranslationResponseStore {
    fn default() -> Self {
        Self::with_limits(
            DEFAULT_TRANSLATION_RESPONSE_STORE_MAX_ENTRIES,
            DEFAULT_TRANSLATION_RESPONSE_STORE_MAX_TOTAL_BYTES,
        )
    }
}

impl TranslationResponseStore {
    fn with_limits(max_entries: usize, max_total_bytes: usize) -> Self {
        Self {
            state: Arc::new(Mutex::new(TranslationResponseStoreState::new(
                max_entries,
                max_total_bytes,
            ))),
        }
    }
}

pub(crate) fn gateway_scoped_response_id(backend_name: &str, response_id: &str) -> String {
    let backend_name = backend_name.trim();
    let response_id = response_id.trim();
    if backend_name.is_empty() || response_id.is_empty() {
        return response_id.to_string();
    }

    format!(
        "{TRANSLATION_RESPONSE_HANDLE_PREFIX}{}_{}_{}",
        backend_name.len(),
        backend_name,
        response_id
    )
}

fn parse_gateway_scoped_response_id(response_id: &str) -> Option<(&str, &str)> {
    let rest = response_id
        .trim()
        .strip_prefix(TRANSLATION_RESPONSE_HANDLE_PREFIX)?;
    let (backend_len, rest) = rest.split_once('_')?;
    let backend_len = backend_len.parse::<usize>().ok()?;
    if backend_len == 0 || rest.len() <= backend_len {
        return None;
    }

    let (backend_name, suffix) = rest.split_at(backend_len);
    let response_id = suffix.strip_prefix('_')?;
    if backend_name.is_empty() || response_id.is_empty() {
        return None;
    }

    Some((backend_name, response_id))
}

impl TranslationResponseStore {
    async fn store_response_record(
        &self,
        response_id: &str,
        owner: TranslationResponseOwner,
        response: Value,
        input_items: Vec<Value>,
    ) {
        let response_id = response_id.trim();
        if response_id.is_empty() {
            return;
        }

        let stored = StoredTranslationResponse {
            owner,
            response,
            input_items,
        };
        let entry_bytes = stored_translation_response_bytes(response_id, &stored);
        let mut state = self.state.lock().await;
        state.store(response_id.to_string(), stored, entry_bytes);
    }

    async fn stored_response(&self, response_id: &str) -> Option<StoredTranslationResponse> {
        self.state.lock().await.get(response_id.trim())
    }

    async fn delete_stored_response(&self, response_id: &str) -> bool {
        self.state
            .lock()
            .await
            .remove_entry(response_id.trim())
            .is_some()
    }
}

fn stored_translation_response_bytes(
    response_id: &str,
    stored: &StoredTranslationResponse,
) -> usize {
    response_id.len()
        + stored.owner.virtual_key_id.as_ref().map_or(0, String::len)
        + stored.owner.tenant_id.as_ref().map_or(0, String::len)
        + stored.owner.project_id.as_ref().map_or(0, String::len)
        + stored.owner.user_id.as_ref().map_or(0, String::len)
        + serde_json::to_vec(&stored.response).map_or(0, |bytes| bytes.len())
        + serde_json::to_vec(&stored.input_items).map_or(0, |bytes| bytes.len())
}

pub(crate) async fn delete_stored_response_from_translation_backends(
    backends: &HashMap<String, super::TranslationBackend>,
    response_id: &str,
    requester: &TranslationResponseOwner,
) -> Option<String> {
    let (backend_name, _) = parse_gateway_scoped_response_id(response_id)?;
    let backend = backends.get(backend_name)?;
    backend
        .delete_stored_response(response_id.trim(), requester)
        .await
        .then(|| backend_name.to_string())
}

pub(crate) async fn find_stored_response_from_translation_backends(
    backends: &HashMap<String, super::TranslationBackend>,
    response_id: &str,
    requester: &TranslationResponseOwner,
) -> Option<(String, StoredTranslationResponse)> {
    let (backend_name, _) = parse_gateway_scoped_response_id(response_id)?;
    let backend = backends.get(backend_name)?;
    backend
        .stored_response(response_id.trim(), requester)
        .await
        .map(|stored| (backend_name.to_string(), stored))
}

impl super::TranslationBackend {
    pub(crate) async fn store_response_record(
        &self,
        response_id: &str,
        owner: TranslationResponseOwner,
        response: Value,
        input_items: Vec<Value>,
    ) {
        self.runtime
            .response_store
            .store_response_record(response_id, owner, response, input_items)
            .await;
    }

    async fn stored_response(
        &self,
        response_id: &str,
        requester: &TranslationResponseOwner,
    ) -> Option<StoredTranslationResponse> {
        self.runtime
            .response_store
            .stored_response(response_id)
            .await
            .filter(|stored| stored.owner.matches(requester))
    }

    async fn delete_stored_response(
        &self,
        response_id: &str,
        requester: &TranslationResponseOwner,
    ) -> bool {
        matches!(
            self.runtime.response_store.stored_response(response_id).await,
            Some(stored) if stored.owner.matches(requester)
        ) && self
            .runtime
            .response_store
            .delete_stored_response(response_id)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn response_store_roundtrips_and_deletes_records() {
        let store = TranslationResponseStore::default();
        let response_id = gateway_scoped_response_id("primary", "resp_123");
        store
            .store_response_record(
                &response_id,
                TranslationResponseOwner {
                    virtual_key_id: Some("key-1".to_string()),
                    ..TranslationResponseOwner::default()
                },
                json!({ "id": response_id }),
                vec![json!({ "type": "message" })],
            )
            .await;

        let stored = store
            .stored_response(&response_id)
            .await
            .expect("stored response");
        assert_eq!(
            stored.response.get("id"),
            Some(&Value::String(response_id.clone()))
        );
        assert_eq!(stored.input_items.len(), 1);

        assert!(store.delete_stored_response(&response_id).await);
        assert!(store.stored_response(&response_id).await.is_none());
    }

    #[tokio::test]
    async fn response_store_rejects_other_virtual_keys() {
        let store = TranslationResponseStore::default();
        let response_id = gateway_scoped_response_id("primary", "resp_123");
        store
            .store_response_record(
                &response_id,
                TranslationResponseOwner {
                    virtual_key_id: Some("key-1".to_string()),
                    tenant_id: Some("tenant-a".to_string()),
                    ..TranslationResponseOwner::default()
                },
                json!({ "id": response_id }),
                vec![],
            )
            .await;

        let requester = TranslationResponseOwner {
            virtual_key_id: Some("key-2".to_string()),
            tenant_id: Some("tenant-a".to_string()),
            ..TranslationResponseOwner::default()
        };
        assert!(store.stored_response(&response_id).await.is_some());
        assert!(
            !store
                .stored_response(&response_id)
                .await
                .is_some_and(|stored| stored.owner.matches(&requester))
        );
    }

    #[test]
    fn gateway_scoped_response_ids_roundtrip() {
        let public_id = gateway_scoped_response_id("primary", "resp_123");
        assert_eq!(
            parse_gateway_scoped_response_id(&public_id),
            Some(("primary", "resp_123"))
        );
        assert!(parse_gateway_scoped_response_id("resp_123").is_none());
    }

    #[tokio::test]
    async fn response_store_skips_entries_larger_than_byte_budget() {
        let store = TranslationResponseStore::with_limits(4, 64);
        let response_id = gateway_scoped_response_id("primary", "resp_big");
        store
            .store_response_record(
                &response_id,
                TranslationResponseOwner::default(),
                json!({ "id": response_id, "output_text": "x".repeat(512) }),
                vec![json!({ "type": "message", "content": "x".repeat(512) })],
            )
            .await;

        assert!(store.stored_response("resp_big").await.is_none());
        assert!(store.stored_response(&response_id).await.is_none());
    }

    #[tokio::test]
    async fn response_store_evicts_oldest_entries_to_respect_byte_budget() {
        let store = TranslationResponseStore::with_limits(4, 220);
        let first_id = gateway_scoped_response_id("primary", "resp_first");
        let second_id = gateway_scoped_response_id("primary", "resp_second");

        store
            .store_response_record(
                &first_id,
                TranslationResponseOwner::default(),
                json!({ "id": first_id, "output_text": "x".repeat(64) }),
                vec![],
            )
            .await;
        store
            .store_response_record(
                &second_id,
                TranslationResponseOwner::default(),
                json!({ "id": second_id, "output_text": "y".repeat(64) }),
                vec![],
            )
            .await;

        assert!(store.stored_response(&first_id).await.is_none());
        assert!(store.stored_response(&second_id).await.is_some());
    }
}
