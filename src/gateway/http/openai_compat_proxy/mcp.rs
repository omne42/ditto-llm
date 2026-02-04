async fn maybe_handle_mcp_tools_chat_completions(
    state: &GatewayHttpState,
    parts: &axum::http::request::Parts,
    parsed_json: &Option<Value>,
    request_id: &str,
    path_and_query: &str,
) -> Result<Option<axum::response::Response>, (StatusCode, Json<OpenAiErrorResponse>)> {
    if parts.headers.contains_key("x-ditto-skip-mcp") {
        return Ok(None);
    }
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);

    match path {
        "/v1/chat/completions" => {
            maybe_handle_mcp_tools_chat_completions_impl(state, parts, parsed_json, request_id)
                .await
        }
        "/v1/responses" => maybe_handle_mcp_tools_responses(state, parts, parsed_json, request_id).await,
        _ => Ok(None),
    }
}

async fn maybe_handle_mcp_tools_chat_completions_impl(
    state: &GatewayHttpState,
    parts: &axum::http::request::Parts,
    parsed_json: &Option<Value>,
    request_id: &str,
) -> Result<Option<axum::response::Response>, (StatusCode, Json<OpenAiErrorResponse>)> {
    let Some(request_json) = parsed_json.as_ref() else {
        return Ok(None);
    };

    let tools = request_json.get("tools").and_then(|v| v.as_array());
    let Some(tools) = tools else {
        return Ok(None);
    };

    let (mcp_tool_cfgs, other_tools) = split_mcp_tool_configs(tools)?;
    if mcp_tool_cfgs.is_empty() {
        return Ok(None);
    }

    let original_stream = request_json
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let auto_execute = mcp_tool_cfgs
        .iter()
        .all(|cfg| cfg.require_approval.as_deref() == Some("never"));
    let max_steps = resolve_mcp_max_steps(&mcp_tool_cfgs)?;

    let requested_servers = resolve_mcp_servers_from_tool_cfgs(&mcp_tool_cfgs);
    let server_ids = match requested_servers {
        Some(servers) => servers,
        None => {
            let mut all: Vec<String> = state.mcp_servers.keys().cloned().collect();
            all.sort();
            all
        }
    };

    if server_ids.is_empty() {
        return Err(openai_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            Some("invalid_mcp_server"),
            "no MCP servers selected",
        ));
    }

    let mcp_tools_value = mcp_list_tools(state, &server_ids, None)
        .await
        .map_err(map_openai_gateway_error)?;
    let mut mcp_tools = mcp_tools_value
        .get("tools")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let allowed_tools = collect_allowed_tools(&mcp_tool_cfgs);
    if !allowed_tools.is_empty() {
        mcp_tools.retain(|tool| tool_name_allowed(tool, &allowed_tools));
    }

    let openai_tools: Vec<Value> = mcp_tools
        .iter()
        .filter_map(mcp_tool_to_openai_tool)
        .collect();
    let tools_for_llm: Vec<Value> = openai_tools
        .iter()
        .cloned()
        .chain(other_tools.into_iter())
        .collect();

    if !auto_execute {
        let mut req_json = request_json.clone();
        set_json_tools(&mut req_json, tools_for_llm.clone());
        let response = call_openai_compat_proxy_with_body(
            state,
            parts,
            request_id,
            &req_json,
            original_stream,
        )
        .await?;
        return Ok(Some(response));
    }

    let Some(messages) = request_json
        .get("messages")
        .and_then(|v| v.as_array())
        .cloned()
    else {
        return Err(openai_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            Some("invalid_mcp_request"),
            "missing messages",
        ));
    };
    let mut messages = messages;

    let mut tool_rounds_executed: usize = 0;
    loop {
        if tool_rounds_executed >= max_steps {
            let mut req_json = request_json.clone();
            set_json_tools(&mut req_json, tools_for_llm.clone());
            if let Some(obj) = req_json.as_object_mut() {
                obj.insert("messages".to_string(), Value::Array(messages));
            }
            let response = call_openai_compat_proxy_with_body(
                state,
                parts,
                request_id,
                &req_json,
                original_stream,
            )
            .await?;
            return Ok(Some(response));
        }

        // Non-stream call to extract tool calls.
        let step_request_id = format!("{request_id}-mcp{tool_rounds_executed}");
        let mut step_req_json = request_json.clone();
        set_json_tools(&mut step_req_json, tools_for_llm.clone());
        if let Some(obj) = step_req_json.as_object_mut() {
            obj.insert("messages".to_string(), Value::Array(messages.clone()));
        }
        let step_response = call_openai_compat_proxy_with_body(
            state,
            parts,
            &step_request_id,
            &step_req_json,
            false,
        )
        .await?;

        let (step_status, step_headers, step_body) =
            split_response(step_response, 8 * 1024 * 1024).await?;

        let step_json: Value = match serde_json::from_slice(&step_body) {
            Ok(value) => value,
            Err(_) => {
                return Ok(Some(rebuild_response(step_status, step_headers, step_body)));
            }
        };
        let assistant_message = extract_chat_assistant_message(&step_json);
        let tool_calls = assistant_message
            .as_ref()
            .map(extract_chat_tool_calls_from_message)
            .unwrap_or_default();

        if tool_calls.is_empty() {
            if original_stream {
                let mut req_json = request_json.clone();
                set_json_tools(&mut req_json, tools_for_llm.clone());
                if let Some(obj) = req_json.as_object_mut() {
                    obj.insert("messages".to_string(), Value::Array(messages));
                }
                let response =
                    call_openai_compat_proxy_with_body(state, parts, request_id, &req_json, true)
                        .await?;
                return Ok(Some(response));
            }
            return Ok(Some(rebuild_response(step_status, step_headers, step_body)));
        }

        if let Some(message) =
            assistant_message.or_else(|| build_chat_assistant_message_from_tool_calls(&tool_calls))
        {
            messages.push(message);
        }

        for call in &tool_calls {
            let result = mcp_call_tool(state, &server_ids, &call.name, call.arguments.clone())
                .await
                .unwrap_or_else(|err| Value::String(format!("MCP tool call failed: {err}")));
            let content = mcp_tool_result_to_text(&result);
            messages.push(serde_json::json!({
                "role": "tool",
                "tool_call_id": call.id,
                "content": content,
            }));
        }

        tool_rounds_executed = tool_rounds_executed.saturating_add(1);
    }
}

