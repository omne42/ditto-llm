use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};

use futures_util::Stream;
use futures_util::StreamExt;
use futures_util::stream;
use futures_util::task::AtomicWaker;

use crate::model::LanguageModel;
use crate::types::GenerateRequest;
use crate::types::{ContentPart, FinishReason, GenerateResponse, StreamChunk, Usage, Warning};
use crate::{Result, StreamResult};

#[derive(Debug, Clone, Default)]
pub struct CollectedStream {
    pub response_id: Option<String>,
    pub response: GenerateResponse,
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

#[derive(Debug, Default)]
pub(crate) struct StreamCollector {
    warnings: Vec<Warning>,
    response_id: Option<String>,
    finish_reason: FinishReason,
    usage: Usage,
    parts: Vec<CollectedPart>,
    tool_calls: HashMap<String, ToolCallBuffer>,
    seen_tool_call_ids: HashSet<String>,
}

impl StreamCollector {
    pub(crate) fn observe(&mut self, chunk: &StreamChunk) {
        match chunk {
            StreamChunk::Warnings { warnings } => self.warnings.extend(warnings.clone()),
            StreamChunk::ResponseId { id } => {
                if self.response_id.is_none() && !id.trim().is_empty() {
                    self.response_id = Some(id.to_string());
                }
            }
            StreamChunk::TextDelta { text } => {
                if text.is_empty() {
                    return;
                }
                match self.parts.last_mut() {
                    Some(CollectedPart::Text(existing)) => existing.push_str(text),
                    _ => self.parts.push(CollectedPart::Text(text.to_string())),
                }
            }
            StreamChunk::ReasoningDelta { text } => {
                if text.is_empty() {
                    return;
                }
                match self.parts.last_mut() {
                    Some(CollectedPart::Reasoning(existing)) => existing.push_str(text),
                    _ => self.parts.push(CollectedPart::Reasoning(text.to_string())),
                }
            }
            StreamChunk::ToolCallStart { id, name } => {
                if id.trim().is_empty() {
                    self.warnings.push(Warning::Compatibility {
                        feature: "tool_call.id".to_string(),
                        details: "stream emitted an empty tool_call id; dropping tool call"
                            .to_string(),
                    });
                    return;
                }

                let slot = self.tool_calls.entry(id.to_string()).or_default();
                if slot.name.is_none() && !name.trim().is_empty() {
                    slot.name = Some(name.to_string());
                }
                if self.seen_tool_call_ids.insert(id.to_string()) {
                    self.parts
                        .push(CollectedPart::ToolCall { id: id.to_string() });
                }
            }
            StreamChunk::ToolCallDelta {
                id,
                arguments_delta,
            } => {
                if id.trim().is_empty() {
                    self.warnings.push(Warning::Compatibility {
                        feature: "tool_call.id".to_string(),
                        details: "stream emitted an empty tool_call id for arguments; dropping tool call delta".to_string(),
                    });
                    return;
                }

                let slot = self.tool_calls.entry(id.to_string()).or_default();
                slot.arguments.push_str(arguments_delta);
                if self.seen_tool_call_ids.insert(id.to_string()) {
                    self.parts
                        .push(CollectedPart::ToolCall { id: id.to_string() });
                }
            }
            StreamChunk::FinishReason(reason) => self.finish_reason = *reason,
            StreamChunk::Usage(usage) => self.usage = usage.clone(),
        }
    }

    pub(crate) fn observe_owned(&mut self, chunk: StreamChunk) {
        match chunk {
            StreamChunk::Warnings { warnings } => self.warnings.extend(warnings),
            StreamChunk::ResponseId { id } => {
                if self.response_id.is_none() && !id.trim().is_empty() {
                    self.response_id = Some(id);
                }
            }
            StreamChunk::TextDelta { text } => {
                if text.is_empty() {
                    return;
                }
                match self.parts.last_mut() {
                    Some(CollectedPart::Text(existing)) => existing.push_str(&text),
                    _ => self.parts.push(CollectedPart::Text(text)),
                }
            }
            StreamChunk::ReasoningDelta { text } => {
                if text.is_empty() {
                    return;
                }
                match self.parts.last_mut() {
                    Some(CollectedPart::Reasoning(existing)) => existing.push_str(&text),
                    _ => self.parts.push(CollectedPart::Reasoning(text)),
                }
            }
            StreamChunk::ToolCallStart { id, name } => {
                if id.trim().is_empty() {
                    self.warnings.push(Warning::Compatibility {
                        feature: "tool_call.id".to_string(),
                        details: "stream emitted an empty tool_call id; dropping tool call"
                            .to_string(),
                    });
                    return;
                }

                let slot = self.tool_calls.entry(id.clone()).or_default();
                if slot.name.is_none() && !name.trim().is_empty() {
                    slot.name = Some(name);
                }
                if self.seen_tool_call_ids.insert(id.clone()) {
                    self.parts.push(CollectedPart::ToolCall { id });
                }
            }
            StreamChunk::ToolCallDelta {
                id,
                arguments_delta,
            } => {
                if id.trim().is_empty() {
                    self.warnings.push(Warning::Compatibility {
                        feature: "tool_call.id".to_string(),
                        details: "stream emitted an empty tool_call id for arguments; dropping tool call delta"
                            .to_string(),
                    });
                    return;
                }

                let slot = self.tool_calls.entry(id.clone()).or_default();
                slot.arguments.push_str(&arguments_delta);
                if self.seen_tool_call_ids.insert(id.clone()) {
                    self.parts.push(CollectedPart::ToolCall { id });
                }
            }
            StreamChunk::FinishReason(reason) => self.finish_reason = reason,
            StreamChunk::Usage(usage) => self.usage = usage,
        }
    }

