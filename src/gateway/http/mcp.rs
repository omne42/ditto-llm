const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
const MCP_MAX_REQUEST_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone)]
pub struct McpServerState {
    server_id: String,
    url: String,
    client: reqwest::Client,
    headers: HeaderMap,
    query_params: BTreeMap<String, String>,
    request_timeout: Option<std::time::Duration>,
}

impl McpServerState {
    pub fn new(server_id: String, url: String) -> Result<Self, GatewayError> {
        let parsed = reqwest::Url::parse(url.trim()).map_err(|err| GatewayError::InvalidRequest {
            reason: format!("invalid MCP server url: {err}"),
        })?;
        match parsed.scheme() {
            "http" | "https" => {}
            other => {
                return Err(GatewayError::InvalidRequest {
                    reason: format!("unsupported MCP server url scheme: {other}"),
                });
            }
        }

        let client =
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .build()
                .map_err(|err| GatewayError::Backend {
                    message: format!("mcp http client error: {err}"),
                })?;

        Ok(Self {
            server_id,
            url: parsed.to_string(),
            client,
            headers: HeaderMap::new(),
            query_params: BTreeMap::new(),
            request_timeout: None,
        })
    }

    pub fn server_id(&self) -> &str {
        &self.server_id
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn with_request_timeout_seconds(mut self, timeout_seconds: Option<u64>) -> Self {
        self.request_timeout = timeout_seconds
            .filter(|seconds| *seconds > 0)
            .map(std::time::Duration::from_secs);
        self
    }

    pub fn with_headers(mut self, headers: BTreeMap<String, String>) -> Result<Self, GatewayError> {
        self.headers = parse_headers(&headers)?;
        Ok(self)
    }

    pub fn with_query_params(mut self, params: BTreeMap<String, String>) -> Self {
        self.query_params = normalize_query_params(&params);
        self
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub async fn jsonrpc(&self, req: Value) -> Result<Value, GatewayError> {
        let mut request = self
            .client
            .post(&self.url)
            .headers(self.headers.clone())
            .json(&req);
        if !self.query_params.is_empty() {
            request = request.query(&self.query_params);
        }
        if let Some(timeout) = self.request_timeout {
            request = request.timeout(timeout);
        }
        let response = request.send().await.map_err(|err| GatewayError::Backend {
            message: format!("mcp request failed: {err}"),
        })?;
        let status = response.status();
        let bytes = response.bytes().await.map_err(|err| GatewayError::Backend {
            message: format!("mcp response read failed: {err}"),
        })?;
        if !status.is_success() {
            return Err(GatewayError::Backend {
                message: format!(
                    "mcp server responded with status={} body={}",
                    status.as_u16(),
                    String::from_utf8_lossy(&bytes)
                ),
            });
        }
        serde_json::from_slice(&bytes).map_err(|err| GatewayError::Backend {
            message: format!("mcp response is not valid JSON: {err}"),
        })
    }
}

fn parse_headers(headers: &BTreeMap<String, String>) -> Result<HeaderMap, GatewayError> {
    let mut out = HeaderMap::new();
    for (name, value) in headers {
        let header_name =
            name.parse::<axum::http::HeaderName>()
                .map_err(|_| GatewayError::InvalidRequest {
                    reason: format!("invalid header name: {name}"),
                })?;
        let header_value =
            value
                .parse::<axum::http::HeaderValue>()
                .map_err(|_| GatewayError::InvalidRequest {
                    reason: format!("invalid header value for {name}"),
                })?;
        out.insert(header_name, header_value);
    }
    Ok(out)
}

fn normalize_query_params(params: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    params
        .iter()
        .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
        .filter(|(name, _)| !name.is_empty())
        .collect()
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[serde(default)]
    jsonrpc: Option<String>,
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct McpToolsListRequest {
    #[serde(default)]
    servers: Option<Vec<String>>,
    #[serde(default)]
    cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct McpToolsCallRequest {
    name: String,
    #[serde(default)]
    arguments: Value,
    #[serde(default)]
    server_id: Option<String>,
}

async fn handle_mcp_root(
    State(state): State<GatewayHttpState>,
    req: axum::http::Request<Body>,
) -> axum::response::Response {
    handle_mcp_impl(state, None, req).await
}

async fn handle_mcp_subpath(
    State(state): State<GatewayHttpState>,
    Path(subpath): Path<String>,
    req: axum::http::Request<Body>,
) -> axum::response::Response {
    handle_mcp_impl(state, Some(subpath), req).await
}

async fn handle_mcp_namespaced_root(
    State(state): State<GatewayHttpState>,
    Path(servers): Path<String>,
    req: axum::http::Request<Body>,
) -> axum::response::Response {
    handle_mcp_impl(state, Some(servers), req).await
}

async fn handle_mcp_namespaced_subpath(
    State(state): State<GatewayHttpState>,
    Path((servers, _path)): Path<(String, String)>,
    req: axum::http::Request<Body>,
) -> axum::response::Response {
    handle_mcp_impl(state, Some(servers), req).await
}

async fn handle_mcp_tools_list(
    State(state): State<GatewayHttpState>,
    req: axum::http::Request<Body>,
) -> axum::response::Response {
    let (parts, body) = req.into_parts();

    if let Err(resp) = enforce_mcp_auth(&state, &parts.headers).await {
        return resp;
    }

    if parts.method != axum::http::Method::POST && parts.method != axum::http::Method::GET {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }

    let payload: Option<McpToolsListRequest> = if parts.method == axum::http::Method::POST {
        match to_bytes(body, MCP_MAX_REQUEST_BYTES).await {
            Ok(bytes) if bytes.is_empty() => None,
            Ok(bytes) => serde_json::from_slice(&bytes).ok(),
            Err(_) => None,
        }
    } else {
        None
    };

    let cursor = payload.as_ref().and_then(|p| p.cursor.clone());
    let server_ids = match resolve_requested_mcp_servers(
        &state,
        payload.and_then(|p| p.servers),
        &parts.headers,
        None,
    ) {
        Ok(ids) => ids,
        Err(resp) => return *resp,
    };

    match mcp_list_tools(&state, &server_ids, cursor).await {
        Ok(result) => Json(result).into_response(),
        Err(err) => (
            StatusCode::BAD_GATEWAY,
            Json(ErrorResponse {
                error: ErrorDetail {
                    code: "mcp_backend_error",
                    message: err.to_string(),
                },
            }),
        )
            .into_response(),
    }
}

async fn handle_mcp_tools_call(
    State(state): State<GatewayHttpState>,
    req: axum::http::Request<Body>,
) -> axum::response::Response {
    let (parts, body) = req.into_parts();

    if let Err(resp) = enforce_mcp_auth(&state, &parts.headers).await {
        return resp;
    }

    if parts.method != axum::http::Method::POST {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }

    let bytes = match to_bytes(body, MCP_MAX_REQUEST_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let parsed: McpToolsCallRequest = match serde_json::from_slice(&bytes) {
        Ok(parsed) => parsed,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };

    let server_ids = match resolve_requested_mcp_servers(
        &state,
        parsed.server_id.clone().map(|id| vec![id]),
        &parts.headers,
        None,
    ) {
        Ok(ids) => ids,
        Err(resp) => return *resp,
    };

    match mcp_call_tool(&state, &server_ids, &parsed.name, parsed.arguments).await {
        Ok(result) => Json(result).into_response(),
        Err(err) => (
            StatusCode::BAD_GATEWAY,
            Json(ErrorResponse {
                error: ErrorDetail {
                    code: "mcp_backend_error",
                    message: err.to_string(),
                },
            }),
        )
            .into_response(),
    }
}

async fn handle_mcp_impl(
    state: GatewayHttpState,
    selector: Option<String>,
    req: axum::http::Request<Body>,
) -> axum::response::Response {
    let (parts, body) = req.into_parts();

    if let Err(resp) = enforce_mcp_auth(&state, &parts.headers).await {
        return resp;
    }

    if parts.method != axum::http::Method::POST {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }

    let bytes = match to_bytes(body, MCP_MAX_REQUEST_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    if bytes.is_empty() {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let value: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    if value.is_array() {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let rpc: JsonRpcRequest = match serde_json::from_value(value) {
        Ok(rpc) => rpc,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };

    let server_ids = match resolve_requested_mcp_servers(&state, None, &parts.headers, selector) {
        Ok(ids) => ids,
        Err(resp) => return *resp,
    };

    let Some(id) = rpc.id.clone() else {
        return StatusCode::NO_CONTENT.into_response();
    };

    if rpc.jsonrpc.as_deref() != Some("2.0") {
        return Json(mcp_jsonrpc_error(id, -32600, "Invalid Request")).into_response();
    }

    match rpc.method.as_str() {
        "initialize" => {
            let result = serde_json::json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "serverInfo": {
                    "name": "ditto-gateway",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "capabilities": {
                    "tools": {},
                },
            });
            Json(mcp_jsonrpc_result(id, result)).into_response()
        }
        "notifications/initialized" => StatusCode::NO_CONTENT.into_response(),
        "tools/list" => {
            let cursor = rpc
                .params
                .as_ref()
                .and_then(|params| params.get("cursor"))
                .and_then(|value| value.as_str())
                .map(|value| value.to_string());
            match mcp_list_tools(&state, &server_ids, cursor).await {
                Ok(result) => Json(mcp_jsonrpc_result(id, result)).into_response(),
                Err(err) => Json(mcp_jsonrpc_error(id, -32000, &err.to_string())).into_response(),
            }
        }
        "tools/call" => {
            let Some(params) = rpc.params.as_ref() else {
                return Json(mcp_jsonrpc_error(id, -32602, "Invalid params")).into_response();
            };
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            if name.trim().is_empty() {
                return Json(mcp_jsonrpc_error(id, -32602, "Invalid params")).into_response();
            }
            let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);
            match mcp_call_tool(&state, &server_ids, name, arguments).await {
                Ok(result) => Json(mcp_jsonrpc_result(id, result)).into_response(),
                Err(err) => Json(mcp_jsonrpc_error(id, -32000, &err.to_string())).into_response(),
            }
        }
        _ => Json(mcp_jsonrpc_error(id, -32601, "Method not found")).into_response(),
    }
}

async fn enforce_mcp_auth(
    state: &GatewayHttpState,
    headers: &HeaderMap,
) -> Result<(), axum::response::Response> {
    let mut gateway = state.gateway.lock().await;
    if gateway.config.virtual_keys.is_empty() {
        return Ok(());
    }
    let token = extract_virtual_key(headers).ok_or_else(|| StatusCode::UNAUTHORIZED.into_response())?;
    let key = gateway
        .config
        .virtual_keys
        .iter()
        .find(|key| key.token == token)
        .cloned()
        .ok_or_else(|| StatusCode::UNAUTHORIZED.into_response())?;
    if !key.enabled {
        return Err(StatusCode::UNAUTHORIZED.into_response());
    }
    gateway.observability.record_request();
    Ok(())
}

fn resolve_requested_mcp_servers(
    state: &GatewayHttpState,
    servers: Option<Vec<String>>,
    headers: &HeaderMap,
    selector: Option<String>,
) -> Result<Vec<String>, Box<axum::response::Response>> {
    let selector = selector
        .as_deref()
        .and_then(|value| value.split('/').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());

    let header_selector = extract_header(headers, "x-mcp-servers");

    let mut requested = if let Some(servers) = servers {
        servers
    } else if let Some(selector) = selector.or(header_selector) {
        selector
            .split(',')
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect()
    } else {
        let mut all: Vec<String> = state.mcp_servers.keys().cloned().collect();
        all.sort();
        all
    };

    requested.sort();
    requested.dedup();
    if requested.is_empty() {
        return Err(Box::new(StatusCode::BAD_REQUEST.into_response()));
    }
    for server_id in &requested {
        if !state.mcp_servers.contains_key(server_id) {
            return Err(Box::new(StatusCode::NOT_FOUND.into_response()));
        }
    }
    Ok(requested)
}

async fn mcp_list_tools(
    state: &GatewayHttpState,
    server_ids: &[String],
    cursor: Option<String>,
) -> Result<Value, GatewayError> {
    let mut tools_out: Vec<Value> = Vec::new();
    let prefix_names = server_ids.len() > 1;

    for server_id in server_ids {
        let server = state.mcp_servers.get(server_id).ok_or_else(|| {
            GatewayError::InvalidRequest {
                reason: format!("unknown MCP server: {server_id}"),
            }
        })?;

        let mut params = serde_json::json!({});
        if let Some(cursor) = cursor.as_deref() {
            if let Some(obj) = params.as_object_mut() {
                obj.insert("cursor".to_string(), Value::String(cursor.to_string()));
            }
        }
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": params,
        });
        let resp = server.jsonrpc(req).await?;

        let result = resp.get("result").cloned().unwrap_or(Value::Null);
        let tools = if let Some(tools) = result.get("tools").and_then(|v| v.as_array()) {
            tools.clone()
        } else if let Some(tools) = result.as_array() {
            tools.clone()
        } else {
            return Err(GatewayError::Backend {
                message: format!("mcp tools/list invalid result for server {server_id}"),
            });
        };

        for tool in tools {
            let mut tool = tool;
            if prefix_names {
                if let Some(obj) = tool.as_object_mut() {
                    if let Some(Value::String(name)) = obj.get("name").cloned() {
                        obj.insert(
                            "name".to_string(),
                            Value::String(format!("{server_id}-{name}")),
                        );
                    }
                }
            }
            tools_out.push(tool);
        }
    }

    Ok(serde_json::json!({ "tools": tools_out }))
}

async fn mcp_call_tool(
    state: &GatewayHttpState,
    server_ids: &[String],
    name: &str,
    arguments: Value,
) -> Result<Value, GatewayError> {
    let (server_id, tool_name) = if server_ids.len() == 1 {
        (server_ids[0].as_str(), name)
    } else if let Some((prefix, rest)) = name.split_once('-') {
        let prefix = prefix.trim();
        if server_ids.iter().any(|id| id == prefix) {
            (prefix, rest)
        } else {
            return Err(GatewayError::InvalidRequest {
                reason: "ambiguous tool name; expected <server_id>-<tool_name>".to_string(),
            });
        }
    } else {
        return Err(GatewayError::InvalidRequest {
            reason: "ambiguous tool name; expected <server_id>-<tool_name>".to_string(),
        });
    };

    let server = state.mcp_servers.get(server_id).ok_or_else(|| GatewayError::InvalidRequest {
        reason: format!("unknown MCP server: {server_id}"),
    })?;

    let req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": tool_name,
            "arguments": arguments,
        },
    });
    let resp = server.jsonrpc(req).await?;

    if let Some(err) = resp.get("error") {
        return Err(GatewayError::Backend {
            message: format!("mcp tool call failed: {err}"),
        });
    }

    Ok(resp.get("result").cloned().unwrap_or(Value::Null))
}

fn mcp_jsonrpc_result(id: Value, result: Value) -> Value {
    serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn mcp_jsonrpc_error(id: Value, code: i64, message: &str) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message,
        }
    })
}