async fn maybe_handle_mcp_tools_responses(
    state: &GatewayHttpState,
    parts: &axum::http::request::Parts,
    parsed_json: &Option<Value>,
    request_id: &str,
) -> Result<Option<axum::response::Response>, (StatusCode, Json<OpenAiErrorResponse>)> {
    let Some(request_json) = parsed_json.as_ref() else {
        return Ok(None);
    };

    let tools = request_json.get("tools").and_then(|v| v.as_array());
    let Some(tools) = tools else {
        return Ok(None);
    };

    let (mcp_tool_cfgs, other_tools) = split_mcp_tool_configs(tools)?;
    if mcp_tool_cfgs.is_empty() {
        return Ok(None);
    }

    let original_stream = request_json
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let auto_execute = mcp_tool_cfgs
        .iter()
        .all(|cfg| cfg.require_approval.as_deref() == Some("never"));
    let max_steps = resolve_mcp_max_steps(&mcp_tool_cfgs)?;

    let requested_servers = resolve_mcp_servers_from_tool_cfgs(&mcp_tool_cfgs);
    let server_ids = match requested_servers {
        Some(servers) => servers,
        None => {
            let mut all: Vec<String> = state.mcp_servers.keys().cloned().collect();
            all.sort();
            all
        }
    };

    if server_ids.is_empty() {
        return Err(openai_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            Some("invalid_mcp_server"),
            "no MCP servers selected",
        ));
    }

    let mcp_tools_value = mcp_list_tools(state, &server_ids, None)
        .await
        .map_err(map_openai_gateway_error)?;
    let mut mcp_tools = mcp_tools_value
        .get("tools")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let allowed_tools = collect_allowed_tools(&mcp_tool_cfgs);
    if !allowed_tools.is_empty() {
        mcp_tools.retain(|tool| tool_name_allowed(tool, &allowed_tools));
    }

    let openai_tools: Vec<Value> = mcp_tools
        .iter()
        .filter_map(mcp_tool_to_openai_tool)
        .collect();
    let tools_for_llm: Vec<Value> = openai_tools
        .iter()
        .cloned()
        .chain(other_tools.into_iter())
        .collect();

    if !auto_execute {
        let mut req_json = request_json.clone();
        set_json_tools(&mut req_json, tools_for_llm.clone());
        let response = call_openai_compat_proxy_with_body(
            state,
            parts,
            request_id,
            &req_json,
            original_stream,
        )
        .await?;
        return Ok(Some(response));
    }

    // 1) Initial non-stream call to extract tool calls.
    let initial_request_id = format!("{request_id}-mcp0");
    let mut initial_req_json = request_json.clone();
    set_json_tools(&mut initial_req_json, tools_for_llm.clone());
    let initial_response = call_openai_compat_proxy_with_body(
        state,
        parts,
        &initial_request_id,
        &initial_req_json,
        false,
    )
    .await?;

    let (initial_status, initial_headers, initial_body) =
        split_response(initial_response, 8 * 1024 * 1024).await?;

    let initial_json: Option<Value> = serde_json::from_slice(&initial_body).ok();
    let tool_calls = initial_json
        .as_ref()
        .map(extract_responses_tool_calls)
        .unwrap_or_default();

    if tool_calls.is_empty() {
        if original_stream {
            let mut req_json = request_json.clone();
            set_json_tools(&mut req_json, tools_for_llm.clone());
            let response =
                call_openai_compat_proxy_with_body(state, parts, request_id, &req_json, true)
                    .await?;
            return Ok(Some(response));
        }
        return Ok(Some(rebuild_response(
            initial_status,
            initial_headers,
            initial_body,
        )));
    }

    let tool_results = execute_mcp_tool_calls(state, &server_ids, &tool_calls).await;

    let initial_is_shim = initial_headers.contains_key("x-ditto-shim");
    let response_id = initial_json
        .as_ref()
        .and_then(|value| value.get("id"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());

    if initial_is_shim || response_id.is_none() {
        let response = follow_up_via_chat_completions_to_responses(
            state,
            parts,
            McpResponsesToolLoopParams {
                request_id,
                request_json,
                server_ids: &server_ids,
                tools_for_llm,
                initial_tool_calls: &tool_calls,
                initial_tool_results: &tool_results,
                max_steps,
            },
        )
        .await?;
        return Ok(Some(response));
    }

    let response_id = response_id.unwrap_or_default();

    // Native /responses: multi-step tool loop is safe only for non-stream.
    if original_stream {
        // 2) Follow-up call with tool results via native /responses.
        let mut follow_up = request_json.clone();
        set_json_tools(&mut follow_up, tools_for_llm);
        if let Some(obj) = follow_up.as_object_mut() {
            obj.insert(
                "previous_response_id".to_string(),
                Value::String(response_id),
            );
            obj.insert("stream".to_string(), Value::Bool(true));
            let outputs = tool_calls
                .iter()
                .zip(tool_results.iter())
                .map(|(call, output)| {
                    serde_json::json!({
                        "type": "function_call_output",
                        "call_id": call.call_id.clone(),
                        "output": output,
                    })
                })
                .collect::<Vec<_>>();
            obj.insert("input".to_string(), Value::Array(outputs));
            obj.remove("messages");
        }

        let response =
            call_openai_compat_proxy_with_body(state, parts, request_id, &follow_up, true).await?;
        return Ok(Some(response));
    }

    let mut tool_rounds_executed: usize = 1;
    let mut prev_response_id = response_id;
    let mut tool_calls = tool_calls;
    let mut tool_results = tool_results;
    loop {
        if tool_rounds_executed >= max_steps {
            let mut follow_up = request_json.clone();
            set_json_tools(&mut follow_up, tools_for_llm.clone());
            if let Some(obj) = follow_up.as_object_mut() {
                obj.insert(
                    "previous_response_id".to_string(),
                    Value::String(prev_response_id.clone()),
                );
                obj.insert("stream".to_string(), Value::Bool(false));
                let outputs = tool_calls
                    .iter()
                    .zip(tool_results.iter())
                    .map(|(call, output)| {
                        serde_json::json!({
                            "type": "function_call_output",
                            "call_id": call.call_id.clone(),
                            "output": output,
                        })
                    })
                    .collect::<Vec<_>>();
                obj.insert("input".to_string(), Value::Array(outputs));
                obj.remove("messages");
            }

            let response = call_openai_compat_proxy_with_body(
                state,
                parts,
                request_id,
                &follow_up,
                false,
            )
            .await?;
            return Ok(Some(response));
        }

        let step_request_id = format!("{request_id}-mcp{tool_rounds_executed}");
        let mut follow_up = request_json.clone();
        set_json_tools(&mut follow_up, tools_for_llm.clone());
        if let Some(obj) = follow_up.as_object_mut() {
            obj.insert(
                "previous_response_id".to_string(),
                Value::String(prev_response_id.clone()),
            );
            obj.insert("stream".to_string(), Value::Bool(false));
            let outputs = tool_calls
                .iter()
                .zip(tool_results.iter())
                .map(|(call, output)| {
                    serde_json::json!({
                        "type": "function_call_output",
                        "call_id": call.call_id.clone(),
                        "output": output,
                    })
                })
                .collect::<Vec<_>>();
            obj.insert("input".to_string(), Value::Array(outputs));
            obj.remove("messages");
        }

        let response = call_openai_compat_proxy_with_body(
            state,
            parts,
            &step_request_id,
            &follow_up,
            false,
        )
        .await?;

        let (status, headers, body) = split_response(response, 8 * 1024 * 1024).await?;
        let value: Value = match serde_json::from_slice(&body) {
            Ok(value) => value,
            Err(_) => return Ok(Some(rebuild_response(status, headers, body))),
        };

        let next_tool_calls = extract_responses_tool_calls(&value);
        if next_tool_calls.is_empty() {
            return Ok(Some(rebuild_response(status, headers, body)));
        }

        let next_response_id = value
            .get("id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());
        let Some(next_response_id) = next_response_id else {
            return Ok(Some(rebuild_response(status, headers, body)));
        };

        tool_results = execute_mcp_tool_calls(state, &server_ids, &next_tool_calls).await;
        tool_calls = next_tool_calls;
        prev_response_id = next_response_id;
        tool_rounds_executed = tool_rounds_executed.saturating_add(1);
    }
}

#[derive(Debug, Clone)]
struct McpToolConfig {
    server_url: Option<String>,
    require_approval: Option<String>,
    allowed_tools: Vec<String>,
    max_steps: Option<usize>,
}

const MCP_MAX_STEPS: usize = 8;

type OpenAiCompatProxyError = (StatusCode, Json<OpenAiErrorResponse>);
type OpenAiCompatProxyResult<T> = Result<T, OpenAiCompatProxyError>;
type McpToolSplit = (Vec<McpToolConfig>, Vec<Value>);

fn split_mcp_tool_configs(
    tools: &[Value],
) -> OpenAiCompatProxyResult<McpToolSplit> {
    let mut mcp = Vec::new();
    let mut other = Vec::new();

    for tool in tools {
        let Some(obj) = tool.as_object() else {
            other.push(tool.clone());
            continue;
        };
        let tool_type = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if tool_type != "mcp" {
            other.push(tool.clone());
            continue;
        }

        let server_url = obj
            .get("server_url")
            .and_then(|v| v.as_str())
            .map(|v| v.to_string());
        let require_approval = obj
            .get("require_approval")
            .and_then(|v| v.as_str())
            .map(|v| v.to_string());
        let max_steps_value = obj
            .get("max_steps")
            .or_else(|| obj.get("maxSteps"))
            .cloned();
        let max_steps = if let Some(value) = max_steps_value {
            let Some(steps) = value.as_u64() else {
                return Err(openai_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request_error",
                    Some("invalid_mcp_request"),
                    "max_steps must be a positive integer",
                ));
            };
            if steps == 0 || steps > MCP_MAX_STEPS as u64 {
                return Err(openai_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request_error",
                    Some("invalid_mcp_request"),
                    format!("max_steps must be between 1 and {MCP_MAX_STEPS}"),
                ));
            }
            Some(steps as usize)
        } else {
            None
        };
        let allowed_tools = obj
            .get("allowed_tools")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        mcp.push(McpToolConfig {
            server_url,
            require_approval,
            allowed_tools,
            max_steps,
        });
    }

    Ok((mcp, other))
}

