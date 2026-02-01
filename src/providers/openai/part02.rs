#[derive(Debug, Deserialize)]
struct ResponsesApiResponse {
    id: String,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    incomplete_details: Option<Value>,
    #[serde(default)]
    output: Vec<Value>,
    #[serde(default)]
    usage: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ResponsesStreamEvent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    response: Option<Value>,
    #[serde(default)]
    item: Option<Value>,
    #[serde(default)]
    delta: Option<String>,
}

fn map_responses_finish_reason(
    status: Option<&str>,
    incomplete_reason: Option<&str>,
    has_tool_calls: bool,
) -> FinishReason {
    match status {
        Some("completed") | Some("done") => {
            if has_tool_calls {
                FinishReason::ToolCalls
            } else {
                FinishReason::Stop
            }
        }
        Some("incomplete") => match incomplete_reason {
            Some("max_output_tokens") | Some("max_tokens") => FinishReason::Length,
            Some("content_filter") | Some("content_filtered") => FinishReason::ContentFilter,
            Some("tool_calls") => FinishReason::ToolCalls,
            _ => FinishReason::Length,
        },
        Some("failed") | Some("cancelled") | Some("canceled") | Some("error") => {
            FinishReason::Error
        }
        _ => FinishReason::Unknown,
    }
}

fn finish_reason_for_final_event(
    event_kind: &str,
    response: Option<&Value>,
    has_tool_calls: bool,
) -> FinishReason {
    let response_status = response.and_then(|resp| resp.get("status").and_then(Value::as_str));
    let response_incomplete_reason = response
        .and_then(|resp| resp.get("incomplete_details"))
        .and_then(|details| details.get("reason"))
        .and_then(Value::as_str);

    let status = response_status.or(match event_kind {
        "response.incomplete" => Some("incomplete"),
        "response.completed" | "response.done" => Some("completed"),
        _ => None,
    });

    map_responses_finish_reason(status, response_incomplete_reason, has_tool_calls)
}

fn parse_openai_output(output: &[Value], warnings: &mut Vec<Warning>) -> Vec<ContentPart> {
    let mut content = Vec::<ContentPart>::new();

    for item in output {
        let Some(kind) = item.get("type").and_then(Value::as_str) else {
            continue;
        };
        match kind {
            "message" => {
                let Some(parts) = item.get("content").and_then(Value::as_array) else {
                    continue;
                };
                for part in parts {
                    if part.get("type").and_then(Value::as_str) != Some("output_text") {
                        continue;
                    }
                    let Some(text) = part.get("text").and_then(Value::as_str) else {
                        continue;
                    };
                    if text.is_empty() {
                        continue;
                    }
                    content.push(ContentPart::Text {
                        text: text.to_string(),
                    });
                }
            }
            "function_call" => {
                let Some(call_id) = item.get("call_id").and_then(Value::as_str) else {
                    continue;
                };
                let Some(name) = item.get("name").and_then(Value::as_str) else {
                    continue;
                };
                let arguments_raw = item.get("arguments").and_then(Value::as_str).unwrap_or("");
                let raw = arguments_raw.trim();
                let raw_json = if raw.is_empty() { "{}" } else { raw };
                let arguments = serde_json::from_str::<Value>(raw_json).unwrap_or_else(|err| {
                    warnings.push(Warning::Compatibility {
                        feature: "tool_call.arguments".to_string(),
                        details: format!(
                            "failed to parse tool_call arguments as JSON for id={call_id}: {err}; preserving raw string"
                        ),
                    });
                    Value::String(arguments_raw.to_string())
                });
                content.push(ContentPart::ToolCall {
                    id: call_id.to_string(),
                    name: name.to_string(),
                    arguments,
                });
            }
            _ => {}
        }
    }

    content
}

