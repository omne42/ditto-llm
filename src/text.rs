use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures_util::StreamExt;
use futures_util::stream;
use serde_json::Value;
use tokio::sync::{Notify, mpsc};

use crate::model::{LanguageModel, StreamResult};
use crate::stream::StreamCollector;
use crate::types::{FinishReason, GenerateRequest, GenerateResponse, StreamChunk, Usage, Warning};
use crate::utils::task::AbortOnDrop;
use crate::{DittoError, Result};

#[derive(Debug, Clone)]
pub struct GenerateTextResponse {
    pub text: String,
    pub response: GenerateResponse,
}

#[derive(Debug, Clone)]
pub struct StreamTextFinal {
    pub text: String,
    pub response_id: Option<String>,
    pub warnings: Vec<Warning>,
    pub finish_reason: FinishReason,
    pub usage: Usage,
}

#[derive(Debug, Default)]
struct StreamTextState {
    done: bool,
    final_response: Option<GenerateResponse>,
    final_error: Option<String>,
}

#[derive(Clone)]
pub struct StreamTextHandle {
    state: Arc<Mutex<StreamTextState>>,
}

impl StreamTextHandle {
    pub fn is_done(&self) -> bool {
        self.state.lock().map(|s| s.done).unwrap_or(false)
    }

    pub fn final_response(&self) -> Result<Option<GenerateResponse>> {
        let state = self.state.lock().map_err(|_| {
            DittoError::InvalidResponse("stream text state lock is poisoned".to_string())
        })?;
        if !state.done {
            return Ok(None);
        }
        if let Some(err) = state.final_error.as_deref() {
            return Err(DittoError::InvalidResponse(err.to_string()));
        }
        Ok(state.final_response.clone())
    }

    pub fn final_text(&self) -> Result<Option<String>> {
        Ok(self.final_response()?.map(|resp| resp.text()))
    }

    pub fn final_summary(&self) -> Result<Option<StreamTextFinal>> {
        let response = self.final_response()?;
        let Some(response) = response else {
            return Ok(None);
        };
        let mut usage = response.usage.clone();
        usage.merge_total();
        Ok(Some(StreamTextFinal {
            text: response.text(),
            response_id: response
                .provider_metadata
                .as_ref()
                .and_then(|m| m.get("id"))
                .and_then(Value::as_str)
                .map(|id| id.to_string()),
            warnings: response.warnings.clone(),
            finish_reason: response.finish_reason,
            usage,
        }))
    }
}

pub struct StreamTextResult {
    handle: StreamTextHandle,
    ready: Arc<Notify>,
    text_enabled: Arc<AtomicBool>,
    full_enabled: Arc<AtomicBool>,
    pub text_stream: stream::BoxStream<'static, Result<String>>,
    pub full_stream: stream::BoxStream<'static, Result<StreamChunk>>,
}

impl StreamTextResult {
    pub fn handle(&self) -> StreamTextHandle {
        self.handle.clone()
    }

    pub fn into_text_stream(
        self,
    ) -> (StreamTextHandle, stream::BoxStream<'static, Result<String>>) {
        self.text_enabled.store(true, Ordering::Relaxed);
        self.full_enabled.store(false, Ordering::Relaxed);
        self.ready.notify_one();
        (self.handle, self.text_stream)
    }

    pub fn into_full_stream(
        self,
    ) -> (
        StreamTextHandle,
        stream::BoxStream<'static, Result<StreamChunk>>,
    ) {
        self.text_enabled.store(false, Ordering::Relaxed);
        self.full_enabled.store(true, Ordering::Relaxed);
        self.ready.notify_one();
        (self.handle, self.full_stream)
    }

    pub fn into_streams(
        self,
    ) -> (
        StreamTextHandle,
        stream::BoxStream<'static, Result<String>>,
        stream::BoxStream<'static, Result<StreamChunk>>,
    ) {
        self.text_enabled.store(true, Ordering::Relaxed);
        self.full_enabled.store(true, Ordering::Relaxed);
        self.ready.notify_one();
        (self.handle, self.text_stream, self.full_stream)
    }

    pub fn is_done(&self) -> bool {
        self.handle.is_done()
    }

    pub fn final_response(&self) -> Result<Option<GenerateResponse>> {
        self.handle.final_response()
    }

    pub fn final_text(&self) -> Result<Option<String>> {
        self.handle.final_text()
    }

