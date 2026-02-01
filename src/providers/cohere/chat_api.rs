#[derive(Debug, Deserialize, Default)]
struct CohereChatResponse {
    #[serde(default)]
    id: String,
    #[serde(default)]
    finish_reason: Option<String>,
    #[serde(default)]
    message: CohereAssistantMessage,
    #[serde(default)]
    usage: Option<Value>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct CohereAssistantMessage {
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    content: Vec<CohereContentBlock>,
    #[serde(default)]
    tool_plan: Option<String>,
    #[serde(default)]
    tool_calls: Vec<CohereToolCall>,
    #[serde(default)]
    citations: Option<Value>,
}

#[derive(Debug, Deserialize, Default)]
struct CohereContentBlock {
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct CohereToolCall {
    #[serde(default)]
    id: String,
    #[serde(default)]
    function: CohereToolFunction,
}

#[derive(Debug, Deserialize, Default)]
struct CohereToolFunction {
    #[serde(default)]
    name: String,
    #[serde(default)]
    arguments: String,
}

#[async_trait]
impl LanguageModel for Cohere {
    fn provider(&self) -> &str {
        "cohere"
    }

    fn model_id(&self) -> &str {
        self.default_model.as_str()
    }

    async fn generate(&self, request: GenerateRequest) -> Result<GenerateResponse> {
        let model = self.resolve_model(&request)?;
        let selected_provider_options = request.provider_options_value_for(self.provider())?;

        let (messages, mut warnings) = Self::messages_to_cohere_messages(&request.messages);
        crate::types::warn_unsupported_generate_request_options(
            "Cohere Chat API",
            &request,
            crate::types::GenerateRequestSupport::NONE,
            &mut warnings,
        );

        let mut body = serde_json::Map::<String, Value>::new();
        body.insert("model".to_string(), Value::String(model.to_string()));
        body.insert("messages".to_string(), Value::Array(messages));

        if let Some(temperature) = request.temperature {
            if let Some(value) = Self::sanitize_temperature(temperature, &mut warnings) {
                body.insert("temperature".to_string(), Value::Number(value));
            }
        }
        if let Some(max_tokens) = request.max_tokens {
            body.insert("max_tokens".to_string(), Value::Number(max_tokens.into()));
        }
        if let Some(top_p) = request.top_p {
            if let Some(value) = crate::utils::params::clamped_number_from_f32(
                "top_p",
                top_p,
                0.01,
                0.99,
                &mut warnings,
            ) {
                body.insert("p".to_string(), Value::Number(value));
            }
        }
        if let Some(stops) = request.stop_sequences.as_ref() {
            let stops = crate::utils::params::sanitize_stop_sequences(stops, None, &mut warnings);
            if !stops.is_empty() {
                body.insert(
                    "stop_sequences".to_string(),
                    Value::Array(stops.into_iter().map(Value::String).collect()),
                );
            }
        }

        let mut tools = request.tools.unwrap_or_default();
        if !tools.is_empty() {
            if cfg!(feature = "tools") {
                let tool_choice = request.tool_choice.unwrap_or(ToolChoice::Auto);
                let (mapped_choice, filtered_tools) =
                    Self::normalize_tool_choice(&tool_choice, Some(&tools), &mut warnings);
                if let Some(filtered) = filtered_tools {
                    tools = filtered;
                }

                let mapped = tools
                    .iter()
                    .map(|t| Self::tool_to_cohere(t, &mut warnings))
                    .collect::<Vec<_>>();
                body.insert("tools".to_string(), Value::Array(mapped));
                if let Some(choice) = mapped_choice {
                    body.insert("tool_choice".to_string(), choice);
                }
            } else {
                warnings.push(Warning::Unsupported {
                    feature: "tools".to_string(),
                    details: Some("ditto-llm built without tools feature".to_string()),
                });
            }
        } else if let Some(tool_choice) = request.tool_choice {
            warnings.push(Warning::Unsupported {
                feature: "tool_choice".to_string(),
                details: Some(format!(
                    "cohere requires tools to be provided when tool_choice is set (got {tool_choice:?})"
                )),
            });
        }

        crate::types::merge_provider_options_into_body(
            &mut body,
            selected_provider_options.as_ref(),
            &["reasoning_effort", "response_format", "parallel_tool_calls"],
            "cohere.provider_options",
            &mut warnings,
        );

        let url = self.chat_url();
        let req = self.http.post(url);
        let parsed = crate::utils::http::send_checked_json::<CohereChatResponse>(
            self.apply_auth(req).json(&body),
        )
        .await?;

        let mut content = Vec::<ContentPart>::new();
        for block in &parsed.message.content {
            if block.kind.as_deref() != Some("text") {
                continue;
            }
            let Some(text) = block.text.as_deref().filter(|t| !t.is_empty()) else {
                continue;
            };
            content.push(ContentPart::Text {
                text: text.to_string(),
            });
        }

        if let Some(plan) = parsed
            .message
            .tool_plan
            .as_deref()
            .filter(|t| !t.trim().is_empty())
        {
            content.push(ContentPart::Reasoning {
                text: plan.to_string(),
            });
        }

        for call in &parsed.message.tool_calls {
            let id = call.id.trim();
            let name = call.function.name.trim();
            if id.is_empty() || name.is_empty() {
                warnings.push(Warning::Compatibility {
                    feature: "tool_call".to_string(),
                    details: "cohere response tool_call missing id or name; dropping tool call"
                        .to_string(),
                });
                continue;
            }

            let arguments_raw = call.function.arguments.as_str();
            let arguments = crate::types::parse_tool_call_arguments_json_or_string(
                arguments_raw,
                &format!("id={id}"),
                &mut warnings,
            );

            content.push(ContentPart::ToolCall {
                id: id.to_string(),
                name: name.to_string(),
                arguments,
            });
        }

        let finish_reason = Self::map_finish_reason(
            parsed.finish_reason.as_deref(),
            !parsed.message.tool_calls.is_empty(),
        );

        let usage = parsed
            .usage
            .as_ref()
            .map(Self::parse_usage)
            .unwrap_or_default();

        let mut provider_metadata = serde_json::Map::<String, Value>::new();
        provider_metadata.insert("id".to_string(), Value::String(parsed.id.clone()));
        if let Some(model) = parsed.model.as_deref() {
            provider_metadata.insert("model".to_string(), Value::String(model.to_string()));
        }
        if let Some(role) = parsed.message.role.as_deref() {
            provider_metadata.insert("role".to_string(), Value::String(role.to_string()));
        }
        if let Some(citations) = parsed.message.citations.clone() {
            provider_metadata.insert("citations".to_string(), citations);
        }

        Ok(GenerateResponse {
            content,
            finish_reason,
            usage,
            warnings,
            provider_metadata: Some(Value::Object(provider_metadata)),
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

            let (messages, mut warnings) = Self::messages_to_cohere_messages(&request.messages);
            crate::types::warn_unsupported_generate_request_options(
                "Cohere Chat API",
                &request,
                crate::types::GenerateRequestSupport::NONE,
                &mut warnings,
            );

            let mut body = serde_json::Map::<String, Value>::new();
            body.insert("model".to_string(), Value::String(model.to_string()));
            body.insert("messages".to_string(), Value::Array(messages));
            body.insert("stream".to_string(), Value::Bool(true));

            if let Some(temperature) = request.temperature {
                if let Some(value) = Self::sanitize_temperature(temperature, &mut warnings) {
                    body.insert("temperature".to_string(), Value::Number(value));
                }
            }
            if let Some(max_tokens) = request.max_tokens {
                body.insert("max_tokens".to_string(), Value::Number(max_tokens.into()));
            }
            if let Some(top_p) = request.top_p {
                if let Some(value) = crate::utils::params::clamped_number_from_f32(
                    "top_p",
                    top_p,
                    0.01,
                    0.99,
                    &mut warnings,
                ) {
                    body.insert("p".to_string(), Value::Number(value));
                }
            }
            if let Some(stops) = request.stop_sequences.as_ref() {
                let stops =
                    crate::utils::params::sanitize_stop_sequences(stops, None, &mut warnings);
                if !stops.is_empty() {
                    body.insert(
                        "stop_sequences".to_string(),
                        Value::Array(stops.into_iter().map(Value::String).collect()),
                    );
                }
            }

            let mut tools = request.tools.unwrap_or_default();
            if !tools.is_empty() {
                if cfg!(feature = "tools") {
                    let tool_choice = request.tool_choice.unwrap_or(ToolChoice::Auto);
                    let (mapped_choice, filtered_tools) =
                        Self::normalize_tool_choice(&tool_choice, Some(&tools), &mut warnings);
                    if let Some(filtered) = filtered_tools {
                        tools = filtered;
                    }

                    let mapped = tools
                        .iter()
                        .map(|t| Self::tool_to_cohere(t, &mut warnings))
                        .collect::<Vec<_>>();
                    body.insert("tools".to_string(), Value::Array(mapped));
                    if let Some(choice) = mapped_choice {
                        body.insert("tool_choice".to_string(), choice);
                    }
                } else {
                    warnings.push(Warning::Unsupported {
                        feature: "tools".to_string(),
                        details: Some("ditto-llm built without tools feature".to_string()),
                    });
                }
            } else if request.tool_choice.is_some() {
                warnings.push(Warning::Unsupported {
                    feature: "tool_choice".to_string(),
                    details: Some(
                        "cohere requires tools to be provided when tool_choice is set".to_string(),
                    ),
                });
            }

            crate::types::merge_provider_options_into_body(
                &mut body,
                selected_provider_options.as_ref(),
                &["reasoning_effort", "response_format", "parallel_tool_calls"],
                "cohere.provider_options",
                &mut warnings,
            );

            let url = self.chat_url();
            let req = self.http.post(url);
            let response = crate::utils::http::send_checked(
                self.apply_auth(req)
                    .header("Accept", "text/event-stream")
                    .json(&body),
            )
            .await?;

            let (data_stream, buffer) =
                crate::utils::streaming::init_sse_stream(response, warnings);

            let stream = stream::unfold(
                (
                    data_stream,
                    buffer,
                    false,
                    false,
                    false,
                    false,
                    Vec::<String>::new(),
                    HashMap::<String, String>::new(),
                ),
                |(
                    mut data_stream,
                    mut buffer,
                    mut done,
                    mut has_tool_calls,
                    mut id_sent,
                    mut finish_sent,
                    mut tool_order,
                    mut tool_args,
                )| async move {
                    loop {
                        if let Some(item) = buffer.pop_front() {
                            return Some((
                                item,
                                (
                                    data_stream,
                                    buffer,
                                    done,
                                    has_tool_calls,
                                    id_sent,
                                    finish_sent,
                                    tool_order,
                                    tool_args,
                                ),
                            ));
                        }

                        if done {
                            return None;
                        }

                        let next = data_stream.next().await;
                        match next {
                            Some(Ok(data)) => {
                                let event = match serde_json::from_str::<Value>(&data) {
                                    Ok(event) => event,
                                    Err(err) => {
                                        buffer.push_back(Err(err.into()));
                                        continue;
                                    }
                                };
                                let kind = event.get("type").and_then(Value::as_str).unwrap_or("");

                                match kind {
                                    "message-start" => {
                                        if !id_sent {
                                            if let Some(id) = event
                                                .get("delta")
                                                .and_then(|v| v.get("message"))
                                                .and_then(|v| v.get("id"))
                                                .and_then(Value::as_str)
                                                .filter(|id| !id.trim().is_empty())
                                            {
                                                id_sent = true;
                                                buffer.push_back(Ok(StreamChunk::ResponseId {
                                                    id: id.to_string(),
                                                }));
                                            }
                                        }
                                    }
                                    "content-delta" => {
                                        if let Some(text) = event
                                            .get("delta")
                                            .and_then(|v| v.get("message"))
                                            .and_then(|v| v.get("content"))
                                            .and_then(|v| v.get("text"))
                                            .and_then(Value::as_str)
                                        {
                                            if !text.is_empty() {
                                                buffer.push_back(Ok(StreamChunk::TextDelta {
                                                    text: text.to_string(),
                                                }));
                                            }
                                        }
                                    }
                                    "tool-plan-delta" => {
                                        if let Some(text) = event
                                            .get("delta")
                                            .and_then(|v| v.get("tool_plan"))
                                            .and_then(Value::as_str)
                                        {
                                            if !text.is_empty() {
                                                buffer.push_back(Ok(StreamChunk::ReasoningDelta {
                                                    text: text.to_string(),
                                                }));
                                            }
                                        }
                                    }
                                    "tool-call-start" => {
                                        has_tool_calls = true;
                                        let tool_call =
                                            event.get("delta").and_then(|v| v.get("tool_call"));
                                        let Some(id) = tool_call
                                            .and_then(|v| v.get("id"))
                                            .and_then(Value::as_str)
                                            .filter(|id| !id.trim().is_empty())
                                        else {
                                            continue;
                                        };
                                        let name = tool_call
                                            .and_then(|v| v.get("function"))
                                            .and_then(|v| v.get("name"))
                                            .and_then(Value::as_str)
                                            .unwrap_or("")
                                            .to_string();

                                        if !name.trim().is_empty() {
                                            buffer.push_back(Ok(StreamChunk::ToolCallStart {
                                                id: id.to_string(),
                                                name,
                                            }));
                                        }

                                        let arguments = tool_call
                                            .and_then(|v| v.get("function"))
                                            .and_then(|v| v.get("arguments"))
                                            .and_then(Value::as_str)
                                            .unwrap_or("")
                                            .to_string();
                                        if !tool_args.contains_key(id) {
                                            tool_order.push(id.to_string());
                                        }
                                        tool_args.insert(id.to_string(), arguments);
                                    }
                                    "tool-call-delta" => {
                                        has_tool_calls = true;
                                        let tool_call =
                                            event.get("delta").and_then(|v| v.get("tool_call"));
                                        let Some(id) = tool_call
                                            .and_then(|v| v.get("id"))
                                            .and_then(Value::as_str)
                                            .filter(|id| !id.trim().is_empty())
                                        else {
                                            continue;
                                        };
                                        let arguments = tool_call
                                            .and_then(|v| v.get("function"))
                                            .and_then(|v| v.get("arguments"))
                                            .and_then(Value::as_str)
                                            .unwrap_or("")
                                            .to_string();
                                        if !tool_args.contains_key(id) {
                                            tool_order.push(id.to_string());
                                        }
                                        tool_args.insert(id.to_string(), arguments);
                                    }
                                    "message-end" => {
                                        let reason = event
                                            .get("delta")
                                            .and_then(|v| v.get("finish_reason"))
                                            .and_then(Value::as_str);

                                        for tool_call_id in std::mem::take(&mut tool_order) {
                                            let Some(arguments) = tool_args.remove(&tool_call_id)
                                            else {
                                                continue;
                                            };
                                            if arguments.is_empty() {
                                                continue;
                                            }
                                            buffer.push_back(Ok(StreamChunk::ToolCallDelta {
                                                id: tool_call_id,
                                                arguments_delta: arguments,
                                            }));
                                        }

                                        let finish_reason =
                                            Cohere::map_finish_reason(reason, has_tool_calls);
                                        buffer.push_back(Ok(StreamChunk::FinishReason(
                                            finish_reason,
                                        )));
                                        finish_sent = true;

                                        if let Some(usage) =
                                            event.get("delta").and_then(|v| v.get("usage"))
                                        {
                                            buffer.push_back(Ok(StreamChunk::Usage(
                                                Cohere::parse_usage(usage),
                                            )));
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            Some(Err(err)) => {
                                buffer.push_back(Err(err));
                            }
                            None => {
                                if !finish_sent {
                                    for tool_call_id in std::mem::take(&mut tool_order) {
                                        let Some(arguments) = tool_args.remove(&tool_call_id)
                                        else {
                                            continue;
                                        };
                                        if arguments.is_empty() {
                                            continue;
                                        }
                                        buffer.push_back(Ok(StreamChunk::ToolCallDelta {
                                            id: tool_call_id,
                                            arguments_delta: arguments,
                                        }));
                                    }
                                    buffer.push_back(Ok(StreamChunk::FinishReason(
                                        if has_tool_calls {
                                            FinishReason::ToolCalls
                                        } else {
                                            FinishReason::Unknown
                                        },
                                    )));
                                    finish_sent = true;
                                }
                                done = true;
                            }
                        }
                    }
                },
            )
            .boxed();

            Ok(Box::pin(stream))
        }
    }
}
