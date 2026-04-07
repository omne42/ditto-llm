use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};

use async_trait::async_trait;
use futures_util::{StreamExt, stream};
#[cfg(feature = "cap-llm-tools")]
use serde_json::Value;
use serde_json::json;
use tokio::time::{Duration, sleep};

#[cfg(feature = "cap-llm-tools")]
use super::ObjectOptions;
#[cfg(feature = "cap-llm-tools")]
use super::core::stream_object_from_stream_with_config;
use super::core::{
    LanguageModelObjectExt, StreamObjectBufferLimits, StreamObjectConfig,
    stream_object_from_stream_with_config_and_limits,
};
use super::json::{parse_json_from_response_text, parse_partial_json};
use super::{ObjectOutput, ObjectStrategy, StreamObjectResult, stream_object_from_stream};
use crate::contracts::{
    ContentPart, FinishReason, GenerateRequest, GenerateResponse, StreamChunk, Warning,
};
use crate::error::Result;
use crate::llm_core::model::{LanguageModel, StreamResult};
use crate::provider_options::JsonSchemaFormat;

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
        Err(crate::invalid_response!(
            "error_detail.text.not_implemented"
        ))
    }
}

#[tokio::test]
async fn generate_object_parses_json_response() -> Result<()> {
    let model = FakeModel {
        provider: "openai",
        response: GenerateResponse {
            content: vec![ContentPart::Text {
                text: "{\"a\":1}".to_string(),
            }],
            ..GenerateResponse::default()
        },
    };

    let schema = JsonSchemaFormat {
        name: "unit_test".to_string(),
        schema: json!({"type":"object"}),
        strict: None,
    };

    let out = model
        .generate_object_json(GenerateRequest::from(vec![]), schema)
        .await?;
    assert_eq!(out.object, json!({"a":1}));
    Ok(())
}

#[cfg(feature = "cap-llm-tools")]
#[tokio::test]
async fn generate_object_prefers_tool_call() -> Result<()> {
    let model = FakeModel {
        provider: "openai-compatible",
        response: GenerateResponse {
            content: vec![ContentPart::ToolCall {
                id: "call_0".to_string(),
                name: "__ditto_object__".to_string(),
                arguments: json!({"value": {"a": 1}}),
            }],
            ..GenerateResponse::default()
        },
    };

    let schema = JsonSchemaFormat {
        name: "unit_test".to_string(),
        schema: json!({"type":"object"}),
        strict: None,
    };

    let out = model
        .generate_object_json_with(
            GenerateRequest::from(vec![]),
            schema,
            ObjectOptions {
                strategy: ObjectStrategy::ToolCall,
                ..ObjectOptions::default()
            },
        )
        .await?;

    assert_eq!(out.object, json!({"a": 1}));
    Ok(())
}

#[cfg(feature = "cap-llm-tools")]
#[tokio::test]
async fn stream_object_tool_call_emits_array_elements() -> Result<()> {
    let chunks = vec![
        Ok(StreamChunk::ToolCallStart {
            id: "call_0".to_string(),
            name: "__ditto_object__".to_string(),
        }),
        Ok(StreamChunk::ToolCallDelta {
            id: "call_0".to_string(),
            arguments_delta: "{\"value\":[".to_string(),
        }),
        Ok(StreamChunk::ToolCallDelta {
            id: "call_0".to_string(),
            arguments_delta: "{\"a\":1},".to_string(),
        }),
        Ok(StreamChunk::ToolCallDelta {
            id: "call_0".to_string(),
            arguments_delta: "{\"a\":2}]".to_string(),
        }),
        Ok(StreamChunk::ToolCallDelta {
            id: "call_0".to_string(),
            arguments_delta: "}".to_string(),
        }),
        Ok(StreamChunk::FinishReason(FinishReason::Stop)),
    ];

    let inner: StreamResult = stream::iter(chunks).boxed();

    let mut result = stream_object_from_stream_with_config(
        inner,
        StreamObjectConfig {
            output: ObjectOutput::Array,
            strategy: ObjectStrategy::ToolCall,
            tool_name: "__ditto_object__".to_string(),
        },
    );

    let mut elements = Vec::<Value>::new();
    while let Some(next) = result.element_stream.next().await {
        elements.push(next?);
    }

    assert_eq!(elements, vec![json!({"a": 1}), json!({"a": 2})]);
    assert_eq!(result.final_json()?.unwrap(), json!([{"a": 1}, {"a": 2}]));
    Ok(())
}

#[test]
fn partial_json_emits_when_object_is_balanced_or_repairable() {
    assert_eq!(parse_partial_json("{\"a\":1"), Some(json!({"a":1})));
    assert_eq!(parse_partial_json("{\"a\":1}"), Some(json!({"a":1})));
    assert_eq!(parse_partial_json("{\"a\":\"x"), None);
}

