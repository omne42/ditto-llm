use serde_json::Map;

#[derive(Clone)]
pub struct A2aAgentState {
    agent_id: String,
    agent_name: Option<String>,
    agent_card_params: Value,
    backend: ProxyBackend,
}

impl A2aAgentState {
    pub fn new(agent_id: String, agent_card_params: Value, backend: ProxyBackend) -> Self {
        let agent_name = agent_card_params
            .as_object()
            .and_then(|obj| obj.get("name"))
            .and_then(|value| value.as_str())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        Self {
            agent_id,
            agent_name,
            agent_card_params,
            backend,
        }
    }

    fn matches(&self, requested: &str) -> bool {
        if self.agent_id == requested {
            return true;
        }
        if let Some(name) = self.agent_name.as_deref() {
            return name == requested;
        }
        false
    }
}

fn forwarded_header(headers: &HeaderMap, name: &str) -> Option<String> {
    let value = headers
        .get(name)
        .and_then(|value| value.to_str().ok())?
        .trim();
    let first = value.split(',').next().unwrap_or("").trim();
    (!first.is_empty()).then(|| first.to_string())
}

fn external_base_url(headers: &HeaderMap) -> Option<String> {
    let host = forwarded_header(headers, "x-forwarded-host")
        .or_else(|| forwarded_header(headers, "host"))?;
    let scheme = forwarded_header(headers, "x-forwarded-proto")
        .or_else(|| forwarded_header(headers, "x-forwarded-scheme"))
        .unwrap_or_else(|| "http".to_string());
    Some(format!("{scheme}://{host}"))
}

fn ensure_a2a_card_defaults(card: &mut Map<String, Value>, requested_agent_id: &str) {
    let name = card
        .get("name")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(requested_agent_id)
        .to_string();

    card.entry("protocolVersion".to_string())
        .or_insert_with(|| Value::String("1.0".to_string()));
    card.entry("name".to_string())
        .or_insert_with(|| Value::String(name.clone()));
    card.entry("description".to_string())
        .or_insert_with(|| Value::String(format!("A2A agent: {name}")));
    card.entry("version".to_string())
        .or_insert_with(|| Value::String("1.0.0".to_string()));
    card.entry("defaultInputModes".to_string())
        .or_insert_with(|| Value::Array(vec![Value::String("text".to_string())]));
    card.entry("defaultOutputModes".to_string())
        .or_insert_with(|| Value::Array(vec![Value::String("text".to_string())]));
    card.entry("capabilities".to_string())
        .or_insert_with(|| serde_json::json!({ "streaming": true }));
    card.entry("skills".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
}

async fn require_virtual_key_if_configured(
    state: &GatewayHttpState,
    headers: &HeaderMap,
) -> Result<bool, axum::response::Response> {
    let gateway = state.gateway.lock().await;
    let strip_authorization = !gateway.config.virtual_keys.is_empty();
    if !strip_authorization {
        return Ok(false);
    }

    let token = extract_bearer(headers)
        .or_else(|| extract_header(headers, "x-ditto-virtual-key"))
        .or_else(|| extract_header(headers, "x-api-key"))
        .ok_or_else(|| {
            error_response(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "missing virtual key",
            )
            .into_response()
        })?;

    let key = gateway
        .config
        .virtual_keys
        .iter()
        .find(|key| key.token == token)
        .cloned()
        .ok_or_else(|| {
            error_response(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "unauthorized virtual key",
            )
            .into_response()
        })?;

    if !key.enabled {
        return Err(
            error_response(StatusCode::UNAUTHORIZED, "unauthorized", "virtual key disabled")
                .into_response(),
        );
    }

    Ok(true)
}

fn jsonrpc_error(id: Option<Value>, code: i64, message: impl Into<String>) -> axum::response::Response {
    let payload = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(Value::Null),
        "error": { "code": code, "message": message.into() },
    });
    (StatusCode::BAD_REQUEST, Json(payload)).into_response()
}

async fn proxy_a2a_request(
    agent: &A2aAgentState,
    outgoing_headers: HeaderMap,
    body: Bytes,
    method: &str,
) -> Result<reqwest::Response, GatewayError> {
    let mut response = agent
        .backend
        .request(reqwest::Method::POST, "", outgoing_headers.clone(), Some(body.clone()))
        .await;

    let retry_path = match method {
        "message/send" => Some("/message/send"),
        "message/stream" => Some("/message/stream"),
        _ => None,
    };

    if let (Ok(resp), Some(path)) = (&response, retry_path) {
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::METHOD_NOT_ALLOWED
        {
            response = agent
                .backend
                .request(reqwest::Method::POST, path, outgoing_headers, Some(body))
                .await;
        }
    }

    response
}

