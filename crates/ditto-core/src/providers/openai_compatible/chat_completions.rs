#[derive(Debug, Deserialize)]
struct ChatCompletionsResponse {
    id: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    choices: Vec<ChatChoice>,
    #[serde(default)]
    usage: Option<Value>,
}

#[derive(Debug, Deserialize, Default)]
struct ChatChoice {
    #[serde(default)]
    message: ChatMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct ChatMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    reasoning: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ChatToolCall>>,
    #[serde(default)]
    function_call: Option<ChatFunctionCall>,
}

#[derive(Debug, Deserialize, Default)]
struct ChatFunctionCall {
    #[serde(default)]
    name: String,
    #[serde(default)]
    arguments: String,
}

#[derive(Debug, Deserialize, Default)]
struct ChatToolCall {
    #[serde(default)]
    id: String,
    #[serde(default, alias = "thoughtSignature")]
    thought_signature: Option<String>,
    #[serde(default)]
    function: ChatToolFunction,
}

#[derive(Debug, Deserialize, Default)]
struct ChatToolFunction {
    #[serde(default)]
    name: String,
    #[serde(default)]
    arguments: String,
    #[serde(default, alias = "thoughtSignature")]
    thought_signature: Option<String>,
}

#[cfg(feature = "cap-llm-streaming")]
#[derive(Debug, Deserialize, Default)]
struct ChatCompletionsChunk {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    choices: Vec<ChatChoiceChunk>,
    #[serde(default)]
    usage: Option<Value>,
}

#[cfg(feature = "cap-llm-streaming")]
#[derive(Debug, Deserialize, Default)]
struct ChatChoiceChunk {
    #[serde(default)]
    delta: ChatDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[cfg(feature = "cap-llm-streaming")]
#[derive(Debug, Deserialize, Default)]
struct ChatDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    reasoning: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ChatToolCallDelta>>,
    #[serde(default)]
    function_call: Option<ChatFunctionCallDelta>,
}

#[cfg(feature = "cap-llm-streaming")]
#[derive(Debug, Deserialize, Default)]
struct ChatFunctionCallDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[cfg(feature = "cap-llm-streaming")]
#[derive(Debug, Deserialize)]
struct ChatToolCallDelta {
    #[serde(default)]
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default, alias = "thoughtSignature")]
    thought_signature: Option<String>,
    #[serde(default)]
    function: Option<ChatToolFunctionDelta>,
}

#[cfg(feature = "cap-llm-streaming")]
#[derive(Debug, Deserialize, Default)]
struct ChatToolFunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
    #[serde(default, alias = "thoughtSignature")]
    thought_signature: Option<String>,
}

#[cfg(feature = "cap-llm-streaming")]
#[derive(Debug, Default)]
struct StreamToolCallState {
    id: Option<String>,
    name: Option<String>,
    thought_signature: Option<String>,
    started: bool,
    pending_arguments: String,
}

#[cfg(feature = "cap-llm-streaming")]
#[derive(Debug, Default)]
struct StreamState {
    response_id: Option<String>,
    tool_calls: Vec<StreamToolCallState>,
    finish_reason: Option<String>,
}

