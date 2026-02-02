use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures_util::StreamExt;
use futures_util::stream;
use tokio::sync::Mutex;

use crate::layer::LanguageModelLayer;
use crate::model::LanguageModel;
use crate::types::{GenerateRequest, GenerateResponse, StreamChunk};
use crate::{Result, StreamResult};

#[derive(Debug, Clone)]
pub struct CacheLayer {
    state: Arc<Mutex<CacheState>>,
    ttl: Option<Duration>,
    max_entries: usize,
    max_value_bytes: usize,
    max_stream_chunks: usize,
}

#[derive(Debug)]
struct CacheState {
    entries: HashMap<CacheKey, CacheEntry>,
    lru: VecDeque<CacheKey>,
}

#[derive(Debug)]
struct CacheEntry {
    inserted_at: Instant,
    value: CacheValue,
}

#[derive(Debug, Clone)]
enum CacheValue {
    Generate(GenerateResponse),
    Stream(Arc<Vec<StreamChunk>>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct CacheKey {
    kind: CacheKind,
    hash: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum CacheKind {
    Generate,
    Stream,
}

impl CacheLayer {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(CacheState {
                entries: HashMap::new(),
                lru: VecDeque::new(),
            })),
            ttl: None,
            max_entries: 256,
            max_value_bytes: 4 * 1024 * 1024,
            max_stream_chunks: 4096,
        }
    }

    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = Some(ttl);
        self
    }

    pub fn with_max_entries(mut self, max_entries: usize) -> Self {
        self.max_entries = max_entries.max(1);
        self
    }

    pub fn with_max_value_bytes(mut self, max_value_bytes: usize) -> Self {
        self.max_value_bytes = max_value_bytes.max(1);
        self
    }

    pub fn with_max_stream_chunks(mut self, max_stream_chunks: usize) -> Self {
        self.max_stream_chunks = max_stream_chunks.max(1);
        self
    }

    async fn get_generate(&self, key: CacheKey) -> Option<GenerateResponse> {
        let mut state = self.state.lock().await;
        state.prune_expired(self.ttl);
        let expired = state
            .entries
            .get(&key)
            .map(|entry| entry.is_expired(self.ttl))
            .unwrap_or(true);
        if expired {
            state.remove_key(&key);
            return None;
        }

        let value = match state.entries.get(&key)?.value.clone() {
            CacheValue::Generate(resp) => Some(resp),
            CacheValue::Stream(_) => None,
        }?;
        state.touch_key(&key);
        Some(value)
    }

    async fn get_stream(&self, key: CacheKey) -> Option<Arc<Vec<StreamChunk>>> {
        let mut state = self.state.lock().await;
        state.prune_expired(self.ttl);
        let expired = state
            .entries
            .get(&key)
            .map(|entry| entry.is_expired(self.ttl))
            .unwrap_or(true);
        if expired {
            state.remove_key(&key);
            return None;
        }

        let value = match state.entries.get(&key)?.value.clone() {
            CacheValue::Stream(chunks) => Some(chunks),
            CacheValue::Generate(_) => None,
        }?;
        state.touch_key(&key);
        Some(value)
    }

    async fn insert_generate(&self, key: CacheKey, value: GenerateResponse) {
        let approx_bytes = approx_generate_response_bytes(&value);
        if approx_bytes > self.max_value_bytes {
            return;
        }

        let mut state = self.state.lock().await;
        state.prune_expired(self.ttl);
        state.insert(
            key,
            CacheEntry {
                inserted_at: Instant::now(),
                value: CacheValue::Generate(value),
            },
            self.max_entries,
        );
    }

    async fn insert_stream(&self, key: CacheKey, chunks: Vec<StreamChunk>, approx_bytes: usize) {
        if chunks.is_empty() {
            return;
        }
        if approx_bytes > self.max_value_bytes {
            return;
        }

        let mut state = self.state.lock().await;
        state.prune_expired(self.ttl);
        state.insert(
            key,
            CacheEntry {
                inserted_at: Instant::now(),
                value: CacheValue::Stream(Arc::new(chunks)),
            },
            self.max_entries,
        );
    }
}

impl Default for CacheLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl CacheState {
    fn insert(&mut self, key: CacheKey, entry: CacheEntry, max_entries: usize) {
        if let Some(existing) = self.entries.insert(key, entry) {
            let _ = existing;
            self.remove_key_from_lru(&key);
        }
        self.lru.push_back(key);
        self.evict_to_capacity(max_entries);
    }

    fn touch_key(&mut self, key: &CacheKey) {
        self.remove_key_from_lru(key);
        self.lru.push_back(*key);
    }

