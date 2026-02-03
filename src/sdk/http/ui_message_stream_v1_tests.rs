#[cfg(test)]
mod ui_message_stream_v1_tests {
    use serde_json::Value;

    use futures_util::StreamExt;
    use futures_util::stream;

    use crate::DittoError;
    use crate::StreamResult;
    use crate::types::{FinishReason, StreamChunk};

    #[derive(Debug, PartialEq)]
    enum Frame {
        Json(Value),
        Done,
    }

    async fn collect_sse_payloads(stream: StreamResult) -> Vec<Frame> {
        let mut out = super::ui_message_stream_v1_sse_with_options(
            stream,
            super::UiMessageStreamV1Options {
                message_id: Some("msg_test".to_string()),
                text_id: Some("text_test".to_string()),
                reasoning_id: Some("reasoning_test".to_string()),
                max_tool_input_bytes: 1024,
                include_usage: true,
                include_warnings: true,
            },
        );

        let mut payloads = Vec::<Frame>::new();
        while let Some(item) = out.next().await {
            let bytes = item.expect("stream item");
            let text = String::from_utf8(bytes.to_vec()).expect("utf8");
            let payload = text
                .strip_prefix("data: ")
                .and_then(|s| s.strip_suffix("\n\n"))
                .expect("sse frame");
            if payload == "[DONE]" {
                payloads.push(Frame::Done);
                continue;
            }
            let value = serde_json::from_str(payload).expect("json");
            payloads.push(Frame::Json(value));
        }
        payloads
    }

    #[tokio::test]
    async fn ui_message_stream_emits_text_tool_and_finish() {
        let chunks = vec![
            Ok(StreamChunk::ResponseId {
                id: "resp_1".to_string(),
            }),
            Ok(StreamChunk::TextDelta {
                text: "hello ".to_string(),
            }),
            Ok(StreamChunk::ReasoningDelta {
                text: "thinking".to_string(),
            }),
            Ok(StreamChunk::ToolCallStart {
                id: "call_1".to_string(),
                name: "getWeather".to_string(),
            }),
            Ok(StreamChunk::ToolCallDelta {
                id: "call_1".to_string(),
                arguments_delta: "{\"city\":\"SF\"}".to_string(),
            }),
            Ok(StreamChunk::FinishReason(FinishReason::ToolCalls)),
        ];
        let stream: StreamResult = stream::iter(chunks).boxed();

        let payloads = collect_sse_payloads(stream).await;
        assert_eq!(
            payloads,
            vec![
                Frame::Json(serde_json::json!({
                    "type": "start",
                    "messageId": "msg_test",
                })),
                Frame::Json(serde_json::json!({
                    "type": "start-step",
                })),
                Frame::Json(serde_json::json!({
                    "type": "message-metadata",
                    "messageMetadata": { "responseId": "resp_1" },
                })),
                Frame::Json(serde_json::json!({
                    "type": "text-start",
                    "id": "text_test",
                })),
                Frame::Json(serde_json::json!({
                    "type": "text-delta",
                    "id": "text_test",
                    "delta": "hello ",
                })),
                Frame::Json(serde_json::json!({
                    "type": "reasoning-start",
                    "id": "reasoning_test",
                })),
                Frame::Json(serde_json::json!({
                    "type": "reasoning-delta",
                    "id": "reasoning_test",
                    "delta": "thinking",
                })),
                Frame::Json(serde_json::json!({
                    "type": "tool-input-start",
                    "toolCallId": "call_1",
                    "toolName": "getWeather",
                })),
                Frame::Json(serde_json::json!({
                    "type": "tool-input-delta",
                    "toolCallId": "call_1",
                    "inputTextDelta": "{\"city\":\"SF\"}",
                })),
                Frame::Json(serde_json::json!({
                    "type": "tool-input-available",
                    "toolCallId": "call_1",
                    "toolName": "getWeather",
                    "input": { "city": "SF" },
                })),
                Frame::Json(serde_json::json!({
                    "type": "text-end",
                    "id": "text_test",
                })),
                Frame::Json(serde_json::json!({
                    "type": "reasoning-end",
                    "id": "reasoning_test",
                })),
                Frame::Json(serde_json::json!({
                    "type": "finish-step",
                })),
                Frame::Json(serde_json::json!({
                    "type": "finish",
                    "finishReason": "tool-calls",
                })),
                Frame::Done,
            ]
        );
    }

    #[tokio::test]
    async fn ui_message_stream_error_then_done() {
        let chunks = vec![
            Ok(StreamChunk::TextDelta {
                text: "hello".to_string(),
            }),
            Err(DittoError::InvalidResponse("boom".to_string())),
        ];
        let stream: StreamResult = stream::iter(chunks).boxed();

        let payloads = collect_sse_payloads(stream).await;
        assert_eq!(
            payloads,
            vec![
                Frame::Json(serde_json::json!({
                    "type": "start",
                    "messageId": "msg_test",
                })),
                Frame::Json(serde_json::json!({
                    "type": "start-step",
                })),
                Frame::Json(serde_json::json!({
                    "type": "text-start",
                    "id": "text_test",
                })),
                Frame::Json(serde_json::json!({
                    "type": "text-delta",
                    "id": "text_test",
                    "delta": "hello",
                })),
                Frame::Json(serde_json::json!({
                    "type": "error",
                    "errorText": "invalid response: boom",
                })),
                Frame::Json(serde_json::json!({
                    "type": "text-end",
                    "id": "text_test",
                })),
                Frame::Json(serde_json::json!({
                    "type": "finish-step",
                })),
                Frame::Json(serde_json::json!({
                    "type": "finish",
                    "finishReason": "error",
                })),
                Frame::Done,
            ]
        );
    }

    #[tokio::test]
    async fn ui_message_stream_emits_tool_start_even_if_delta_arrives_first() {
        let chunks = vec![
            Ok(StreamChunk::ToolCallDelta {
                id: "call_1".to_string(),
                arguments_delta: "{\"q\":1}".to_string(),
            }),
            Ok(StreamChunk::FinishReason(FinishReason::ToolCalls)),
        ];
        let stream: StreamResult = stream::iter(chunks).boxed();

        let payloads = collect_sse_payloads(stream).await;
        assert_eq!(
            payloads,
            vec![
                Frame::Json(serde_json::json!({
                    "type": "start",
                    "messageId": "msg_test",
                })),
                Frame::Json(serde_json::json!({
                    "type": "start-step",
                })),
                Frame::Json(serde_json::json!({
                    "type": "tool-input-start",
                    "toolCallId": "call_1",
                    "toolName": "unknown",
                })),
                Frame::Json(serde_json::json!({
                    "type": "tool-input-delta",
                    "toolCallId": "call_1",
                    "inputTextDelta": "{\"q\":1}",
                })),
                Frame::Json(serde_json::json!({
                    "type": "tool-input-available",
                    "toolCallId": "call_1",
                    "toolName": "unknown",
                    "input": { "q": 1 },
                })),
                Frame::Json(serde_json::json!({
                    "type": "finish-step",
                })),
                Frame::Json(serde_json::json!({
                    "type": "finish",
                    "finishReason": "tool-calls",
                })),
                Frame::Done,
            ]
        );
    }
}
