use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures_util::StreamExt;
use futures_util::stream;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use tokio::sync::mpsc;

use crate::model::{LanguageModel, StreamResult};
use crate::types::{
    ContentPart, FinishReason, GenerateRequest, GenerateResponse, JsonSchemaFormat, ResponseFormat,
    StreamChunk, Tool, ToolChoice, Usage, Warning,
};
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
    state: Arc<Mutex<StreamObjectState>>,
    pub partial_object_stream: stream::BoxStream<'static, Result<Value>>,
    pub element_stream: stream::BoxStream<'static, Result<Value>>,
}

struct TaskAbortOnDrop(tokio::task::AbortHandle);

impl Drop for TaskAbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl StreamObjectResult {
    pub fn is_done(&self) -> bool {
        self.state.lock().map(|s| s.done).unwrap_or(false)
    }

    pub fn final_json(&self) -> Result<Option<Value>> {
        let state = self.state.lock().map_err(|_| {
            DittoError::InvalidResponse("stream object state lock is poisoned".to_string())
        })?;
        if !state.done {
            return Ok(None);
        }
        if let Some(err) = state.final_error.as_deref() {
            return Err(DittoError::InvalidResponse(err.to_string()));
        }
        Ok(state.final_object.clone())
    }

    pub fn final_object<T: DeserializeOwned>(&self) -> Result<Option<T>> {
        self.final_json()?
            .map(|value| {
                serde_json::from_value::<T>(value).map_err(|err| {
                    DittoError::InvalidResponse(format!(
                        "failed to deserialize final object: {err}"
                    ))
                })
            })
            .transpose()
    }

