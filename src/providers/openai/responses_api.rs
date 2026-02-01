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
                let context = format!("id={call_id}");
                let arguments = crate::types::parse_tool_call_arguments_json_or_string(
                    arguments_raw,
                    &context,
                    warnings,
                );
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
        let (body, mut warnings) = Self::build_responses_body(
            &request,
            model,
            &provider_options,
            selected_provider_options.as_ref(),
            false,
            "generate.provider_options",
        )?;

        let url = self.responses_url();
        let req = self.client.http.post(url);
        let parsed = crate::utils::http::send_checked_json::<ResponsesApiResponse>(
            self.apply_auth(req).json(&body),
        )
        .await?;
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
            let (body, warnings) = Self::build_responses_body(
                &request,
                model,
                &provider_options,
                selected_provider_options.as_ref(),
                true,
                "stream.provider_options",
            )?;

            let url = self.responses_url();
            let req = self.client.http.post(url);
            let response = crate::utils::http::send_checked(
                self.apply_auth(req)
                    .header("Accept", "text/event-stream")
                    .json(&body),
            )
            .await?;

            let (data_stream, buffer) =
                crate::utils::streaming::init_sse_stream(response, warnings);

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
