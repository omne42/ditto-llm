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
    #[serde(default)]
    function: ChatToolFunction,
}

#[derive(Debug, Deserialize, Default)]
struct ChatToolFunction {
    #[serde(default)]
    name: String,
    #[serde(default)]
    arguments: String,
}

#[cfg(feature = "streaming")]
#[derive(Debug, Deserialize, Default)]
struct ChatCompletionsChunk {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    choices: Vec<ChatChoiceChunk>,
    #[serde(default)]
    usage: Option<Value>,
}

#[cfg(feature = "streaming")]
#[derive(Debug, Deserialize, Default)]
struct ChatChoiceChunk {
    #[serde(default)]
    delta: ChatDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[cfg(feature = "streaming")]
#[derive(Debug, Deserialize, Default)]
struct ChatDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ChatToolCallDelta>>,
    #[serde(default)]
    function_call: Option<ChatFunctionCallDelta>,
}

#[cfg(feature = "streaming")]
#[derive(Debug, Deserialize, Default)]
struct ChatFunctionCallDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[cfg(feature = "streaming")]
#[derive(Debug, Deserialize)]
struct ChatToolCallDelta {
    #[serde(default)]
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<ChatToolFunctionDelta>,
}

#[cfg(feature = "streaming")]
#[derive(Debug, Deserialize, Default)]
struct ChatToolFunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[cfg(feature = "streaming")]
#[derive(Debug, Default)]
struct StreamToolCallState {
    id: Option<String>,
    name: Option<String>,
    started: bool,
    pending_arguments: String,
}

#[cfg(feature = "streaming")]
#[derive(Debug, Default)]
struct StreamState {
    response_id: Option<String>,
    tool_calls: Vec<StreamToolCallState>,
}

#[cfg(feature = "streaming")]
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

#[cfg(feature = "streaming")]
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
            if let Some(function) = tool_call.function.as_ref() {
                if let Some(name) = function.name.as_deref().filter(|v| !v.trim().is_empty()) {
                    slot.name = Some(name.to_string());
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
        let selected_provider_options = request.provider_options_value_for(self.provider())?;
        let provider_options = selected_provider_options
            .as_ref()
            .map(crate::types::ProviderOptions::from_value)
            .transpose()?
            .unwrap_or_default();
        let (body, mut warnings) = Self::build_chat_completions_body(
            &request,
            model,
            &provider_options,
            selected_provider_options.as_ref(),
            false,
            "generate.provider_options",
        )?;

        let url = self.chat_completions_url();
        let mut req = self.client.http.post(url);
        req = self.apply_auth(req);
        let response = req.json(&body).send().await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(DittoError::Api { status, body: text });
        }

        let parsed = response.json::<ChatCompletionsResponse>().await?;
        let choice = parsed.choices.first().ok_or_else(|| {
            DittoError::InvalidResponse("chat/completions response has no choices".to_string())
        })?;

        let mut content = Vec::<ContentPart>::new();
        if let Some(text) = choice.message.content.as_deref().filter(|t| !t.is_empty()) {
            content.push(ContentPart::Text {
                text: text.to_string(),
            });
        }
        match choice.message.tool_calls.as_ref() {
            Some(tool_calls) if !tool_calls.is_empty() => {
                for tool_call in tool_calls {
                    let arguments_raw = tool_call.function.arguments.as_str();
                    let raw = arguments_raw.trim();
                    let raw_json = if raw.is_empty() { "{}" } else { raw };
                    let arguments = serde_json::from_str::<Value>(raw_json).unwrap_or_else(|err| {
                        warnings.push(Warning::Compatibility {
                            feature: "tool_call.arguments".to_string(),
                            details: format!(
                                "failed to parse tool_call arguments as JSON for id={}: {err}; preserving raw string",
                                tool_call.id
                            ),
                        });
                        Value::String(arguments_raw.to_string())
                    });
                    content.push(ContentPart::ToolCall {
                        id: tool_call.id.clone(),
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
                        let raw = arguments_raw.trim();
                        let raw_json = if raw.is_empty() { "{}" } else { raw };
                        let arguments = serde_json::from_str::<Value>(raw_json).unwrap_or_else(|err| {
                    warnings.push(Warning::Compatibility {
                        feature: "tool_call.arguments".to_string(),
                        details: format!(
                            "failed to parse function_call arguments as JSON for name={name}: {err}; preserving raw string",
                        ),
                    });
                    Value::String(arguments_raw.to_string())
                });
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
        #[cfg(not(feature = "streaming"))]
        {
            let _ = request;
            return Err(DittoError::InvalidResponse(
                "ditto-llm built without streaming feature".to_string(),
            ));
        }

        #[cfg(feature = "streaming")]
        {
            let model = self.resolve_model(&request)?;
            let selected_provider_options = request.provider_options_value_for(self.provider())?;
            let provider_options = selected_provider_options
                .as_ref()
                .map(crate::types::ProviderOptions::from_value)
                .transpose()?
                .unwrap_or_default();
            let (body, warnings) = Self::build_chat_completions_body(
                &request,
                model,
                &provider_options,
                selected_provider_options.as_ref(),
                true,
                "stream.provider_options",
            )?;

            let url = self.chat_completions_url();
            let req = self
                .client
                .http
                .post(url)
                .header("Accept", "text/event-stream")
                .json(&body);
            let response = self.apply_auth(req).send().await?;

            let status = response.status();
            if !status.is_success() {
                let text = response.text().await.unwrap_or_default();
                return Err(DittoError::Api { status, body: text });
            }

            let data_stream = crate::utils::sse::sse_data_stream_from_response(response);
            let mut buffer = VecDeque::<Result<StreamChunk>>::new();
            if !warnings.is_empty() {
                buffer.push_back(Ok(StreamChunk::Warnings { warnings }));
            }
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
                                Ok((chunks, is_done)) => {
                                    for chunk in chunks {
                                        buffer.push_back(Ok(chunk));
                                    }
                                    if is_done {
                                        done = true;
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
                },
            );

            Ok(Box::pin(stream))
        }
    }
}
