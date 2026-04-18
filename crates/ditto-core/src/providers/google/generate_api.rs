#[derive(Debug, Deserialize)]
struct GoogleGenerateResponse {
    #[serde(default)]
    candidates: Vec<Value>,
    #[serde(default)]
    usage_metadata: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[cfg(feature = "cap-llm-streaming")]
struct GoogleApiFailureEnvelope {
    error: GoogleApiFailureBody,
}

#[derive(Debug, Deserialize)]
#[cfg(feature = "cap-llm-streaming")]
struct GoogleApiFailureBody {
    #[serde(default)]
    message: String,
    #[serde(default, rename = "type")]
    error_type: String,
    #[serde(default)]
    code: String,
}

#[derive(Debug)]
struct PreparedGoogleRequest {
    model: String,
    body: Map<String, Value>,
    warnings: Vec<Warning>,
}

#[cfg(feature = "cap-llm-streaming")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GoogleStreamFallbackReason {
    EmptyResponse,
}

fn parse_google_candidate(
    candidate: &Value,
    tool_call_seq: &mut u64,
    has_tool_calls: &mut bool,
) -> Vec<ContentPart> {
    genai::parse_google_candidate(candidate, tool_call_seq, has_tool_calls)
}

#[cfg(feature = "cap-llm-streaming")]
// YUNWU_GEMINI_STREAM_FALLBACK:
// Yunwu's Gemini native streaming endpoint can fail before emitting any deltas
// with an upstream "empty_response" style error. Treat that as a transport-path
// failure, not as a final model answer failure: retry once via non-streaming
// generate() and re-wrap the response as stream chunks so upstream callers can
// stay on the streaming interface.
fn classify_google_stream_fallback(err: &DittoError) -> Option<GoogleStreamFallbackReason> {
    match err {
        DittoError::Api { body, .. } => {
            let parsed = serde_json::from_str::<GoogleApiFailureEnvelope>(body).ok()?;
            let code = parsed.error.code.trim();
            if code == "channel:empty_response" {
                return Some(GoogleStreamFallbackReason::EmptyResponse);
            }

            let error_type = parsed.error.error_type.trim();
            let message = parsed.error.message.to_ascii_lowercase();
            if error_type == "channel_error"
                && message.contains("no meaningful content in candidates")
            {
                return Some(GoogleStreamFallbackReason::EmptyResponse);
            }
            None
        }
        _ => None,
    }
}

#[cfg(feature = "cap-llm-streaming")]
fn usage_has_token_counts(usage: &Usage) -> bool {
    usage.input_tokens.is_some()
        || usage.cache_input_tokens.is_some()
        || usage.cache_creation_input_tokens.is_some()
        || usage.output_tokens.is_some()
        || usage.total_tokens.is_some()
}

#[cfg(feature = "cap-llm-streaming")]
fn stream_chunks_from_generate_response(
    response: GenerateResponse,
) -> Vec<Result<crate::contracts::StreamChunk>> {
    let mut out = Vec::<Result<crate::contracts::StreamChunk>>::new();
    if !response.warnings.is_empty() {
        out.push(Ok(crate::contracts::StreamChunk::Warnings {
            warnings: response.warnings,
        }));
    }

    for part in response.content {
        match part {
            ContentPart::Text { text } => {
                if !text.is_empty() {
                    out.push(Ok(crate::contracts::StreamChunk::TextDelta { text }));
                }
            }
            ContentPart::Reasoning { text } => {
                if !text.is_empty() {
                    out.push(Ok(crate::contracts::StreamChunk::ReasoningDelta { text }));
                }
            }
            ContentPart::ToolCall {
                id,
                name,
                arguments,
            } => {
                out.push(Ok(crate::contracts::StreamChunk::ToolCallStart {
                    id: id.clone(),
                    name,
                }));
                out.push(Ok(crate::contracts::StreamChunk::ToolCallDelta {
                    id,
                    arguments_delta: arguments.to_string(),
                }));
            }
            _ => {}
        }
    }

    if usage_has_token_counts(&response.usage) {
        out.push(Ok(crate::contracts::StreamChunk::Usage(response.usage)));
    }
    out.push(Ok(crate::contracts::StreamChunk::FinishReason(
        response.finish_reason,
    )));
    out
}

