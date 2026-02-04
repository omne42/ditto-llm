#[derive(Debug, Serialize)]
struct AnthropicErrorDetail {
    #[serde(rename = "type")]
    kind: &'static str,
    message: String,
}

#[derive(Debug, Serialize)]
struct AnthropicErrorResponse {
    #[serde(rename = "type")]
    kind: &'static str,
    error: AnthropicErrorDetail,
}

fn anthropic_error(
    status: StatusCode,
    kind: &'static str,
    message: impl Into<String>,
) -> (StatusCode, Json<AnthropicErrorResponse>) {
    (
        status,
        Json(AnthropicErrorResponse {
            kind: "error",
            error: AnthropicErrorDetail {
                kind,
                message: message.into(),
            },
        }),
    )
}

async fn handle_anthropic_messages(
    State(state): State<GatewayHttpState>,
    req: axum::http::Request<Body>,
) -> Result<axum::response::Response, (StatusCode, Json<AnthropicErrorResponse>)> {
    const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;

    let (parts, body) = req.into_parts();
    let body = to_bytes(body, MAX_BODY_BYTES).await.map_err(|err| {
        anthropic_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            err.to_string(),
        )
    })?;

    let request_json: serde_json::Value = serde_json::from_slice(&body).map_err(|err| {
        anthropic_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            format!("invalid JSON: {err}"),
        )
    })?;

    let openai_request = interop::anthropic_messages_request_to_openai_chat_completions(
        &request_json,
    )
    .map_err(|err| anthropic_error(StatusCode::BAD_REQUEST, "invalid_request_error", err))?;

    let stream_requested = openai_request
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    #[cfg(not(feature = "streaming"))]
    if stream_requested {
        return Err(anthropic_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "streaming is not enabled",
        ));
    }

    let openai_bytes = serde_json::to_vec(&openai_request).map_err(|err| {
        anthropic_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            format!("failed to serialize request: {err}"),
        )
    })?;

    let mut headers = HeaderMap::new();
    headers.insert("content-type", "application/json".parse().unwrap());
    headers.insert("x-ditto-protocol", "anthropic".parse().unwrap());
    if stream_requested {
        headers.insert("accept", "text/event-stream".parse().unwrap());
    }
    if let Some(value) = parts.headers.get("authorization") {
        headers.insert("authorization", value.clone());
    }
    if !headers.contains_key("authorization") {
        if let Some(token) = extract_virtual_key(&parts.headers)
            .as_deref()
            .and_then(synthesize_bearer_header)
        {
            headers.insert("authorization", token);
        }
    }
    if let Some(value) = parts.headers.get("x-request-id") {
        headers.insert("x-request-id", value.clone());
    }

    let mut openai_req = axum::http::Request::builder()
        .method(axum::http::Method::POST)
        .uri("/v1/chat/completions")
        .body(Body::from(openai_bytes))
        .map_err(|err| {
            anthropic_error(
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                err.to_string(),
            )
        })?;
    *openai_req.headers_mut() = headers;

    let openai_resp = handle_openai_compat_proxy(
        State(state.clone()),
        Path("chat/completions".to_string()),
        openai_req,
    )
    .await
    .map_err(|(status, err)| anthropic_error(status, "api_error", err.0.error.message))?;

    let status = openai_resp.status();
    let content_type = openai_resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if content_type.starts_with("text/event-stream") {
        #[cfg(feature = "streaming")]
        {
            use tokio_util::io::StreamReader;

            let (mut parts, body) = openai_resp.into_parts();
            parts
                .headers
                .insert("content-type", "text/event-stream".parse().unwrap());
            parts.headers.remove("content-length");

            let data_stream = body
                .into_data_stream()
                .map(|result| result.map_err(|err| std::io::Error::other(err.to_string())));
            let reader = StreamReader::new(data_stream);
            let reader = tokio::io::BufReader::new(reader);
            let data_stream = crate::utils::sse::sse_data_stream_from_reader(reader);

            let fallback_id =
                extract_header(&parts.headers, "x-request-id").unwrap_or_else(generate_request_id);
            let encoder = Some(interop::AnthropicSseEncoder::new(fallback_id));

            let stream = stream::unfold(
                (
                    data_stream,
                    encoder,
                    VecDeque::<Result<Bytes, std::io::Error>>::new(),
                    false,
                ),
                |(mut data_stream, mut encoder, mut buffer, mut done)| async move {
                    loop {
                        if let Some(item) = buffer.pop_front() {
                            return Some((item, (data_stream, encoder, buffer, done)));
                        }
                        if done {
                            return None;
                        }

                        match data_stream.next().await {
                            Some(Ok(data)) => {
                                let Some(encoder_ref) = encoder.as_mut() else {
                                    done = true;
                                    continue;
                                };
                                match encoder_ref.push_openai_chunk(&data) {
                                    Ok(chunks) => {
                                        for chunk in chunks {
                                            buffer.push_back(Ok(chunk));
                                        }
                                    }
                                    Err(err) => {
                                        done = true;
                                        buffer.push_back(Err(std::io::Error::other(err)));
                                    }
                                }
                            }
                            Some(Err(err)) => {
                                done = true;
                                buffer.push_back(Err(std::io::Error::other(err.to_string())));
                            }
                            None => {
                                if let Some(encoder) = encoder.take() {
                                    for chunk in encoder.finish() {
                                        buffer.push_back(Ok(chunk));
                                    }
                                }
                                done = true;
                            }
                        }
                    }
                },
            );

            let mut response = axum::response::Response::new(Body::from_stream(stream));
            *response.status_mut() = status;
            *response.headers_mut() = parts.headers;
            return Ok(response);
        }
        #[cfg(not(feature = "streaming"))]
        {
            return Err(anthropic_error(
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                "streaming is not enabled",
            ));
        }
    }

    let (openai_parts, openai_body) = openai_resp.into_parts();
    let status = openai_parts.status;
    let request_id_header = openai_parts.headers.get("x-ditto-request-id").cloned();
    let bytes = to_bytes(openai_body, MAX_BODY_BYTES)
        .await
        .unwrap_or_default();

    if !status.is_success() {
        let message = serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|value| {
                value
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(Value::as_str)
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| String::from_utf8_lossy(&bytes).to_string());
        return Err(anthropic_error(status, "api_error", message));
    }

    let openai_json: serde_json::Value = serde_json::from_slice(&bytes).map_err(|err| {
        anthropic_error(
            StatusCode::BAD_GATEWAY,
            "api_error",
            format!("invalid backend JSON: {err}"),
        )
    })?;

    let anthropic_json =
        interop::openai_chat_completions_response_to_anthropic_message(&openai_json)
            .map_err(|err| anthropic_error(StatusCode::BAD_GATEWAY, "api_error", err))?;
    let out_bytes = serde_json::to_vec(&anthropic_json)
        .unwrap_or_else(|_| anthropic_json.to_string().into_bytes());

    let mut response = axum::response::Response::new(Body::from(out_bytes));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert("content-type", "application/json".parse().unwrap());
    if let Some(value) = request_id_header {
        response.headers_mut().insert("x-ditto-request-id", value);
    }
    Ok(response)
}

