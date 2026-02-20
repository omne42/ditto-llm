use std::collections::HashMap;
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
    truncated: bool,
    emitted_part: bool,
}

#[derive(Debug, Clone, Copy)]
struct StreamCollectorLimits {
    max_total_bytes: usize,
    max_tool_arguments_bytes: usize,
    max_parts: usize,
    max_tool_calls: usize,
}

impl Default for StreamCollectorLimits {
    fn default() -> Self {
        Self {
            max_total_bytes: 64 * 1024 * 1024,
            max_tool_arguments_bytes: 4 * 1024 * 1024,
            max_parts: 4096,
            max_tool_calls: 256,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct StreamCollector {
    warnings: Vec<Warning>,
    response_id: Option<String>,
    finish_reason: FinishReason,
    usage: Usage,
    parts: Vec<CollectedPart>,
    tool_calls: HashMap<String, ToolCallBuffer>,
    limits: StreamCollectorLimits,
    total_bytes: usize,
    bytes_truncated: bool,
    warned_max_total_bytes: bool,
    warned_max_parts: bool,
    warned_max_tool_calls: bool,
    warned_empty_tool_call_id_start: bool,
    warned_empty_tool_call_id_delta: bool,
}

impl StreamCollector {
    fn warn_empty_tool_call_id_start(&mut self) {
        if self.warned_empty_tool_call_id_start {
            return;
        }
        self.warned_empty_tool_call_id_start = true;
        self.warnings.push(Warning::Compatibility {
            feature: "tool_call.id".to_string(),
            details: "stream emitted an empty tool_call id; dropping tool call".to_string(),
        });
    }

    fn warn_empty_tool_call_id_delta(&mut self) {
        if self.warned_empty_tool_call_id_delta {
            return;
        }
        self.warned_empty_tool_call_id_delta = true;
        self.warnings.push(Warning::Compatibility {
            feature: "tool_call.id".to_string(),
            details: "stream emitted an empty tool_call id for arguments; dropping tool call delta"
                .to_string(),
        });
    }

    fn try_add_bytes(&mut self, bytes: usize) -> bool {
        if self.bytes_truncated {
            return false;
        }
        if self.total_bytes.saturating_add(bytes) > self.limits.max_total_bytes {
            self.bytes_truncated = true;
            if !self.warned_max_total_bytes {
                self.warned_max_total_bytes = true;
                self.warnings.push(Warning::Compatibility {
                    feature: "stream.collector.max_total_bytes".to_string(),
                    details: format!(
                        "stream collector reached max_total_bytes={}; final response will be truncated",
                        self.limits.max_total_bytes
                    ),
                });
            }
            return false;
        }
        self.total_bytes = self.total_bytes.saturating_add(bytes);
        true
    }

    fn can_push_part(&mut self) -> bool {
        if self.parts.len() >= self.limits.max_parts {
            if !self.warned_max_parts {
                self.warned_max_parts = true;
                self.warnings.push(Warning::Compatibility {
                    feature: "stream.collector.max_parts".to_string(),
                    details: format!(
                        "stream collector reached max_parts={}; final response will be truncated",
                        self.limits.max_parts
                    ),
                });
            }
            return false;
        }
        true
    }

    fn warn_max_tool_calls(&mut self) {
        if self.warned_max_tool_calls {
            return;
        }
        self.warned_max_tool_calls = true;
        self.warnings.push(Warning::Compatibility {
            feature: "stream.collector.max_tool_calls".to_string(),
            details: format!(
                "stream collector reached max_tool_calls={}; additional tool calls will be dropped",
                self.limits.max_tool_calls
            ),
        });
    }

    pub(crate) fn observe(&mut self, chunk: &StreamChunk) {
        match chunk {
            StreamChunk::Warnings { warnings } => self.warnings.extend(warnings.clone()),
            StreamChunk::ResponseId { id } => {
                if self.response_id.is_none() && !id.trim().is_empty() {
                    self.response_id = Some(id.clone());
                }
            }
            StreamChunk::TextDelta { text } => {
                if text.is_empty() {
                    return;
                }
                let can_merge = matches!(self.parts.last(), Some(CollectedPart::Text(_)));
                if can_merge {
                    if self.try_add_bytes(text.len()) {
                        if let Some(CollectedPart::Text(existing)) = self.parts.last_mut() {
                            existing.push_str(text);
                        }
                    }
                    return;
                }

                if !self.can_push_part() {
                    return;
                }
                if self.try_add_bytes(text.len()) {
                    self.parts.push(CollectedPart::Text(text.clone()));
                }
            }
            StreamChunk::ReasoningDelta { text } => {
                if text.is_empty() {
                    return;
                }
                let can_merge = matches!(self.parts.last(), Some(CollectedPart::Reasoning(_)));
                if can_merge {
                    if self.try_add_bytes(text.len()) {
                        if let Some(CollectedPart::Reasoning(existing)) = self.parts.last_mut() {
                            existing.push_str(text);
                        }
                    }
                    return;
                }

                if !self.can_push_part() {
                    return;
                }
                if self.try_add_bytes(text.len()) {
                    self.parts.push(CollectedPart::Reasoning(text.clone()));
                }
            }
            StreamChunk::ToolCallStart { id, name } => {
                if id.trim().is_empty() {
                    self.warn_empty_tool_call_id_start();
                    return;
                }

                use std::collections::hash_map::Entry;
                let at_capacity = self.tool_calls.len() >= self.limits.max_tool_calls;
                let should_emit_part = {
                    let slot = match self.tool_calls.entry(id.clone()) {
                        Entry::Occupied(entry) => entry.into_mut(),
                        Entry::Vacant(entry) => {
                            if at_capacity {
                                self.warn_max_tool_calls();
                                return;
                            }
                            entry.insert(ToolCallBuffer::default())
                        }
                    };
                    if slot.name.is_none() && !name.trim().is_empty() {
                        slot.name = Some(name.clone());
                    }
                    !slot.emitted_part
                };
                if should_emit_part && self.can_push_part() {
                    if let Some(slot) = self.tool_calls.get_mut(id) {
                        slot.emitted_part = true;
                    }
                    self.parts.push(CollectedPart::ToolCall { id: id.clone() });
                }
            }
            StreamChunk::ToolCallDelta {
                id,
                arguments_delta,
            } => {
                if id.trim().is_empty() {
                    self.warn_empty_tool_call_id_delta();
                    return;
                }

                use std::collections::hash_map::Entry;
                let at_capacity = self.tool_calls.len() >= self.limits.max_tool_calls;
                let mut should_push_part = false;
                {
                    let slot = match self.tool_calls.entry(id.clone()) {
                        Entry::Occupied(entry) => entry.into_mut(),
                        Entry::Vacant(entry) => {
                            if at_capacity {
                                self.warn_max_tool_calls();
                                return;
                            }
                            entry.insert(ToolCallBuffer::default())
                        }
                    };

                    if slot.truncated {
                        return;
                    }
                    if slot.arguments.len().saturating_add(arguments_delta.len())
                        > self.limits.max_tool_arguments_bytes
                    {
                        slot.truncated = true;
                        self.warnings.push(Warning::Compatibility {
                            feature: "stream.collector.max_tool_arguments_bytes".to_string(),
                            details: format!(
                                "tool call arguments exceeded max_tool_arguments_bytes={} for id={id}; arguments will be truncated",
                                self.limits.max_tool_arguments_bytes
                            ),
                        });
                        return;
                    }
                    if !slot.emitted_part {
                        should_push_part = true;
                    }
                }

                if self.try_add_bytes(arguments_delta.len()) {
                    if let Some(slot) = self.tool_calls.get_mut(id) {
                        slot.arguments.push_str(arguments_delta);
                    }
                }
                if should_push_part && self.can_push_part() {
                    if let Some(slot) = self.tool_calls.get_mut(id) {
                        slot.emitted_part = true;
                    }
                    self.parts.push(CollectedPart::ToolCall { id: id.clone() });
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
                let can_merge = matches!(self.parts.last(), Some(CollectedPart::Text(_)));
                if can_merge {
                    if self.try_add_bytes(text.len()) {
                        if let Some(CollectedPart::Text(existing)) = self.parts.last_mut() {
                            existing.push_str(&text);
                        }
                    }
                    return;
                }

                if !self.can_push_part() {
                    return;
                }
                if self.try_add_bytes(text.len()) {
                    self.parts.push(CollectedPart::Text(text));
                }
            }
            StreamChunk::ReasoningDelta { text } => {
                if text.is_empty() {
                    return;
                }
                let can_merge = matches!(self.parts.last(), Some(CollectedPart::Reasoning(_)));
                if can_merge {
                    if self.try_add_bytes(text.len()) {
                        if let Some(CollectedPart::Reasoning(existing)) = self.parts.last_mut() {
                            existing.push_str(&text);
                        }
                    }
                    return;
                }

                if !self.can_push_part() {
                    return;
                }
                if self.try_add_bytes(text.len()) {
                    self.parts.push(CollectedPart::Reasoning(text));
                }
            }
            StreamChunk::ToolCallStart { id, name } => {
                if id.trim().is_empty() {
                    self.warn_empty_tool_call_id_start();
                    return;
                }

                use std::collections::hash_map::Entry;
                let at_capacity = self.tool_calls.len() >= self.limits.max_tool_calls;
                let should_emit_part = {
                    let slot = match self.tool_calls.entry(id.clone()) {
                        Entry::Occupied(entry) => entry.into_mut(),
                        Entry::Vacant(entry) => {
                            if at_capacity {
                                self.warn_max_tool_calls();
                                return;
                            }
                            entry.insert(ToolCallBuffer::default())
                        }
                    };
                    if slot.name.is_none() && !name.trim().is_empty() {
                        slot.name = Some(name);
                    }
                    !slot.emitted_part
                };
                if should_emit_part && self.can_push_part() {
                    if let Some(slot) = self.tool_calls.get_mut(&id) {
                        slot.emitted_part = true;
                    }
                    self.parts.push(CollectedPart::ToolCall { id });
                }
            }
            StreamChunk::ToolCallDelta {
                id,
                arguments_delta,
            } => {
                if id.trim().is_empty() {
                    self.warn_empty_tool_call_id_delta();
                    return;
                }

                use std::collections::hash_map::Entry;
                let at_capacity = self.tool_calls.len() >= self.limits.max_tool_calls;
                let mut should_push_part = false;
                {
                    let slot = match self.tool_calls.entry(id.clone()) {
                        Entry::Occupied(entry) => entry.into_mut(),
                        Entry::Vacant(entry) => {
                            if at_capacity {
                                self.warn_max_tool_calls();
                                return;
                            }
                            entry.insert(ToolCallBuffer::default())
                        }
                    };

                    if slot.truncated {
                        return;
                    }
                    if slot.arguments.len().saturating_add(arguments_delta.len())
                        > self.limits.max_tool_arguments_bytes
                    {
                        slot.truncated = true;
                        self.warnings.push(Warning::Compatibility {
                            feature: "stream.collector.max_tool_arguments_bytes".to_string(),
                            details: format!(
                                "tool call arguments exceeded max_tool_arguments_bytes={} for id={id}; arguments will be truncated",
                                self.limits.max_tool_arguments_bytes
                            ),
                        });
                        return;
                    }
                    if !slot.emitted_part {
                        should_push_part = true;
                    }
                }

                if self.try_add_bytes(arguments_delta.len()) {
                    if let Some(slot) = self.tool_calls.get_mut(&id) {
                        slot.arguments.push_str(&arguments_delta);
                    }
                }
                if should_push_part && self.can_push_part() {
                    if let Some(slot) = self.tool_calls.get_mut(&id) {
                        slot.emitted_part = true;
                    }
                    self.parts.push(CollectedPart::ToolCall { id });
                }
            }
            StreamChunk::FinishReason(reason) => self.finish_reason = reason,
            StreamChunk::Usage(usage) => self.usage = usage,
        }
    }

    pub(crate) fn finish(mut self) -> GenerateResponse {
        self.usage.merge_total();

        let mut content = Vec::<ContentPart>::with_capacity(self.parts.len());

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

    #[test]
    fn stream_collector_truncates_when_over_max_bytes() {
        let mut collector = StreamCollector::default();
        collector.limits.max_total_bytes = 8;

        collector.observe_owned(StreamChunk::TextDelta {
            text: "12345678".to_string(),
        });
        collector.observe_owned(StreamChunk::TextDelta {
            text: "9".to_string(),
        });

        let response = collector.finish();
        assert_eq!(response.text(), "12345678".to_string());
        assert!(response.warnings.iter().any(|warning| matches!(
            warning,
            Warning::Compatibility { feature, .. } if feature == "stream.collector.max_total_bytes"
        )));
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

    #[test]
    fn empty_tool_call_id_warnings_are_deduplicated_in_observe_owned() {
        let mut collector = StreamCollector::default();

        for _ in 0..3 {
            collector.observe_owned(StreamChunk::ToolCallStart {
                id: "   ".to_string(),
                name: "ignored".to_string(),
            });
            collector.observe_owned(StreamChunk::ToolCallDelta {
                id: "".to_string(),
                arguments_delta: "{}".to_string(),
            });
        }

        let response = collector.finish();
        let id_warnings = response
            .warnings
            .iter()
            .filter(|warning| matches!(warning, Warning::Compatibility { feature, .. } if feature == "tool_call.id"))
            .count();
        assert_eq!(id_warnings, 2);
    }

    #[test]
    fn empty_tool_call_id_warnings_are_deduplicated_in_observe() {
        let mut collector = StreamCollector::default();

        let start = StreamChunk::ToolCallStart {
            id: "  ".to_string(),
            name: "ignored".to_string(),
        };
        let delta = StreamChunk::ToolCallDelta {
            id: "".to_string(),
            arguments_delta: "{}".to_string(),
        };

        for _ in 0..3 {
            collector.observe(&start);
            collector.observe(&delta);
        }

        let response = collector.finish();
        let id_warnings = response
            .warnings
            .iter()
            .filter(|warning| matches!(warning, Warning::Compatibility { feature, .. } if feature == "tool_call.id"))
            .count();
        assert_eq!(id_warnings, 2);
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
