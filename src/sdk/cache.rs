use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::io;
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
    last_prune_at: Option<Instant>,
}

#[derive(Debug)]
struct CacheEntry {
    inserted_at: Instant,
    value: CacheValue,
}

#[derive(Debug, Clone)]
enum CacheValue {
    Generate(Arc<GenerateResponse>),
    Stream(Arc<[StreamChunk]>),
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
    const READ_PRUNE_INTERVAL: Duration = Duration::from_secs(1);

    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(CacheState {
                entries: HashMap::new(),
                lru: VecDeque::new(),
                last_prune_at: None,
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
        state.maybe_prune_expired_on_read(self.ttl, Self::READ_PRUNE_INTERVAL);
        let entry = state.entries.get(&key)?;
        let (expired, value) = if entry.is_expired(self.ttl) {
            (true, None)
        } else {
            let value = match &entry.value {
                CacheValue::Generate(resp) => Some(Arc::clone(resp)),
                CacheValue::Stream(_) => None,
            };
            (false, value)
        };
        if expired {
            state.remove_key(&key);
            return None;
        }
        let value = value?;
        state.touch_key(&key);
        drop(state);
        Some((*value).clone())
    }

    async fn get_stream(&self, key: CacheKey) -> Option<Arc<[StreamChunk]>> {
        let mut state = self.state.lock().await;
        state.maybe_prune_expired_on_read(self.ttl, Self::READ_PRUNE_INTERVAL);
        let entry = state.entries.get(&key)?;
        let (expired, value) = if entry.is_expired(self.ttl) {
            (true, None)
        } else {
            let value = match &entry.value {
                CacheValue::Stream(chunks) => Some(chunks.clone()),
                CacheValue::Generate(_) => None,
            };
            (false, value)
        };
        if expired {
            state.remove_key(&key);
            return None;
        }
        let value = value?;
        state.touch_key(&key);
        drop(state);
        Some(value)
    }

    async fn insert_generate(&self, key: CacheKey, value: &GenerateResponse) {
        let approx_bytes = approx_generate_response_bytes(value);
        if approx_bytes > self.max_value_bytes {
            return;
        }

        let value = Arc::new(value.clone());
        let mut state = self.state.lock().await;
        if let Some(ttl) = self.ttl {
            state.prune_expired(ttl, Instant::now());
        }
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
        if let Some(ttl) = self.ttl {
            state.prune_expired(ttl, Instant::now());
        }
        state.insert(
            key,
            CacheEntry {
                inserted_at: Instant::now(),
                value: CacheValue::Stream(Arc::from(chunks)),
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
    fn maybe_prune_expired_on_read(&mut self, ttl: Option<Duration>, interval: Duration) {
        let Some(ttl) = ttl else {
            return;
        };
        let now = Instant::now();
        if self
            .last_prune_at
            .is_some_and(|last| now.saturating_duration_since(last) < interval)
        {
            return;
        }
        self.prune_expired(ttl, now);
    }

    fn insert(&mut self, key: CacheKey, entry: CacheEntry, max_entries: usize) {
        if self.entries.insert(key, entry).is_some() {
            self.remove_key_from_lru(&key);
        }
        self.lru.push_back(key);
        self.evict_to_capacity(max_entries);
    }

    fn touch_key(&mut self, key: &CacheKey) {
        if self.lru.back().is_some_and(|existing| existing == key) {
            return;
        }
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
        if self.lru.front().is_some_and(|candidate| candidate == key) {
            self.lru.pop_front();
            return;
        }
        if self.lru.back().is_some_and(|candidate| candidate == key) {
            self.lru.pop_back();
            return;
        }
        if let Some(index) = self.lru.iter().position(|candidate| candidate == key) {
            self.lru.remove(index);
        }
    }

    fn evict_to_capacity(&mut self, max_entries: usize) {
        while self.entries.len() > max_entries {
            let Some(oldest) = self.lru.pop_front() else {
                break;
            };
            self.entries.remove(&oldest);
        }
    }

    fn prune_expired(&mut self, ttl: Duration, now: Instant) {
        if self.lru.is_empty() {
            self.last_prune_at = Some(now);
            return;
        }

        let mut keep = VecDeque::with_capacity(self.lru.len());
        while let Some(key) = self.lru.pop_front() {
            let expired = self
                .entries
                .get(&key)
                .map(|entry| now.saturating_duration_since(entry.inserted_at) >= ttl)
                .unwrap_or(true);
            if expired {
                self.entries.remove(&key);
            } else {
                keep.push_back(key);
            }
        }
        self.lru = keep;
        self.last_prune_at = Some(now);
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
        self.insert_generate(key, &response).await;
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
            let replay = stream::unfold((hit, 0usize), |(chunks, idx)| async move {
                if idx >= chunks.len() {
                    return None;
                }
                let chunk = chunks[idx].clone();
                Some((Ok(chunk), (chunks, idx.saturating_add(1))))
            })
            .boxed();
            return Ok(replay);
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
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    inner.provider().hash(&mut hasher);
    inner.model_id().hash(&mut hasher);
    serde_json::to_writer(HasherWriter(&mut hasher), request)?;
    Ok(hasher.finish())
}

fn approx_generate_response_bytes(resp: &GenerateResponse) -> usize {
    let mut total = 0usize;
    for part in &resp.content {
        total = total.saturating_add(part.approx_bytes());
    }
    for warning in &resp.warnings {
        total = total.saturating_add(approx_warning_bytes(warning));
    }
    total = total.saturating_add(
        resp.provider_metadata
            .as_ref()
            .map(approx_json_value_bytes)
            .unwrap_or(0),
    );
    total = total.saturating_add(64);
    total
}

fn approx_stream_chunk_bytes(chunk: &StreamChunk) -> usize {
    match chunk {
        StreamChunk::Warnings { warnings } => warnings.iter().fold(0usize, |total, warning| {
            total.saturating_add(approx_warning_bytes(warning))
        }),
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

struct HasherWriter<'a, H>(&'a mut H);

impl<H: Hasher> io::Write for HasherWriter<'_, H> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl ApproxBytes for crate::types::ContentPart {
    fn approx_bytes(&self) -> usize {
        match self {
            crate::types::ContentPart::Text { text } => text.len(),
            crate::types::ContentPart::Reasoning { text } => text.len(),
            crate::types::ContentPart::Image { source } => match source {
                crate::types::ImageSource::Url { url } => url.len(),
                crate::types::ImageSource::Base64 { media_type, data } => {
                    media_type.len().saturating_add(data.len())
                }
            },
            crate::types::ContentPart::File {
                filename,
                media_type,
                source,
            } => {
                let source_bytes = match source {
                    crate::types::FileSource::Url { url } => url.len(),
                    crate::types::FileSource::Base64 { data } => data.len(),
                    crate::types::FileSource::FileId { file_id } => file_id.len(),
                };
                media_type
                    .len()
                    .saturating_add(filename.as_deref().map(str::len).unwrap_or(0))
                    .saturating_add(source_bytes)
            }
            crate::types::ContentPart::ToolCall {
                id,
                name,
                arguments,
            } => id
                .len()
                .saturating_add(name.len())
                .saturating_add(approx_json_value_bytes(arguments)),
            crate::types::ContentPart::ToolResult {
                tool_call_id,
                content,
                is_error,
            } => tool_call_id
                .len()
                .saturating_add(content.len())
                .saturating_add(usize::from(is_error.is_some())),
        }
    }
}

fn approx_warning_bytes(warning: &crate::types::Warning) -> usize {
    match warning {
        crate::types::Warning::Unsupported { feature, details } => feature
            .len()
            .saturating_add(details.as_deref().map(str::len).unwrap_or(0)),
        crate::types::Warning::Clamped {
            parameter,
            original: _,
            clamped_to: _,
        } => parameter.len().saturating_add(16),
        crate::types::Warning::Compatibility { feature, details } => {
            feature.len().saturating_add(details.len())
        }
        crate::types::Warning::Other { message } => message.len(),
    }
}

fn approx_json_value_bytes(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Null => 0,
        serde_json::Value::Bool(_) => 1,
        serde_json::Value::Number(number) => number.to_string().len(),
        serde_json::Value::String(text) => text.len(),
        serde_json::Value::Array(items) => {
            let mut total = 2usize;
            for item in items {
                total = total
                    .saturating_add(1)
                    .saturating_add(approx_json_value_bytes(item));
            }
            total
        }
        serde_json::Value::Object(items) => {
            let mut total = 2usize;
            for (key, item) in items {
                total = total
                    .saturating_add(key.len())
                    .saturating_add(3)
                    .saturating_add(approx_json_value_bytes(item));
            }
            total
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, VecDeque};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

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
    async fn generate_with_large_tool_arguments_is_not_cached_when_over_budget() -> Result<()> {
        #[derive(Clone)]
        struct LargeToolArgsModel {
            calls: Arc<AtomicUsize>,
        }

        #[async_trait]
        impl LanguageModel for LargeToolArgsModel {
            fn provider(&self) -> &str {
                "fake"
            }

            fn model_id(&self) -> &str {
                "fake-model"
            }

            async fn generate(&self, _request: GenerateRequest) -> Result<GenerateResponse> {
                let n = self.calls.fetch_add(1, Ordering::SeqCst);
                Ok(GenerateResponse {
                    content: vec![ContentPart::ToolCall {
                        id: "call_1".to_string(),
                        name: "tool".to_string(),
                        arguments: serde_json::json!({
                            "payload": "x".repeat(2_048),
                            "n": n,
                        }),
                    }],
                    finish_reason: FinishReason::ToolCalls,
                    usage: Usage::default(),
                    warnings: Vec::new(),
                    provider_metadata: None,
                })
            }

            async fn stream(&self, _request: GenerateRequest) -> Result<StreamResult> {
                Ok(stream::empty().boxed())
            }
        }

        let calls = Arc::new(AtomicUsize::new(0));
        let model = LargeToolArgsModel {
            calls: Arc::clone(&calls),
        };
        let cached = crate::LayeredLanguageModel::new(
            Arc::new(model),
            CacheLayer::new().with_max_value_bytes(512),
        );
        let req: GenerateRequest = vec![Message::user("hi")].into();

        let first = cached.generate(req.clone()).await?;
        let second = cached.generate(req).await?;

        let first_n = first
            .content
            .first()
            .and_then(|part| match part {
                ContentPart::ToolCall { arguments, .. } => arguments.get("n"),
                _ => None,
            })
            .and_then(serde_json::Value::as_u64);
        let second_n = second
            .content
            .first()
            .and_then(|part| match part {
                ContentPart::ToolCall { arguments, .. } => arguments.get("n"),
                _ => None,
            })
            .and_then(serde_json::Value::as_u64);

        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_ne!(first_n, second_n);
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

    #[tokio::test]
    async fn stream_with_large_warning_is_not_cached_when_over_budget() -> Result<()> {
        #[derive(Clone)]
        struct LargeWarningModel {
            stream_calls: Arc<AtomicUsize>,
        }

        #[async_trait]
        impl LanguageModel for LargeWarningModel {
            fn provider(&self) -> &str {
                "fake"
            }

            fn model_id(&self) -> &str {
                "fake-model"
            }

            async fn generate(&self, _request: GenerateRequest) -> Result<GenerateResponse> {
                Ok(GenerateResponse {
                    content: vec![ContentPart::Text {
                        text: "unused".to_string(),
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
                    Ok(StreamChunk::Warnings {
                        warnings: vec![Warning::Other {
                            message: format!("warn-{n}-{}", "x".repeat(2_048)),
                        }],
                    }),
                    Ok(StreamChunk::TextDelta {
                        text: "ok".to_string(),
                    }),
                    Ok(StreamChunk::FinishReason(FinishReason::Stop)),
                ];
                Ok(stream::iter(chunks).boxed())
            }
        }

        let stream_calls = Arc::new(AtomicUsize::new(0));
        let model = LargeWarningModel {
            stream_calls: Arc::clone(&stream_calls),
        };

        let cached = crate::LayeredLanguageModel::new(
            Arc::new(model),
            CacheLayer::new().with_max_value_bytes(256),
        );
        let req: GenerateRequest = vec![Message::user("hi")].into();

        let _first: Vec<StreamChunk> = cached
            .stream(req.clone())
            .await?
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>>>()?;

        let _second: Vec<StreamChunk> = cached
            .stream(req)
            .await?
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>>>()?;

        assert_eq!(stream_calls.load(Ordering::SeqCst), 2);
        Ok(())
    }

    #[test]
    fn prune_expired_removes_non_front_expired_entries() {
        let fresh_key = CacheKey {
            kind: CacheKind::Generate,
            hash: 1,
        };
        let expired_key = CacheKey {
            kind: CacheKind::Generate,
            hash: 2,
        };

        let response = GenerateResponse {
            content: vec![ContentPart::Text {
                text: "ok".to_string(),
            }],
            finish_reason: FinishReason::Stop,
            usage: Usage::default(),
            warnings: Vec::new(),
            provider_metadata: None,
        };
        let now = Instant::now();

        let mut state = CacheState {
            entries: HashMap::from([
                (
                    fresh_key,
                    CacheEntry {
                        inserted_at: now,
                        value: CacheValue::Generate(Arc::new(response.clone())),
                    },
                ),
                (
                    expired_key,
                    CacheEntry {
                        inserted_at: now - Duration::from_secs(10),
                        value: CacheValue::Generate(Arc::new(response)),
                    },
                ),
            ]),
            lru: VecDeque::from([fresh_key, expired_key]),
            last_prune_at: None,
        };

        state.prune_expired(Duration::from_secs(5), now);

        assert!(state.entries.contains_key(&fresh_key));
        assert!(!state.entries.contains_key(&expired_key));
        assert_eq!(state.lru, VecDeque::from([fresh_key]));
    }

    #[test]
    fn touch_key_hot_entry_avoids_duplicate_lru_nodes() {
        let key = CacheKey {
            kind: CacheKind::Generate,
            hash: 1,
        };
        let response = GenerateResponse {
            content: vec![ContentPart::Text {
                text: "ok".to_string(),
            }],
            finish_reason: FinishReason::Stop,
            usage: Usage::default(),
            warnings: Vec::new(),
            provider_metadata: None,
        };
        let now = Instant::now();
        let mut state = CacheState {
            entries: HashMap::from([(
                key,
                CacheEntry {
                    inserted_at: now,
                    value: CacheValue::Generate(Arc::new(response)),
                },
            )]),
            lru: VecDeque::from([key]),
            last_prune_at: None,
        };

        for _ in 0..5 {
            state.touch_key(&key);
        }

        assert_eq!(state.lru, VecDeque::from([key]));
    }

    #[tokio::test]
    async fn get_generate_still_rejects_expired_entry_when_read_prune_is_throttled() {
        let key = CacheKey {
            kind: CacheKind::Generate,
            hash: 42,
        };
        let response = GenerateResponse {
            content: vec![ContentPart::Text {
                text: "stale".to_string(),
            }],
            finish_reason: FinishReason::Stop,
            usage: Usage::default(),
            warnings: Vec::new(),
            provider_metadata: None,
        };

        let layer = CacheLayer::new().with_ttl(Duration::from_secs(5));
        {
            let mut state = layer.state.lock().await;
            state.entries.insert(
                key,
                CacheEntry {
                    inserted_at: Instant::now() - Duration::from_secs(10),
                    value: CacheValue::Generate(Arc::new(response)),
                },
            );
            state.lru.push_back(key);
            state.last_prune_at = Some(Instant::now());
        }

        assert!(layer.get_generate(key).await.is_none());

        let state = layer.state.lock().await;
        assert!(!state.entries.contains_key(&key));
        assert!(!state.lru.contains(&key));
    }
}