fn resolve_mcp_max_steps(
    cfgs: &[McpToolConfig],
) -> OpenAiCompatProxyResult<usize> {
    let mut max_steps = 1usize;
    for cfg in cfgs {
        if let Some(steps) = cfg.max_steps {
            max_steps = max_steps.max(steps);
        }
    }
    if max_steps == 0 || max_steps > MCP_MAX_STEPS {
        return Err(openai_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            Some("invalid_mcp_request"),
            format!("max_steps must be between 1 and {MCP_MAX_STEPS}"),
        ));
    }
    Ok(max_steps)
}

fn collect_allowed_tools(cfgs: &[McpToolConfig]) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    for cfg in cfgs {
        for tool in &cfg.allowed_tools {
            out.insert(tool.clone());
        }
    }
    out
}

fn tool_name_allowed(tool: &Value, allowed: &std::collections::BTreeSet<String>) -> bool {
    let Some(name) = tool.get("name").and_then(|v| v.as_str()) else {
        return false;
    };
    if allowed.contains(name) {
        return true;
    }
    let unprefixed = name.split_once('-').map(|(_, rest)| rest).unwrap_or(name);
    allowed.contains(unprefixed)
}

fn resolve_mcp_servers_from_tool_cfgs(cfgs: &[McpToolConfig]) -> Option<Vec<String>> {
    let mut out = std::collections::BTreeSet::new();
    let mut saw_all = false;

    for cfg in cfgs {
        let Some(url) = cfg.server_url.as_deref() else {
            saw_all = true;
            continue;
        };
        if url.trim().is_empty() {
            saw_all = true;
            continue;
        }
        match parse_mcp_server_selector(url) {
            None => saw_all = true,
            Some(list) => {
                for server in list {
                    out.insert(server);
                }
            }
        }
    }

    if saw_all {
        None
    } else {
        Some(out.into_iter().collect())
    }
}

