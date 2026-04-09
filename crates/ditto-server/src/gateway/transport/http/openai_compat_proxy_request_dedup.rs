use super::*;

use crate::gateway::{
    ProxyRequestFingerprint, ProxyRequestIdempotencyBeginOutcome, ProxyRequestIdempotencyRecord,
    ProxyRequestIdempotencyState, ProxyRequestIdempotencyStore, ProxyRequestIdempotencyStoreError,
    ProxyRequestReplayError, ProxyRequestReplayOutcome, ProxyRequestReplayResponse,
    StoredHttpHeader,
};
use async_trait::async_trait;
use bytes::BytesMut;
use omne_integrity_primitives::{Sha256Hasher, hash_sha256};
use tokio::task::JoinHandle;
use tokio::time::{Duration, sleep};

const REQUEST_DEDUP_LEASE_TTL_MS: u64 = 30_000;
const REQUEST_DEDUP_HEARTBEAT_INTERVAL_MS: u64 = 5_000;
const REQUEST_DEDUP_REPLAY_TTL_MS: u64 = 24 * 60 * 60_000;
const REQUEST_DEDUP_POLL_INTERVAL_MS: u64 = 50;

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
pub(super) struct LocalProxyRequestIdempotencyStore {
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

type ProxyRequestDedupPersistence = Arc<dyn ProxyRequestIdempotencyStore>;

pub(super) enum ProxyRequestDedupDecision {
    Disabled,
    Replay(Result<axum::response::Response, (StatusCode, Json<OpenAiErrorResponse>)>),
    Leader(ProxyRequestDedupLeader),
}

pub(super) struct ProxyRequestDedupLeader {
    persistence: ProxyRequestDedupPersistence,
    request_id: String,
    owner_token: String,
    max_snapshot_bytes: usize,
    heartbeat: Option<JoinHandle<()>>,
    completed: bool,
}

impl ProxyRequestDedupLeader {
    fn new(
        persistence: ProxyRequestDedupPersistence,
        request_id: &str,
        owner_token: &str,
        max_snapshot_bytes: usize,
    ) -> Self {
        let request_id_owned = request_id.to_string();
        let owner_token_owned = owner_token.to_string();
        let heartbeat_persistence = persistence.clone();
        let heartbeat_request_id = request_id_owned.clone();
        let heartbeat_owner_token = owner_token_owned.clone();
        let heartbeat = tokio::spawn(async move {
            loop {
                sleep(Duration::from_millis(REQUEST_DEDUP_HEARTBEAT_INTERVAL_MS)).await;
                let now_ms = now_epoch_millis();
                if !heartbeat_persistence
                    .refresh_proxy_request_idempotency_lease(
                        &heartbeat_request_id,
                        &heartbeat_owner_token,
                        now_ms,
                        REQUEST_DEDUP_LEASE_TTL_MS,
                    )
                    .await
                    .unwrap_or(false)
                {
                    break;
                }
            }
        });

        Self {
            persistence,
            request_id: request_id_owned,
            owner_token: owner_token_owned,
            max_snapshot_bytes,
            heartbeat: Some(heartbeat),
            completed: false,
        }
    }

    async fn complete_outcome(&mut self, outcome: &ProxyRequestReplayOutcome, replay_ttl_ms: u64) {
        if let Some(heartbeat) = self.heartbeat.take() {
            heartbeat.abort();
        }
        if self
            .persistence
            .complete_proxy_request_idempotency(
                &self.request_id,
                &self.owner_token,
                outcome,
                now_epoch_millis(),
                replay_ttl_ms,
            )
            .await
            .unwrap_or(false)
        {
            self.completed = true;
        } else {
            let _ = self
                .persistence
                .release_proxy_request_idempotency(&self.request_id, &self.owner_token)
                .await;
            self.completed = true;
        }
    }

