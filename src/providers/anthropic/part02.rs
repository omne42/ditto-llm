#[derive(Debug, Deserialize)]
struct MessagesApiResponse {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    content: Vec<Value>,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    usage: Option<Value>,
}

fn parse_anthropic_content(blocks: &[Value]) -> Vec<ContentPart> {
    let mut out = Vec::<ContentPart>::new();
    for block in blocks {
        let Some(kind) = block.get("type").and_then(Value::as_str) else {
            continue;
        };
        match kind {
            "text" => {
                let Some(text) = block.get("text").and_then(Value::as_str) else {
                    continue;
                };
                if !text.is_empty() {
                    out.push(ContentPart::Text {
                        text: text.to_string(),
                    });
                }
            }
            "tool_use" => {
                let Some(id) = block.get("id").and_then(Value::as_str) else {
                    continue;
                };
                let Some(name) = block.get("name").and_then(Value::as_str) else {
                    continue;
                };
                let arguments = block.get("input").cloned().unwrap_or(Value::Null);
                out.push(ContentPart::ToolCall {
                    id: id.to_string(),
                    name: name.to_string(),
                    arguments,
                });
            }
            "thinking" => {
                let Some(thinking) = block.get("thinking").and_then(Value::as_str) else {
                    continue;
                };
                if !thinking.is_empty() {
                    out.push(ContentPart::Reasoning {
                        text: thinking.to_string(),
                    });
                }
            }
            _ => {}
        }
    }
    out
}

#[async_trait]
impl LanguageModel for Anthropic {
    fn provider(&self) -> &str {
        "anthropic"
    }

    fn model_id(&self) -> &str {
        self.default_model.as_str()
    }