impl Google {
    fn build_generate_request_body(
        &self,
        request: &GenerateRequest,
        provider_options_scope: &'static str,
    ) -> Result<PreparedGoogleRequest> {
        let model = self.resolve_model(request)?.to_string();
        let selected_provider_options =
            crate::provider_options::request_provider_options_value_for(request, self.provider())?;
        let provider_options = selected_provider_options
            .as_ref()
            .map(crate::provider_options::ProviderOptions::from_value_ref)
            .transpose()?
            .unwrap_or_default();

        let mut warnings = Vec::<Warning>::new();
        crate::provider_options::warn_unsupported_provider_options(
            "Google GenAI",
            &provider_options,
            crate::provider_options::ProviderOptionsSupport::NONE,
            &mut warnings,
        );
        crate::types::warn_unsupported_generate_request_options(
            "Google GenAI",
            request,
            crate::types::GenerateRequestSupport::NONE,
            &mut warnings,
        );
        let tool_names = Self::build_tool_name_map(&request.messages);
        let (contents, system_instruction) =
            Self::convert_messages(&model, &request.messages, &tool_names, &mut warnings)?;

        let mut body = Map::<String, Value>::new();
        body.insert("contents".to_string(), Value::Array(contents));

        if let Some(system_instruction) = system_instruction {
            body.insert("systemInstruction".to_string(), system_instruction);
        }

        let mut generation_config = Map::<String, Value>::new();
        if let Some(max_tokens) = request.max_tokens {
            generation_config.insert(
                "maxOutputTokens".to_string(),
                Value::Number(max_tokens.into()),
            );
        }
        if let Some(temperature) = request.temperature
            && let Some(value) = crate::utils::params::clamped_number_from_f32(
                "temperature",
                temperature,
                0.0,
                2.0,
                &mut warnings,
            ) {
                generation_config.insert("temperature".to_string(), Value::Number(value));
            }
        if let Some(top_p) = request.top_p
            && let Some(value) = crate::utils::params::clamped_number_from_f32(
                "top_p",
                top_p,
                0.0,
                1.0,
                &mut warnings,
            ) {
                generation_config.insert("topP".to_string(), Value::Number(value));
            }
        if let Some(stop_sequences) = request.stop_sequences.as_ref() {
            let stop_sequences =
                crate::utils::params::sanitize_stop_sequences(stop_sequences, None, &mut warnings);
            if !stop_sequences.is_empty() {
                generation_config.insert(
                    "stopSequences".to_string(),
                    Value::Array(stop_sequences.into_iter().map(Value::String).collect()),
                );
            }
        }
        if !generation_config.is_empty() {
            body.insert(
                "generationConfig".to_string(),
                Value::Object(generation_config),
            );
        }

        if let Some(tools) = request.tools.as_ref() {
            if cfg!(feature = "cap-llm-tools") {
                let decls = tools
                    .iter()
                    .cloned()
                    .map(|tool| Self::tool_to_google(tool, &mut warnings))
                    .collect::<Vec<_>>();
                body.insert(
                    "tools".to_string(),
                    Value::Array(vec![serde_json::json!({ "functionDeclarations": decls })]),
                );
            } else {
                warnings.push(Warning::Unsupported {
                    feature: "tools".to_string(),
                    details: Some("ditto-core built without tools feature".to_string()),
                });
            }
        }

        if let Some(tool_choice) = request.tool_choice.as_ref()
            && cfg!(feature = "cap-llm-tools")
                && let Some(tool_config) = Self::tool_config(Some(tool_choice)) {
                    body.insert("toolConfig".to_string(), tool_config);
                }

        crate::provider_options::merge_provider_options_into_body(
            &mut body,
            selected_provider_options.as_ref(),
            &["reasoning_effort", "response_format", "parallel_tool_calls"],
            provider_options_scope,
            &mut warnings,
        );

        Ok(PreparedGoogleRequest {
            model,
            body,
            warnings,
        })
    }
}

#[async_trait]
impl LanguageModel for Google {
    fn provider(&self) -> &str {
        "google"
    }

    fn model_id(&self) -> &str {
        self.default_model.as_str()
    }

    async fn generate(&self, request: GenerateRequest) -> Result<GenerateResponse> {
        let PreparedGoogleRequest {
            model,
            body,
            warnings,
        } = self.build_generate_request_body(&request, "generate.provider_options")?;

        let url = self.generate_url(&model);
        let req = self.http.post(url);
        let parsed = crate::provider_transport::send_checked_json::<GoogleGenerateResponse>(
            self.apply_auth(req).json(&body),
        )
        .await?;
        let mut tool_call_seq = 0u64;
        let mut has_tool_calls = false;
        let mut content = Vec::<ContentPart>::new();

        let finish_reason_str = parsed
            .candidates
            .first()
            .and_then(|c| c.get("finishReason"))
            .and_then(Value::as_str);

        if let Some(candidate) = parsed.candidates.first() {
            content.extend(parse_google_candidate(
                candidate,
                &mut tool_call_seq,
                &mut has_tool_calls,
            ));
        }

        let usage = parsed
            .usage_metadata
            .as_ref()
            .map(Self::parse_usage_metadata)
            .unwrap_or_default();

        let finish_reason = Self::map_finish_reason(finish_reason_str, has_tool_calls);

        Ok(GenerateResponse {
            content,
            finish_reason,
            usage,
            warnings,
            provider_metadata: None,
        })
    }