    async fn release(&mut self) {
        if let Some(heartbeat) = self.heartbeat.take() {
            heartbeat.abort();
        }
        let _ = self
            .persistence
            .release_proxy_request_idempotency(&self.request_id, &self.owner_token)
            .await;
        self.completed = true;
    }
}

impl Drop for ProxyRequestDedupLeader {
    fn drop(&mut self) {
        if let Some(heartbeat) = self.heartbeat.take() {
            heartbeat.abort();
        }
        if self.completed {
            return;
        }

        let persistence = self.persistence.clone();
        let request_id = self.request_id.clone();
        let owner_token = self.owner_token.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let _ = persistence
                    .release_proxy_request_idempotency(&request_id, &owner_token)
                    .await;
            });
        }
    }
}

pub(super) struct PrepareProxyRequestDedupInput<'a> {
    pub state: &'a GatewayHttpState,
    pub method: &'a axum::http::Method,
    pub path_and_query: &'a str,
    pub headers: &'a HeaderMap,
    pub body: &'a Bytes,
    pub request_id: &'a str,
    pub client_supplied_request_id: bool,
    pub virtual_key_id: Option<&'a str>,
}

pub(super) async fn prepare_proxy_request_dedup(
    input: PrepareProxyRequestDedupInput<'_>,
) -> Result<ProxyRequestDedupDecision, (StatusCode, Json<OpenAiErrorResponse>)> {
    let PrepareProxyRequestDedupInput {
        state,
        method,
        path_and_query,
        headers,
        body,
        request_id,
        client_supplied_request_id,
        virtual_key_id,
    } = input;

    if !client_supplied_request_id || method.is_safe() {
        return Ok(ProxyRequestDedupDecision::Disabled);
    }

    let mut persistence = select_proxy_request_dedup_persistence(state);
    let local_fallback = local_proxy_request_dedup_persistence(state);
    let mut tried_local_fallback = !has_persistent_proxy_request_dedup_store(state);
    let dedup_subject_scope = request_dedup_subject_scope(headers);
    let scoped_request_id =
        scoped_proxy_request_id(request_id, virtual_key_id, dedup_subject_scope.as_deref());
    let (fingerprint, fingerprint_key) =
        request_dedup_fingerprint(method, path_and_query, virtual_key_id, headers, body);
    let owner_token = format!("dedup-{}", generate_request_id());

    loop {
        let now_ms = now_epoch_millis();
        match persistence
            .begin_proxy_request_idempotency(
                &scoped_request_id,
                &fingerprint,
                &fingerprint_key,
                &owner_token,
                now_ms,
                REQUEST_DEDUP_LEASE_TTL_MS,
            )
            .await
        {
            Ok(ProxyRequestIdempotencyBeginOutcome::Acquired) => {
                emit_json_log(
                    state,
                    "proxy.request_dedup_leader",
                    serde_json::json!({
                        "request_id": request_id,
                        "method": method.as_str(),
                        "path": path_and_query,
                        "virtual_key_id": virtual_key_id,
                    }),
                );
                return Ok(ProxyRequestDedupDecision::Leader(
                    ProxyRequestDedupLeader::new(
                        persistence,
                        &scoped_request_id,
                        &owner_token,
                        state.proxy.max_body_bytes,
                    ),
                ));
            }
            Ok(ProxyRequestIdempotencyBeginOutcome::Replay { record }) => {
                emit_json_log(
                    state,
                    "proxy.request_dedup_replay",
                    serde_json::json!({
                        "request_id": request_id,
                        "method": method.as_str(),
                        "path": path_and_query,
                        "virtual_key_id": virtual_key_id,
                    }),
                );
                return Ok(ProxyRequestDedupDecision::Replay(
                    response_from_idempotency_record(record, true),
                ));
            }
            Ok(ProxyRequestIdempotencyBeginOutcome::Conflict { .. }) => {
                emit_json_log(
                    state,
                    "proxy.request_dedup_conflict",
                    serde_json::json!({
                        "request_id": request_id,
                        "method": method.as_str(),
                        "path": path_and_query,
                        "virtual_key_id": virtual_key_id,
                    }),
                );
                return Ok(ProxyRequestDedupDecision::Replay(Err(openai_error(
                    StatusCode::CONFLICT,
                    "invalid_request_error",
                    Some("request_id_conflict"),
                    "x-request-id was already used for a different request",
                ))));
            }
            Ok(ProxyRequestIdempotencyBeginOutcome::InFlight { .. }) => {
                sleep(Duration::from_millis(REQUEST_DEDUP_POLL_INTERVAL_MS)).await;
            }
            Err(err) => {
                if !tried_local_fallback {
                    tried_local_fallback = true;
                    persistence = local_fallback.clone();
                    emit_json_log(
                        state,
                        "proxy.request_dedup_store_fallback",
                        serde_json::json!({
                            "request_id": request_id,
                            "method": method.as_str(),
                            "path": path_and_query,
                            "virtual_key_id": virtual_key_id,
                            "error": err.to_string(),
                        }),
                    );
                    continue;
                }
                return Err(openai_error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "api_error",
                    Some("request_dedup_unavailable"),
                    format!("request dedup store unavailable: {err}"),
                ));
            }
        }
    }
}