#[async_trait]
impl LanguageModel for OpenAI {
    fn provider(&self) -> &str {
        "openai"
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
        let (instructions, input, mut warnings) = Self::messages_to_input(&request.messages);

        if request.stop_sequences.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "stop_sequences".to_string(),
                details: Some("OpenAI Responses API stop sequences are not supported".to_string()),
            });
        }

        let mut body = Map::<String, Value>::new();
        body.insert("model".to_string(), Value::String(model.to_string()));
        if let Some(instructions) = instructions {
            body.insert("instructions".to_string(), Value::String(instructions));
        }
        body.insert("input".to_string(), Value::Array(input));
        body.insert("stream".to_string(), Value::Bool(false));
        body.insert("store".to_string(), Value::Bool(false));

        if let Some(temperature) = request.temperature {
            if let Some(value) = crate::utils::params::clamped_number_from_f32(
                "temperature",
                temperature,
                0.0,
                2.0,
                &mut warnings,
            ) {
                body.insert("temperature".to_string(), Value::Number(value));
            }
        }
        if let Some(max_tokens) = request.max_tokens {
            body.insert(
                "max_output_tokens".to_string(),
                Value::Number(max_tokens.into()),
            );
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

        if let Some(tools) = request.tools {
            if cfg!(feature = "tools") {
                let mapped = tools
                    .into_iter()
                    .map(|t| Self::tool_to_openai(&t))
                    .collect();
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
                body.insert(
                    "tool_choice".to_string(),
                    Self::tool_choice_to_openai(&tool_choice),
                );
            } else {
                warnings.push(Warning::Unsupported {
                    feature: "tool_choice".to_string(),
                    details: Some("ditto-llm built without tools feature".to_string()),
                });
            }
        }

        apply_provider_options(&mut body, &provider_options)?;
        crate::types::merge_provider_options_into_body(
            &mut body,
            selected_provider_options.as_ref(),
            &["reasoning_effort", "response_format", "parallel_tool_calls"],
            "generate.provider_options",
            &mut warnings,
        );

        let url = self.responses_url();
        let req = self.client.http.post(url);
        let response = self.apply_auth(req).json(&body).send().await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(DittoError::Api { status, body: text });
        }

        let parsed = response.json::<ResponsesApiResponse>().await?;
        let content = parse_openai_output(&parsed.output, &mut warnings);
        let has_tool_calls = content
            .iter()
            .any(|part| matches!(part, ContentPart::ToolCall { .. }));
        let usage = parsed
            .usage
            .as_ref()
            .map(Self::parse_usage)
            .unwrap_or_default();
        let finish_reason = map_responses_finish_reason(
            parsed.status.as_deref(),
            parsed
                .incomplete_details
                .as_ref()
                .and_then(|details| details.get("reason"))
                .and_then(Value::as_str),
            has_tool_calls,
        );

        Ok(GenerateResponse {
            content,
            finish_reason,
            usage,
            warnings,
            provider_metadata: Some(serde_json::json!({ "id": parsed.id })),
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
            let (instructions, input, mut warnings) = Self::messages_to_input(&request.messages);

            let mut body = Map::<String, Value>::new();
            body.insert("model".to_string(), Value::String(model.to_string()));
            if let Some(instructions) = instructions {
                body.insert("instructions".to_string(), Value::String(instructions));
            }
            body.insert("input".to_string(), Value::Array(input));
            body.insert("stream".to_string(), Value::Bool(true));
            body.insert("store".to_string(), Value::Bool(false));

            if request.stop_sequences.is_some() {
                warnings.push(Warning::Unsupported {
                    feature: "stop_sequences".to_string(),
                    details: Some(
                        "OpenAI Responses API stop sequences are not supported".to_string(),
                    ),
                });
            }

            if let Some(temperature) = request.temperature {
                if let Some(value) = crate::utils::params::clamped_number_from_f32(
                    "temperature",
                    temperature,
                    0.0,
                    2.0,
                    &mut warnings,
                ) {
                    body.insert("temperature".to_string(), Value::Number(value));
                }
            }
            if let Some(max_tokens) = request.max_tokens {
                body.insert(
                    "max_output_tokens".to_string(),
                    Value::Number(max_tokens.into()),
                );
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

            if let Some(tools) = request.tools {
                if cfg!(feature = "tools") {
                    let mapped = tools
                        .into_iter()
                        .map(|t| Self::tool_to_openai(&t))
                        .collect();
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
                    body.insert(
                        "tool_choice".to_string(),
                        Self::tool_choice_to_openai(&tool_choice),
                    );
                } else {
                    warnings.push(Warning::Unsupported {
                        feature: "tool_choice".to_string(),
                        details: Some("ditto-llm built without tools feature".to_string()),
                    });
                }
            }

            apply_provider_options(&mut body, &provider_options)?;
            crate::types::merge_provider_options_into_body(
                &mut body,
                selected_provider_options.as_ref(),
                &["reasoning_effort", "response_format", "parallel_tool_calls"],
                "stream.provider_options",
                &mut warnings,
            );

            let url = self.responses_url();
            let req = self.client.http.post(url);
            let response = self
                .apply_auth(req)
                .header("Accept", "text/event-stream")
                .json(&body)
                .send()
                .await?;

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
                (data_stream, buffer, false, false, None::<String>),
                |(mut data_stream, mut buffer, mut done, mut has_tool_calls, mut response_id)| async move {
                    loop {
                        if let Some(item) = buffer.pop_front() {
                            return Some((
                                item,
                                (data_stream, buffer, done, has_tool_calls, response_id),
                            ));
                        }

                        if done {
                            return None;
                        }

                        let next = data_stream.next().await;
                        match next {
                            Some(Ok(data)) => {
                                match serde_json::from_str::<ResponsesStreamEvent>(&data) {
                                    Ok(event) => match event.kind.as_str() {
                                        "response.created" => {
                                            if response_id.is_none() {
                                                if let Some(id) = event
                                                    .response
                                                    .as_ref()
                                                    .and_then(|resp| {
                                                        resp.get("id").and_then(Value::as_str)
                                                    })
                                                    .filter(|id| !id.trim().is_empty())
                                                {
                                                    response_id = Some(id.to_string());
                                                    buffer.push_back(Ok(StreamChunk::ResponseId {
                                                        id: id.to_string(),
                                                    }));
                                                }
                                            }
                                        }
                                        "response.output_text.delta" => {
                                            if let Some(delta) = event.delta {
                                                buffer.push_back(Ok(StreamChunk::TextDelta {
                                                    text: delta,
                                                }));
                                            }
                                        }
                                        "response.reasoning_text.delta" => {
                                            if let Some(delta) = event.delta {
                                                buffer.push_back(Ok(StreamChunk::ReasoningDelta {
                                                    text: delta,
                                                }));
                                            }
                                        }
                                        "response.output_item.done" => {
                                            let Some(item) = event.item else {
                                                continue;
                                            };
                                            if item.get("type").and_then(Value::as_str)
                                                != Some("function_call")
                                            {
                                                continue;
                                            }
                                            has_tool_calls = true;
                                            let Some(call_id) =
                                                item.get("call_id").and_then(Value::as_str)
                                            else {
                                                continue;
                                            };
                                            let Some(name) =
                                                item.get("name").and_then(Value::as_str)
                                            else {
                                                continue;
                                            };
                                            let arguments = item
                                                .get("arguments")
                                                .and_then(Value::as_str)
                                                .unwrap_or("")
                                                .to_string();
                                            buffer.push_back(Ok(StreamChunk::ToolCallStart {
                                                id: call_id.to_string(),
                                                name: name.to_string(),
                                            }));
                                            buffer.push_back(Ok(StreamChunk::ToolCallDelta {
                                                id: call_id.to_string(),
                                                arguments_delta: arguments,
                                            }));
                                        }
                                        "response.failed" => {
                                            done = true;
                                            buffer.push_back(Err(DittoError::InvalidResponse(
                                                event
                                                    .response
                                                    .map(|v| v.to_string())
                                                    .unwrap_or_else(|| {
                                                        "openai response.failed".to_string()
                                                    }),
                                            )));
                                        }
                                        "response.completed"
                                        | "response.done"
                                        | "response.incomplete" => {
                                            done = true;
                                            if let Some(resp) = event.response {
                                                if response_id.is_none() {
                                                    if let Some(id) =
                                                        resp.get("id").and_then(Value::as_str)
                                                    {
                                                        if !id.trim().is_empty() {
                                                            response_id = Some(id.to_string());
                                                            buffer.push_back(Ok(
                                                                StreamChunk::ResponseId {
                                                                    id: id.to_string(),
                                                                },
                                                            ));
                                                        }
                                                    }
                                                }
                                                if let Some(usage) = resp.get("usage") {
                                                    buffer.push_back(Ok(StreamChunk::Usage(
                                                        Self::parse_usage(usage),
                                                    )));
                                                }
                                                let finish_reason = finish_reason_for_final_event(
                                                    &event.kind,
                                                    Some(&resp),
                                                    has_tool_calls,
                                                );
                                                buffer.push_back(Ok(StreamChunk::FinishReason(
                                                    finish_reason,
                                                )));
                                            } else {
                                                let finish_reason = finish_reason_for_final_event(
                                                    &event.kind,
                                                    None,
                                                    has_tool_calls,
                                                );
                                                buffer.push_back(Ok(StreamChunk::FinishReason(
                                                    finish_reason,
                                                )));
                                            }
                                        }
                                        _ => {}
                                    },
                                    Err(err) => {
                                        done = true;
                                        buffer.push_back(Err(err.into()));
                                    }
                                }
                            }
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
