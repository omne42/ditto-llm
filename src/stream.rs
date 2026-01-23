use std::collections::{HashMap, HashSet};

use futures_util::StreamExt;
use serde_json::Value;

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

pub async fn collect_stream(mut stream: StreamResult) -> Result<CollectedStream> {
    let mut warnings = Vec::<Warning>::new();
    let mut response_id: Option<String> = None;
    let mut finish_reason = FinishReason::Unknown;
    let mut usage = Usage::default();

    let mut parts = Vec::<CollectedPart>::new();
    let mut tool_calls = HashMap::<String, ToolCallBuffer>::new();
    let mut seen_tool_call_ids = HashSet::<String>::new();

    while let Some(chunk) = stream.next().await {
        match chunk? {
            StreamChunk::Warnings { warnings: w } => warnings.extend(w),
            StreamChunk::ResponseId { id } => {
                if response_id.is_none() && !id.trim().is_empty() {
                    response_id = Some(id);
                }
            }
            StreamChunk::TextDelta { text } => {
                if text.is_empty() {
                    continue;
                }
                match parts.last_mut() {
                    Some(CollectedPart::Text(existing)) => existing.push_str(&text),
                    _ => parts.push(CollectedPart::Text(text)),
                }
            }
            StreamChunk::ReasoningDelta { text } => {
                if text.is_empty() {
                    continue;
                }
                match parts.last_mut() {
                    Some(CollectedPart::Reasoning(existing)) => existing.push_str(&text),
                    _ => parts.push(CollectedPart::Reasoning(text)),
                }
            }
            StreamChunk::ToolCallStart { id, name } => {
                if id.trim().is_empty() {
                    warnings.push(Warning::Compatibility {
                        feature: "tool_call.id".to_string(),
                        details: "stream emitted an empty tool_call id; dropping tool call"
                            .to_string(),
                    });
                    continue;
                }

                let slot = tool_calls.entry(id.clone()).or_default();
                if slot.name.is_none() && !name.trim().is_empty() {
                    slot.name = Some(name);
                }
                if seen_tool_call_ids.insert(id.clone()) {
                    parts.push(CollectedPart::ToolCall { id });
                }
            }
            StreamChunk::ToolCallDelta {
                id,
                arguments_delta,
            } => {
                if id.trim().is_empty() {
                    warnings.push(Warning::Compatibility {
                        feature: "tool_call.id".to_string(),
                        details: "stream emitted an empty tool_call id for arguments; dropping tool call delta"
                            .to_string(),
                    });
                    continue;
                }

                let slot = tool_calls.entry(id.clone()).or_default();
                slot.arguments.push_str(&arguments_delta);
                if seen_tool_call_ids.insert(id.clone()) {
                    parts.push(CollectedPart::ToolCall { id });
                }
            }
            StreamChunk::FinishReason(reason) => finish_reason = reason,
            StreamChunk::Usage(u) => usage = u,
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
                let arguments =
                    serde_json::from_str::<Value>(raw_json).unwrap_or_else(|err| {
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

    Ok(CollectedStream {
        response_id,
        response: GenerateResponse {
            content,
            finish_reason,
            usage,
            warnings,
            provider_metadata,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;

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
}
