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

    let mut output_text = String::new();
    let mut output_reasoning = String::new();

    let mut tool_call_order = Vec::<String>::new();
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
            StreamChunk::TextDelta { text } => output_text.push_str(&text),
            StreamChunk::ReasoningDelta { text } => output_reasoning.push_str(&text),
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
                    tool_call_order.push(id);
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
                    tool_call_order.push(id);
                }
            }
            StreamChunk::FinishReason(reason) => finish_reason = reason,
            StreamChunk::Usage(u) => usage = u,
        }
    }

    usage.merge_total();

    let mut content = Vec::<ContentPart>::new();

    if !output_reasoning.is_empty() {
        content.push(ContentPart::Reasoning {
            text: output_reasoning,
        });
    }

    if !output_text.is_empty() {
        content.push(ContentPart::Text { text: output_text });
    }

    for tool_call_id in tool_call_order {
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

        let raw = tool_call.arguments.as_str();
        let arguments =
            serde_json::from_str::<Value>(raw).unwrap_or_else(|_| Value::String(raw.to_string()));
        content.push(ContentPart::ToolCall {
            id: tool_call_id,
            name: name.to_string(),
            arguments,
        });
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
