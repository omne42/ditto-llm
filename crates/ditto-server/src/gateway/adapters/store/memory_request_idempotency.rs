use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;

use crate::gateway::{
    ProxyRequestFingerprint, ProxyRequestIdempotencyBeginOutcome, ProxyRequestIdempotencyRecord,
    ProxyRequestIdempotencyState, ProxyRequestIdempotencyStore, ProxyRequestIdempotencyStoreError,
    ProxyRequestReplayOutcome, lock_unpoisoned,
};

#[derive(Default)]
struct LocalProxyRequestDedupStore {
    entries: HashMap<String, ProxyRequestIdempotencyRecord>,
}

impl LocalProxyRequestDedupStore {
    fn begin(
        &mut self,
        request_id: &str,
        fingerprint: &ProxyRequestFingerprint,
        fingerprint_key: &str,
        owner_token: &str,
        now_ms: u64,
        lease_ttl_ms: u64,
    ) -> ProxyRequestIdempotencyBeginOutcome {
        self.entries
            .retain(|_, record| record.expires_at_ms >= now_ms);

        match self.entries.get_mut(request_id) {
            None => {
                self.entries.insert(
                    request_id.to_string(),
                    new_local_proxy_request_idempotency_record(
                        request_id,
                        fingerprint,
                        fingerprint_key,
                        owner_token,
                        now_ms,
                        lease_ttl_ms,
                    ),
                );
                ProxyRequestIdempotencyBeginOutcome::Acquired
            }
            Some(record) if record.fingerprint_key != fingerprint_key => {
                ProxyRequestIdempotencyBeginOutcome::Conflict {
                    record: record.clone(),
                }
            }
            Some(record) if record.expires_at_ms >= now_ms => match record.state {
                ProxyRequestIdempotencyState::Completed => {
                    ProxyRequestIdempotencyBeginOutcome::Replay {
                        record: record.clone(),
                    }
                }
                ProxyRequestIdempotencyState::InFlight => {
                    ProxyRequestIdempotencyBeginOutcome::InFlight {
                        record: record.clone(),
                    }
                }
            },
            Some(record) => {
                *record = new_local_proxy_request_idempotency_record(
                    request_id,
                    fingerprint,
                    fingerprint_key,
                    owner_token,
                    now_ms,
                    lease_ttl_ms,
                );
                ProxyRequestIdempotencyBeginOutcome::Acquired
            }
        }
    }

    fn refresh(
        &mut self,
        request_id: &str,
        owner_token: &str,
        now_ms: u64,
        lease_ttl_ms: u64,
    ) -> bool {
        let Some(record) = self.entries.get_mut(request_id) else {
            return false;
        };
        if record.state != ProxyRequestIdempotencyState::InFlight
            || record.owner_token.as_deref() != Some(owner_token)
        {
            return false;
        }

        let lease_until_ms = now_ms.saturating_add(lease_ttl_ms);
        record.updated_at_ms = now_ms;
        record.lease_until_ms = Some(lease_until_ms);
        record.expires_at_ms = lease_until_ms;
        true
    }

    fn complete(
        &mut self,
        request_id: &str,
        owner_token: &str,
        outcome: &ProxyRequestReplayOutcome,
        now_ms: u64,
        replay_ttl_ms: u64,
    ) -> bool {
        let Some(record) = self.entries.get_mut(request_id) else {
            return false;
        };
        if record.state != ProxyRequestIdempotencyState::InFlight
            || record.owner_token.as_deref() != Some(owner_token)
        {
            return false;
        }

        record.state = ProxyRequestIdempotencyState::Completed;
        record.owner_token = None;
        record.lease_until_ms = None;
        record.completed_at_ms = Some(now_ms);
        record.updated_at_ms = now_ms;
        record.expires_at_ms = now_ms.saturating_add(replay_ttl_ms);
        record.outcome = Some(outcome.clone());
        true
    }