    pub fn final_summary(&self) -> Result<Option<StreamTextFinal>> {
        self.handle.final_summary()
    }
}

#[async_trait]
pub trait LanguageModelTextExt: LanguageModel {
    async fn generate_text(&self, request: GenerateRequest) -> Result<GenerateTextResponse> {
        let response = self.generate(request).await?;
        let text = response.text();
        Ok(GenerateTextResponse { text, response })
    }

    async fn stream_text(&self, request: GenerateRequest) -> Result<StreamTextResult> {
        let stream = self.stream(request).await?;
        Ok(stream_text_from_stream(stream))
    }
}

impl<T> LanguageModelTextExt for T where T: LanguageModel + ?Sized {}

pub fn stream_text_from_stream(stream: StreamResult) -> StreamTextResult {
    const FANOUT_BUFFER: usize = 256;

    let state = Arc::new(Mutex::new(StreamTextState::default()));
    let state_task = state.clone();
    let handle = StreamTextHandle { state };

    let ready = Arc::new(Notify::new());
    let text_enabled = Arc::new(AtomicBool::new(false));
    let full_enabled = Arc::new(AtomicBool::new(false));

    let ready_task = ready.clone();
    let text_enabled_task = text_enabled.clone();
    let full_enabled_task = full_enabled.clone();

    let (text_tx, text_rx) = mpsc::channel::<Result<String>>(FANOUT_BUFFER);
    let (full_tx, full_rx) = mpsc::channel::<Result<StreamChunk>>(FANOUT_BUFFER);

    let task = tokio::spawn(async move {
        let mut inner = stream;

        loop {
            if text_enabled_task.load(Ordering::Acquire)
                || full_enabled_task.load(Ordering::Acquire)
            {
                break;
            }
            ready_task.notified().await;
        }

        let mut collector = StreamCollector::default();

        while let Some(next) = inner.next().await {
            match next {
                Ok(chunk) => {
                    if let StreamChunk::TextDelta { text } = &chunk {
                        if !text.is_empty() && text_enabled_task.load(Ordering::Relaxed) {
                            let _ = text_tx.send(Ok(text.to_string())).await;
                        }
                    }

                    collector.observe(&chunk);
                    if full_enabled_task.load(Ordering::Relaxed) {
                        let _ = full_tx.send(Ok(chunk)).await;
                    }
                }
                Err(err) => {
                    let err_string = err.to_string();
                    if let Ok(mut state) = state_task.lock() {
                        state.done = true;
                        state.final_error = Some(format!("stream failed: {err_string}"));
                    }
                    if text_enabled_task.load(Ordering::Relaxed) {
                        let _ = text_tx
                            .send(Err(DittoError::InvalidResponse(err_string)))
                            .await;
                    }
                    if full_enabled_task.load(Ordering::Relaxed) {
                        let _ = full_tx.send(Err(err)).await;
                    }
                    return;
                }
            }
        }

        let response = collector.finish();

        if let Ok(mut state) = state_task.lock() {
            state.done = true;
            state.final_response = Some(response);
        }
    });

    let aborter = Arc::new(AbortOnDrop::new(task.abort_handle()));

    let text_stream = stream::unfold(
        (
            text_rx,
            aborter.clone(),
            text_enabled.clone(),
            ready.clone(),
        ),
        |(mut rx, aborter, enabled, ready)| async move {
            if !enabled.swap(true, Ordering::AcqRel) {
                ready.notify_one();
            }
            rx.recv()
                .await
                .map(|item| (item, (rx, aborter, enabled, ready)))
        },
    )
    .boxed();

    let full_stream = stream::unfold(
        (full_rx, aborter, full_enabled.clone(), ready.clone()),
        |(mut rx, aborter, enabled, ready)| async move {
            if !enabled.swap(true, Ordering::AcqRel) {
                ready.notify_one();
            }
            rx.recv()
                .await
                .map(|item| (item, (rx, aborter, enabled, ready)))
        },
    )
    .boxed();

    StreamTextResult {
        handle,
        ready,
        text_enabled,
        full_enabled,
        text_stream,
        full_stream,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ContentPart;
    use futures_util::StreamExt;
    use serde_json::json;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::{Context, Poll};

    struct DropFlagStream {
        dropped: Arc<AtomicBool>,
    }

    impl futures_util::Stream for DropFlagStream {
        type Item = Result<StreamChunk>;

        fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            Poll::Pending
        }
    }

    impl Drop for DropFlagStream {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
        }
    }

    struct FakeModel {
        provider: &'static str,
        response: GenerateResponse,
    }

    #[async_trait]
    impl LanguageModel for FakeModel {
        fn provider(&self) -> &str {
            self.provider
        }

        fn model_id(&self) -> &str {
            "fake"
        }

        async fn generate(&self, _request: GenerateRequest) -> Result<GenerateResponse> {
            Ok(self.response.clone())
        }

        async fn stream(&self, _request: GenerateRequest) -> Result<StreamResult> {
            Err(DittoError::InvalidResponse("not implemented".to_string()))
        }
    }

    #[tokio::test]
    async fn generate_text_returns_text_and_response() -> Result<()> {
        let model = FakeModel {
            provider: "openai",
            response: GenerateResponse {
                content: vec![
                    ContentPart::Reasoning {
                        text: "think".to_string(),
                    },
                    ContentPart::Text {
                        text: "hello".to_string(),
                    },
                ],
                ..GenerateResponse::default()
            },
        };

        let out = model.generate_text(GenerateRequest::from(vec![])).await?;
        assert_eq!(out.text, "hello".to_string());
        assert_eq!(out.response.text(), "hello".to_string());
        Ok(())
    }

    #[tokio::test]
    async fn stream_text_fans_out_text_and_collects_final_response() -> Result<()> {
        let chunks = vec![
            Ok(StreamChunk::Warnings {
                warnings: vec![Warning::Other {
                    message: "pre".to_string(),
                }],
            }),
            Ok(StreamChunk::ResponseId {
                id: "resp_123".to_string(),
            }),
            Ok(StreamChunk::TextDelta {
                text: "hello".to_string(),
            }),
            Ok(StreamChunk::TextDelta {
                text: " world".to_string(),
            }),
            Ok(StreamChunk::ReasoningDelta {
                text: "think".to_string(),
            }),
            Ok(StreamChunk::ToolCallStart {
                id: "call_1".to_string(),
                name: "add".to_string(),
            }),
            Ok(StreamChunk::ToolCallDelta {
                id: "call_1".to_string(),
                arguments_delta: "{\"a\":1}".to_string(),
            }),
            Ok(StreamChunk::Usage(Usage {
                input_tokens: Some(3),
                cache_input_tokens: None,
                cache_creation_input_tokens: None,
                output_tokens: Some(7),
                total_tokens: None,
            })),
            Ok(StreamChunk::FinishReason(FinishReason::ToolCalls)),
        ];

        let inner: StreamResult = stream::iter(chunks).boxed();
        let (handle, mut text_stream, mut full_stream) =
            stream_text_from_stream(inner).into_streams();

        let collect_text = async move {
            let mut deltas = String::new();
            while let Some(delta) = text_stream.next().await {
                deltas.push_str(&delta?);
            }
            Ok::<_, DittoError>(deltas)
        };

        let collect_full = async move {
            let mut seen_finish = false;
            while let Some(evt) = full_stream.next().await {
                if matches!(evt?, StreamChunk::FinishReason(_)) {
                    seen_finish = true;
                }
            }
            Ok::<_, DittoError>(seen_finish)
        };

        let (deltas, seen_finish) = tokio::try_join!(collect_text, collect_full)?;
        assert_eq!(deltas, "hello world".to_string());
        assert!(seen_finish);

        let response = handle.final_response()?.unwrap();
        assert_eq!(response.text(), "hello world".to_string());
        assert_eq!(response.finish_reason, FinishReason::ToolCalls);
        assert_eq!(response.usage.total_tokens, Some(10));
        assert!(response.content.iter().any(|part| matches!(
            part,
            ContentPart::ToolCall { id, name, arguments } if id == "call_1" && name == "add" && arguments == &json!({"a":1})
        )));
        Ok(())
    }

    #[tokio::test]
    async fn dropping_streams_aborts_background_task() -> Result<()> {
        let dropped = Arc::new(AtomicBool::new(false));
        let inner: StreamResult = Box::pin(DropFlagStream {
            dropped: dropped.clone(),
        })
        .boxed();

        let StreamTextResult {
            text_stream,
            full_stream,
            ..
        } = stream_text_from_stream(inner);

        drop(text_stream);
        drop(full_stream);

        for _ in 0..16 {
            if dropped.load(Ordering::SeqCst) {
                break;
            }
            tokio::task::yield_now().await;
        }

        assert!(dropped.load(Ordering::SeqCst));
        Ok(())
    }
}