pub(super) async fn finish_proxy_request_dedup_result(
    leader: Option<ProxyRequestDedupLeader>,
    result: Result<axum::response::Response, (StatusCode, Json<OpenAiErrorResponse>)>,
) -> Result<axum::response::Response, (StatusCode, Json<OpenAiErrorResponse>)> {
    let Some(mut leader) = leader else {
        return result;
    };

    match result {
        Ok(response) => {
            if !should_persist_request_dedup_status(response.status()) {
                leader.release().await;
                return Ok(response);
            }
            if should_buffer_request_dedup_response(&response, leader.max_snapshot_bytes) {
                let (outcome, mut buffered) =
                    buffer_response_into_replay_outcome(response, leader.max_snapshot_bytes)
                        .await?;
                leader
                    .complete_outcome(&outcome, REQUEST_DEDUP_REPLAY_TTL_MS)
                    .await;
                buffered.headers_mut().insert(
                    "x-ditto-request-dedup",
                    axum::http::HeaderValue::from_static("leader"),
                );
                Ok(buffered)
            } else {
                Ok(wrap_response_for_request_dedup(response, leader))
            }
        }
        Err(err) => {
            leader.release().await;
            Err(err)
        }
    }
}

fn select_proxy_request_dedup_persistence(
    state: &GatewayHttpState,
) -> ProxyRequestDedupPersistence {
    state.proxy_request_idempotency_store()
}

fn local_proxy_request_dedup_persistence(state: &GatewayHttpState) -> ProxyRequestDedupPersistence {
    state.proxy.request_dedup.clone()
}

fn has_persistent_proxy_request_dedup_store(_state: &GatewayHttpState) -> bool {
    #[cfg(feature = "gateway-store-redis")]
    if _state.stores.redis.is_some() {
        return true;
    }
    #[cfg(feature = "gateway-store-postgres")]
    if _state.stores.postgres.is_some() {
        return true;
    }
    #[cfg(feature = "gateway-store-mysql")]
    if _state.stores.mysql.is_some() {
        return true;
    }
    #[cfg(feature = "gateway-store-sqlite")]
    if _state.stores.sqlite.is_some() {
        return true;
    }

    false
}

fn scoped_proxy_request_id(
    request_id: &str,
    virtual_key_id: Option<&str>,
    dedup_subject_scope: Option<&str>,
) -> String {
    let scope = virtual_key_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or(dedup_subject_scope)
        .unwrap_or("_global");
    format!("ditto-proxy-request-dedup-v1|{scope}|{request_id}")
}

