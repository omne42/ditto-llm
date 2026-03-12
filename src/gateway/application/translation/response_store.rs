use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::Mutex;

use crate::gateway::adapters::cache::LocalLruCache;

const DEFAULT_TRANSLATION_RESPONSE_STORE_MAX_ENTRIES: usize = 128;

#[derive(Debug, Clone)]
pub(crate) struct StoredTranslationResponse {
    pub(crate) response: Value,
    pub(crate) input_items: Vec<Value>,
}

#[derive(Clone, Default)]
pub(super) struct TranslationResponseStore {
    entries: Arc<Mutex<LocalLruCache<StoredTranslationResponse>>>,
}

impl TranslationResponseStore {
    async fn store_response_record(
        &self,
        response_id: &str,
        response: Value,
        input_items: Vec<Value>,
    ) {
        let response_id = response_id.trim();
        if response_id.is_empty() {
            return;
        }

        let mut entries = self.entries.lock().await;
        entries.insert(
            response_id.to_string(),
            StoredTranslationResponse {
                response,
                input_items,
            },
            DEFAULT_TRANSLATION_RESPONSE_STORE_MAX_ENTRIES,
        );
    }

    async fn stored_response(&self, response_id: &str) -> Option<StoredTranslationResponse> {
        self.entries.lock().await.get(response_id.trim())
    }

    async fn delete_stored_response(&self, response_id: &str) -> bool {
        self.entries
            .lock()
            .await
            .remove(response_id.trim())
            .is_some()
    }
}

pub(crate) async fn delete_stored_response_from_translation_backends(
    backends: &HashMap<String, super::TranslationBackend>,
    response_id: &str,
) -> Option<(String, String)> {
    let response_id = response_id.trim();
    if response_id.is_empty() {
        return None;
    }

    let mut backend_names = backends.keys().cloned().collect::<Vec<_>>();
    backend_names.sort();

    for backend_name in backend_names {
        let Some(backend) = backends.get(&backend_name) else {
            continue;
        };
        if !backend.delete_stored_response(response_id).await {
            continue;
        }
        let provider = backend.provider_name().trim();
        let provider = if provider.is_empty() {
            backend_name.clone()
        } else {
            provider.to_string()
        };
        return Some((backend_name, provider));
    }

    None
}

pub(crate) async fn find_stored_response_from_translation_backends(
    backends: &HashMap<String, super::TranslationBackend>,
    response_id: &str,
) -> Option<(String, String, StoredTranslationResponse)> {
    let response_id = response_id.trim();
    if response_id.is_empty() {
        return None;
    }

    let mut backend_names = backends.keys().cloned().collect::<Vec<_>>();
    backend_names.sort();

    for backend_name in backend_names {
        let Some(backend) = backends.get(&backend_name) else {
            continue;
        };
        let Some(stored) = backend.stored_response(response_id).await else {
            continue;
        };
        let provider = backend.provider_name().trim();
        let provider = if provider.is_empty() {
            backend_name.clone()
        } else {
            provider.to_string()
        };
        return Some((backend_name, provider, stored));
    }

    None
}

impl super::TranslationBackend {
    pub(crate) async fn store_response_record(
        &self,
        response_id: &str,
        response: Value,
        input_items: Vec<Value>,
    ) {
        self.runtime
            .response_store
            .store_response_record(response_id, response, input_items)
            .await;
    }

    async fn stored_response(&self, response_id: &str) -> Option<StoredTranslationResponse> {
        self.runtime
            .response_store
            .stored_response(response_id)
            .await
    }

    async fn delete_stored_response(&self, response_id: &str) -> bool {
        self.runtime
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
        store
            .store_response_record(
                "resp_123",
                json!({ "id": "resp_123" }),
                vec![json!({ "type": "message" })],
            )
            .await;

        let stored = store
            .stored_response("resp_123")
            .await
            .expect("stored response");
        assert_eq!(
            stored.response.get("id"),
            Some(&Value::String("resp_123".to_string()))
        );
        assert_eq!(stored.input_items.len(), 1);

        assert!(store.delete_stored_response("resp_123").await);
        assert!(store.stored_response("resp_123").await.is_none());
    }
}