    pub fn final_summary(&self) -> Result<Option<StreamObjectFinal>> {
        let state = self.state.lock().map_err(|_| {
            DittoError::InvalidResponse("stream object state lock is poisoned".to_string())
        })?;
        if !state.done {
            return Ok(None);
        }
        if let Some(err) = state.final_error.as_deref() {
            return Err(DittoError::InvalidResponse(err.to_string()));
        }
        let Some(object) = state.final_object.clone() else {
            return Ok(None);
        };
        let mut usage = state.usage.clone();
        usage.merge_total();
        Ok(Some(StreamObjectFinal {
            object,
            response_id: state.response_id.clone(),
            warnings: state.warnings.clone(),
            finish_reason: state.finish_reason,
            usage,
        }))
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

fn stream_object_from_stream_with_config(
    stream: StreamResult,
    config: StreamObjectConfig,
) -> StreamObjectResult {
    let state = Arc::new(Mutex::new(StreamObjectState {
        output: config.output,
        strategy: config.strategy,
        ..StreamObjectState::default()
    }));

    let (partial_tx, partial_rx) = mpsc::unbounded_channel::<Result<Value>>();
    let (element_tx, element_rx) = mpsc::unbounded_channel::<Result<Value>>();

    let state_task = state.clone();

    let task = tokio::spawn(async move {
        let mut inner = stream;

        while let Some(next) = inner.next().await {
            match next {
                Ok(chunk) => {
                    let (parsed, new_elements) = {
                        let mut state = match state_task.lock() {
                            Ok(guard) => guard,
                            Err(_) => {
                                let _ = partial_tx.send(Err(DittoError::InvalidResponse(
                                    "stream object state lock is poisoned".to_string(),
                                )));
                                let _ = element_tx.send(Err(DittoError::InvalidResponse(
                                    "stream object state lock is poisoned".to_string(),
                                )));
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
                                    state.text_buffer.push_str(text);
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
                                        state.tool_buffer.push_str(arguments_delta);
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
                        let _ = partial_tx.send(Ok(value));
                    }
                    for element in new_elements {
                        let _ = element_tx.send(Ok(element));
                    }
                }
                Err(err) => {
                    let err_string = err.to_string();
                    if let Ok(mut state) = state_task.lock() {
                        state.done = true;
                        state.final_error = Some(format!("stream failed: {err_string}"));
                    }
                    let _ = partial_tx.send(Err(err));
                    let _ = element_tx.send(Err(DittoError::InvalidResponse(err_string)));
                    return;
                }
            }
        }

        let (text, tool, output) = {
            let mut state = match state_task.lock() {
                Ok(guard) => guard,
                Err(_) => {
                    let _ = partial_tx.send(Err(DittoError::InvalidResponse(
                        "stream object state lock is poisoned".to_string(),
                    )));
                    let _ = element_tx.send(Err(DittoError::InvalidResponse(
                        "stream object state lock is poisoned".to_string(),
                    )));
                    return;
                }
            };
            state.done = true;
            (
                state.text_buffer.clone(),
                state.tool_buffer.clone(),
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
                    let _ = partial_tx.send(Ok(value));
                }
                for element in remaining_elements {
                    let _ = element_tx.send(Ok(element));
                }
            }
            Err(err) => {
                let err_string = err.to_string();
                if let Ok(mut state) = state_task.lock() {
                    state.final_error = Some(err_string.clone());
                }
                let _ = partial_tx.send(Err(err));
                let _ = element_tx.send(Err(DittoError::InvalidResponse(err_string)));
            }
        }
    });

    let aborter = Arc::new(TaskAbortOnDrop(task.abort_handle()));

    let partial_object_stream = stream::unfold(
        (partial_rx, aborter.clone()),
        |(mut rx, aborter)| async move { rx.recv().await.map(|item| (item, (rx, aborter))) },
    )
    .boxed();

    let element_stream = stream::unfold((element_rx, aborter), |(mut rx, aborter)| async move {
        rx.recv().await.map(|item| (item, (rx, aborter)))
    })
    .boxed();

    StreamObjectResult {
        state,
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

fn parse_json_from_response_text(text: &str) -> Result<(Value, Option<Warning>)> {
    let raw = text.trim();
    if raw.is_empty() {
        return Err(DittoError::InvalidResponse(
            "model returned an empty response; expected JSON".to_string(),
        ));
    }

    if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
        return Ok((parsed, None));
    }

    if let Some(block) = extract_code_fence(raw) {
        if let Ok(parsed) = serde_json::from_str::<Value>(block.trim()) {
            return Ok((
                parsed,
                Some(Warning::Compatibility {
                    feature: "object.json_extraction".to_string(),
                    details: "extracted JSON from a fenced code block".to_string(),
                }),
            ));
        }
    }

    if let Some(substring) = extract_balanced_json(raw) {
        if let Ok(parsed) = serde_json::from_str::<Value>(substring.trim()) {
            return Ok((
                parsed,
                Some(Warning::Compatibility {
                    feature: "object.json_extraction".to_string(),
                    details: "extracted JSON from a larger text response".to_string(),
                }),
            ));
        }
    }

    Err(DittoError::InvalidResponse(format!(
        "failed to parse model response as JSON (response starts with {:?})",
        raw.chars().take(120).collect::<String>()
    )))
}

fn extract_code_fence(text: &str) -> Option<String> {
    let start = text.find("```")?;
    let after_start = &text[start + 3..];
    let start_content_rel = after_start.find('\n').map(|idx| idx + 1)?;
    let start_content = start + 3 + start_content_rel;

    let remaining = &text[start_content..];
    let end_rel = remaining.find("```")?;
    let end = start_content + end_rel;
    let block = text[start_content..end].trim();
    if block.is_empty() {
        None
    } else {
        Some(block.to_string())
    }
}

fn extract_balanced_json(text: &str) -> Option<&str> {
    let start = text.find(['{', '['])?;
    let bytes = text.as_bytes();
    let mut in_string = false;
    let mut escape = false;
    let mut stack: Vec<u8> = Vec::new();
    let mut last_end: Option<usize> = None;

    for (offset, &b) in bytes[start..].iter().enumerate() {
        if in_string {
            if escape {
                escape = false;
                continue;
            }
            match b {
                b'\\' => escape = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }

        match b {
            b'"' => in_string = true,
            b'{' => stack.push(b'}'),
            b'[' => stack.push(b']'),
            b'}' | b']' => {
                if stack.last() == Some(&b) {
                    stack.pop();
                    if stack.is_empty() {
                        last_end = Some(start + offset + 1);
                    }
                }
            }
            _ => {}
        }
    }

    last_end.map(|end| &text[start..end])
}

fn parse_partial_json(text: &str) -> Option<Value> {
    let start = text.find(['{', '['])?;
    let bytes = text.as_bytes();
    let mut in_string = false;
    let mut escape = false;
    let mut stack: Vec<u8> = Vec::new();
    let mut last_complete_end: Option<usize> = None;

    for (offset, &b) in bytes[start..].iter().enumerate() {
        if in_string {
            if escape {
                escape = false;
                continue;
            }
            match b {
                b'\\' => escape = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }

        match b {
            b'"' => in_string = true,
            b'{' => stack.push(b'}'),
            b'[' => stack.push(b']'),
            b'}' | b']' => {
                if stack.last() == Some(&b) {
                    stack.pop();
                    if stack.is_empty() {
                        last_complete_end = Some(start + offset + 1);
                    }
                }
            }
            _ => {}
        }
    }

    if in_string || escape {
        return None;
    }

    if let Some(end) = last_complete_end {
        return serde_json::from_str::<Value>(text[start..end].trim()).ok();
    }

    let mut candidate = text[start..].to_string();

    loop {
        let trimmed = candidate.trim_end();
        let Some(last) = trimmed.as_bytes().last().copied() else {
            break;
        };
        if last == b',' || last == b':' {
            candidate.truncate(trimmed.len().saturating_sub(1));
            continue;
        }
        break;
    }

    for &closing in stack.iter().rev() {
        candidate.push(closing as char);
    }

    serde_json::from_str::<Value>(candidate.trim()).ok()
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
    async fn generate_object_parses_json_response() -> Result<()> {
        let model = FakeModel {
            provider: "openai",
            response: GenerateResponse {
                content: vec![crate::types::ContentPart::Text {
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

    #[tokio::test]
    async fn generate_object_prefers_tool_call() -> Result<()> {
        let model = FakeModel {
            provider: "openai-compatible",
            response: GenerateResponse {
                content: vec![crate::types::ContentPart::ToolCall {
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
}