async fn handle_a2a_agent_card(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
) -> axum::response::Response {
    let _strip_authorization = match require_virtual_key_if_configured(&state, &headers).await {
        Ok(strip) => strip,
        Err(resp) => return resp,
    };

    let agent = state
        .a2a_agents
        .values()
        .find(|agent| agent.matches(&agent_id))
        .cloned();
    let Some(agent) = agent else {
        return error_response(StatusCode::NOT_FOUND, "not_found", "agent not found")
            .into_response();
    };

    let base_url = external_base_url(&headers);
    let url = if let Some(base) = base_url.as_deref() {
        format!("{}/a2a/{}", base.trim_end_matches('/'), agent_id)
    } else {
        format!("/a2a/{agent_id}")
    };

    let mut value = agent.agent_card_params.clone();
    let mut obj = match value {
        Value::Object(obj) => obj,
        _ => Map::new(),
    };

    ensure_a2a_card_defaults(&mut obj, &agent_id);
    obj.insert("url".to_string(), Value::String(url));

    (StatusCode::OK, Json(Value::Object(obj))).into_response()
}

async fn handle_a2a_invoke(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
    body: Body,
) -> axum::response::Response {
    let strip_authorization = match require_virtual_key_if_configured(&state, &headers).await {
        Ok(strip) => strip,
        Err(resp) => return resp,
    };

    let agent = state
        .a2a_agents
        .values()
        .find(|agent| agent.matches(&agent_id))
        .cloned();
    let Some(agent) = agent else {
        return jsonrpc_error(None, -32000, format!("Agent '{agent_id}' not found"));
    };

    let body = match to_bytes(body, state.proxy_max_body_bytes).await {
        Ok(bytes) => bytes,
        Err(err) => {
            return jsonrpc_error(None, -32603, format!("Failed to read request body: {err}"));
        }
    };

    let parsed: Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(err) => return jsonrpc_error(None, -32700, format!("Parse error: {err}")),
    };
    let request_id = parsed.get("id").cloned();

    let obj = match parsed.as_object() {
        Some(obj) => obj,
        None => return jsonrpc_error(request_id, -32600, "Invalid Request: expected JSON object"),
    };

    match obj.get("jsonrpc").and_then(|value| value.as_str()) {
        Some("2.0") => {}
        _ => return jsonrpc_error(request_id, -32600, "Invalid Request: jsonrpc must be '2.0'"),
    }

    let method = match obj.get("method").and_then(|value| value.as_str()) {
        Some(method) if !method.trim().is_empty() => method.trim(),
        _ => return jsonrpc_error(request_id, -32600, "Invalid Request: missing method"),
    };

    match method {
        "message/send" | "message/stream" => {}
        _ => return jsonrpc_error(request_id, -32601, format!("Method '{method}' not found")),
    }

    let mut outgoing_headers = headers.clone();
    sanitize_proxy_headers(&mut outgoing_headers, strip_authorization);
    apply_backend_headers(&mut outgoing_headers, agent.backend.headers());

    let upstream = match proxy_a2a_request(&agent, outgoing_headers, body.clone(), method).await {
        Ok(resp) => resp,
        Err(err) => return jsonrpc_error(request_id, -32603, format!("Backend error: {err}")),
    };

    let status = upstream.status();
    let upstream_headers = upstream.headers().clone();
    let content_type = upstream_headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if content_type.starts_with("application/x-ndjson") || content_type.starts_with("text/event-stream") {
        let mut headers = upstream_headers;
        headers.remove("content-length");
        let stream = upstream
            .bytes_stream()
            .map(|chunk| chunk.map_err(std::io::Error::other))
            .boxed();
        let mut response = axum::response::Response::new(Body::from_stream(stream));
        *response.status_mut() = status;
        *response.headers_mut() = headers;
        return response;
    }

    let bytes = match read_reqwest_body_bytes_bounded_with_content_length(
        upstream,
        &upstream_headers,
        state.proxy_max_body_bytes,
    )
    .await
    {
        Ok(bytes) => bytes,
        Err(err) => return jsonrpc_error(request_id, -32603, format!("Backend response error: {err}")),
    };

    let mut headers = upstream_headers;
    headers.remove("content-length");
    let mut response = axum::response::Response::new(Body::from(bytes));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
}