#[cfg(feature = "cap-llm-streaming")]
fn finalize_stream_state(state: &mut StreamState) -> Vec<StreamChunk> {
    let mut out = Vec::<StreamChunk>::new();
    let mut warnings = Vec::<Warning>::new();

    for (idx, slot) in state.tool_calls.iter_mut().enumerate() {
        if slot.started {
            continue;
        }

        let name = slot.name.as_deref().unwrap_or("").trim();
        let has_any_data = slot.id.as_deref().is_some_and(|v| !v.trim().is_empty())
            || !name.is_empty()
            || !slot.pending_arguments.is_empty();

        if !has_any_data {
            continue;
        }

        let id = match slot.id.as_deref().filter(|v| !v.trim().is_empty()) {
            Some(id) => id.to_string(),
            None => {
                let synthesized = format!("call_{idx}");
                slot.id = Some(synthesized.clone());
                warnings.push(Warning::Compatibility {
                    feature: "tool_call.id".to_string(),
                    details: format!(
                        "stream ended before tool_call id was received; synthesizing {synthesized}"
                    ),
                });
                synthesized
            }
        };
        let id = encode_tool_call_id_with_thought_signature(&id, slot.thought_signature.as_deref());
        slot.id = Some(id.clone());

        if name.is_empty() {
            warnings.push(Warning::Compatibility {
                feature: "tool_call.name".to_string(),
                details: format!(
                    "stream ended before tool_call name was received for id={id}; dropping tool call"
                ),
            });
            slot.pending_arguments.clear();
            continue;
        }

        out.push(StreamChunk::ToolCallStart {
            id: id.clone(),
            name: name.to_string(),
        });
        slot.started = true;

        if !slot.pending_arguments.is_empty() {
            out.push(StreamChunk::ToolCallDelta {
                id,
                arguments_delta: std::mem::take(&mut slot.pending_arguments),
            });
        }
    }

    if !warnings.is_empty() {
        out.insert(0, StreamChunk::Warnings { warnings });
    }

    out
}

