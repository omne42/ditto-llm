async fn handle_google_genai(
    State(state): State<GatewayHttpState>,
    Path(path): Path<String>,
    req: axum::http::Request<Body>,
) -> Result<axum::response::Response, (StatusCode, Json<GoogleApiErrorResponse>)> {
    const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;

    let (model_raw, action) = path
        .rsplit_once(':')
        .ok_or_else(|| google_error(StatusCode::NOT_FOUND, "unsupported endpoint"))?;
    let model = model_raw.trim().trim_start_matches("models/").to_string();
    let stream_requested = action.starts_with("streamGenerateContent");

    #[cfg(not(feature = "streaming"))]
    if stream_requested {
        return Err(google_error(
            StatusCode::BAD_REQUEST,
            "streaming is not enabled",
        ));
    }

    let (parts, body) = req.into_parts();
    let body = to_bytes(body, MAX_BODY_BYTES)
        .await
        .map_err(|err| google_error(StatusCode::BAD_REQUEST, err.to_string()))?;
    let request_json: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|err| google_error(StatusCode::BAD_REQUEST, format!("invalid JSON: {err}")))?;

    let openai_request = interop::google_generate_content_request_to_openai_chat_completions(
        &model,
        &request_json,
        stream_requested,
    )
    .map_err(|err| google_error(StatusCode::BAD_REQUEST, err))?;

    let openai_bytes = serde_json::to_vec(&openai_request).map_err(|err| {
        google_error(
            StatusCode::BAD_REQUEST,
            format!("failed to serialize request: {err}"),
        )
    })?;

    let use_virtual_keys = gateway_uses_virtual_keys(&state).await;

    let mut headers = HeaderMap::new();
    headers.insert("content-type", "application/json".parse().unwrap());
    if stream_requested {
        headers.insert("accept", "text/event-stream".parse().unwrap());
    }
    if let Some(value) = parts.headers.get("authorization") {
        headers.insert("authorization", value.clone());
    }
    if use_virtual_keys && !headers.contains_key("authorization") {
        let token = extract_header(&parts.headers, "x-ditto-virtual-key")
            .or_else(|| extract_header(&parts.headers, "x-goog-api-key"))
            .or_else(|| extract_litellm_api_key(&parts.headers))
            .or_else(|| extract_bearer(&parts.headers));
        if let Some(token) = token.as_deref().and_then(synthesize_bearer_header) {
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
        .map_err(|err| google_error(StatusCode::BAD_REQUEST, err.to_string()))?;
    *openai_req.headers_mut() = headers;

    let openai_resp = handle_openai_compat_proxy(
        State(state.clone()),
        Path("chat/completions".to_string()),
        openai_req,
    )
    .await
    .map_err(|(status, err)| google_error(status, err.0.error.message))?;

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
            let encoder = Some(interop::GoogleSseEncoder::new(fallback_id, false));

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
                                    buffer.push_back(Ok(encoder.finish()));
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
            return Err(google_error(
                StatusCode::BAD_REQUEST,
                "streaming is not enabled",
            ));
        }
    }

    let bytes = to_bytes(openai_resp.into_body(), MAX_BODY_BYTES)
        .await
        .unwrap_or_default();

    if !status.is_success() {
        let message = String::from_utf8_lossy(&bytes).to_string();
        return Err(google_error(status, message));
    }

    let openai_json: serde_json::Value = serde_json::from_slice(&bytes).map_err(|err| {
        google_error(
            StatusCode::BAD_GATEWAY,
            format!("invalid backend JSON: {err}"),
        )
    })?;
    let google_json =
        interop::openai_chat_completions_response_to_google_generate_content(&openai_json)
            .map_err(|err| google_error(StatusCode::BAD_GATEWAY, err))?;
    let out_bytes =
        serde_json::to_vec(&google_json).unwrap_or_else(|_| google_json.to_string().into_bytes());

    let mut response = axum::response::Response::new(Body::from(out_bytes));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert("content-type", "application/json".parse().unwrap());
    Ok(response)
}

