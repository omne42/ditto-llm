use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::Mutex;

use crate::gateway::adapters::cache::LocalLruCache;

const DEFAULT_TRANSLATION_RESPONSE_STORE_MAX_ENTRIES: usize = 128;
const GATEWAY_RESPONSE_ID_PREFIX: &str = "resp_ditto_";

#[derive(Debug, Clone)]
pub(crate) struct StoredTranslationResponse {
    pub(crate) response: Value,
    pub(crate) input_items: Vec<Value>,
}

#[derive(Clone, Default)]
pub(crate) struct TranslationResponseStore {
    entries: Arc<Mutex<LocalLruCache<StoredTranslationResponse>>>,
}

impl TranslationResponseStore {
    pub(super) async fn store_response_record(
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

pub(crate) fn gateway_stored_response_id(backend_name: &str, provider_response_id: &str) -> String {
    let backend_name = backend_name.trim();
    let provider_response_id = provider_response_id.trim();
    if backend_name.is_empty() || provider_response_id.is_empty() {
        return provider_response_id.to_string();
    }
    format!(
        "{GATEWAY_RESPONSE_ID_PREFIX}{}_{}_{}",
        backend_name.len(),
        backend_name,
        provider_response_id
    )
}

fn parse_gateway_stored_response_id(response_id: &str) -> Option<(&str, &str)> {
    let response_id = response_id.trim();
    let rest = response_id.strip_prefix(GATEWAY_RESPONSE_ID_PREFIX)?;
    let (backend_len, rest) = rest.split_once('_')?;
    let backend_len = backend_len.parse::<usize>().ok()?;
    if rest.len() <= backend_len {
        return None;
    }
    let (backend_name, provider_response_id) = rest.split_at(backend_len);
    let provider_response_id = provider_response_id.strip_prefix('_')?;
    if backend_name.trim().is_empty() || provider_response_id.trim().is_empty() {
        return None;
    }
    Some((backend_name, provider_response_id))
}

fn resolve_backend_response_key<'a>(
    backends: &'a HashMap<String, super::TranslationBackend>,
    response_id: &'a str,
) -> Option<(&'a str, &'a super::TranslationBackend, &'a str)> {
    if let Some((backend_name, _provider_response_id)) = parse_gateway_stored_response_id(response_id)
    {
        let backend = backends.get(backend_name)?;
        return Some((backend_name, backend, response_id.trim()));
    }

    if backends.len() == 1 {
        let (backend_name, backend) = backends.iter().next()?;
        return Some((backend_name.as_str(), backend, response_id.trim()));
    }

    None
}

pub(crate) async fn delete_stored_response_from_translation_backends(
    backends: &HashMap<String, super::TranslationBackend>,
    response_id: &str,
) -> Option<(String, String)> {
    let response_id = response_id.trim();
    if response_id.is_empty() {
        return None;
    }

    let (backend_name, backend, lookup_id) = resolve_backend_response_key(backends, response_id)?;
    if !backend.delete_stored_response(lookup_id).await {
        return None;
    }
    let provider = backend.provider_name().trim();
    let provider = if provider.is_empty() {
        backend_name.to_string()
    } else {
        provider.to_string()
    };
    Some((backend_name.to_string(), provider))
}

pub(crate) async fn find_stored_response_from_translation_backends(
    backends: &HashMap<String, super::TranslationBackend>,
    response_id: &str,
) -> Option<(String, String, StoredTranslationResponse)> {
    let response_id = response_id.trim();
    if response_id.is_empty() {
        return None;
    }

    let (backend_name, backend, lookup_id) = resolve_backend_response_key(backends, response_id)?;
    let stored = backend.stored_response(lookup_id).await?;
    let provider = backend.provider_name().trim();
    let provider = if provider.is_empty() {
        backend_name.to_string()
    } else {
        provider.to_string()
    };
    Some((backend_name.to_string(), provider, stored))
}

impl super::TranslationBackend {
    pub(crate) fn response_store(&self) -> TranslationResponseStore {
        self.runtime.response_store.clone()
    }

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

    #[test]
    fn gateway_response_id_roundtrips_backend_identity() {
        let gateway_id = gateway_stored_response_id("primary", "resp_fake");
        assert_eq!(
            parse_gateway_stored_response_id(&gateway_id),
            Some(("primary", "resp_fake"))
        );
    }
}