fn request_dedup_fingerprint(
    method: &axum::http::Method,
    path_and_query: &str,
    virtual_key_id: Option<&str>,
    headers: &HeaderMap,
    body: &Bytes,
) -> (ProxyRequestFingerprint, String) {
    let body_sha256 = hash_sha256(body).to_string();
    let upstream_headers = request_dedup_upstream_headers(headers);
    let fingerprint = ProxyRequestFingerprint {
        method: method.as_str().to_string(),
        path: path_and_query.to_string(),
        virtual_key_id: virtual_key_id.map(ToString::to_string),
        upstream_headers,
        body_sha256,
    };

    let fingerprint_key = {
        let mut hasher = Sha256Hasher::new();
        hasher.update(b"ditto-proxy-request-dedup-v2|");
        hasher.update(fingerprint.method.as_bytes());
        hasher.update(b"|");
        hasher.update(fingerprint.path.as_bytes());
        hasher.update(b"|");
        hasher.update(
            fingerprint
                .virtual_key_id
                .as_deref()
                .unwrap_or_default()
                .as_bytes(),
        );
        hasher.update(b"|");
        for header in &fingerprint.upstream_headers {
            hasher.update(header.name.as_bytes());
            hasher.update(b":");
            hasher.update(&header.value);
            hasher.update(b"|");
        }
        hasher.update(fingerprint.body_sha256.as_bytes());
        hasher.finalize().to_string()
    };

    (fingerprint, fingerprint_key)
}

fn request_dedup_upstream_headers(headers: &HeaderMap) -> Vec<StoredHttpHeader> {
    let mut header_names: Vec<&str> = headers
        .keys()
        .map(|name| name.as_str())
        .filter(|name| request_dedup_header_affects_upstream(name))
        .collect();
    header_names.sort_unstable();
    header_names.dedup();

    let mut stored = Vec::new();
    for header_name in header_names {
        for value in headers.get_all(header_name).iter() {
            stored.push(StoredHttpHeader {
                name: header_name.to_string(),
                value: value.as_bytes().to_vec(),
            });
        }
    }

    stored
}

fn request_dedup_subject_scope(headers: &HeaderMap) -> Option<String> {
    let mut hasher = Sha256Hasher::new();
    let mut saw_identity = false;

    for header_name in [
        "authorization",
        "x-api-key",
        "x-litellm-api-key",
        "proxy-authorization",
        "x-forwarded-authorization",
    ] {
        for value in headers.get_all(header_name).iter() {
            hasher.update(header_name.as_bytes());
            hasher.update(b":");
            hasher.update(value.as_bytes());
            hasher.update(b"|");
            saw_identity = true;
        }
    }

    saw_identity.then(|| format!("auth:{}", hasher.finalize()))
}

fn request_dedup_header_affects_upstream(header: &str) -> bool {
    !matches!(
        header,
        "authorization"
            | "x-api-key"
            | "x-litellm-api-key"
            | "proxy-authorization"
            | "x-forwarded-authorization"
            | "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-connection"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "x-ditto-virtual-key"
            | "x-ditto-protocol"
            | "x-ditto-cache-bypass"
            | "x-ditto-bypass-cache"
            | "content-length"
            | "x-request-id"
            | "traceparent"
            | "tracestate"
            | "baggage"
    )
}

fn should_buffer_request_dedup_response(
    response: &axum::response::Response,
    max_snapshot_bytes: usize,
) -> bool {
    let is_event_stream = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().starts_with("text/event-stream"));
    if is_event_stream {
        return false;
    }

    response
        .headers()
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|len| len <= max_snapshot_bytes)
}

fn should_persist_request_dedup_status(status: StatusCode) -> bool {
    !(status.is_server_error()
        || status == StatusCode::TOO_MANY_REQUESTS
        || status == StatusCode::REQUEST_TIMEOUT)
}

async fn buffer_response_into_replay_outcome(
    response: axum::response::Response,
    max_snapshot_bytes: usize,
) -> Result<
    (ProxyRequestReplayOutcome, axum::response::Response),
    (StatusCode, Json<OpenAiErrorResponse>),