async fn handle_fallback(
    State(state): State<GatewayHttpState>,
    req: axum::http::Request<Body>,
) -> axum::response::Response {
    match req.uri().path() {
        "/v1internal:generateContent" => {
            match handle_cloudcode_generate_content_inner(state.clone(), req, false).await {
                Ok(response) => response,
                Err(err) => err.into_response(),
            }
        }
        "/v1internal:streamGenerateContent" => {
            match handle_cloudcode_generate_content_inner(state.clone(), req, true).await {
                Ok(response) => response,
                Err(err) => err.into_response(),
            }
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn handle_cloudcode_generate_content_inner(
    state: GatewayHttpState,
    req: axum::http::Request<Body>,
    stream_requested: bool,
) -> Result<axum::response::Response, (StatusCode, Json<GoogleApiErrorResponse>)> {
    const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;

    #[cfg(not(feature = "streaming"))]
    if stream_requested {
        return Err(google_error(
            StatusCode::BAD_REQUEST,
            "streaming is not enabled",
        ));
    }

    let (parts, body) = req.into_parts();
    let body = to_bytes(body, MAX_BODY_BYTES)
        .await
        .map_err(|err| google_error(StatusCode::BAD_REQUEST, err.to_string()))?;
    let request_json: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|err| google_error(StatusCode::BAD_REQUEST, format!("invalid JSON: {err}")))?;
    let obj = request_json.as_object().ok_or_else(|| {
        google_error(
            StatusCode::BAD_REQUEST,
            "request body must be a JSON object",
        )
    })?;
    let model = obj
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| google_error(StatusCode::BAD_REQUEST, "missing field `model`"))?
        .to_string();
    let inner_request = obj
        .get("request")
        .ok_or_else(|| google_error(StatusCode::BAD_REQUEST, "missing field `request`"))?;

    let openai_request = interop::google_generate_content_request_to_openai_chat_completions(
        &model,
        inner_request,
        stream_requested,
    )
    .map_err(|err| google_error(StatusCode::BAD_REQUEST, err))?;

    let openai_bytes = serde_json::to_vec(&openai_request).map_err(|err| {
        google_error(
            StatusCode::BAD_REQUEST,
            format!("failed to serialize request: {err}"),
        )
    })?;

    let use_virtual_keys = gateway_uses_virtual_keys(&state).await;
    let mut headers = HeaderMap::new();
    headers.insert("content-type", "application/json".parse().unwrap());
    if stream_requested {
        headers.insert("accept", "text/event-stream".parse().unwrap());
    }
    if let Some(value) = parts.headers.get("authorization") {
        headers.insert("authorization", value.clone());
    }
    if use_virtual_keys && !headers.contains_key("authorization") {
        let token = extract_header(&parts.headers, "x-ditto-virtual-key")
            .or_else(|| extract_header(&parts.headers, "x-goog-api-key"))
            .or_else(|| extract_litellm_api_key(&parts.headers))
            .or_else(|| extract_bearer(&parts.headers));
        if let Some(token) = token.as_deref().and_then(synthesize_bearer_header) {
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
        .map_err(|err| google_error(StatusCode::BAD_REQUEST, err.to_string()))?;
    *openai_req.headers_mut() = headers;

    let openai_resp = handle_openai_compat_proxy(
        State(state.clone()),
        Path("chat/completions".to_string()),
        openai_req,
    )
    .await
    .map_err(|(status, err)| google_error(status, err.0.error.message))?;

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
            let encoder = Some(interop::GoogleSseEncoder::new(fallback_id, true));

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
                                    buffer.push_back(Ok(encoder.finish()));
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
            return Err(google_error(
                StatusCode::BAD_REQUEST,
                "streaming is not enabled",
            ));
        }
    }

    let bytes = to_bytes(openai_resp.into_body(), MAX_BODY_BYTES)
        .await
        .unwrap_or_default();

    if !status.is_success() {
        let message = String::from_utf8_lossy(&bytes).to_string();
        return Err(google_error(status, message));
    }

    let openai_json: serde_json::Value = serde_json::from_slice(&bytes).map_err(|err| {
        google_error(
            StatusCode::BAD_GATEWAY,
            format!("invalid backend JSON: {err}"),
        )
    })?;
    let cloudcode_json =
        interop::openai_chat_completions_response_to_cloudcode_generate_content(&openai_json)
            .map_err(|err| google_error(StatusCode::BAD_GATEWAY, err))?;
    let out_bytes = serde_json::to_vec(&cloudcode_json)
        .unwrap_or_else(|_| cloudcode_json.to_string().into_bytes());

    let mut response = axum::response::Response::new(Body::from(out_bytes));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert("content-type", "application/json".parse().unwrap());
    Ok(response)
}