#[cfg(feature = "cap-llm-streaming")]
fn parse_stream_data(state: &mut StreamState, data: &str) -> Result<(Vec<StreamChunk>, bool)> {
    let chunk = serde_json::from_str::<ChatCompletionsChunk>(data)?;
    let mut out = Vec::<StreamChunk>::new();
    let mut done = false;

    if state.response_id.is_none() {
        if let Some(id) = chunk.id.as_deref().filter(|id| !id.trim().is_empty()) {
            state.response_id = Some(id.to_string());
            out.push(StreamChunk::ResponseId { id: id.to_string() });
        }
    }

    if let Some(usage) = chunk.usage.as_ref() {
        out.push(StreamChunk::Usage(OpenAICompatible::parse_usage(usage)));
    }

    let Some(choice) = chunk.choices.first() else {
        return Ok((out, done));
    };

    if let Some(reasoning) = choice
        .delta
        .reasoning_content
        .as_deref()
        .or(choice.delta.reasoning.as_deref())
    {
        if !reasoning.is_empty() {
            out.push(StreamChunk::ReasoningDelta {
                text: reasoning.to_string(),
            });
        }
    }

    if let Some(content) = choice.delta.content.as_deref() {
        if !content.is_empty() {
            out.push(StreamChunk::TextDelta {
                text: content.to_string(),
            });
        }
    }

    if let Some(tool_calls) = choice.delta.tool_calls.as_ref() {
        for tool_call in tool_calls {
            let idx = tool_call.index;
            while state.tool_calls.len() <= idx {
                state.tool_calls.push(StreamToolCallState::default());
            }
            let slot = &mut state.tool_calls[idx];

            if let Some(id) = tool_call.id.as_deref().filter(|v| !v.trim().is_empty()) {
                slot.id = Some(id.to_string());
            }
            if let Some(thought_signature) = tool_call
                .thought_signature
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
            {
                slot.thought_signature = Some(thought_signature.to_string());
            }
            if let Some(function) = tool_call.function.as_ref() {
                if let Some(name) = function.name.as_deref().filter(|v| !v.trim().is_empty()) {
                    slot.name = Some(name.to_string());
                }
                if let Some(thought_signature) = function
                    .thought_signature
                    .as_deref()
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                {
                    slot.thought_signature = Some(thought_signature.to_string());
                }

                let arguments = function.arguments.as_deref().unwrap_or("");
                if !arguments.is_empty() {
                    if slot.started {
                        if let Some(id) = slot.id.as_deref() {
                            out.push(StreamChunk::ToolCallDelta {
                                id: id.to_string(),
                                arguments_delta: arguments.to_string(),
                            });
                        }
                    } else {
                        slot.pending_arguments.push_str(arguments);
                    }
                }
            }

            if !slot.started {
                if let (Some(id), Some(name)) = (slot.id.as_deref(), slot.name.as_deref()) {
                    let id = encode_tool_call_id_with_thought_signature(
                        id,
                        slot.thought_signature.as_deref(),
                    );
                    slot.id = Some(id.clone());
                    out.push(StreamChunk::ToolCallStart {
                        id: id.clone(),
                        name: name.to_string(),
                    });
                    slot.started = true;
                    if !slot.pending_arguments.is_empty() {
                        out.push(StreamChunk::ToolCallDelta {
                            id,
                            arguments_delta: std::mem::take(&mut slot.pending_arguments),
                        });
                    }
                }
            }
        }
    }

    if let Some(function_call) = choice.delta.function_call.as_ref() {
        let mut warnings = Vec::<Warning>::new();
        while state.tool_calls.is_empty() {
            state.tool_calls.push(StreamToolCallState::default());
        }
        let slot = &mut state.tool_calls[0];
        if slot.id.is_none() {
            slot.id = Some("call_0".to_string());
            warnings.push(Warning::Compatibility {
                feature: "tool_call.id".to_string(),
                details: "legacy function_call does not provide tool_call ids; synthesizing call_0"
                    .to_string(),
            });
        }
        if !warnings.is_empty() {
            out.push(StreamChunk::Warnings { warnings });
        }

        if let Some(name) = function_call
            .name
            .as_deref()
            .filter(|v| !v.trim().is_empty())
        {
            slot.name = Some(name.to_string());
        }

        let arguments = function_call.arguments.as_deref().unwrap_or("");
        if !arguments.is_empty() {
            if slot.started {
                if let Some(id) = slot.id.as_deref() {
                    out.push(StreamChunk::ToolCallDelta {
                        id: id.to_string(),
                        arguments_delta: arguments.to_string(),
                    });
                }
            } else {
                slot.pending_arguments.push_str(arguments);
            }
        }

        if !slot.started {
            if let (Some(id), Some(name)) = (slot.id.as_deref(), slot.name.as_deref()) {
                out.push(StreamChunk::ToolCallStart {
                    id: id.to_string(),
                    name: name.to_string(),
                });
                slot.started = true;
                if !slot.pending_arguments.is_empty() {
                    out.push(StreamChunk::ToolCallDelta {
                        id: id.to_string(),
                        arguments_delta: std::mem::take(&mut slot.pending_arguments),
                    });
                }
            }
        }
    }

    if let Some(reason) = choice.finish_reason.as_deref() {
        if state.finish_reason.is_none() && !reason.trim().is_empty() {
            state.finish_reason = Some(reason.to_string());
        }
        out.extend(finalize_stream_state(state));
        done = true;
        out.push(StreamChunk::FinishReason(
            OpenAICompatible::parse_finish_reason(Some(reason)),
        ));
    }

    Ok((out, done))
}

#[async_trait]
impl LanguageModel for OpenAICompatible {
    fn provider(&self) -> &str {
        "openai-compatible"
    }

    fn model_id(&self) -> &str {
        self.client.model.as_str()
    }