> {
    let (parts, body) = response.into_parts();
    let bytes = to_bytes(body, max_snapshot_bytes).await.map_err(|err| {
        openai_error(
            StatusCode::BAD_GATEWAY,
            "api_error",
            Some("request_dedup_snapshot_failed"),
            format!("failed to buffer response for request dedup: {err}"),
        )
    })?;

    let outcome = ProxyRequestReplayOutcome::Response(ProxyRequestReplayResponse {
        status: parts.status.as_u16(),
        headers: header_map_to_record(&parts.headers),
        body: bytes.clone().to_vec(),
    });

    let mut response = axum::response::Response::new(Body::from(bytes));
    *response.status_mut() = parts.status;
    *response.headers_mut() = parts.headers;
    Ok((outcome, response))
}

fn response_from_idempotency_record(
    record: ProxyRequestIdempotencyRecord,
    replay_hit: bool,
) -> Result<axum::response::Response, (StatusCode, Json<OpenAiErrorResponse>)> {
    response_from_replay_outcome(record.outcome, replay_hit)
}

fn response_from_replay_outcome(
    outcome: Option<ProxyRequestReplayOutcome>,
    replay_hit: bool,
) -> Result<axum::response::Response, (StatusCode, Json<OpenAiErrorResponse>)> {
    let mut response = match outcome {
        Some(ProxyRequestReplayOutcome::Response(response)) => {
            response_from_replay_response(response)
        }
        Some(ProxyRequestReplayOutcome::Error { status, error }) => {
            return Err(error_tuple_from_replay_error(status, error));
        }
        None => {
            return Err(openai_error(
                StatusCode::BAD_GATEWAY,
                "api_error",
                Some("request_dedup_missing_outcome"),
                "request dedup replay record is missing outcome",
            ));
        }
    };
    response.headers_mut().insert(
        "x-ditto-request-dedup",
        axum::http::HeaderValue::from_static(if replay_hit { "replay" } else { "leader" }),
    );
    Ok(response)
}

fn response_from_replay_response(replay: ProxyRequestReplayResponse) -> axum::response::Response {
    let status = StatusCode::from_u16(replay.status).unwrap_or(StatusCode::OK);
    let mut response = axum::response::Response::new(Body::from(replay.body));
    *response.status_mut() = status;
    *response.headers_mut() = header_map_from_record(replay.headers);
    response
}

fn error_tuple_from_replay_error(
    status: u16,
    error: ProxyRequestReplayError,
) -> (StatusCode, Json<OpenAiErrorResponse>) {
    (
        StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
        Json(OpenAiErrorResponse {
            error: OpenAiErrorDetail {
                message: error.message,
                kind: error.kind,
                code: error.code,
            },
        }),
    )
}

fn replay_unavailable_outcome(
    status: StatusCode,
    code: &str,
    message: &str,
) -> ProxyRequestReplayOutcome {
    ProxyRequestReplayOutcome::Error {
        status: status.as_u16(),
        error: ProxyRequestReplayError {
            message: message.to_string(),
            kind: "invalid_request_error".to_string(),
            code: Some(code.to_string()),
        },
    }
}