    fn get(&self, request_id: &str, now_ms: u64) -> Option<ProxyRequestIdempotencyRecord> {
        let record = self.entries.get(request_id)?;
        if record.expires_at_ms < now_ms {
            return None;
        }
        Some(record.clone())
    }

    fn release(&mut self, request_id: &str, owner_token: &str) -> bool {
        let Some(record) = self.entries.get(request_id) else {
            return false;
        };
        if record.state != ProxyRequestIdempotencyState::InFlight
            || record.owner_token.as_deref() != Some(owner_token)
        {
            return false;
        }
        self.entries.remove(request_id);
        true
    }
}

#[derive(Clone, Default)]
pub(crate) struct LocalProxyRequestIdempotencyStore {
    inner: Arc<StdMutex<LocalProxyRequestDedupStore>>,
}

#[async_trait]
impl ProxyRequestIdempotencyStore for LocalProxyRequestIdempotencyStore {
    async fn begin_proxy_request_idempotency(
        &self,
        request_id: &str,
        fingerprint: &ProxyRequestFingerprint,
        fingerprint_key: &str,
        owner_token: &str,
        now_ms: u64,
        lease_ttl_ms: u64,
    ) -> Result<ProxyRequestIdempotencyBeginOutcome, ProxyRequestIdempotencyStoreError> {
        Ok(lock_unpoisoned(&self.inner).begin(
            request_id,
            fingerprint,
            fingerprint_key,
            owner_token,
            now_ms,
            lease_ttl_ms,
        ))
    }

    async fn get_proxy_request_idempotency(
        &self,
        request_id: &str,
        now_ms: u64,
    ) -> Result<Option<ProxyRequestIdempotencyRecord>, ProxyRequestIdempotencyStoreError> {
        Ok(lock_unpoisoned(&self.inner).get(request_id, now_ms))
    }

    async fn refresh_proxy_request_idempotency_lease(
        &self,
        request_id: &str,
        owner_token: &str,
        now_ms: u64,
        lease_ttl_ms: u64,
    ) -> Result<bool, ProxyRequestIdempotencyStoreError> {
        Ok(lock_unpoisoned(&self.inner).refresh(request_id, owner_token, now_ms, lease_ttl_ms))
    }

    async fn complete_proxy_request_idempotency(
        &self,
        request_id: &str,
        owner_token: &str,
        outcome: &ProxyRequestReplayOutcome,
        now_ms: u64,
        replay_ttl_ms: u64,
    ) -> Result<bool, ProxyRequestIdempotencyStoreError> {
        Ok(lock_unpoisoned(&self.inner).complete(
            request_id,
            owner_token,
            outcome,
            now_ms,
            replay_ttl_ms,
        ))
    }

    async fn release_proxy_request_idempotency(
        &self,
        request_id: &str,
        owner_token: &str,
    ) -> Result<bool, ProxyRequestIdempotencyStoreError> {
        Ok(lock_unpoisoned(&self.inner).release(request_id, owner_token))
    }
}

fn new_local_proxy_request_idempotency_record(
    request_id: &str,
    fingerprint: &ProxyRequestFingerprint,
    fingerprint_key: &str,
    owner_token: &str,
    now_ms: u64,
    lease_ttl_ms: u64,
) -> ProxyRequestIdempotencyRecord {
    let lease_until_ms = now_ms.saturating_add(lease_ttl_ms);
    ProxyRequestIdempotencyRecord {
        request_id: request_id.to_string(),
        fingerprint: fingerprint.clone(),
        fingerprint_key: fingerprint_key.to_string(),
        state: ProxyRequestIdempotencyState::InFlight,
        owner_token: Some(owner_token.to_string()),
        started_at_ms: now_ms,
        updated_at_ms: now_ms,
        lease_until_ms: Some(lease_until_ms),
        completed_at_ms: None,
        expires_at_ms: lease_until_ms,
        outcome: None,
    }
}
