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

#[async_trait]
impl LanguageModel for Bedrock {
    fn provider(&self) -> &str {
        "bedrock"
    }

    fn model_id(&self) -> &str {
        &self.default_model
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
        crate::types::warn_unsupported_provider_options(
            "Bedrock Anthropic",
            &provider_options,
            crate::types::ProviderOptionsSupport::NONE,
            &mut warnings,
        );
        crate::types::warn_unsupported_generate_request_options(
            "Bedrock Anthropic",
            &request,
            crate::types::GenerateRequestSupport::NONE,
            &mut warnings,
        );

        let body = Self::build_bedrock_body(&request, model, &mut warnings)?;
        let url = self.build_url_with_query(&self.invoke_url(model))?;
        let response = self.post_json(&url, &body, None).await?;

        let parsed = response.json::<MessagesApiResponse>().await?;
        let content = Self::parse_anthropic_content(&parsed.content);
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
            crate::types::warn_unsupported_provider_options(
                "Bedrock Anthropic",
                &provider_options,
                crate::types::ProviderOptionsSupport::NONE,
                &mut warnings,
            );
            crate::types::warn_unsupported_generate_request_options(
                "Bedrock Anthropic",
                &request,
                crate::types::GenerateRequestSupport::NONE,
                &mut warnings,
            );

            let body = Self::build_bedrock_body(&request, model, &mut warnings)?;
            let url = self.build_url_with_query(&self.invoke_stream_url(model))?;
            let response = self
                .post_json(&url, &body, Some("application/vnd.amazon.eventstream"))
                .await?;

            let data_stream = Box::pin(bedrock_event_stream_from_response(response));
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

                        let next = data_stream.as_mut().next().await;
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