    async fn generate(&self, request: GenerateRequest) -> Result<GenerateResponse> {
        let model = self.resolve_model(&request)?;
        let selected_provider_options = request.provider_options_value_for(self.provider())?;
        let provider_options = selected_provider_options
            .as_ref()
            .map(crate::types::ProviderOptions::from_value)
            .transpose()?
            .unwrap_or_default();

        let mut warnings = Vec::<Warning>::new();
        if provider_options.reasoning_effort.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "reasoning_effort".to_string(),
                details: Some(
                    "Anthropic Messages API does not support reasoning_effort".to_string(),
                ),
            });
        }
        if provider_options.response_format.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "response_format".to_string(),
                details: Some(
                    "Anthropic Messages API does not support response_format".to_string(),
                ),
            });
        }
        if provider_options.parallel_tool_calls == Some(true) {
            warnings.push(Warning::Unsupported {
                feature: "parallel_tool_calls".to_string(),
                details: Some(
                    "Anthropic Messages API does not support parallel_tool_calls".to_string(),
                ),
            });
        }
        let tool_names = Self::build_tool_name_map(&request.messages);

        let mut system = Vec::<String>::new();
        let mut saw_non_system = false;
        let mut messages = Vec::<Value>::new();

        for message in &request.messages {
            if message.role == Role::System && !saw_non_system {
                if let Some(text) = Self::extract_system_text(message, &mut warnings) {
                    system.push(text);
                }
                continue;
            }
            saw_non_system = true;

            if let Some((role, content)) =
                Self::message_to_anthropic_blocks(message, &tool_names, &mut warnings)
            {
                messages.push(serde_json::json!({ "role": role, "content": content }));
            }
        }

        let mut body = Map::<String, Value>::new();
        body.insert("model".to_string(), Value::String(model.to_string()));
        body.insert("messages".to_string(), Value::Array(messages));
        body.insert(
            "max_tokens".to_string(),
            Value::Number(request.max_tokens.unwrap_or(1024).into()),
        );
        body.insert("stream".to_string(), Value::Bool(false));

        if !system.is_empty() {
            body.insert("system".to_string(), Value::String(system.join("\n\n")));
        }

        if let Some(temperature) = request.temperature {
            if let Some(value) = crate::utils::params::clamped_number_from_f32(
                "temperature",
                temperature,
                0.0,
                1.0,
                &mut warnings,
            ) {
                body.insert("temperature".to_string(), Value::Number(value));
            }
        }
        if let Some(top_p) = request.top_p {
            if let Some(value) = crate::utils::params::clamped_number_from_f32(
                "top_p",
                top_p,
                0.0,
                1.0,
                &mut warnings,
            ) {
                body.insert("top_p".to_string(), Value::Number(value));
            }
        }
        if let Some(stop_sequences) = request.stop_sequences {
            let stop_sequences = crate::utils::params::sanitize_stop_sequences(
                &stop_sequences,
                Some(4),
                &mut warnings,
            );
            if !stop_sequences.is_empty() {
                body.insert(
                    "stop_sequences".to_string(),
                    Value::Array(stop_sequences.into_iter().map(Value::String).collect()),
                );
            }
        }

        if let Some(tools) = request.tools {
            if cfg!(feature = "tools") {
                let mapped = tools
                    .into_iter()
                    .map(|tool| Self::tool_to_anthropic(&tool, &mut warnings))
                    .collect::<Vec<_>>();
                body.insert("tools".to_string(), Value::Array(mapped));
            } else {
                warnings.push(Warning::Unsupported {
                    feature: "tools".to_string(),
                    details: Some("ditto-llm built without tools feature".to_string()),
                });
            }
        }

        if let Some(tool_choice) = request.tool_choice {
            if cfg!(feature = "tools") {
                if tool_choice == ToolChoice::None {
                    body.remove("tools");
                } else if let Some(mapped) = Self::tool_choice_to_anthropic(&tool_choice) {
                    body.insert("tool_choice".to_string(), mapped);
                }
            } else {
                warnings.push(Warning::Unsupported {
                    feature: "tool_choice".to_string(),
                    details: Some("ditto-llm built without tools feature".to_string()),
                });
            }
        }

        crate::types::merge_provider_options_into_body(
            &mut body,
            selected_provider_options.as_ref(),
            &["reasoning_effort", "response_format", "parallel_tool_calls"],
            "generate.provider_options",
            &mut warnings,
        );

        let url = self.messages_url();
        let mut request_builder = self
            .http
            .post(url)
            .header("anthropic-version", &self.version);
        request_builder = self.apply_auth(request_builder);
        let betas = Self::required_betas(&request.messages);
        if !betas.is_empty() {
            request_builder = request_builder.header("anthropic-beta", betas.join(","));
        }

        let response = request_builder.json(&body).send().await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(DittoError::Api { status, body: text });
        }

        let parsed = response.json::<MessagesApiResponse>().await?;
        let content = parse_anthropic_content(&parsed.content);
        let finish_reason = Self::stop_reason_to_finish_reason(parsed.stop_reason.as_deref());
        let usage = parsed
            .usage
            .as_ref()
            .map(Self::parse_usage)
            .unwrap_or_default();

        Ok(GenerateResponse {
            content,
            finish_reason,
            usage,
            warnings,
            provider_metadata: parsed.id.map(|id| serde_json::json!({ "id": id })),
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

            let mut warnings = Vec::<Warning>::new();
            if provider_options.reasoning_effort.is_some() {
                warnings.push(Warning::Unsupported {
                    feature: "reasoning_effort".to_string(),
                    details: Some(
                        "Anthropic Messages API does not support reasoning_effort".to_string(),
                    ),
                });
            }
            if provider_options.response_format.is_some() {
                warnings.push(Warning::Unsupported {
                    feature: "response_format".to_string(),
                    details: Some(
                        "Anthropic Messages API does not support response_format".to_string(),
                    ),
                });
            }
            if provider_options.parallel_tool_calls == Some(true) {
                warnings.push(Warning::Unsupported {
                    feature: "parallel_tool_calls".to_string(),
                    details: Some(
                        "Anthropic Messages API does not support parallel_tool_calls".to_string(),
                    ),
                });
            }
            let tool_names = Self::build_tool_name_map(&request.messages);

            let mut system = Vec::<String>::new();
            let mut saw_non_system = false;
            let mut messages = Vec::<Value>::new();

            for message in &request.messages {
                if message.role == Role::System && !saw_non_system {
                    if let Some(text) = Self::extract_system_text(message, &mut warnings) {
                        system.push(text);
                    }
                    continue;
                }
                saw_non_system = true;

                if let Some((role, content)) =
                    Self::message_to_anthropic_blocks(message, &tool_names, &mut warnings)
                {
                    messages.push(serde_json::json!({ "role": role, "content": content }));
                }
            }

            let mut body = Map::<String, Value>::new();
            body.insert("model".to_string(), Value::String(model.to_string()));
            body.insert("messages".to_string(), Value::Array(messages));
            body.insert(
                "max_tokens".to_string(),
                Value::Number(request.max_tokens.unwrap_or(1024).into()),
            );
            body.insert("stream".to_string(), Value::Bool(true));

            if !system.is_empty() {
                body.insert("system".to_string(), Value::String(system.join("\n\n")));
            }

            if let Some(temperature) = request.temperature {
                if let Some(value) = crate::utils::params::clamped_number_from_f32(
                    "temperature",
                    temperature,
                    0.0,
                    1.0,
                    &mut warnings,
                ) {
                    body.insert("temperature".to_string(), Value::Number(value));
                }
            }
            if let Some(top_p) = request.top_p {
                if let Some(value) = crate::utils::params::clamped_number_from_f32(
                    "top_p",
                    top_p,
                    0.0,
                    1.0,
                    &mut warnings,
                ) {
                    body.insert("top_p".to_string(), Value::Number(value));
                }
            }
            if let Some(stop_sequences) = request.stop_sequences {
                let stop_sequences = crate::utils::params::sanitize_stop_sequences(
                    &stop_sequences,
                    Some(4),
                    &mut warnings,
                );
                if !stop_sequences.is_empty() {
                    body.insert(
                        "stop_sequences".to_string(),
                        Value::Array(stop_sequences.into_iter().map(Value::String).collect()),
                    );
                }
            }

            if let Some(tools) = request.tools {
                if cfg!(feature = "tools") {
                    let mapped = tools
                        .into_iter()
                        .map(|tool| Self::tool_to_anthropic(&tool, &mut warnings))
                        .collect::<Vec<_>>();
                    body.insert("tools".to_string(), Value::Array(mapped));
                }
            }

            if let Some(tool_choice) = request.tool_choice {
                if cfg!(feature = "tools") {
                    if tool_choice == ToolChoice::None {
                        body.remove("tools");
                    } else if let Some(mapped) = Self::tool_choice_to_anthropic(&tool_choice) {
                        body.insert("tool_choice".to_string(), mapped);
                    }
                }
            }

            crate::types::merge_provider_options_into_body(
                &mut body,
                selected_provider_options.as_ref(),
                &["reasoning_effort", "response_format", "parallel_tool_calls"],
                "stream.provider_options",
                &mut warnings,
            );

            let url = self.messages_url();
            let mut request_builder = self
                .http
                .post(url)
                .header("anthropic-version", &self.version)
                .header("Accept", "text/event-stream");
            request_builder = self.apply_auth(request_builder);
            let betas = Self::required_betas(&request.messages);
            if !betas.is_empty() {
                request_builder = request_builder.header("anthropic-beta", betas.join(","));
            }

            let response = request_builder.json(&body).send().await?;

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

            #[derive(Debug, Deserialize)]
            struct StreamEvent {
                #[serde(rename = "type")]
                kind: String,
                #[serde(default)]
                index: Option<usize>,
                #[serde(default)]
                delta: Option<Value>,
                #[serde(default)]
                content_block: Option<Value>,
                #[serde(default)]
                message: Option<Value>,
                #[serde(default)]
                usage: Option<Value>,
            }

            let stream = stream::unfold(
                (
                    data_stream,
                    buffer,
                    false,
                    HashMap::<usize, (String, String)>::new(),
                    None::<Usage>,
                    None::<FinishReason>,
                ),
                |(
                    mut data_stream,
                    mut buffer,
                    mut done,
                    mut tool_calls,
                    mut pending_usage,
                    mut pending_finish,
                )| async move {
                    loop {
                        if let Some(item) = buffer.pop_front() {
                            return Some((
                                item,
                                (
                                    data_stream,
                                    buffer,
                                    done,
                                    tool_calls,
                                    pending_usage,
                                    pending_finish,
                                ),
                            ));
                        }

                        if done {
                            return None;
                        }

                        let next = data_stream.next().await;
                        match next {
                            Some(Ok(data)) => match serde_json::from_str::<StreamEvent>(&data) {
                                Ok(event) => match event.kind.as_str() {
                                    "content_block_start" => {
                                        let Some(index) = event.index else { continue };
                                        let Some(block) = event.content_block else {
                                            continue;
                                        };
                                        let Some(block_type) =
                                            block.get("type").and_then(Value::as_str)
                                        else {
                                            continue;
                                        };
                                        if block_type == "tool_use" {
                                            let Some(id) = block.get("id").and_then(Value::as_str)
                                            else {
                                                continue;
                                            };
                                            let Some(name) =
                                                block.get("name").and_then(Value::as_str)
                                            else {
                                                continue;
                                            };
                                            tool_calls
                                                .insert(index, (id.to_string(), name.to_string()));
                                            buffer.push_back(Ok(StreamChunk::ToolCallStart {
                                                id: id.to_string(),
                                                name: name.to_string(),
                                            }));
                                            if let Some(input) = block.get("input") {
                                                buffer.push_back(Ok(StreamChunk::ToolCallDelta {
                                                    id: id.to_string(),
                                                    arguments_delta: input.to_string(),
                                                }));
                                            }
                                        }
                                    }
                                    "content_block_delta" => {
                                        let Some(index) = event.index else { continue };
                                        let Some(delta) = event.delta else { continue };
                                        let Some(delta_type) =
                                            delta.get("type").and_then(Value::as_str)
                                        else {
                                            continue;
                                        };
                                        match delta_type {
                                            "text_delta" => {
                                                if let Some(text) =
                                                    delta.get("text").and_then(Value::as_str)
                                                {
                                                    buffer.push_back(Ok(StreamChunk::TextDelta {
                                                        text: text.to_string(),
                                                    }));
                                                }
                                            }
                                            "thinking_delta" => {
                                                if let Some(thinking) =
                                                    delta.get("thinking").and_then(Value::as_str)
                                                {
                                                    buffer.push_back(Ok(
                                                        StreamChunk::ReasoningDelta {
                                                            text: thinking.to_string(),
                                                        },
                                                    ));
                                                }
                                            }
                                            "input_json_delta" => {
                                                let Some((tool_call_id, _name)) =
                                                    tool_calls.get(&index)
                                                else {
                                                    continue;
                                                };
                                                if let Some(partial) = delta
                                                    .get("partial_json")
                                                    .and_then(Value::as_str)
                                                {
                                                    buffer.push_back(Ok(
                                                        StreamChunk::ToolCallDelta {
                                                            id: tool_call_id.clone(),
                                                            arguments_delta: partial.to_string(),
                                                        },
                                                    ));
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                    "message_delta" => {
                                        if let Some(usage) = event.usage.as_ref() {
                                            pending_usage = Some(Self::parse_usage(usage));
                                        }
                                        if let Some(message) =
                                            event.message.as_ref().or(event.delta.as_ref())
                                        {
                                            if let Some(stop_reason) =
                                                message.get("stop_reason").and_then(Value::as_str)
                                            {
                                                pending_finish =
                                                    Some(Self::stop_reason_to_finish_reason(Some(
                                                        stop_reason,
                                                    )));
                                            }
                                        }
                                        if let Some(delta) = event.delta.as_ref() {
                                            if let Some(stop_reason) =
                                                delta.get("stop_reason").and_then(Value::as_str)
                                            {
                                                pending_finish =
                                                    Some(Self::stop_reason_to_finish_reason(Some(
                                                        stop_reason,
                                                    )));
                                            }
                                        }
                                    }
                                    "message_stop" => {
                                        done = true;
                                        if let Some(usage) = pending_usage.take() {
                                            buffer.push_back(Ok(StreamChunk::Usage(usage)));
                                        }
                                        buffer.push_back(Ok(StreamChunk::FinishReason(
                                            pending_finish.take().unwrap_or(FinishReason::Stop),
                                        )));
                                    }
                                    "error" => {
                                        done = true;
                                        buffer.push_back(Err(DittoError::InvalidResponse(data)));
                                    }
                                    _ => {}
                                },
                                Err(err) => {
                                    done = true;
                                    buffer.push_back(Err(err.into()));
                                }
                            },
                            Some(Err(err)) => {
                                done = true;
                                buffer.push_back(Err(err));
                            }
                            None => return None,
                        }
                    }
                },
            );

            Ok(Box::pin(stream))
        }
    }
}

