use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::types::{FinishReason, StreamChunk};

type UiStreamItem = std::result::Result<bytes::Bytes, std::io::Error>;

static UI_MESSAGE_STREAM_SEQ: AtomicU64 = AtomicU64::new(0);

fn now_ts_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

fn generate_id(prefix: &str) -> String {
    let ts_ms = now_ts_ms();
    let seq = UI_MESSAGE_STREAM_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{ts_ms}_{seq}")
}

#[derive(Clone, Debug)]
pub struct UiMessageStreamV1Options {
    pub message_id: Option<String>,
    pub text_id: Option<String>,
    pub reasoning_id: Option<String>,
    pub max_tool_input_bytes: usize,
    pub include_usage: bool,
    pub include_warnings: bool,
}

impl Default for UiMessageStreamV1Options {
    fn default() -> Self {
        Self {
            message_id: None,
            text_id: None,
            reasoning_id: None,
            max_tool_input_bytes: 256 * 1024,
            include_usage: true,
            include_warnings: true,
        }
    }
}

#[derive(Debug, Default)]
struct ToolState {
    name: String,
    input_json: String,
    truncated: bool,
    started: bool,
}

#[derive(Debug)]
struct UiStreamState {
    started: bool,
    done: bool,
    message_id: String,
    text_id: String,
    reasoning_id: String,
    step_started: bool,
    text_started: bool,
    reasoning_started: bool,
    finish_reason: FinishReason,
    tool_calls: HashMap<String, ToolState>,
    max_tool_input_bytes: usize,
    include_usage: bool,
    include_warnings: bool,
    buffer: std::collections::VecDeque<UiStreamItem>,
}

fn to_io_error_other(msg: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(msg.to_string())
}

fn encode_sse_json(value: serde_json::Value) -> UiStreamItem {
    let json = serde_json::to_string(&value).map_err(to_io_error_other)?;
    Ok(bytes::Bytes::from(format!("data: {json}\n\n")))
}

fn encode_sse_done() -> bytes::Bytes {
    bytes::Bytes::from_static(b"data: [DONE]\n\n")
}

fn finish_reason_to_ui(reason: FinishReason) -> &'static str {
    match reason {
        FinishReason::Stop => "stop",
        FinishReason::Length => "length",
        FinishReason::ToolCalls => "tool-calls",
        FinishReason::ContentFilter => "content-filter",
        FinishReason::Error => "error",
        FinishReason::Unknown => "other",
    }
}

impl UiStreamState {
    fn new(options: UiMessageStreamV1Options) -> Self {
        let message_id = options
            .message_id
            .unwrap_or_else(|| generate_id("ditto_msg"));
        let text_id = options.text_id.unwrap_or_else(|| generate_id("ditto_text"));
        let reasoning_id = options
            .reasoning_id
            .unwrap_or_else(|| generate_id("ditto_reasoning"));

        Self {
            started: false,
            done: false,
            message_id,
            text_id,
            reasoning_id,
            step_started: false,
            text_started: false,
            reasoning_started: false,
            finish_reason: FinishReason::Unknown,
            tool_calls: HashMap::new(),
            max_tool_input_bytes: options.max_tool_input_bytes,
            include_usage: options.include_usage,
            include_warnings: options.include_warnings,
            buffer: std::collections::VecDeque::new(),
        }
    }

    fn push_json(&mut self, value: serde_json::Value) {
        self.buffer.push_back(encode_sse_json(value));
    }

    fn push_error(&mut self, error_text: &str) {
        self.push_json(serde_json::json!({
            "type": "error",
            "errorText": error_text,
        }));
    }

    fn ensure_started(&mut self) {
        if self.started {
            return;
        }
        self.started = true;
        self.push_json(serde_json::json!({
            "type": "start",
            "messageId": self.message_id,
        }));
        self.ensure_step_started();
    }

    fn ensure_step_started(&mut self) {
        if self.step_started {
            return;
        }
        self.step_started = true;
        self.push_json(serde_json::json!({
            "type": "start-step",
        }));
    }

    fn ensure_text_started(&mut self) {
        if self.text_started {
            return;
        }
        self.text_started = true;
        self.push_json(serde_json::json!({
            "type": "text-start",
            "id": self.text_id,
        }));
    }

    fn ensure_reasoning_started(&mut self) {
        if self.reasoning_started {
            return;
        }
        self.reasoning_started = true;
        self.push_json(serde_json::json!({
            "type": "reasoning-start",
            "id": self.reasoning_id,
        }));
    }

    fn ensure_tool_started(&mut self, id: &str) {
        let tool_name = {
            let entry = self.tool_calls.entry(id.to_string()).or_default();
            if entry.started {
                return;
            }
            entry.started = true;

            if entry.name.trim().is_empty() {
                "unknown".to_string()
            } else {
                entry.name.clone()
            }
        };

        self.push_json(serde_json::json!({
            "type": "tool-input-start",
            "toolCallId": id,
            "toolName": tool_name,
        }));
    }

    fn append_tool_input(&mut self, id: &str, delta: &str) {
        let entry = self.tool_calls.entry(id.to_string()).or_default();
        if entry.truncated {
            return;
        }
        let next_len = entry.input_json.len().saturating_add(delta.len());
        if next_len > self.max_tool_input_bytes {
            entry.truncated = true;
            return;
        }
        entry.input_json.push_str(delta);
    }