fn parse_mcp_server_selector(server_url: &str) -> Option<Vec<String>> {
    let trimmed = server_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }

    // LiteLLM tool shortcut: litellm_proxy/mcp/<servers>
    if let Some(rest) = trimmed.strip_prefix("litellm_proxy") {
        let rest = rest.trim_start_matches('/');
        if rest.is_empty() || rest == "mcp" {
            return None;
        }
        let servers = rest.strip_prefix("mcp/").unwrap_or(rest);
        if servers.is_empty() {
            return None;
        }
        return Some(split_csv(servers));
    }

    // URL/path forms: /mcp/<servers> or /<servers>/mcp
    if let Ok(uri) = trimmed.parse::<axum::http::Uri>() {
        if let Some(path) = uri.path_and_query().map(|pq| pq.path()) {
            return parse_mcp_selector_from_path(path);
        }
    }

    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        if let Ok(url) = reqwest::Url::parse(trimmed) {
            return parse_mcp_selector_from_path(url.path());
        }
    }

    parse_mcp_selector_from_path(trimmed)
}

fn parse_mcp_selector_from_path(path: &str) -> Option<Vec<String>> {
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    let path = path.trim_end_matches('/');

    // /mcp => all
    if path == "/mcp" {
        return None;
    }
    // /mcp/<servers>
    if let Some(rest) = path.strip_prefix("/mcp/") {
        if rest.is_empty() {
            return None;
        }
        return Some(split_csv(rest));
    }
    // /<servers>/mcp
    if let Some(rest) = path.strip_suffix("/mcp") {
        let rest = rest.trim_end_matches('/');
        let servers = rest.rsplit('/').next().unwrap_or_default();
        if servers.is_empty() {
            return None;
        }
        return Some(split_csv(servers));
    }
    None
}

