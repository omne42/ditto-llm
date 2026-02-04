#[derive(Debug, Clone)]
struct ResponsesToolCall {
    call_id: String,
    name: String,
    arguments: Value,
}

fn extract_responses_tool_calls(response: &Value) -> Vec<ResponsesToolCall> {
    let mut out = Vec::new();
    let Some(items) = response.get("output").and_then(|v| v.as_array()) else {
        return out;
    };

    for (idx, item) in items.iter().enumerate() {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let item_type = obj
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if item_type != "function_call" {
            continue;
        }

        let call_id = obj
            .get("call_id")
            .or_else(|| obj.get("id"))
            .and_then(|v| v.as_str())
            .map(|v| v.to_string())
            .unwrap_or_else(|| format!("call_{idx}"));
        let name = obj
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if name.trim().is_empty() {
            continue;
        }

        let arguments = obj.get("arguments").cloned().unwrap_or(Value::Null);
        let arguments = match arguments {
            Value::String(text) => serde_json::from_str(&text).unwrap_or(Value::Null),
            other => other,
        };

        out.push(ResponsesToolCall {
            call_id,
            name,
            arguments,
        });
    }

    out
}

async fn execute_mcp_tool_calls(
    state: &GatewayHttpState,
    server_ids: &[String],
    tool_calls: &[ResponsesToolCall],
) -> Vec<String> {
    let mut out = Vec::with_capacity(tool_calls.len());
    for call in tool_calls {
        let result = mcp_call_tool(state, server_ids, &call.name, call.arguments.clone())
            .await
            .unwrap_or_else(|err| Value::String(format!("MCP tool call failed: {err}")));
        out.push(mcp_tool_result_to_text(&result));
    }
    out
}

async fn follow_up_via_chat_completions_to_responses(
    state: &GatewayHttpState,
    parts: &axum::http::request::Parts,
    request_id: &str,
    request_json: &Value,
    tools_for_llm: Vec<Value>,
    tool_calls: &[ResponsesToolCall],
    tool_results: &[String],
) -> Result<axum::response::Response, (StatusCode, Json<OpenAiErrorResponse>)> {
    let original_stream = request_json
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut request_with_tools = request_json.clone();
    set_json_tools(&mut request_with_tools, tools_for_llm);

    let chat_req =
        responses_shim::responses_request_to_chat_completions(&request_with_tools).ok_or_else(
            || {
                openai_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request_error",
                    Some("invalid_mcp_request"),
                    "missing input/messages",
                )
            },
        )?;
    let mut chat_req = chat_req;

    let Some(obj) = chat_req.as_object_mut() else {
        return Err(openai_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            Some("invalid_mcp_request"),
            "invalid /responses request",
        ));
    };

    let Some(messages) = obj.get("messages").and_then(|v| v.as_array()).cloned() else {
        return Err(openai_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            Some("invalid_mcp_request"),
            "missing messages",
        ));
    };

    let mut messages = messages;

    let tool_calls_value = tool_calls
        .iter()
        .map(|call| {
            let args = match &call.arguments {
                Value::Null => "{}".to_string(),
                other => serde_json::to_string(other).unwrap_or_else(|_| "{}".to_string()),
            };
            serde_json::json!({
                "id": call.call_id.clone(),
                "type": "function",
                "function": { "name": call.name.clone(), "arguments": args }
            })
        })
        .collect::<Vec<_>>();

    messages.push(serde_json::json!({
        "role": "assistant",
        "content": "",
        "tool_calls": tool_calls_value,
    }));

    for (call, output) in tool_calls.iter().zip(tool_results.iter()) {
        messages.push(serde_json::json!({
            "role": "tool",
            "tool_call_id": call.call_id.clone(),
            "content": output,
        }));
    }

    obj.insert("messages".to_string(), Value::Array(messages));
    obj.insert("stream".to_string(), Value::Bool(original_stream));

    let response = call_openai_compat_proxy_with_body_and_path(
        state,
        parts,
        request_id,
        &chat_req,
        original_stream,
        "/v1/chat/completions",
    )
    .await?;

    convert_chat_response_to_responses(response, request_id.to_string()).await
}

async fn call_openai_compat_proxy_with_body_and_path(
    state: &GatewayHttpState,
    parts: &axum::http::request::Parts,
    request_id: &str,
    body_json: &Value,
    stream: bool,
    path_and_query: &str,
) -> Result<axum::response::Response, (StatusCode, Json<OpenAiErrorResponse>)> {
    let mut body_json = body_json.clone();
    if let Some(obj) = body_json.as_object_mut() {
        obj.insert("stream".to_string(), Value::Bool(stream));
    }
    let bytes = match serde_json::to_vec(&body_json) {
        Ok(bytes) => Bytes::from(bytes),
        Err(err) => {
            return Err(openai_error(
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                Some("invalid_json"),
                err,
            ));
        }
    };

    let uri = path_and_query.parse::<axum::http::Uri>().map_err(|err| {
        openai_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            Some("invalid_request"),
            format!("invalid proxy uri {path_and_query:?}: {err}"),
        )
    })?;

    let mut headers = parts.headers.clone();
    headers.insert(
        "x-ditto-skip-mcp",
        axum::http::HeaderValue::from_static("1"),
    );
    if let Ok(value) = axum::http::HeaderValue::from_str(request_id) {
        headers.insert("x-request-id", value);
    }
    headers.insert(
        "content-type",
        axum::http::HeaderValue::from_static("application/json"),
    );

    let mut req = axum::http::Request::new(Body::from(bytes));
    *req.method_mut() = parts.method.clone();
    *req.uri_mut() = uri;
    *req.headers_mut() = headers;

    let fut = Box::pin(handle_openai_compat_proxy(
        State(state.clone()),
        Path(String::new()),
        req,
    ));
    fut.await
}

async fn convert_chat_response_to_responses(
    response: axum::response::Response,
    fallback_request_id: String,
) -> Result<axum::response::Response, (StatusCode, Json<OpenAiErrorResponse>)> {
    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if content_type.starts_with("text/event-stream") {
        use tokio_util::io::StreamReader;

        let (mut parts, body) = response.into_parts();
        parts.headers.insert(
            "x-ditto-shim",
            "responses_via_chat_completions".parse().unwrap(),
        );
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

        let stream = responses_shim::chat_completions_sse_to_responses_sse(
            data_stream,
            fallback_request_id,
        );

        let mut response = axum::response::Response::new(Body::from_stream(stream));
        *response.status_mut() = status;
        *response.headers_mut() = parts.headers;
        return Ok(response);
    }

    let (status, mut headers, body) = split_response(response, 8 * 1024 * 1024).await?;
    let value: Value = serde_json::from_slice(&body).map_err(|err| {
        openai_error(
            StatusCode::BAD_GATEWAY,
            "api_error",
            Some("invalid_backend_response"),
            format!("invalid chat/completions response: {err}"),
        )
    })?;
    let mapped = responses_shim::chat_completions_response_to_responses(&value).ok_or_else(|| {
        openai_error(
            StatusCode::BAD_GATEWAY,
            "api_error",
            Some("invalid_backend_response"),
            "chat/completions response cannot be mapped to /responses",
        )
    })?;
    let bytes = serde_json::to_vec(&mapped)
        .map(Bytes::from)
        .unwrap_or_else(|_| Bytes::from(mapped.to_string()));

    headers.insert(
        "x-ditto-shim",
        "responses_via_chat_completions".parse().unwrap(),
    );
    headers.insert("content-type", "application/json".parse().unwrap());
    headers.remove("content-length");

    Ok(rebuild_response(status, headers, bytes))
}

