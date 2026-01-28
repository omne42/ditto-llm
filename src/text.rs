use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures_util::StreamExt;
use futures_util::stream;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::model::{LanguageModel, StreamResult};
use crate::types::{
    ContentPart, FinishReason, GenerateRequest, GenerateResponse, StreamChunk, Usage, Warning,
};
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

pub struct StreamTextResult {
    state: Arc<Mutex<StreamTextState>>,
    pub text_stream: stream::BoxStream<'static, Result<String>>,
    pub full_stream: stream::BoxStream<'static, Result<StreamChunk>>,
}

struct TaskAbortOnDrop(tokio::task::AbortHandle);

impl Drop for TaskAbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl StreamTextResult {
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
    let state = Arc::new(Mutex::new(StreamTextState::default()));
    let state_task = state.clone();

    let (text_tx, text_rx) = mpsc::unbounded_channel::<Result<String>>();
    let (full_tx, full_rx) = mpsc::unbounded_channel::<Result<StreamChunk>>();

    let task = tokio::spawn(async move {
        let mut inner = stream;

        let mut warnings = Vec::<Warning>::new();
        let mut response_id: Option<String> = None;
        let mut finish_reason = FinishReason::Unknown;
        let mut usage = Usage::default();

        let mut parts = Vec::<CollectedPart>::new();
        let mut tool_calls = HashMap::<String, ToolCallBuffer>::new();
        let mut seen_tool_call_ids = HashSet::<String>::new();

        while let Some(next) = inner.next().await {
            match next {
                Ok(chunk) => {
                    match &chunk {
                        StreamChunk::Warnings { warnings: w } => warnings.extend(w.clone()),
                        StreamChunk::ResponseId { id } => {
                            if response_id.is_none() && !id.trim().is_empty() {
                                response_id = Some(id.to_string());
                            }
                        }
                        StreamChunk::TextDelta { text } => {
                            if !text.is_empty() {
                                match parts.last_mut() {
                                    Some(CollectedPart::Text(existing)) => existing.push_str(text),
                                    _ => parts.push(CollectedPart::Text(text.to_string())),
                                }
                                let _ = text_tx.send(Ok(text.to_string()));
                            }
                        }
                        StreamChunk::ReasoningDelta { text } => {
                            if !text.is_empty() {
                                match parts.last_mut() {
                                    Some(CollectedPart::Reasoning(existing)) => {
                                        existing.push_str(text);
                                    }
                                    _ => parts.push(CollectedPart::Reasoning(text.to_string())),
                                }
                            }
                        }
                        StreamChunk::ToolCallStart { id, name } => {
                            if id.trim().is_empty() {
                                warnings.push(Warning::Compatibility {
                                    feature: "tool_call.id".to_string(),
                                    details:
                                        "stream emitted an empty tool_call id; dropping tool call"
                                            .to_string(),
                                });
                            } else {
                                let slot = tool_calls.entry(id.clone()).or_default();
                                if slot.name.is_none() && !name.trim().is_empty() {
                                    slot.name = Some(name.to_string());
                                }
                                if seen_tool_call_ids.insert(id.clone()) {
                                    parts.push(CollectedPart::ToolCall { id: id.clone() });
                                }
                            }
                        }
                        StreamChunk::ToolCallDelta {
                            id,
                            arguments_delta,
                        } => {
                            if id.trim().is_empty() {
                                warnings.push(Warning::Compatibility {
                                    feature: "tool_call.id".to_string(),
                                    details: "stream emitted an empty tool_call id for arguments; dropping tool call delta".to_string(),
                                });
                            } else {
                                let slot = tool_calls.entry(id.clone()).or_default();
                                slot.arguments.push_str(arguments_delta);
                                if seen_tool_call_ids.insert(id.clone()) {
                                    parts.push(CollectedPart::ToolCall { id: id.clone() });
                                }
                            }
                        }
                        StreamChunk::FinishReason(reason) => finish_reason = *reason,
                        StreamChunk::Usage(u) => usage = u.clone(),
                    }

                    let _ = full_tx.send(Ok(chunk));
                }
                Err(err) => {
                    let err_string = err.to_string();
                    if let Ok(mut state) = state_task.lock() {
                        state.done = true;
                        state.final_error = Some(format!("stream failed: {err_string}"));
                    }
                    let _ = text_tx.send(Err(DittoError::InvalidResponse(err_string)));
                    let _ = full_tx.send(Err(err));
                    return;
                }
            }
        }

        usage.merge_total();

        let mut content = Vec::<ContentPart>::new();
        for part in parts {
            match part {
                CollectedPart::Text(text) => {
                    if !text.is_empty() {
                        content.push(ContentPart::Text { text });
                    }
                }
                CollectedPart::Reasoning(text) => {
                    if !text.is_empty() {
                        content.push(ContentPart::Reasoning { text });
                    }
                }
                CollectedPart::ToolCall { id: tool_call_id } => {
                    let Some(tool_call) = tool_calls.get(&tool_call_id) else {
                        continue;
                    };
                    let Some(name) = tool_call
                        .name
                        .as_deref()
                        .map(str::trim)
                        .filter(|name| !name.is_empty())
                    else {
                        warnings.push(Warning::Compatibility {
                            feature: "tool_call.name".to_string(),
                            details: format!(
                                "stream ended before tool_call name was received for id={tool_call_id}; dropping tool call"
                            ),
                        });
                        continue;
                    };

                    let raw = tool_call.arguments.trim();
                    let raw_json = if raw.is_empty() { "{}" } else { raw };
                    let arguments = serde_json::from_str::<Value>(raw_json).unwrap_or_else(|err| {
                        warnings.push(Warning::Compatibility {
                            feature: "tool_call.arguments".to_string(),
                            details: format!(
                                "failed to parse tool_call arguments as JSON for id={tool_call_id}: {err}; preserving raw string"
                            ),
                        });
                        Value::String(tool_call.arguments.clone())
                    });

                    content.push(ContentPart::ToolCall {
                        id: tool_call_id,
                        name: name.to_string(),
                        arguments,
                    });
                }
            }
        }

        let provider_metadata = response_id
            .as_deref()
            .map(|id| serde_json::json!({ "id": id }));

        let response = GenerateResponse {
            content,
            finish_reason,
            usage,
            warnings,
            provider_metadata,
        };

        if let Ok(mut state) = state_task.lock() {
            state.done = true;
            state.final_response = Some(response);
        }
    });