    fn remove_key(&mut self, key: &CacheKey) {
        self.entries.remove(key);
        self.remove_key_from_lru(key);
    }

    fn remove_key_from_lru(&mut self, key: &CacheKey) {
        if self.lru.is_empty() {
            return;
        }
        self.lru.retain(|k| k != key);
    }

    fn evict_to_capacity(&mut self, max_entries: usize) {
        while self.entries.len() > max_entries {
            let Some(oldest) = self.lru.pop_front() else {
                break;
            };
            self.entries.remove(&oldest);
        }
    }

    fn prune_expired(&mut self, ttl: Option<Duration>) {
        let Some(ttl) = ttl else {
            return;
        };

        loop {
            let Some(front) = self.lru.front().copied() else {
                break;
            };
            let Some(entry) = self.entries.get(&front) else {
                self.lru.pop_front();
                continue;
            };
            if entry.inserted_at.elapsed() < ttl {
                break;
            }
            self.lru.pop_front();
            self.entries.remove(&front);
        }
    }
}

impl CacheEntry {
    fn is_expired(&self, ttl: Option<Duration>) -> bool {
        ttl.map(|ttl| self.inserted_at.elapsed() >= ttl)
            .unwrap_or(false)
    }
}

#[async_trait]
impl LanguageModelLayer for CacheLayer {
    async fn generate(
        &self,
        inner: &dyn LanguageModel,
        request: GenerateRequest,
    ) -> Result<GenerateResponse> {
        let key = CacheKey {
            kind: CacheKind::Generate,
            hash: fingerprint_request(inner, &request)?,
        };
        if let Some(hit) = self.get_generate(key).await {
            return Ok(hit);
        }
        let response = inner.generate(request).await?;
        self.insert_generate(key, response.clone()).await;
        Ok(response)
    }

    async fn stream(
        &self,
        inner: &dyn LanguageModel,
        request: GenerateRequest,
    ) -> Result<StreamResult> {
        let key = CacheKey {
            kind: CacheKind::Stream,
            hash: fingerprint_request(inner, &request)?,
        };
        if let Some(hit) = self.get_stream(key).await {
            let chunks = hit.as_ref().clone();
            return Ok(stream::iter(chunks.into_iter().map(Ok)).boxed());
        }

        let inner_stream = inner.stream(request).await?;
        let layer = self.clone();

        struct RecordingState {
            stream: StreamResult,
            chunks: Vec<StreamChunk>,
            bytes: usize,
            cacheable: bool,
            done: bool,
        }

        let state = RecordingState {
            stream: inner_stream,
            chunks: Vec::new(),
            bytes: 0,
            cacheable: true,
            done: false,
        };

        let max_chunks = layer.max_stream_chunks;
        let max_bytes = layer.max_value_bytes;

        let out = stream::unfold(state, move |mut state| {
            let layer = layer.clone();
            async move {
                if state.done {
                    return None;
                }

                let next = state.stream.next().await;
                match next {
                    Some(Ok(chunk)) => {
                        if state.cacheable {
                            if state.chunks.len() >= max_chunks {
                                state.cacheable = false;
                                state.chunks.clear();
                            } else {
                                state.bytes = state
                                    .bytes
                                    .saturating_add(approx_stream_chunk_bytes(&chunk));
                                if state.bytes > max_bytes {
                                    state.cacheable = false;
                                    state.chunks.clear();
                                } else {
                                    state.chunks.push(chunk.clone());
                                }
                            }
                        }
                        Some((Ok(chunk), state))
                    }
                    Some(Err(err)) => {
                        state.cacheable = false;
                        state.chunks.clear();
                        state.done = true;
                        Some((Err(err), state))
                    }
                    None => {
                        if state.cacheable && !state.chunks.is_empty() {
                            layer.insert_stream(key, state.chunks, state.bytes).await;
                        }
                        None
                    }
                }
            }
        })
        .boxed();

        Ok(out)
    }
}

fn fingerprint_request(inner: &dyn LanguageModel, request: &GenerateRequest) -> Result<u64> {
    let serialized = serde_json::to_vec(request)?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    inner.provider().hash(&mut hasher);
    inner.model_id().hash(&mut hasher);
    serialized.hash(&mut hasher);
    Ok(hasher.finish())
}

fn approx_generate_response_bytes(resp: &GenerateResponse) -> usize {
    let mut total = 0usize;
    for part in &resp.content {
        total = total.saturating_add(part.approx_bytes());
    }
    total = total.saturating_add(resp.warnings.len().saturating_mul(64));
    total = total.saturating_add(128);
    total
}