fn wrap_response_for_request_dedup(
    response: axum::response::Response,
    leader: ProxyRequestDedupLeader,
) -> axum::response::Response {
    struct ReplayRecorder {
        max_body_bytes: usize,
        buffer: BytesMut,
        overflowed: bool,
    }

    impl ReplayRecorder {
        fn new(max_body_bytes: usize) -> Self {
            Self {
                max_body_bytes,
                buffer: BytesMut::new(),
                overflowed: max_body_bytes == 0,
            }
        }

        fn ingest(&mut self, chunk: &Bytes) {
            if self.overflowed {
                return;
            }
            let next_len = self.buffer.len().saturating_add(chunk.len());
            if next_len > self.max_body_bytes {
                self.buffer.clear();
                self.overflowed = true;
                return;
            }
            self.buffer.extend_from_slice(chunk);
        }

        fn finish(&mut self) -> Option<Bytes> {
            if self.overflowed {
                None
            } else {
                Some(std::mem::take(&mut self.buffer).freeze())
            }
        }
    }

    struct StreamState {
        stream: axum::body::BodyDataStream,
        leader: Option<ProxyRequestDedupLeader>,
        status: StatusCode,
        headers: HeaderMap,
        recorder: ReplayRecorder,
    }

    impl Drop for StreamState {
        fn drop(&mut self) {
            let _ = self.leader.take();
        }
    }

    let snapshot_limit = leader.max_snapshot_bytes;
    let (mut parts, body) = response.into_parts();
    parts.headers.insert(
        "x-ditto-request-dedup",
        axum::http::HeaderValue::from_static("leader"),
    );

    let state = StreamState {
        stream: body.into_data_stream(),
        leader: Some(leader),
        status: parts.status,
        headers: parts.headers.clone(),
        recorder: ReplayRecorder::new(snapshot_limit),
    };

    let stream = futures_util::stream::try_unfold(state, |mut state| async move {
        match state.stream.next().await {
            Some(Ok(chunk)) => {
                state.recorder.ingest(&chunk);
                Ok(Some((chunk, state)))
            }
            Some(Err(err)) => {
                if let Some(mut leader) = state.leader.take() {
                    let outcome = replay_unavailable_outcome(
                        StatusCode::CONFLICT,
                        "request_id_replay_unavailable",
                        "x-request-id cannot be replayed because the gateway could not snapshot the full response body",
                    );
                    leader
                        .complete_outcome(&outcome, REQUEST_DEDUP_REPLAY_TTL_MS)
                        .await;
                }
                Err(std::io::Error::other(err.to_string()))
            }
            None => {
                if let Some(mut leader) = state.leader.take() {
                    if should_persist_request_dedup_status(state.status) {
                        let outcome = match state.recorder.finish() {
                            Some(bytes) => {
                                ProxyRequestReplayOutcome::Response(ProxyRequestReplayResponse {
                                    status: state.status.as_u16(),
                                    headers: header_map_to_record(&state.headers),
                                    body: bytes.to_vec(),
                                })
                            }
                            None => replay_unavailable_outcome(
                                StatusCode::CONFLICT,
                                "request_id_replay_unavailable",
                                "x-request-id cannot be replayed because the gateway could not snapshot the full response body",
                            ),
                        };
                        leader
                            .complete_outcome(&outcome, REQUEST_DEDUP_REPLAY_TTL_MS)
                            .await;
                    } else {
                        leader.release().await;
                    }
                }
                Ok(None)
            }
        }
    });

    let mut response = axum::response::Response::new(Body::from_stream(stream));
    *response.status_mut() = parts.status;
    *response.headers_mut() = parts.headers;
    response
}

fn header_map_to_record(headers: &HeaderMap) -> Vec<StoredHttpHeader> {
    let mut out = Vec::with_capacity(headers.len());
    for (name, value) in headers {
        out.push(StoredHttpHeader {
            name: name.as_str().to_string(),
            value: value.as_bytes().to_vec(),
        });
    }
    out
}

