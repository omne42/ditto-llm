use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures_util::StreamExt;
use futures_util::stream;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use tokio::sync::{mpsc, Notify};

use crate::model::{LanguageModel, StreamResult};
use crate::types::{
    ContentPart, FinishReason, GenerateRequest, GenerateResponse, JsonSchemaFormat, ResponseFormat,
    StreamChunk, Tool, ToolChoice, Usage, Warning,
};
use crate::utils::task::AbortOnDrop;
use crate::{DittoError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ObjectOutput {
    #[default]
    Object,
    Array,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ObjectStrategy {
    #[default]
    Auto,
    NativeSchema,
    ToolCall,
    TextJson,
}

#[derive(Debug, Clone)]
pub struct ObjectOptions {
    pub output: ObjectOutput,
    pub strategy: ObjectStrategy,
    pub tool_name: String,
}

impl Default for ObjectOptions {
    fn default() -> Self {
        Self {
            output: ObjectOutput::Object,
            strategy: ObjectStrategy::Auto,
            tool_name: "__ditto_object__".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct GenerateObjectResponse<T> {
    pub object: T,
    pub response: GenerateResponse,
}

#[derive(Debug, Clone)]
pub struct StreamObjectFinal {
    pub object: Value,
    pub response_id: Option<String>,
    pub warnings: Vec<Warning>,
    pub finish_reason: FinishReason,
    pub usage: Usage,
}

#[derive(Debug, Default)]
struct StreamObjectState {
    text_buffer: String,
    tool_buffer: String,
    text_truncated: bool,
    tool_truncated: bool,
    last_emitted: Option<Value>,
    last_emitted_element: usize,
    final_object: Option<Value>,
    final_error: Option<String>,
    done: bool,
    response_id: Option<String>,
    warnings: Vec<Warning>,
    finish_reason: FinishReason,
    usage: Usage,
    tool_call_id: Option<String>,
    output: ObjectOutput,
    strategy: ObjectStrategy,
}

pub struct StreamObjectResult {
    handle: StreamObjectHandle,
    ready: Arc<Notify>,
    partial_enabled: Arc<AtomicBool>,
    element_enabled: Arc<AtomicBool>,
    pub partial_object_stream: stream::BoxStream<'static, Result<Value>>,
    pub element_stream: stream::BoxStream<'static, Result<Value>>,
}

impl StreamObjectResult {
    pub fn handle(&self) -> StreamObjectHandle {
        self.handle.clone()
    }

    pub fn into_partial_stream(
        self,
    ) -> (StreamObjectHandle, stream::BoxStream<'static, Result<Value>>) {
        self.partial_enabled.store(true, Ordering::Relaxed);
        self.element_enabled.store(false, Ordering::Relaxed);
        self.ready.notify_one();
        (self.handle, self.partial_object_stream)
    }

    pub fn into_element_stream(
        self,
    ) -> (StreamObjectHandle, stream::BoxStream<'static, Result<Value>>) {
        self.partial_enabled.store(false, Ordering::Relaxed);
        self.element_enabled.store(true, Ordering::Relaxed);
        self.ready.notify_one();
        (self.handle, self.element_stream)
    }

    pub fn into_streams(
        self,
    ) -> (
        StreamObjectHandle,
        stream::BoxStream<'static, Result<Value>>,
        stream::BoxStream<'static, Result<Value>>,
    ) {
        self.partial_enabled.store(true, Ordering::Relaxed);
        self.element_enabled.store(true, Ordering::Relaxed);
        self.ready.notify_one();
        (self.handle, self.partial_object_stream, self.element_stream)
    }

    pub fn is_done(&self) -> bool {
        self.handle.is_done()
    }

    pub fn final_json(&self) -> Result<Option<Value>> {
        self.handle.final_json()
    }

    pub fn final_object<T: DeserializeOwned>(&self) -> Result<Option<T>> {
        self.handle.final_object()
    }

    pub fn final_summary(&self) -> Result<Option<StreamObjectFinal>> {
        self.handle.final_summary()
    }
}

#[async_trait]
pub trait LanguageModelObjectExt: LanguageModel {
    async fn generate_object_json(
        &self,
        request: GenerateRequest,
        schema: JsonSchemaFormat,
    ) -> Result<GenerateObjectResponse<Value>> {
        self.generate_object_json_with(request, schema, ObjectOptions::default())
            .await
    }

    async fn generate_object_json_with(
        &self,
        request: GenerateRequest,
        schema: JsonSchemaFormat,
        options: ObjectOptions,
    ) -> Result<GenerateObjectResponse<Value>> {
        let provider = self.provider();
        let strategy = resolve_object_strategy(provider, options.strategy);
        let schema = schema_for_output(schema, options.output)?;

        let request = match strategy {
            ObjectStrategy::NativeSchema => request_with_json_schema(request, provider, schema)?,
            ObjectStrategy::ToolCall => {
                request_with_tool_call(request, &options.tool_name, schema)?
            }
            ObjectStrategy::TextJson => request,
            ObjectStrategy::Auto => request,
        };

        let mut response = self.generate(request).await?;

        let mut extra_warnings = Vec::<Warning>::new();
        let object = match strategy {
            ObjectStrategy::ToolCall => match extract_object_from_tool_calls(
                &response,
                &options.tool_name,
                options.output,
                &mut extra_warnings,
            )? {
                Some(value) => value,
                None => {
                    let (value, warn) = parse_json_from_response_text(&response.text())?;
                    if let Some(warn) = warn {
                        extra_warnings.push(warn);
                    }
                    value
                }
            },
            _ => {
                let (value, warn) = parse_json_from_response_text(&response.text())?;
                if let Some(warn) = warn {
                    extra_warnings.push(warn);
                }
                value
            }
        };

        ensure_output_matches(&object, options.output)?;
        response.warnings.extend(extra_warnings);

        Ok(GenerateObjectResponse { object, response })
    }

    async fn generate_object<T: DeserializeOwned>(
        &self,
        request: GenerateRequest,
        schema: JsonSchemaFormat,
    ) -> Result<GenerateObjectResponse<T>> {
        let out = self.generate_object_json(request, schema).await?;
        let object = serde_json::from_value::<T>(out.object).map_err(|err| {
            DittoError::InvalidResponse(format!("failed to deserialize object: {err}"))
        })?;
        Ok(GenerateObjectResponse {
            object,
            response: out.response,
        })
    }

    async fn generate_object_with<T: DeserializeOwned>(
        &self,
        request: GenerateRequest,
        schema: JsonSchemaFormat,
        options: ObjectOptions,
    ) -> Result<GenerateObjectResponse<T>> {
        let out = self
            .generate_object_json_with(request, schema, options)
            .await?;
        let object = serde_json::from_value::<T>(out.object).map_err(|err| {
            DittoError::InvalidResponse(format!("failed to deserialize object: {err}"))
        })?;
        Ok(GenerateObjectResponse {
            object,
            response: out.response,
        })
    }

    async fn stream_object(
        &self,
        request: GenerateRequest,
        schema: JsonSchemaFormat,
    ) -> Result<StreamObjectResult> {
        self.stream_object_with(request, schema, ObjectOptions::default())
            .await
    }

    async fn stream_object_with(
        &self,
        request: GenerateRequest,
        schema: JsonSchemaFormat,
        options: ObjectOptions,
    ) -> Result<StreamObjectResult> {
        let provider = self.provider();
        let strategy = resolve_object_strategy(provider, options.strategy);
        let schema = schema_for_output(schema, options.output)?;

        let request = match strategy {
            ObjectStrategy::NativeSchema => request_with_json_schema(request, provider, schema)?,
            ObjectStrategy::ToolCall => {
                request_with_tool_call(request, &options.tool_name, schema)?
            }
            ObjectStrategy::TextJson => request,
            ObjectStrategy::Auto => request,
        };

        let inner = self.stream(request).await?;
        Ok(stream_object_from_stream_with_config(
            inner,
            StreamObjectConfig {
                output: options.output,
                strategy,
                tool_name: options.tool_name.clone(),
            },
        ))
    }
}

impl<T> LanguageModelObjectExt for T where T: LanguageModel + ?Sized {}

pub fn stream_object_from_stream(stream: StreamResult) -> StreamObjectResult {
    stream_object_from_stream_with_config(
        stream,
        StreamObjectConfig {
            output: ObjectOutput::Object,
            strategy: ObjectStrategy::TextJson,
            tool_name: "__ditto_object__".to_string(),
        },
    )
}

#[derive(Debug, Clone)]
struct StreamObjectConfig {
    output: ObjectOutput,
    strategy: ObjectStrategy,
    tool_name: String,
}

#[derive(Debug, Clone, Copy)]
struct StreamObjectBufferLimits {
    max_text_bytes: usize,
    max_tool_bytes: usize,
}

impl Default for StreamObjectBufferLimits {
    fn default() -> Self {
        Self {
            max_text_bytes: 64 * 1024 * 1024,
            max_tool_bytes: 64 * 1024 * 1024,
        }
    }
}

fn stream_object_from_stream_with_config(
    stream: StreamResult,
    config: StreamObjectConfig,
) -> StreamObjectResult {
    stream_object_from_stream_with_config_and_limits(stream, config, StreamObjectBufferLimits::default())
}

fn stream_object_from_stream_with_config_and_limits(
    stream: StreamResult,
    config: StreamObjectConfig,
    buffer_limits: StreamObjectBufferLimits,
) -> StreamObjectResult {
    const FANOUT_BUFFER: usize = 64;

    let state = Arc::new(Mutex::new(StreamObjectState {
        output: config.output,
        strategy: config.strategy,
        ..StreamObjectState::default()
    }));
    let state_task = state.clone();
    let handle = StreamObjectHandle { state };

    let ready = Arc::new(Notify::new());
    let partial_enabled = Arc::new(AtomicBool::new(false));
    let element_enabled = Arc::new(AtomicBool::new(false));

    let ready_task = ready.clone();
    let partial_enabled_task = partial_enabled.clone();
    let element_enabled_task = element_enabled.clone();

    let (partial_tx, partial_rx) = mpsc::channel::<Result<Value>>(FANOUT_BUFFER);
    let (element_tx, element_rx) = mpsc::channel::<Result<Value>>(FANOUT_BUFFER);

    let task = tokio::spawn(async move {
        let mut inner = stream;

        loop {
            if partial_enabled_task.load(Ordering::Acquire)
                || element_enabled_task.load(Ordering::Acquire)
            {
                break;
            }
            ready_task.notified().await;
        }

        while let Some(next) = inner.next().await {
            match next {
                Ok(chunk) => {
                    let (parsed, new_elements) = {
                        let mut state = match state_task.lock() {
                            Ok(guard) => guard,
                            Err(_) => {
                                if partial_enabled_task.load(Ordering::Relaxed) {
                                    let _ = partial_tx.try_send(Err(DittoError::InvalidResponse(
                                        "stream object state lock is poisoned".to_string(),
                                    )));
                                }
                                if element_enabled_task.load(Ordering::Relaxed) {
                                    let _ = element_tx.try_send(Err(DittoError::InvalidResponse(
                                        "stream object state lock is poisoned".to_string(),
                                    )));
                                }
                                return;
                            }
                        };

                        match &chunk {
                            StreamChunk::Warnings { warnings } => {
                                state.warnings.extend(warnings.clone());
                            }
                            StreamChunk::ResponseId { id } => {
                                if state.response_id.is_none() && !id.trim().is_empty() {
                                    state.response_id = Some(id.to_string());
                                }
                            }
                            StreamChunk::Usage(usage) => state.usage = usage.clone(),
                            StreamChunk::FinishReason(reason) => {
                                state.finish_reason = *reason;
                            }
                            StreamChunk::TextDelta { text } => {
                                if !text.is_empty() {
                                    if state.text_truncated
                                        || state
                                            .text_buffer
                                            .len()
                                            .saturating_add(text.len())
                                            > buffer_limits.max_text_bytes
                                    {
                                        if !state.text_truncated {
                                            state.text_truncated = true;
                                            state.warnings.push(Warning::Compatibility {
                                                feature: "stream_object.max_text_bytes"
                                                    .to_string(),
                                                details: format!(
                                                    "stream object text buffer exceeded max_text_bytes={}; further text will be ignored",
                                                    buffer_limits.max_text_bytes
                                                ),
                                            });
                                        }
                                    } else {
                                        state.text_buffer.push_str(text);
                                    }
                                }
                            }
                            StreamChunk::ToolCallStart { id, name } => {
                                if state.tool_call_id.is_none() {
                                    if name == &config.tool_name {
                                        state.tool_call_id = Some(id.to_string());
                                    } else if state.strategy == ObjectStrategy::ToolCall {
                                        state.warnings.push(Warning::Compatibility {
                                            feature: "object.tool_call.name".to_string(),
                                            details: format!(
                                                "tool call name mismatch (expected {expected:?}, got {got:?}); accepting first tool call",
                                                expected = config.tool_name,
                                                got = name
                                            ),
                                        });
                                        state.tool_call_id = Some(id.to_string());
                                    }
                                }
                            }
                            StreamChunk::ToolCallDelta {
                                id,
                                arguments_delta,
                            } => {
                                if state.tool_call_id.as_deref() == Some(id.as_str())
                                    || (state.tool_call_id.is_none()
                                        && state.strategy == ObjectStrategy::ToolCall)
                                {
                                    if state.tool_call_id.is_none() {
                                        state.tool_call_id = Some(id.to_string());
                                    }
                                    if !arguments_delta.is_empty() {
                                        if state.tool_truncated
                                            || state
                                                .tool_buffer
                                                .len()
                                                .saturating_add(arguments_delta.len())
                                                > buffer_limits.max_tool_bytes
                                        {
                                            if !state.tool_truncated {
                                                state.tool_truncated = true;
                                                state.warnings.push(Warning::Compatibility {
                                                    feature: "stream_object.max_tool_bytes"
                                                        .to_string(),
                                                    details: format!(
                                                        "stream object tool buffer exceeded max_tool_bytes={}; further tool deltas will be ignored",
                                                        buffer_limits.max_tool_bytes
                                                    ),
                                                });
                                            }
                                        } else {
                                            state.tool_buffer.push_str(arguments_delta);
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }

                        let mut parsed = parse_partial_object_value(&state, &config);
                        if let Some(value) = parsed.as_ref() {
                            if state.last_emitted.as_ref() == Some(value) {
                                parsed = None;
                            } else {
                                state.last_emitted = Some(value.clone());
                            }
                        }

                        let mut new_elements = Vec::<Value>::new();
                        if state.output == ObjectOutput::Array {
                            if let Some(arr) = parsed.as_ref().and_then(|v| v.as_array()) {
                                let complete_len = arr.len().saturating_sub(1);
                                while state.last_emitted_element < complete_len {
                                    new_elements.push(arr[state.last_emitted_element].clone());
                                    state.last_emitted_element =
                                        state.last_emitted_element.saturating_add(1);
                                }
                            }
                        }

                        (parsed, new_elements)
                    };

                    if let Some(value) = parsed {
                        if partial_enabled_task.load(Ordering::Relaxed) {
                            let _ = partial_tx.send(Ok(value)).await;
                        }
                    }
                    if element_enabled_task.load(Ordering::Relaxed) {
                        for element in new_elements {
                            let _ = element_tx.send(Ok(element)).await;
                        }
                    }
                }
                Err(err) => {
                    let err_string = err.to_string();
                    if let Ok(mut state) = state_task.lock() {
                        state.done = true;
                        state.final_error = Some(format!("stream failed: {err_string}"));
                    }
                    if partial_enabled_task.load(Ordering::Relaxed) {
                        let _ = partial_tx.send(Err(err)).await;
                    }
                    if element_enabled_task.load(Ordering::Relaxed) {
                        let _ = element_tx
                            .send(Err(DittoError::InvalidResponse(err_string)))
                            .await;
                    }
                    return;
                }
            }
        }

        let (text, tool, output) = {
            let mut state = match state_task.lock() {
                Ok(guard) => guard,
                Err(_) => {
                    if partial_enabled_task.load(Ordering::Relaxed) {
                        let _ = partial_tx.try_send(Err(DittoError::InvalidResponse(
                            "stream object state lock is poisoned".to_string(),
                        )));
                    }
                    if element_enabled_task.load(Ordering::Relaxed) {
                        let _ = element_tx.try_send(Err(DittoError::InvalidResponse(
                            "stream object state lock is poisoned".to_string(),
                        )));
                    }
                    return;
                }
            };
            state.done = true;
            (
                std::mem::take(&mut state.text_buffer),
                std::mem::take(&mut state.tool_buffer),
                state.output,
            )
        };

        match parse_final_object(&text, &tool, &config) {
            Ok((value, extra_warnings)) => {
                let mut should_emit_final = false;
                let mut remaining_elements = Vec::<Value>::new();
                {
                    if let Ok(mut state) = state_task.lock() {
                        state.warnings.extend(extra_warnings);
                        state.final_object = Some(value.clone());

                        if state.last_emitted.as_ref() != Some(&value) {
                            state.last_emitted = Some(value.clone());
                            should_emit_final = true;
                        }

                        if output == ObjectOutput::Array {
                            if let Some(arr) = value.as_array() {
                                while state.last_emitted_element < arr.len() {
                                    remaining_elements
                                        .push(arr[state.last_emitted_element].clone());
                                    state.last_emitted_element =
                                        state.last_emitted_element.saturating_add(1);
                                }
                            }
                        }
                    }
                }

                if should_emit_final {
                    if partial_enabled_task.load(Ordering::Relaxed) {
                        let _ = partial_tx.send(Ok(value)).await;
                    }
                }
                if element_enabled_task.load(Ordering::Relaxed) {
                    for element in remaining_elements {
                        let _ = element_tx.send(Ok(element)).await;
                    }
                }
            }
            Err(err) => {
                let err_string = err.to_string();
                if let Ok(mut state) = state_task.lock() {
                    state.final_error = Some(err_string.clone());
                }
                if partial_enabled_task.load(Ordering::Relaxed) {
                    let _ = partial_tx.send(Err(err)).await;
                }
                if element_enabled_task.load(Ordering::Relaxed) {
                    let _ = element_tx
                        .send(Err(DittoError::InvalidResponse(err_string)))
                        .await;
                }
            }
        }
    });

    let aborter = Arc::new(AbortOnDrop::new(task.abort_handle()));

    let partial_object_stream = stream::unfold(
        (partial_rx, aborter.clone(), partial_enabled.clone(), ready.clone()),
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

    let element_stream = stream::unfold(
        (element_rx, aborter, element_enabled.clone(), ready.clone()),
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

    StreamObjectResult {
        handle,
        ready,
        partial_enabled,
        element_enabled,
        partial_object_stream,
        element_stream,
    }
}

fn parse_partial_object_value(
    state: &StreamObjectState,
    config: &StreamObjectConfig,
) -> Option<Value> {
    match config.strategy {
        ObjectStrategy::ToolCall => {
            if !state.tool_buffer.trim().is_empty() {
                let parsed = parse_partial_json(&state.tool_buffer)?;
                extract_tool_call_value(&parsed).or(Some(parsed))
            } else {
                parse_partial_json(&state.text_buffer)
            }
        }
        _ => parse_partial_json(&state.text_buffer),
    }
}

fn parse_final_object(
    text: &str,
    tool: &str,
    config: &StreamObjectConfig,
) -> Result<(Value, Vec<Warning>)> {
    let mut extra_warnings = Vec::<Warning>::new();

    let value = match config.strategy {
        ObjectStrategy::ToolCall => {
            if !tool.trim().is_empty() {
                let parsed = serde_json::from_str::<Value>(tool).or_else(|_| {
                    parse_partial_json(tool).ok_or_else(|| {
                        DittoError::InvalidResponse(
                            "failed to parse tool_call arguments as JSON".to_string(),
                        )
                    })
                })?;
                extract_tool_call_value(&parsed).unwrap_or(parsed)
            } else {
                let (parsed, warn) = parse_json_from_response_text(text)?;
                if let Some(warn) = warn {
                    extra_warnings.push(warn);
                }
                parsed
            }
        }
        _ => {
            let (parsed, warn) = parse_json_from_response_text(text)?;
            if let Some(warn) = warn {
                extra_warnings.push(warn);
            }
            parsed
        }
    };

    ensure_output_matches(&value, config.output)?;
    Ok((value, extra_warnings))
}

fn request_with_json_schema(
    mut request: GenerateRequest,
    provider: &str,
    schema: JsonSchemaFormat,
) -> Result<GenerateRequest> {
    let response_format = ResponseFormat::JsonSchema {
        json_schema: schema,
    };

    let existing = request.provider_options.take();
    let merged = merge_response_format_into_provider_options(existing, provider, response_format)?;
    request.provider_options = Some(merged);
    Ok(request)
}

fn resolve_object_strategy(provider: &str, requested: ObjectStrategy) -> ObjectStrategy {
    match requested {
        ObjectStrategy::Auto => {
            if provider == "openai" {
                ObjectStrategy::NativeSchema
            } else if cfg!(feature = "tools") {
                ObjectStrategy::ToolCall
            } else {
                ObjectStrategy::TextJson
            }
        }
        other => other,
    }
}

fn schema_for_output(
    mut schema: JsonSchemaFormat,
    output: ObjectOutput,
) -> Result<JsonSchemaFormat> {
    if output == ObjectOutput::Array {
        schema.schema = serde_json::json!({
            "type": "array",
            "items": schema.schema,
        });
    }
    Ok(schema)
}

fn ensure_output_matches(value: &Value, output: ObjectOutput) -> Result<()> {
    let ok = match output {
        ObjectOutput::Object => value.is_object(),
        ObjectOutput::Array => value.is_array(),
    };
    if ok {
        return Ok(());
    }
    Err(DittoError::InvalidResponse(format!(
        "model returned {actual}, but {expected} was requested",
        actual = match value {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        },
        expected = match output {
            ObjectOutput::Object => "an object",
            ObjectOutput::Array => "an array",
        }
    )))
}

fn request_with_tool_call(
    mut request: GenerateRequest,
    tool_name: &str,
    schema: JsonSchemaFormat,
) -> Result<GenerateRequest> {
    if !cfg!(feature = "tools") {
        return Err(DittoError::InvalidResponse(
            "ditto-llm built without tools feature; ObjectStrategy::ToolCall is unavailable"
                .to_string(),
        ));
    }
    let name = tool_name.trim();
    if name.is_empty() {
        return Err(DittoError::InvalidResponse(
            "tool_name must not be empty".to_string(),
        ));
    }

    let tool = Tool {
        name: name.to_string(),
        description: None,
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "value": schema.schema,
            },
            "required": ["value"],
            "additionalProperties": false,
        }),
        strict: schema.strict,
    };

    let mut tools = request.tools.take().unwrap_or_default();
    if let Some(idx) = tools.iter().position(|t| t.name == name) {
        tools[idx] = tool;
    } else {
        tools.push(tool);
    }
    request.tools = Some(tools);
    request.tool_choice = Some(ToolChoice::Tool {
        name: name.to_string(),
    });
    Ok(request)
}

fn extract_tool_call_value(arguments: &Value) -> Option<Value> {
    let Value::Object(obj) = arguments else {
        return None;
    };
    obj.get("value").cloned()
}

fn extract_object_from_tool_calls(
    response: &GenerateResponse,
    tool_name: &str,
    output: ObjectOutput,
    warnings: &mut Vec<Warning>,
) -> Result<Option<Value>> {
    for part in &response.content {
        let ContentPart::ToolCall {
            name, arguments, ..
        } = part
        else {
            continue;
        };
        if name != tool_name {
            continue;
        }

        let parsed_arguments = match arguments {
            Value::String(raw) => {
                let raw_trimmed = raw.trim();
                if raw_trimmed.is_empty() {
                    Value::Object(Map::new())
                } else {
                    serde_json::from_str::<Value>(raw_trimmed).unwrap_or_else(|err| {
                        warnings.push(Warning::Compatibility {
                            feature: "tool_call.arguments".to_string(),
                            details: format!(
                                "failed to parse tool_call arguments as JSON for name={tool_name}: {err}; preserving raw string"
                            ),
                        });
                        Value::String(raw.to_string())
                    })
                }
            }
            other => other.clone(),
        };

        let value = match extract_tool_call_value(&parsed_arguments) {
            Some(value) => value,
            None => {
                warnings.push(Warning::Compatibility {
                    feature: "object.tool_call.arguments".to_string(),
                    details: "tool_call arguments missing `value`; using entire arguments object"
                        .to_string(),
                });
                parsed_arguments
            }
        };

        ensure_output_matches(&value, output)?;
        return Ok(Some(value));
    }

    Ok(None)
}

fn merge_response_format_into_provider_options(
    provider_options: Option<Value>,
    provider: &str,
    response_format: ResponseFormat,
) -> Result<Value> {
    let response_format_value = serde_json::to_value(response_format)?;

    match provider_options {
        None => {
            let mut obj = Map::<String, Value>::new();
            obj.insert("response_format".to_string(), response_format_value);
            Ok(Value::Object(obj))
        }
        Some(Value::Object(mut obj)) => {
            if crate::types::provider_options_object_is_bucketed(&obj) {
                let slot = obj
                    .entry(provider.to_string())
                    .or_insert_with(|| Value::Object(Map::new()));
                let Value::Object(bucket) = slot else {
                    return Err(DittoError::InvalidResponse(format!(
                        "invalid provider_options: bucket {provider:?} must be a JSON object"
                    )));
                };
                bucket.insert("response_format".to_string(), response_format_value);
                Ok(Value::Object(obj))
            } else {
                obj.insert("response_format".to_string(), response_format_value);
                Ok(Value::Object(obj))
            }
        }
        Some(_) => Err(DittoError::InvalidResponse(
            "provider_options must be a JSON object".to_string(),
        )),
    }
}