fn approx_stream_chunk_bytes(chunk: &StreamChunk) -> usize {
    match chunk {
        StreamChunk::Warnings { warnings } => warnings.len().saturating_mul(64),
        StreamChunk::ResponseId { id } => id.len(),
        StreamChunk::TextDelta { text } => text.len(),
        StreamChunk::ToolCallStart { id, name } => id.len().saturating_add(name.len()),
        StreamChunk::ToolCallDelta {
            id,
            arguments_delta,
        } => id.len().saturating_add(arguments_delta.len()),
        StreamChunk::ReasoningDelta { text } => text.len(),
        StreamChunk::FinishReason(_) => 16,
        StreamChunk::Usage(_) => 64,
    }
}

trait ApproxBytes {
    fn approx_bytes(&self) -> usize;
}

impl ApproxBytes for crate::types::ContentPart {
    fn approx_bytes(&self) -> usize {
        match self {
            crate::types::ContentPart::Text { text } => text.len(),
            crate::types::ContentPart::Reasoning { text } => text.len(),
            crate::types::ContentPart::Image { .. } => 256,
            crate::types::ContentPart::File {
                filename,
                media_type,
                ..
            } => media_type
                .len()
                .saturating_add(filename.as_deref().map(str::len).unwrap_or(0))
                .saturating_add(256),
            crate::types::ContentPart::ToolCall { id, name, .. } => {
                id.len().saturating_add(name.len()).saturating_add(256)
            }
            crate::types::ContentPart::ToolResult {
                tool_call_id,
                content,
                ..
            } => tool_call_id.len().saturating_add(content.len()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use futures_util::StreamExt;
    use futures_util::stream;

    use super::*;
    use crate::types::{ContentPart, FinishReason, Message, Usage, Warning};

    #[derive(Clone)]
    struct FakeModel {
        generate_calls: Arc<AtomicUsize>,
        stream_calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl LanguageModel for FakeModel {
        fn provider(&self) -> &str {
            "fake"
        }

        fn model_id(&self) -> &str {
            "fake-model"
        }

        async fn generate(&self, _request: GenerateRequest) -> Result<GenerateResponse> {
            let n = self.generate_calls.fetch_add(1, Ordering::SeqCst);
            Ok(GenerateResponse {
                content: vec![ContentPart::Text {
                    text: format!("hello-{n}"),
                }],
                finish_reason: FinishReason::Stop,
                usage: Usage::default(),
                warnings: Vec::new(),
                provider_metadata: None,
            })
        }

        async fn stream(&self, _request: GenerateRequest) -> Result<StreamResult> {
            let n = self.stream_calls.fetch_add(1, Ordering::SeqCst);
            let chunks = vec![
                Ok(StreamChunk::ResponseId {
                    id: format!("resp-{n}"),
                }),
                Ok(StreamChunk::Warnings {
                    warnings: vec![Warning::Other {
                        message: "hello".to_string(),
                    }],
                }),
                Ok(StreamChunk::TextDelta {
                    text: format!("hi-{n}"),
                }),
                Ok(StreamChunk::Usage(Usage::default())),
                Ok(StreamChunk::FinishReason(FinishReason::Stop)),
            ];
            Ok(stream::iter(chunks).boxed())
        }
    }

    #[tokio::test]
    async fn caches_generate_responses() -> Result<()> {
        let model = FakeModel {
            generate_calls: Arc::new(AtomicUsize::new(0)),
            stream_calls: Arc::new(AtomicUsize::new(0)),
        };

        let cached = crate::LayeredLanguageModel::new(Arc::new(model), CacheLayer::new());
        let req: GenerateRequest = vec![Message::user("hi")].into();

        let a = cached.generate(req.clone()).await?;
        let b = cached.generate(req.clone()).await?;

        assert_eq!(a.text(), "hello-0");
        assert_eq!(b.text(), "hello-0");
        Ok(())
    }

    #[tokio::test]
    async fn caches_streams_and_replays_chunks() -> Result<()> {
        let stream_calls = Arc::new(AtomicUsize::new(0));
        let model = FakeModel {
            generate_calls: Arc::new(AtomicUsize::new(0)),
            stream_calls: Arc::clone(&stream_calls),
        };

        let cached = crate::LayeredLanguageModel::new(Arc::new(model), CacheLayer::new());
        let req: GenerateRequest = vec![Message::user("hi")].into();

        let a_chunks: Vec<StreamChunk> = cached
            .stream(req.clone())
            .await?
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>>>()?;

        let b_chunks: Vec<StreamChunk> = cached
            .stream(req.clone())
            .await?
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>>>()?;

        assert_eq!(a_chunks, b_chunks);
        assert_eq!(stream_calls.load(Ordering::SeqCst), 1);
        Ok(())
    }
}