    async fn generate(&self, request: GenerateRequest) -> Result<GenerateResponse> {
        let model = self.resolve_model(&request)?;
        let request_quirks = self.request_quirks_for_model(model);
        let raw_selected_provider_options =
            crate::provider_options::request_provider_options_value_for(&request, self.provider())?;
        let provider_options = raw_selected_provider_options
            .as_ref()
            .map(crate::provider_options::ProviderOptions::from_value_ref)
            .transpose()?
            .unwrap_or_default();
        let schema = apply_openai_compatible_provider_options_schema(
            request_quirks.family,
            raw_selected_provider_options,
            OPENAI_COMPAT_RESERVED_PROVIDER_OPTION_KEYS,
            "generate.provider_options",
        );
        let selected_provider_options = schema.selected_provider_options;
        let (body, mut warnings) = Self::build_chat_completions_body(
            &request,
            model,
            request_quirks,
            &provider_options,
            selected_provider_options.as_ref(),
            false,
            "generate.provider_options",
        )?;
        warnings.extend(schema.warnings);

        let url = self.chat_completions_url();
        let mut req = self.client.http.post(url);
        req = self.apply_auth(req);
        let parsed = crate::provider_transport::send_checked_json::<ChatCompletionsResponse>(
            req.json(&body),
        )
        .await?;
        let choice = parsed.choices.first().ok_or_else(|| {
            crate::invalid_response!("error_detail.openai.chat_completions_response_no_choices")
        })?;

        let mut content = Vec::<ContentPart>::new();
        if let Some(reasoning) = choice
            .message
            .reasoning_content
            .as_deref()
            .or(choice.message.reasoning.as_deref())
            .filter(|t| !t.is_empty())
        {
            content.push(ContentPart::Reasoning {
                text: reasoning.to_string(),
            });
        }
        if let Some(text) = choice.message.content.as_deref().filter(|t| !t.is_empty()) {
            content.push(ContentPart::Text {
                text: text.to_string(),
            });
        }
        match choice.message.tool_calls.as_ref() {
            Some(tool_calls) if !tool_calls.is_empty() => {
                for tool_call in tool_calls {
                    let arguments_raw = tool_call.function.arguments.as_str();
                    let context = format!("id={}", tool_call.id);
                    let arguments = crate::contracts::parse_tool_call_arguments_json_or_string(
                        arguments_raw,
                        &context,
                        &mut warnings,
                    );
                    let thought_signature = tool_call
                        .function
                        .thought_signature
                        .as_deref()
                        .or(tool_call.thought_signature.as_deref())
                        .map(str::trim)
                        .filter(|v| !v.is_empty());
                    let id = encode_tool_call_id_with_thought_signature(
                        &tool_call.id,
                        thought_signature,
                    );
                    content.push(ContentPart::ToolCall {
                        id,
                        name: tool_call.function.name.clone(),
                        arguments,
                    });
                }
            }
            _ => {
                if let Some(function_call) = choice.message.function_call.as_ref() {
                    warnings.push(Warning::Compatibility {
                feature: "tool_call.id".to_string(),
                details:
                    "legacy function_call does not provide tool_call ids; synthesizing call_0"
                        .to_string(),
            });

                    let name = function_call.name.trim();
                    if !name.is_empty() {
                        let arguments_raw = function_call.arguments.as_str();
                        let context = format!("name={name}");
                        let arguments = crate::contracts::parse_tool_call_arguments_json_or_string(
                            arguments_raw,
                            &context,
                            &mut warnings,
                        );
                        content.push(ContentPart::ToolCall {
                            id: "call_0".to_string(),
                            name: name.to_string(),
                            arguments,
                        });
                    } else {
                        warnings.push(Warning::Compatibility {
                            feature: "tool_call.name".to_string(),
                            details: "function_call.name is empty; dropping tool call".to_string(),
                        });
                    }
                }
            }
        }

        let usage = parsed
            .usage
            .as_ref()
            .map(Self::parse_usage)
            .unwrap_or_default();

        let finish_reason = Self::parse_finish_reason(choice.finish_reason.as_deref());

        Ok(GenerateResponse {
            content,
            finish_reason,
            usage,
            warnings,
            provider_metadata: Some(serde_json::json!({ "id": parsed.id, "model": parsed.model })),
        })
    }