    let aborter = Arc::new(TaskAbortOnDrop(task.abort_handle()));

    let text_stream = stream::unfold((text_rx, aborter.clone()), |(mut rx, aborter)| async move {
        rx.recv().await.map(|item| (item, (rx, aborter)))
    })
    .boxed();

    let full_stream = stream::unfold((full_rx, aborter), |(mut rx, aborter)| async move {
        rx.recv().await.map(|item| (item, (rx, aborter)))
    })
    .boxed();

    StreamTextResult {
        state,
        text_stream,
        full_stream,
    }
}

#[derive(Debug)]
enum CollectedPart {
    Text(String),
    Reasoning(String),
    ToolCall { id: String },
}

#[derive(Debug, Default)]
struct ToolCallBuffer {
    name: Option<String>,
    arguments: String,
}

#[cfg(test)]
mod tests {
    use super::*;
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
                output_tokens: Some(7),
                total_tokens: None,
            })),
            Ok(StreamChunk::FinishReason(FinishReason::ToolCalls)),
        ];

        let inner: StreamResult = stream::iter(chunks).boxed();
        let mut result = stream_text_from_stream(inner);

        let mut deltas = String::new();
        while let Some(delta) = result.text_stream.next().await {
            deltas.push_str(&delta?);
        }
        assert_eq!(deltas, "hello world".to_string());

        let mut seen_finish = false;
        while let Some(evt) = result.full_stream.next().await {
            if matches!(evt?, StreamChunk::FinishReason(_)) {
                seen_finish = true;
            }
        }
        assert!(seen_finish);

        let response = result.final_response()?.unwrap();
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