fn header_map_from_record(headers: Vec<StoredHttpHeader>) -> HeaderMap {
    let mut out = HeaderMap::new();
    for header in headers {
        let Ok(name) = header.name.parse::<axum::http::HeaderName>() else {
            continue;
        };
        let Ok(value) = axum::http::HeaderValue::from_bytes(&header.value) else {
            continue;
        };
        out.append(name, value);
    }
    out
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

fn now_epoch_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::{Gateway, GatewayConfig, RouterConfig};

    #[tokio::test]
    async fn failed_dedup_result_releases_inflight_record() {
        let store = Arc::new(LocalProxyRequestIdempotencyStore::default());
        let request_id = "req-dedup-error";
        let owner_token = "owner-1";
        let now_ms = now_epoch_millis();
        let fingerprint = ProxyRequestFingerprint {
            method: "POST".to_string(),
            path: "/v1/responses".to_string(),
            virtual_key_id: Some("key-1".to_string()),
            upstream_headers: Vec::new(),
            body_sha256: "body".to_string(),
        };

        let begin = store
            .begin_proxy_request_idempotency(
                request_id,
                &fingerprint,
                "fingerprint-1",
                owner_token,
                now_ms,
                REQUEST_DEDUP_LEASE_TTL_MS,
            )
            .await
            .expect("begin dedup");
        assert!(matches!(
            begin,
            ProxyRequestIdempotencyBeginOutcome::Acquired
        ));

        let leader = ProxyRequestDedupLeader::new(store.clone(), request_id, owner_token, 1024);
        let result = finish_proxy_request_dedup_result(
            Some(leader),
            Err(openai_error(
                StatusCode::BAD_GATEWAY,
                "api_error",
                Some("upstream_failure"),
                "upstream failed",
            )),
        )
        .await;
        assert_eq!(result.expect_err("dedup error").0, StatusCode::BAD_GATEWAY);

        let stored = store
            .get_proxy_request_idempotency(request_id, now_epoch_millis())
            .await
            .expect("lookup dedup");
        assert!(stored.is_none());
    }

    #[tokio::test]
    async fn oversized_stream_records_replay_unavailable_outcome() {
        let store = Arc::new(LocalProxyRequestIdempotencyStore::default());
        let persistence: ProxyRequestDedupPersistence = store.clone();
        let request_id = "req-dedup-overflow";
        let owner_token = "owner-1";

        let fingerprint = ProxyRequestFingerprint {
            method: "POST".to_string(),
            path: "/v1/responses".to_string(),
            virtual_key_id: Some("key-1".to_string()),
            upstream_headers: Vec::new(),
            body_sha256: "body".to_string(),
        };
        let outcome = store
            .begin_proxy_request_idempotency(
                request_id,
                &fingerprint,
                "fingerprint",
                owner_token,
                now_epoch_millis(),
                REQUEST_DEDUP_LEASE_TTL_MS,
            )
            .await
            .expect("begin dedup");
        assert!(matches!(
            outcome,
            ProxyRequestIdempotencyBeginOutcome::Acquired
        ));

        let body = Body::from_stream(futures_util::stream::iter(vec![
            Ok::<Bytes, std::io::Error>(Bytes::from_static(b"1234")),
            Ok::<Bytes, std::io::Error>(Bytes::from_static(b"56")),
        ]));
        let response = axum::response::Response::builder()
            .status(StatusCode::OK)
            .body(body)
            .expect("response");

        let wrapped = wrap_response_for_request_dedup(
            response,
            ProxyRequestDedupLeader::new(persistence, request_id, owner_token, 4),
        );
        let bytes = to_bytes(wrapped.into_body(), usize::MAX)
            .await
            .expect("consume wrapped response");
        assert_eq!(bytes.as_ref(), b"123456");

        let record = store
            .get_proxy_request_idempotency(request_id, now_epoch_millis())
            .await
            .expect("load record")
            .expect("completed record");
        let replay = response_from_idempotency_record(record, true)
            .expect_err("overflowed response should replay as unavailable");
        assert_eq!(replay.0, StatusCode::CONFLICT);
        assert_eq!(
            replay.1.error.code.as_deref(),
            Some("request_id_replay_unavailable")
        );
    }

    fn test_gateway_http_state() -> GatewayHttpState {
        GatewayHttpState::new(Gateway::new(GatewayConfig {
            backends: Vec::new(),
            virtual_keys: Vec::new(),
            router: RouterConfig {
                default_backends: Vec::new(),
                rules: Vec::new(),
            },
            a2a_agents: Vec::new(),
            mcp_servers: Vec::new(),
            observability: Default::default(),
        }))
    }

    #[tokio::test]
    async fn request_id_dedup_is_isolated_per_virtual_key() {
        let state = test_gateway_http_state();
        let method = axum::http::Method::POST;
        let headers = HeaderMap::new();
        let request_id = "req-shared";

        let first = prepare_proxy_request_dedup(PrepareProxyRequestDedupInput {
            state: &state,
            method: &method,
            path_and_query: "/v1/chat/completions",
            headers: &headers,
            body: &Bytes::from_static(br#"{"prompt":"hello"}"#),
            request_id,
            client_supplied_request_id: true,
            virtual_key_id: Some("key-a"),
        })
        .await
        .expect("first request should acquire dedup leadership");
        assert!(matches!(first, ProxyRequestDedupDecision::Leader(_)));

        let second = prepare_proxy_request_dedup(PrepareProxyRequestDedupInput {
            state: &state,
            method: &method,
            path_and_query: "/v1/chat/completions",
            headers: &headers,
            body: &Bytes::from_static(br#"{"prompt":"world"}"#),
            request_id,
            client_supplied_request_id: true,
            virtual_key_id: Some("key-b"),
        })
        .await
        .expect("different virtual keys should not conflict");
        assert!(matches!(second, ProxyRequestDedupDecision::Leader(_)));
    }

    #[tokio::test]
    async fn request_id_dedup_still_conflicts_within_same_virtual_key() {
        let state = test_gateway_http_state();
        let method = axum::http::Method::POST;
        let headers = HeaderMap::new();
        let request_id = "req-conflict";

        let first = prepare_proxy_request_dedup(PrepareProxyRequestDedupInput {
            state: &state,
            method: &method,
            path_and_query: "/v1/chat/completions",
            headers: &headers,
            body: &Bytes::from_static(br#"{"prompt":"hello"}"#),
            request_id,
            client_supplied_request_id: true,
            virtual_key_id: Some("key-a"),
        })
        .await
        .expect("first request should acquire dedup leadership");
        assert!(matches!(first, ProxyRequestDedupDecision::Leader(_)));

        let second = prepare_proxy_request_dedup(PrepareProxyRequestDedupInput {
            state: &state,
            method: &method,
            path_and_query: "/v1/chat/completions",
            headers: &headers,
            body: &Bytes::from_static(br#"{"prompt":"different"}"#),
            request_id,
            client_supplied_request_id: true,
            virtual_key_id: Some("key-a"),
        })
        .await
        .expect("same virtual key should return a dedup decision");
        let replay = match second {
            ProxyRequestDedupDecision::Replay(Err(err)) => err,
            ProxyRequestDedupDecision::Replay(Ok(_)) => {
                panic!("expected conflict replay, got buffered replay response")
            }
            ProxyRequestDedupDecision::Leader(_) => {
                panic!("expected conflict replay, got a new dedup leader")
            }
            ProxyRequestDedupDecision::Disabled => {
                panic!("expected conflict replay, dedup was unexpectedly disabled")
            }
        };
        assert_eq!(replay.0, StatusCode::CONFLICT);
        assert_eq!(replay.1.error.code.as_deref(), Some("request_id_conflict"));
    }

    #[tokio::test]
    async fn request_id_dedup_is_isolated_by_auth_subject_without_virtual_key() {
        let state = test_gateway_http_state();
        let method = axum::http::Method::POST;
        let request_id = "req-auth-scope";

        let mut first_headers = HeaderMap::new();
        first_headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer token-a"),
        );
        let first = prepare_proxy_request_dedup(PrepareProxyRequestDedupInput {
            state: &state,
            method: &method,
            path_and_query: "/v1/chat/completions",
            headers: &first_headers,
            body: &Bytes::from_static(br#"{"prompt":"hello"}"#),
            request_id,
            client_supplied_request_id: true,
            virtual_key_id: None,
        })
        .await
        .expect("first request should acquire dedup leadership");
        assert!(matches!(first, ProxyRequestDedupDecision::Leader(_)));

        let mut second_headers = HeaderMap::new();
        second_headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer token-b"),
        );
        let second = prepare_proxy_request_dedup(PrepareProxyRequestDedupInput {
            state: &state,
            method: &method,
            path_and_query: "/v1/chat/completions",
            headers: &second_headers,
            body: &Bytes::from_static(br#"{"prompt":"hello"}"#),
            request_id,
            client_supplied_request_id: true,
            virtual_key_id: None,
        })
        .await
        .expect("different auth subjects should not conflict");
        assert!(matches!(second, ProxyRequestDedupDecision::Leader(_)));
    }
}