    fn handle_chunk(&mut self, chunk: StreamChunk) {
        self.ensure_started();

        match chunk {
            StreamChunk::Warnings { warnings } => {
                if !self.include_warnings {
                    return;
                }
                self.push_json(serde_json::json!({
                    "type": "data-ditto-warnings",
                    "data": { "warnings": warnings },
                }));
            }
            StreamChunk::ResponseId { id } => {
                self.push_json(serde_json::json!({
                    "type": "message-metadata",
                    "messageMetadata": { "responseId": id },
                }));
            }
            StreamChunk::TextDelta { text } => {
                self.ensure_text_started();
                self.push_json(serde_json::json!({
                    "type": "text-delta",
                    "id": self.text_id,
                    "delta": text,
                }));
            }
            StreamChunk::ToolCallStart { id, name } => {
                let entry = self.tool_calls.entry(id.clone()).or_default();
                entry.name = name.clone();
                if !entry.started {
                    entry.started = true;
                    self.push_json(serde_json::json!({
                        "type": "tool-input-start",
                        "toolCallId": id,
                        "toolName": name,
                    }));
                }
            }
            StreamChunk::ToolCallDelta {
                id,
                arguments_delta,
            } => {
                self.ensure_tool_started(&id);
                self.append_tool_input(&id, &arguments_delta);
                self.push_json(serde_json::json!({
                    "type": "tool-input-delta",
                    "toolCallId": id,
                    "inputTextDelta": arguments_delta,
                }));
            }
            StreamChunk::ReasoningDelta { text } => {
                self.ensure_reasoning_started();
                self.push_json(serde_json::json!({
                    "type": "reasoning-delta",
                    "id": self.reasoning_id,
                    "delta": text,
                }));
            }
            StreamChunk::FinishReason(reason) => {
                self.finish_reason = reason;
            }
            StreamChunk::Usage(usage) => {
                if !self.include_usage {
                    return;
                }
                self.push_json(serde_json::json!({
                    "type": "data-ditto-usage",
                    "data": usage,
                }));
            }
        }
    }

    fn flush_tool_inputs(&mut self) {
        if self.tool_calls.is_empty() {
            return;
        }

        let mut ids = self.tool_calls.keys().cloned().collect::<Vec<_>>();
        ids.sort();

        for id in ids {
            let Some(tool) = self.tool_calls.get(&id) else {
                continue;
            };
            let tool_name = if tool.name.trim().is_empty() {
                "unknown"
            } else {
                tool.name.as_str()
            };

            if tool.truncated {
                self.push_json(serde_json::json!({
                    "type": "tool-input-error",
                    "toolCallId": id,
                    "toolName": tool_name,
                    "input": tool.input_json,
                    "errorText": format!("tool input exceeded {} bytes", self.max_tool_input_bytes),
                }));
                continue;
            }

            let parsed: Result<serde_json::Value, serde_json::Error> =
                serde_json::from_str(&tool.input_json);
            match parsed {
                Ok(value) => {
                    self.push_json(serde_json::json!({
                        "type": "tool-input-available",
                        "toolCallId": id,
                        "toolName": tool_name,
                        "input": value,
                    }));
                }
                Err(err) => {
                    self.push_json(serde_json::json!({
                        "type": "tool-input-error",
                        "toolCallId": id,
                        "toolName": tool_name,
                        "input": tool.input_json,
                        "errorText": err.to_string(),
                    }));
                }
            }
        }
    }

    fn finalize(&mut self, override_finish_reason: Option<FinishReason>) {
        if self.done {
            return;
        }

        self.ensure_started();
        self.flush_tool_inputs();

        if self.text_started {
            self.push_json(serde_json::json!({
                "type": "text-end",
                "id": self.text_id,
            }));
        }
        if self.reasoning_started {
            self.push_json(serde_json::json!({
                "type": "reasoning-end",
                "id": self.reasoning_id,
            }));
        }

        if self.step_started {
            self.push_json(serde_json::json!({
                "type": "finish-step",
            }));
        }

        let finish_reason = override_finish_reason.unwrap_or(self.finish_reason);
        self.push_json(serde_json::json!({
            "type": "finish",
            "finishReason": finish_reason_to_ui(finish_reason),
        }));
        self.buffer.push_back(Ok(encode_sse_done()));
        self.done = true;
    }
}

/// Convert a Ditto `StreamResult` into a Vercel AI SDK UI Message Stream (SSE).
///
/// Notes:
/// - Output format matches the AI SDK UI message stream protocol v1 (SSE frames + terminal `[DONE]`).
/// - Consumers should set `x-vercel-ai-ui-message-stream: v1` on the HTTP response.
/// - Tool call argument deltas are forwarded as `tool-input-delta`, and a best-effort
///   `tool-input-available` (or `tool-input-error`) is emitted at the end of the stream.
pub fn ui_message_stream_v1_sse(stream: StreamResult) -> BoxStream<'static, IoResult<Bytes>> {
    ui_message_stream_v1_sse_with_options(stream, UiMessageStreamV1Options::default())
}

pub fn ui_message_stream_v1_sse_with_options(
    stream: StreamResult,
    options: UiMessageStreamV1Options,
) -> BoxStream<'static, IoResult<Bytes>> {
    futures_util::stream::unfold(
        (stream, UiStreamState::new(options)),
        move |(mut inner, mut state)| async move {
            loop {
                if let Some(item) = state.buffer.pop_front() {
                    return Some((item, (inner, state)));
                }
                if state.done {
                    return None;
                }

                match futures_util::StreamExt::next(&mut inner).await {
                    Some(Ok(chunk)) => {
                        state.handle_chunk(chunk);
                    }
                    Some(Err(err)) => {
                        state.push_error(&err.to_string());
                        state.finalize(Some(FinishReason::Error));
                    }
                    None => {
                        state.finalize(None);
                    }
                }
            }
        },
    )
    .boxed()
}