    async fn stream(&self, request: GenerateRequest) -> Result<StreamResult> {
        #[cfg(not(feature = "cap-llm-streaming"))]
        {
            let _ = request;
            return Err(DittoError::builder_capability_feature_missing(
                "openai-compatible",
                "streaming",
            ));
        }

        #[cfg(feature = "cap-llm-streaming")]
        {
            let model = self.resolve_model(&request)?;
            let request_quirks = self.request_quirks_for_model(model);
            let raw_selected_provider_options =
                crate::provider_options::request_provider_options_value_for(
                    &request,
                    self.provider(),
                )?;
            let provider_options = raw_selected_provider_options
                .as_ref()
                .map(crate::provider_options::ProviderOptions::from_value_ref)
                .transpose()?
                .unwrap_or_default();
            let schema = apply_openai_compatible_provider_options_schema(
                request_quirks.family,
                raw_selected_provider_options,
                OPENAI_COMPAT_RESERVED_PROVIDER_OPTION_KEYS,
                "stream.provider_options",
            );
            let selected_provider_options = schema.selected_provider_options;
            let (body, mut warnings) = Self::build_chat_completions_body(
                &request,
                model,
                request_quirks,
                &provider_options,
                selected_provider_options.as_ref(),
                true,
                "stream.provider_options",
            )?;
            warnings.extend(schema.warnings);

            let url = self.chat_completions_url();
            let req = self
                .client
                .http
                .post(url)
                .header("Accept", "text/event-stream")
                .json(&body);
            let response = crate::provider_transport::send_checked(self.apply_auth(req)).await?;

            let (data_stream, buffer) =
                crate::session_transport::init_sse_stream(response, warnings);
            let stream = stream::unfold(
                (data_stream, buffer, StreamState::default(), false),
                |(mut data_stream, mut buffer, mut state, mut done)| async move {
                    loop {
                        if let Some(item) = buffer.pop_front() {
                            return Some((item, (data_stream, buffer, state, done)));
                        }

                        if done {
                            return None;
                        }

                        let next = data_stream.next().await;
                        match next {
                            Some(Ok(data)) => match parse_stream_data(&mut state, &data) {
                                Ok((chunks, _is_done)) => {
                                    for chunk in chunks {
                                        buffer.push_back(Ok(chunk));
                                    }
                                }
                                Err(err) => {
                                    done = true;
                                    buffer.push_back(Err(err));
                                }
                            },
                            Some(Err(err)) => {
                                done = true;
                                buffer.push_back(Err(err));
                            }
                            None => {
                                done = true;
                                for chunk in finalize_stream_state(&mut state) {
                                    buffer.push_back(Ok(chunk));
                                }
                                let has_tool_calls =
                                    state.tool_calls.iter().any(|slot| slot.started);
                                if state.finish_reason.is_none() {
                                    buffer.push_back(Ok(StreamChunk::FinishReason(
                                        if has_tool_calls {
                                            FinishReason::ToolCalls
                                        } else {
                                            FinishReason::Stop
                                        },
                                    )));
                                }
                            }
                        }
                    }
                },
            );

            Ok(Box::pin(stream))
        }
    }
}

#[cfg(all(test, feature = "cap-llm-streaming"))]
mod chat_completions_tests {
    use super::*;

    #[test]
    fn streaming_finish_reason_can_arrive_before_text() {
        let mut state = StreamState::default();

        let data1 = r#"{
            "id": "resp_1",
            "choices": [{"delta": {}, "finish_reason": "stop"}]
        }"#;
        let (chunks1, done1) = parse_stream_data(&mut state, data1).unwrap();
        assert!(done1);
        assert!(
            chunks1
                .iter()
                .any(|c| matches!(c, StreamChunk::FinishReason(FinishReason::Stop)))
        );
        assert_eq!(state.finish_reason.as_deref(), Some("stop"));