fn split_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn mcp_tool_to_openai_tool(tool: &Value) -> Option<Value> {
    let name = tool.get("name")?.as_str()?.to_string();
    let description = tool
        .get("description")
        .and_then(|v| v.as_str())
        .map(|v| v.to_string());
    let input_schema = tool.get("inputSchema").cloned().unwrap_or(Value::Null);

    let mut function = serde_json::json!({
        "name": name,
        "parameters": input_schema,
    });
    if let (Some(desc), Some(obj)) = (description, function.as_object_mut()) {
        obj.insert("description".to_string(), Value::String(desc));
    }

    Some(serde_json::json!({
        "type": "function",
        "function": function,
    }))
}

fn set_json_tools(request: &mut Value, tools: Vec<Value>) {
    let Some(obj) = request.as_object_mut() else {
        return;
    };
    obj.insert("tools".to_string(), Value::Array(tools));
}

async fn call_openai_compat_proxy_with_body(
    state: &GatewayHttpState,
    parts: &axum::http::request::Parts,
    request_id: &str,
    body_json: &Value,
    stream: bool,
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
    *req.uri_mut() = parts.uri.clone();
    *req.headers_mut() = headers;

    let fut = Box::pin(handle_openai_compat_proxy(
        State(state.clone()),
        Path(String::new()),
        req,
    ));
    fut.await
}