    async fn stream(&self, request: GenerateRequest) -> Result<StreamResult> {
        #[cfg(not(feature = "cap-llm-streaming"))]
        {
            let _ = request;
            Err(crate::error::DittoError::builder_capability_feature_missing(
                "google",
                "streaming",
            ))
        }

        #[cfg(feature = "cap-llm-streaming")]
        {
            let PreparedGoogleRequest {
                model,
                body,
                warnings,
            } = self.build_generate_request_body(&request, "stream.provider_options")?;
            let url = self.stream_url(&model);
            let req = self.http.post(url);
            let response = match crate::provider_transport::send_checked(
                self.apply_auth(req)
                    .header("Accept", "text/event-stream")
                    .json(&body),
            )
            .await
            {
                Ok(response) => response,
                // YUNWU_GEMINI_STREAM_FALLBACK:
                // Keep the public surface streaming-first, but switch to plain
                // generate() when Yunwu returns the known empty-response stream error.
                Err(err)
                    if matches!(
                        classify_google_stream_fallback(&err),
                        Some(GoogleStreamFallbackReason::EmptyResponse)
                    ) =>
                {
                    let mut generated = self.generate(request).await?;
                    generated.warnings.push(Warning::Compatibility {
                        feature: "stream.empty_response_error".to_string(),
                        details:
                            "streaming failed before emitting output; fell back to non-streaming generate after upstream returned an empty-response error"
                                .to_string(),
                    });
                    return Ok(Box::pin(stream::iter(
                        stream_chunks_from_generate_response(generated),
                    )));
                }
                Err(err) => return Err(err),
            };

            let (data_stream, buffer) =
                crate::session_transport::init_sse_stream(response, warnings);

            let stream = stream::unfold(
                (
                    data_stream,
                    buffer,
                    false,
                    String::new(),
                    false,
                    None::<String>,
                    None::<Usage>,
                    0u64,
                ),
                |(
                    mut data_stream,
                    mut buffer,
                    mut done,
                    mut last_text,
                    mut has_tool_calls,
                    mut pending_finish_reason,
                    mut pending_usage,
                    mut tool_call_seq,
                )| async move {
                    loop {
                        if let Some(item) = buffer.pop_front() {
                            return Some((
                                item,
                                (
                                    data_stream,
                                    buffer,
                                    done,
                                    last_text,
                                    has_tool_calls,
                                    pending_finish_reason,
                                    pending_usage,
                                    tool_call_seq,
                                ),
                            ));
                        }

                        if done {
                            return None;
                        }

                        let next = data_stream.next().await;
                        match next {
                            Some(Ok(data)) => match serde_json::from_str::<Value>(&data) {
                                Ok(chunk) => {
                                    if let Some(usage) = chunk.get("usageMetadata") {
                                        pending_usage = Some(Self::parse_usage_metadata(usage));
                                    }
                                    if let Some(finish) = chunk
                                        .get("candidates")
                                        .and_then(Value::as_array)
                                        .and_then(|c| c.first())
                                        .and_then(|c| c.get("finishReason"))
                                        .and_then(Value::as_str)
                                    {
                                        pending_finish_reason = Some(finish.to_string());
                                    }

                                    if let Some(candidate) = chunk
                                        .get("candidates")
                                        .and_then(Value::as_array)
                                        .and_then(|c| c.first())
                                    {
                                        let parts = candidate
                                            .get("content")
                                            .and_then(|c| c.get("parts"))
                                            .and_then(Value::as_array)
                                            .cloned()
                                            .unwrap_or_default();

                                        for part in parts {
                                            if let Some(text) =
                                                part.get("text").and_then(Value::as_str)
                                            {
                                                let delta = if text.starts_with(&last_text) {
                                                    text[last_text.len()..].to_string()
                                                } else {
                                                    text.to_string()
                                                };
                                                last_text = text.to_string();
                                                if !delta.is_empty() {
                                                    buffer.push_back(Ok(StreamChunk::TextDelta {
                                                        text: delta,
                                                    }));
                                                }
                                                continue;
                                            }
                                            if let Some(call) = part.get("functionCall") {
                                                let Some(name) =
                                                    call.get("name").and_then(Value::as_str)
                                                else {
                                                    continue;
                                                };
                                                let args = call
                                                    .get("args")
                                                    .cloned()
                                                    .unwrap_or(Value::Null);
                                                let thought_signature =
                                                    genai::extract_google_part_thought_signature(
                                                        &part, call,
                                                    );
                                                let id = genai::build_google_tool_call_id(
                                                    tool_call_seq,
                                                    thought_signature,
                                                );
                                                tool_call_seq = tool_call_seq.saturating_add(1);
                                                has_tool_calls = true;
                                                buffer.push_back(Ok(StreamChunk::ToolCallStart {
                                                    id: id.clone(),
                                                    name: name.to_string(),
                                                }));
                                                buffer.push_back(Ok(StreamChunk::ToolCallDelta {
                                                    id,
                                                    arguments_delta: args.to_string(),
                                                }));
                                            }
                                        }
                                    }
                                }
                                Err(err) => {
                                    done = true;
                                    buffer.push_back(Err(err.into()));
                                }
                            },
                            Some(Err(err)) => {
                                done = true;
                                buffer.push_back(Err(err));
                            }
                            None => {
                                done = true;
                                if let Some(usage) = pending_usage.take() {
                                    buffer.push_back(Ok(StreamChunk::Usage(usage)));
                                }
                                buffer.push_back(Ok(StreamChunk::FinishReason(
                                    Self::map_finish_reason(
                                        pending_finish_reason.as_deref(),
                                        has_tool_calls,
                                    ),
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