#[derive(Debug, Serialize)]
struct AnthropicCountTokensResponse {
    input_tokens: u32,
}

async fn handle_anthropic_count_tokens(
    State(state): State<GatewayHttpState>,
    req: axum::http::Request<Body>,
) -> Result<Json<AnthropicCountTokensResponse>, (StatusCode, Json<AnthropicErrorResponse>)> {
    const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

    let (parts, body) = req.into_parts();
    if gateway_uses_virtual_keys(&state).await {
        let token = extract_virtual_key(&parts.headers).ok_or_else(|| {
                anthropic_error(
                    StatusCode::UNAUTHORIZED,
                    "authentication_error",
                    "missing api key",
                )
        })?;
        let gateway = state.gateway.lock().await;
        let authorized = gateway
            .config
            .virtual_keys
            .iter()
            .any(|key| key.enabled && key.token == token);
        if !authorized {
            return Err(anthropic_error(
                StatusCode::UNAUTHORIZED,
                "authentication_error",
                "unauthorized api key",
            ));
        }
    }

    let body = to_bytes(body, MAX_BODY_BYTES).await.map_err(|err| {
        anthropic_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            err.to_string(),
        )
    })?;
    #[cfg(feature = "gateway-tokenizer")]
    let request_json: serde_json::Value = serde_json::from_slice(&body).map_err(|err| {
        anthropic_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            format!("invalid JSON: {err}"),
        )
    })?;

    #[cfg(not(feature = "gateway-tokenizer"))]
    let _request_json: serde_json::Value = serde_json::from_slice(&body).map_err(|err| {
        anthropic_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            format!("invalid JSON: {err}"),
        )
    })?;

    #[cfg(feature = "gateway-tokenizer")]
    let openai_request = interop::anthropic_messages_request_to_openai_chat_completions(
        &request_json,
    )
    .map_err(|err| anthropic_error(StatusCode::BAD_REQUEST, "invalid_request_error", err))?;

    #[cfg(feature = "gateway-tokenizer")]
    let input_tokens = {
        let model = openai_request
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default();
        token_count::estimate_input_tokens("/v1/chat/completions", model, &openai_request)
            .unwrap_or_else(|| estimate_tokens_from_bytes(&body))
    };

    #[cfg(not(feature = "gateway-tokenizer"))]
    let input_tokens = estimate_tokens_from_bytes(&body);

    Ok(Json(AnthropicCountTokensResponse { input_tokens }))
}