        let data2 = r#"{
            "id": "resp_1",
            "choices": [{"delta": {"content": "OK"}}]
        }"#;
        let (chunks2, done2) = parse_stream_data(&mut state, data2).unwrap();
        assert!(!done2);
        assert!(
            chunks2
                .iter()
                .any(|c| matches!(c, StreamChunk::TextDelta { text } if text == "OK"))
        );
    }

    #[test]
    fn streaming_reasoning_content_emits_reasoning_delta() {
        let mut state = StreamState::default();

        let data = r#"{
            "id": "resp_1",
            "choices": [{"delta": {"reasoning_content": "thinking..."}}]
        }"#;
        let (chunks, done) = parse_stream_data(&mut state, data).unwrap();
        assert!(!done);
        assert!(
            chunks.iter().any(
                |c| matches!(c, StreamChunk::ReasoningDelta { text } if text == "thinking...")
            )
        );
    }

    #[test]
    fn streaming_tool_call_thought_signature_encodes_tool_call_id() {
        let mut state = StreamState::default();

        let data = r#"{
            "id": "resp_1",
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_1",
                        "function": {
                            "name": "workspace",
                            "arguments": "{\"op\":\"help\"}",
                            "thought_signature": "hi"
                        }
                    }]
                }
            }]
        }"#;
        let (chunks, done) = parse_stream_data(&mut state, data).unwrap();
        assert!(!done);

        let start_id = chunks.iter().find_map(|chunk| match chunk {
            StreamChunk::ToolCallStart { id, .. } => Some(id.clone()),
            _ => None,
        });
        let start_id = start_id.expect("tool call start id");
        let (base_id, thought_signature) = split_tool_call_id_and_thought_signature(&start_id);
        assert_eq!(base_id, "call_1");
        assert_eq!(thought_signature.as_deref(), Some("hi"));
    }
}

#[cfg(test)]
mod chat_completions_generate_tests {
    use super::*;

    #[test]
    fn message_reasoning_content_parses_into_reasoning_part() {
        let raw = r#"{
            "id": "resp_1",
            "choices": [{
                "message": {
                    "reasoning_content": "thinking...",
                    "content": "OK"
                }
            }]
        }"#;
        let parsed = serde_json::from_str::<ChatCompletionsResponse>(raw).unwrap();
        let choice = parsed.choices.first().unwrap();

        let mut parts = Vec::<ContentPart>::new();
        if let Some(reasoning) = choice
            .message
            .reasoning_content
            .as_deref()
            .or(choice.message.reasoning.as_deref())
            .filter(|t| !t.is_empty())
        {
            parts.push(ContentPart::Reasoning {
                text: reasoning.to_string(),
            });
        }
        if let Some(text) = choice.message.content.as_deref().filter(|t| !t.is_empty()) {
            parts.push(ContentPart::Text {
                text: text.to_string(),
            });
        }

        assert!(matches!(
            parts.first(),
            Some(ContentPart::Reasoning { text }) if text == "thinking..."
        ));
        assert!(matches!(
            parts.get(1),
            Some(ContentPart::Text { text }) if text == "OK"
        ));
    }

    #[test]
    fn message_tool_call_with_thought_signature_is_encoded_into_id() {
        let raw = r#"{
            "id": "resp_1",
            "choices": [{
                "message": {
                    "tool_calls": [{
                        "id": "call_9",
                        "function": {
                            "name": "workspace",
                            "arguments": "{\"op\":\"help\"}",
                            "thought_signature": "hi"
                        }
                    }]
                }
            }]
        }"#;
        let parsed = serde_json::from_str::<ChatCompletionsResponse>(raw).unwrap();
        let choice = parsed.choices.first().unwrap();
        let tool_call = choice
            .message
            .tool_calls
            .as_ref()
            .and_then(|calls| calls.first())
            .expect("tool call");
        let thought_signature = tool_call
            .function
            .thought_signature
            .as_deref()
            .or(tool_call.thought_signature.as_deref())
            .map(str::trim)
            .filter(|v| !v.is_empty());
        let encoded_id =
            encode_tool_call_id_with_thought_signature(&tool_call.id, thought_signature);
        let (base_id, replayed_signature) = split_tool_call_id_and_thought_signature(&encoded_id);
        assert_eq!(base_id, "call_9");
        assert_eq!(replayed_signature.as_deref(), Some("hi"));
    }
}
