use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::Mutex;

use crate::gateway::adapters::cache::LocalLruCache;

const DEFAULT_TRANSLATION_RESPONSE_STORE_MAX_ENTRIES: usize = 128;
const TRANSLATION_RESPONSE_HANDLE_PREFIX: &str = "resp_ditto_";

#[derive(Debug, Clone)]
pub(crate) struct StoredTranslationResponse {
    pub(crate) response: Value,
    pub(crate) input_items: Vec<Value>,
}

#[derive(Clone, Default)]
pub(super) struct TranslationResponseStore {
    entries: Arc<Mutex<LocalLruCache<StoredTranslationResponse>>>,
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
) -> Option<String> {
    let (backend_name, _) = parse_gateway_scoped_response_id(response_id)?;
    let backend = backends.get(backend_name)?;
    backend
        .delete_stored_response(response_id.trim())
        .await
        .then(|| backend_name.to_string())
}

pub(crate) async fn find_stored_response_from_translation_backends(
    backends: &HashMap<String, super::TranslationBackend>,
    response_id: &str,
) -> Option<(String, StoredTranslationResponse)> {
    let (backend_name, _) = parse_gateway_scoped_response_id(response_id)?;
    let backend = backends.get(backend_name)?;
    backend
        .stored_response(response_id.trim())
        .await
        .map(|stored| (backend_name.to_string(), stored))
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
        let response_id = gateway_scoped_response_id("primary", "resp_123");
        store
            .store_response_record(
                &response_id,
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

    #[test]
    fn gateway_scoped_response_ids_roundtrip() {
        let public_id = gateway_scoped_response_id("primary", "resp_123");
        assert_eq!(
            parse_gateway_scoped_response_id(&public_id),
            Some(("primary", "resp_123"))
        );
        assert!(parse_gateway_scoped_response_id("resp_123").is_none());
    }
}