#[test]
fn parse_json_from_text_extracts_code_fence() -> Result<()> {
    let text = "Here:\n```json\n{\"a\":1}\n```\n";
    let (value, warn) = parse_json_from_response_text(text)?;
    assert_eq!(value, json!({"a":1}));
    assert!(warn.is_some());
    Ok(())
}

#[tokio::test]
async fn stream_object_truncates_text_buffer_without_oom() -> Result<()> {
    let chunks = vec![
        Ok(StreamChunk::TextDelta {
            text: "{\"a\":1}".to_string(),
        }),
        Ok(StreamChunk::TextDelta {
            text: " trailing".to_string(),
        }),
        Ok(StreamChunk::FinishReason(FinishReason::Stop)),
    ];

    let inner: StreamResult = stream::iter(chunks).boxed();

    let (handle, mut partial_object_stream) = stream_object_from_stream_with_config_and_limits(
        inner,
        StreamObjectConfig {
            output: ObjectOutput::Object,
            strategy: ObjectStrategy::TextJson,
            tool_name: "__ditto_object__".to_string(),
        },
        StreamObjectBufferLimits {
            max_text_bytes: 7,
            max_tool_bytes: 64 * 1024,
        },
    )
    .into_partial_stream();

    while let Some(next) = partial_object_stream.next().await {
        let _ = next?;
    }

    let summary = handle.final_summary()?.unwrap();
    assert_eq!(summary.object, json!({"a": 1}));
    assert!(summary.warnings.iter().any(|warning| matches!(
        warning,
        Warning::Compatibility { feature, .. } if feature == "stream_object.max_text_bytes"
    )));
    Ok(())
}

#[tokio::test]
async fn handle_only_mode_collects_final_object() -> Result<()> {
    let chunks = vec![
        Ok(StreamChunk::TextDelta {
            text: "{\"a\":".to_string(),
        }),
        Ok(StreamChunk::TextDelta {
            text: "1}".to_string(),
        }),
        Ok(StreamChunk::FinishReason(FinishReason::Stop)),
    ];

    let inner: StreamResult = stream::iter(chunks).boxed();
    let result = stream_object_from_stream(inner);
    let handle = result.handle();

    for _ in 0..16 {
        if handle.is_done() {
            break;
        }
        tokio::task::yield_now().await;
    }

    let summary = handle.final_summary()?.unwrap();
    assert_eq!(summary.object, json!({"a": 1}));
    assert_eq!(summary.finish_reason, FinishReason::Stop);
    drop(result);
    Ok(())
}

#[tokio::test]
async fn late_partial_subscription_receives_final_object() -> Result<()> {
    let inner: StreamResult = stream::unfold(0u8, |step| async move {
        match step {
            0 => Some((
                Ok(StreamChunk::TextDelta {
                    text: "{\"a\":1}".to_string(),
                }),
                1,
            )),
            1 => {
                sleep(Duration::from_millis(20)).await;
                Some((Ok(StreamChunk::FinishReason(FinishReason::Stop)), 2))
            }
            _ => None,
        }
    })
    .boxed();

    let result = stream_object_from_stream(inner);
    sleep(Duration::from_millis(5)).await;

    let (_handle, mut partial_object_stream) = result.into_partial_stream();
    let mut partials = Vec::new();
    while let Some(next) = partial_object_stream.next().await {
        partials.push(next?);
    }

    assert_eq!(partials.last(), Some(&json!({"a": 1})));
    Ok(())
}

#[tokio::test]
async fn dropping_streams_aborts_background_task() -> Result<()> {
    let dropped = Arc::new(AtomicBool::new(false));
    let inner: StreamResult = Box::pin(DropFlagStream {
        dropped: dropped.clone(),
    })
    .boxed();

    let StreamObjectResult {
        partial_object_stream,
        element_stream,
        ..
    } = stream_object_from_stream(inner);

    drop(partial_object_stream);
    drop(element_stream);

    for _ in 0..16 {
        if dropped.load(Ordering::SeqCst) {
            break;
        }
        tokio::task::yield_now().await;
    }

    assert!(dropped.load(Ordering::SeqCst));
    Ok(())
}

#[test]
fn poisoned_object_handle_reports_done_and_keeps_lock_error() {
    let state = Arc::new(std::sync::Mutex::new(
        super::core::StreamObjectState::default(),
    ));
    let poisoned = state.clone();
    let _ = std::thread::spawn(move || {
        let _guard = poisoned.lock().expect("lock");
        panic!("poison object state");
    })
    .join();

    let handle = super::handle::StreamObjectHandle { state };
    assert!(handle.is_done());
    assert!(handle.final_json().is_err());
    assert!(handle.final_summary().is_err());
    let typed: Result<Option<serde_json::Value>> = handle.final_object();
    assert!(typed.is_err());
}