    pub(crate) fn finish(mut self) -> GenerateResponse {
        self.usage.merge_total();

        let mut content = Vec::<ContentPart>::new();

        for part in self.parts {
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
                    let Some(tool_call) = self.tool_calls.get(&tool_call_id) else {
                        continue;
                    };

                    let Some(name) = tool_call
                        .name
                        .as_deref()
                        .map(str::trim)
                        .filter(|name| !name.is_empty())
                    else {
                        self.warnings.push(Warning::Compatibility {
                            feature: "tool_call.name".to_string(),
                            details: format!(
                                "stream ended before tool_call name was received for id={tool_call_id}; dropping tool call"
                            ),
                        });
                        continue;
                    };

                    let context = format!("id={tool_call_id}");
                    let arguments = crate::types::parse_tool_call_arguments_json_or_string(
                        tool_call.arguments.as_str(),
                        &context,
                        &mut self.warnings,
                    );

                    content.push(ContentPart::ToolCall {
                        id: tool_call_id,
                        name: name.to_string(),
                        arguments,
                    });
                }
            }
        }

        let provider_metadata = self
            .response_id
            .as_deref()
            .map(|id| serde_json::json!({ "id": id }));

        GenerateResponse {
            content,
            finish_reason: self.finish_reason,
            usage: self.usage,
            warnings: self.warnings,
            provider_metadata,
        }
    }

    pub(crate) fn finish_collected(self) -> CollectedStream {
        let response_id = self.response_id.clone();
        CollectedStream {
            response_id,
            response: self.finish(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct StreamAbortHandle {
    aborted: Arc<AtomicBool>,
    waker: Arc<AtomicWaker>,
}

impl StreamAbortHandle {
    pub fn abort(&self) {
        self.aborted.store(true, Ordering::SeqCst);
        self.waker.wake();
    }
}

pub struct AbortableStream {
    pub handle: StreamAbortHandle,
    pub stream: StreamResult,
}

pub fn abortable_stream(stream: StreamResult) -> AbortableStream {
    let aborted = Arc::new(AtomicBool::new(false));
    let waker = Arc::new(AtomicWaker::new());
    let handle = StreamAbortHandle {
        aborted: aborted.clone(),
        waker: waker.clone(),
    };

    let mut inner = Some(stream);
    let stream = stream::poll_fn(move |cx: &mut Context<'_>| {
        waker.register(cx.waker());

        if aborted.load(Ordering::SeqCst) {
            inner.take();
            return Poll::Ready(None);
        }

        let Some(stream) = inner.as_mut() else {
            return Poll::Ready(None);
        };
        Pin::new(stream).poll_next(cx)
    })
    .boxed();

    AbortableStream { handle, stream }
}

#[async_trait::async_trait]
pub trait LanguageModelExt: LanguageModel {
    async fn stream_abortable(&self, request: GenerateRequest) -> Result<AbortableStream> {
        let stream = self.stream(request).await?;
        Ok(abortable_stream(stream))
    }
}

impl<T> LanguageModelExt for T where T: LanguageModel + ?Sized {}

pub async fn collect_stream(mut stream: StreamResult) -> Result<CollectedStream> {
    let mut collector = StreamCollector::default();
    while let Some(chunk) = stream.next().await {
        collector.observe_owned(chunk?);
    }
    Ok(collector.finish_collected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::FutureExt;
    use futures_util::stream;
    use serde_json::Value;

    #[tokio::test]
    async fn collects_text_usage_finish_reason_and_id() -> Result<()> {
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
            Ok(StreamChunk::Usage(Usage {
                input_tokens: Some(3),
                cache_input_tokens: None,
                cache_creation_input_tokens: None,
                output_tokens: Some(7),
                total_tokens: None,
            })),
            Ok(StreamChunk::FinishReason(FinishReason::Stop)),
        ];

        let stream = stream::iter(chunks).boxed();
        let collected = collect_stream(stream).await?;

        assert_eq!(collected.response_id.as_deref(), Some("resp_123"));
        assert_eq!(collected.response.finish_reason, FinishReason::Stop);
        assert_eq!(collected.response.usage.total_tokens, Some(10));
        assert_eq!(collected.response.text(), "hello world".to_string());
        assert_eq!(
            collected.response.provider_metadata,
            Some(serde_json::json!({ "id": "resp_123" }))
        );

        Ok(())
    }

    #[tokio::test]
    async fn collects_tool_calls_and_reasoning() -> Result<()> {
        let chunks = vec![
            Ok(StreamChunk::ReasoningDelta {
                text: "think".to_string(),
            }),
            Ok(StreamChunk::ReasoningDelta {
                text: " more".to_string(),
            }),
            Ok(StreamChunk::ToolCallStart {
                id: "call_1".to_string(),
                name: "get_weather".to_string(),
            }),
            Ok(StreamChunk::ToolCallDelta {
                id: "call_1".to_string(),
                arguments_delta: "{\"a\":1".to_string(),
            }),
            Ok(StreamChunk::ToolCallDelta {
                id: "call_1".to_string(),
                arguments_delta: ",\"b\":2}".to_string(),
            }),
            Ok(StreamChunk::FinishReason(FinishReason::ToolCalls)),
        ];

        let stream = stream::iter(chunks).boxed();
        let collected = collect_stream(stream).await?;

        assert_eq!(collected.response.finish_reason, FinishReason::ToolCalls);

        assert!(matches!(
            collected.response.content.first(),
            Some(ContentPart::Reasoning { .. })
        ));

        assert!(collected
            .response
            .content
            .iter()
            .any(|part| matches!(part, ContentPart::ToolCall { id, name, arguments } if id == "call_1" && name == "get_weather" && arguments == &serde_json::json!({"a":1,"b":2}))));

        Ok(())
    }

    #[tokio::test]
    async fn preserves_interleaved_text_and_tool_call_order() -> Result<()> {
        let chunks = vec![
            Ok(StreamChunk::TextDelta {
                text: "before".to_string(),
            }),
            Ok(StreamChunk::ToolCallStart {
                id: "call_1".to_string(),
                name: "add".to_string(),
            }),
            Ok(StreamChunk::ToolCallDelta {
                id: "call_1".to_string(),
                arguments_delta: "{\"a\":1}".to_string(),
            }),
            Ok(StreamChunk::TextDelta {
                text: " after".to_string(),
            }),
        ];

        let stream = stream::iter(chunks).boxed();
        let collected = collect_stream(stream).await?;

        assert_eq!(collected.response.content.len(), 3);
        assert!(
            matches!(&collected.response.content[0], ContentPart::Text { text } if text == "before")
        );
        assert!(
            matches!(&collected.response.content[1], ContentPart::ToolCall { id, name, arguments } if id == "call_1" && name == "add" && arguments == &serde_json::json!({ "a": 1 }))
        );
        assert!(
            matches!(&collected.response.content[2], ContentPart::Text { text } if text == " after")
        );

        Ok(())
    }

    #[tokio::test]
    async fn preserves_invalid_tool_call_arguments_with_warning() -> Result<()> {
        let chunks = vec![
            Ok(StreamChunk::ToolCallStart {
                id: "call_1".to_string(),
                name: "add".to_string(),
            }),
            Ok(StreamChunk::ToolCallDelta {
                id: "call_1".to_string(),
                arguments_delta: "{\"a\":1".to_string(),
            }),
        ];

        let stream = stream::iter(chunks).boxed();
        let collected = collect_stream(stream).await?;

        assert!(collected.response.warnings.iter().any(|w| matches!(
            w,
            Warning::Compatibility { feature, .. } if feature == "tool_call.arguments"
        )));
        assert!(collected.response.content.iter().any(|part| matches!(
            part,
            ContentPart::ToolCall { id, arguments, .. } if id == "call_1" && arguments == &Value::String("{\"a\":1".to_string())
        )));
        Ok(())
    }

    #[tokio::test]
    async fn drops_tool_call_without_name_with_warning() -> Result<()> {
        let chunks = vec![
            Ok(StreamChunk::ToolCallStart {
                id: "call_1".to_string(),
                name: "".to_string(),
            }),
            Ok(StreamChunk::ToolCallDelta {
                id: "call_1".to_string(),
                arguments_delta: "{\"a\":1}".to_string(),
            }),
        ];

        let stream = stream::iter(chunks).boxed();
        let collected = collect_stream(stream).await?;

        assert!(collected.response.content.is_empty());
        assert!(collected.response.warnings.iter().any(|w| matches!(
            w,
            Warning::Compatibility { feature, .. } if feature == "tool_call.name"
        )));

        Ok(())
    }

    #[tokio::test]
    async fn abortable_stream_stops_stream() -> Result<()> {
        let pending = stream::pending::<Result<StreamChunk>>().boxed();
        let AbortableStream { handle, mut stream } = abortable_stream(pending);

        handle.abort();
        assert!(stream.next().await.is_none());

        Ok(())
    }

    #[test]
    fn abort_handle_drop_does_not_stop_stream() {
        let pending = stream::pending::<Result<StreamChunk>>().boxed();
        let AbortableStream { handle, mut stream } = abortable_stream(pending);
        drop(handle);

        assert!(stream.next().now_or_never().is_none());
    }
}