async fn split_response(
    response: axum::response::Response,
    max_bytes: usize,
) -> Result<(StatusCode, HeaderMap, Bytes), (StatusCode, Json<OpenAiErrorResponse>)> {
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), max_bytes)
        .await
        .map_err(|err| {
            openai_error(
                StatusCode::BAD_GATEWAY,
                "api_error",
                Some("backend_error"),
                err,
            )
        })?;
    Ok((status, headers, bytes))
}

fn rebuild_response(status: StatusCode, headers: HeaderMap, body: Bytes) -> axum::response::Response {
    let mut response = axum::response::Response::new(Body::from(body));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
}

include!("mcp_responses.rs");

#[derive(Debug, Clone)]
struct ChatToolCall {
    id: String,
    name: String,
    arguments: Value,
}

fn extract_chat_assistant_message(response: &Value) -> Option<Value> {
    response
        .get("choices")
        .and_then(|v| v.as_array())
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .cloned()
}

fn extract_chat_tool_calls_from_message(message: &Value) -> Vec<ChatToolCall> {
    let mut out = Vec::new();

    let tool_calls = message.get("tool_calls").and_then(|v| v.as_array());
    let Some(tool_calls) = tool_calls else {
        return out;
    };

    for (idx, call) in tool_calls.iter().enumerate() {
        let id = call
            .get("id")
            .and_then(|v| v.as_str())
            .map(|v| v.to_string())
            .unwrap_or_else(|| format!("call_{idx}"));
        let name = call
            .get("function")
            .and_then(|v| v.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if name.trim().is_empty() {
            continue;
        }
        let arguments_raw = call
            .get("function")
            .and_then(|v| v.get("arguments"))
            .cloned()
            .unwrap_or(Value::Null);
        let arguments = match arguments_raw {
            Value::String(text) => serde_json::from_str(&text).unwrap_or(Value::Null),
            other => other,
        };
        out.push(ChatToolCall {
            id,
            name,
            arguments,
        });
    }

    out
}

fn build_chat_assistant_message_from_tool_calls(tool_calls: &[ChatToolCall]) -> Option<Value> {
    if tool_calls.is_empty() {
        return None;
    }

    let tool_calls_value = tool_calls
        .iter()
        .map(|call| {
            let args = match &call.arguments {
                Value::Null => "{}".to_string(),
                other => serde_json::to_string(other).unwrap_or_else(|_| "{}".to_string()),
            };
            serde_json::json!({
                "id": call.id.clone(),
                "type": "function",
                "function": { "name": call.name.clone(), "arguments": args }
            })
        })
        .collect::<Vec<_>>();

    Some(serde_json::json!({
        "role": "assistant",
        "content": "",
        "tool_calls": tool_calls_value,
    }))
}

fn mcp_tool_result_to_text(result: &Value) -> String {
    if let Some(text) = result.as_str() {
        return text.to_string();
    }

    if let Some(content) = result.get("content").and_then(|v| v.as_array()) {
        let mut texts = Vec::new();
        for item in content {
            if item.get("type").and_then(|v| v.as_str()) == Some("text") {
                if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                    texts.push(text.to_string());
                }
            }
        }
        if !texts.is_empty() {
            return texts.join("\n");
        }
    }

    serde_json::to_string(result).unwrap_or_else(|_| "tool executed".to_string())
}
