const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
const MCP_MAX_REQUEST_BYTES: usize = 2 * 1024 * 1024;
const MCP_MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MCP_MAX_ERROR_RESPONSE_BYTES: usize = 64 * 1024;
const MCP_MAX_ERROR_BODY_SNIPPET_BYTES: usize = 8 * 1024;
const MCP_TOOLS_LIST_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(60);
const MCP_TOOLS_LIST_MAX_PAGES: usize = 8;
const MCP_TOOLS_LIST_MAX_CURSOR_BYTES: usize = 1024;

#[derive(Debug, Clone)]
struct McpToolsListResult {
    tools: Vec<Value>,
    next_cursor: Option<String>,
}

#[derive(Debug, Clone)]
struct McpToolsListCacheEntry {
    expires_at: std::time::Instant,
    tools: Vec<Value>,
    next_cursor: Option<String>,
}

#[derive(Debug, Default)]
struct McpToolsListCache {
    entries: std::collections::HashMap<Option<String>, McpToolsListCacheEntry>,
}

#[derive(Clone)]
pub struct McpServerState {
    server_id: String,
    url: String,
    client: reqwest::Client,
    headers: HeaderMap,
    query_params: BTreeMap<String, String>,
    request_timeout: Option<std::time::Duration>,
    tools_list_cache: std::sync::Arc<tokio::sync::Mutex<McpToolsListCache>>,
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
            tools_list_cache: std::sync::Arc::new(tokio::sync::Mutex::new(
                McpToolsListCache::default(),
            )),
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

    fn parse_tools_list_result(
        &self,
        result: Value,
    ) -> Result<McpToolsListResult, GatewayError> {
        match result {
            Value::Array(tools) => Ok(McpToolsListResult {
                tools,
                next_cursor: None,
            }),
            Value::Object(mut obj) => {
                let tools = obj
                    .remove("tools")
                    .and_then(|value| match value {
                        Value::Array(tools) => Some(tools),
                        _ => None,
                    })
                    .ok_or_else(|| GatewayError::Backend {
                        message: format!(
                            "mcp tools/list invalid result for server {}",
                            self.server_id
                        ),
                    })?;
                let next_cursor = obj
                    .remove("nextCursor")
                    .and_then(|value| match value {
                        Value::String(value) => Some(value),
                        _ => None,
                    })
                    .or_else(|| {
                        obj.remove("next_cursor").and_then(|value| match value {
                            Value::String(value) => Some(value),
                            _ => None,
                        })
                    })
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty());

                Ok(McpToolsListResult { tools, next_cursor })
            }
            _ => Err(GatewayError::Backend {
                message: format!("mcp tools/list invalid result for server {}", self.server_id),
            }),
        }
    }

    async fn list_tools_page_uncached(
        &self,
        cursor: Option<&str>,
    ) -> Result<McpToolsListResult, GatewayError> {
        let mut params = serde_json::json!({});
        if let Some(cursor) = cursor {
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
        let resp = self.jsonrpc(req).await?;

        if let Some(err) = resp.get("error") {
            return Err(GatewayError::Backend {
                message: format!("mcp tools/list failed for server {}: {err}", self.server_id),
            });
        }

        let result = resp.get("result").cloned().unwrap_or(Value::Null);
        self.parse_tools_list_result(result)
    }

    async fn list_tools_all_uncached(&self) -> Result<Vec<Value>, GatewayError> {
        let mut tools_out = Vec::<Value>::new();
        let mut cursor: Option<String> = None;
        let mut seen_cursors = std::collections::HashSet::<String>::new();

        for _ in 0..MCP_TOOLS_LIST_MAX_PAGES {
            let page = self.list_tools_page_uncached(cursor.as_deref()).await?;
            tools_out.extend(page.tools);
            let Some(next_cursor) = page.next_cursor else {
                return Ok(tools_out);
            };
            if !seen_cursors.insert(next_cursor.clone()) {
                return Err(GatewayError::Backend {
                    message: format!(
                        "mcp tools/list cursor loop detected for server {}",
                        self.server_id
                    ),
                });
            }
            cursor = Some(next_cursor);
        }

        Err(GatewayError::Backend {
            message: format!(
                "mcp tools/list exceeded max pages ({MCP_TOOLS_LIST_MAX_PAGES}) for server {}",
                self.server_id
            ),
        })
    }

    pub async fn list_tools_cached(
        &self,
        cursor: Option<String>,
    ) -> Result<Vec<Value>, GatewayError> {
        Ok(self.list_tools_page_cached(cursor).await?.tools)
    }

    async fn list_tools_page_cached(
        &self,
        cursor: Option<String>,
    ) -> Result<McpToolsListResult, GatewayError> {
        let now = std::time::Instant::now();

        if cursor
            .as_ref()
            .is_some_and(|cursor| cursor.len() > MCP_TOOLS_LIST_MAX_CURSOR_BYTES)
        {
            return Err(GatewayError::InvalidRequest {
                reason: format!(
                    "cursor exceeded max bytes ({MCP_TOOLS_LIST_MAX_CURSOR_BYTES})"
                ),
            });
        }

        if let Some(cursor) = cursor.as_deref() {
            return self.list_tools_page_uncached(Some(cursor)).await;
        }

        {
            let cache = self.tools_list_cache.lock().await;
            if let Some(entry) = cache.entries.get(&cursor) {
                if entry.expires_at > now {
                    return Ok(McpToolsListResult {
                        tools: entry.tools.clone(),
                        next_cursor: entry.next_cursor.clone(),
                    });
                }
            }
        }

        let result = McpToolsListResult {
            tools: self.list_tools_all_uncached().await?,
            next_cursor: None,
        };

        {
            let mut cache = self.tools_list_cache.lock().await;
            cache.entries.clear();
            cache.entries.insert(
                cursor,
                McpToolsListCacheEntry {
                    expires_at: now
                        .checked_add(MCP_TOOLS_LIST_CACHE_TTL)
                        .unwrap_or(now),
                    tools: result.tools.clone(),
                    next_cursor: result.next_cursor.clone(),
                },
            );
        }

        Ok(result)
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
        let headers = response.headers().clone();
        let max_bytes = if status.is_success() {
            MCP_MAX_RESPONSE_BYTES
        } else {
            MCP_MAX_ERROR_RESPONSE_BYTES
        };
        let bytes =
            read_reqwest_body_bytes_bounded_with_content_length(response, &headers, max_bytes)
                .await
                .map_err(|err| GatewayError::Backend {
                    message: format!("mcp response read failed: {err}"),
                })?;
        if !status.is_success() {
            let body_slice = bytes.as_ref();
            let truncated = body_slice.len() > MCP_MAX_ERROR_BODY_SNIPPET_BYTES;
            let body_slice = &body_slice[..body_slice.len().min(MCP_MAX_ERROR_BODY_SNIPPET_BYTES)];
            let mut body = String::from_utf8_lossy(body_slice).to_string();
            if truncated {
                body.push('…');
            }

            return Err(GatewayError::Backend {
                message: format!(
                    "mcp server responded with status={} body={}",
                    status.as_u16(),
                    body
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
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(parsed) => Some(parsed),
                Err(_) => {
                    return error_response(
                        StatusCode::BAD_REQUEST,
                        "invalid_json",
                        "invalid JSON body",
                    )
                    .into_response();
                }
            },
            Err(_) => {
                return error_response(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "request_too_large",
                    format!("request exceeded max bytes ({MCP_MAX_REQUEST_BYTES})"),
                )
                .into_response();
            }
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
        Err(err) => map_mcp_gateway_error(err).into_response(),
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
        Err(_) => {
            return error_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                "request_too_large",
                format!("request exceeded max bytes ({MCP_MAX_REQUEST_BYTES})"),
            )
            .into_response();
        }
    };
    let parsed: McpToolsCallRequest = match serde_json::from_slice(&bytes) {
        Ok(parsed) => parsed,
        Err(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_json",
                "invalid JSON body",
            )
            .into_response();
        }
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
        Err(err) => map_mcp_gateway_error(err).into_response(),
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
        Err(_) => {
            let message = format!("request exceeded max bytes ({MCP_MAX_REQUEST_BYTES})");
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(mcp_jsonrpc_error(Value::Null, -32600, &message)),
            )
                .into_response();
        }
    };
    if bytes.is_empty() {
        return Json(mcp_jsonrpc_error(Value::Null, -32600, "Invalid Request")).into_response();
    }

    let value: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return Json(mcp_jsonrpc_error(Value::Null, -32700, "Parse error")).into_response(),
    };
    if value.is_array() {
        return Json(mcp_jsonrpc_error(
            Value::Null,
            -32600,
            "Batch requests are not supported",
        ))
        .into_response();
    }

    let rpc: JsonRpcRequest = match serde_json::from_value(value) {
        Ok(rpc) => rpc,
        Err(_) => return Json(mcp_jsonrpc_error(Value::Null, -32600, "Invalid Request")).into_response(),
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
            let server_ids = match resolve_requested_mcp_servers_jsonrpc(
                &state,
                &parts.headers,
                selector.as_deref(),
            ) {
                Ok(ids) => ids,
                Err(reason) => {
                    return Json(mcp_jsonrpc_error(id, -32602, &reason)).into_response();
                }
            };
            let cursor = rpc
                .params
                .as_ref()
                .and_then(|params| params.get("cursor"))
                .and_then(|value| value.as_str())
                .map(|value| value.to_string());
            match mcp_list_tools(&state, &server_ids, cursor).await {
                Ok(result) => Json(mcp_jsonrpc_result(id, result)).into_response(),
                Err(GatewayError::InvalidRequest { reason }) => {
                    Json(mcp_jsonrpc_error(id, -32602, &reason)).into_response()
                }
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
            let server_ids = match resolve_requested_mcp_servers_jsonrpc(
                &state,
                &parts.headers,
                selector.as_deref(),
            ) {
                Ok(ids) => ids,
                Err(reason) => {
                    return Json(mcp_jsonrpc_error(id, -32602, &reason)).into_response();
                }
            };
            match mcp_call_tool(&state, &server_ids, name, arguments).await {
                Ok(result) => Json(mcp_jsonrpc_result(id, result)).into_response(),
                Err(GatewayError::InvalidRequest { reason }) => {
                    Json(mcp_jsonrpc_error(id, -32602, &reason)).into_response()
                }
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
        .virtual_key_by_token(&token)
        .cloned()
        .ok_or_else(|| StatusCode::UNAUTHORIZED.into_response())?;
    if !key.enabled {
        return Err(StatusCode::UNAUTHORIZED.into_response());
    }
    gateway.observability.record_request();
    drop(gateway);
    Ok(())
}

fn map_mcp_gateway_error(err: GatewayError) -> (StatusCode, Json<ErrorResponse>) {
    match err {
        GatewayError::Unauthorized => {
            error_response(StatusCode::UNAUTHORIZED, "unauthorized", err.to_string())
        }
        GatewayError::RateLimited { limit } => error_response(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            format!("rate limit exceeded: {limit}"),
        ),
        GatewayError::GuardrailRejected { reason } => error_response(
            StatusCode::FORBIDDEN,
            "guardrail_rejected",
            format!("guardrail rejected: {reason}"),
        ),
        GatewayError::BudgetExceeded { limit, attempted } => error_response(
            StatusCode::PAYMENT_REQUIRED,
            "budget_exceeded",
            format!("budget exceeded: limit={limit} attempted={attempted}"),
        ),
        GatewayError::CostBudgetExceeded {
            limit_usd_micros,
            attempted_usd_micros,
        } => error_response(
            StatusCode::PAYMENT_REQUIRED,
            "cost_budget_exceeded",
            format!(
                "cost budget exceeded: limit_usd_micros={limit_usd_micros} attempted_usd_micros={attempted_usd_micros}"
            ),
        ),
        GatewayError::BackendNotFound { name } => error_response(
            StatusCode::NOT_FOUND,
            "backend_not_found",
            format!("backend not found: {name}"),
        ),
        GatewayError::Backend { message } => {
            error_response(StatusCode::BAD_GATEWAY, "mcp_backend_error", message)
        }
        GatewayError::InvalidRequest { reason } => {
            error_response(StatusCode::BAD_REQUEST, "invalid_request", reason)
        }
    }
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
        let message = if state.mcp_servers.is_empty() {
            "no MCP servers configured"
        } else {
            "no MCP servers selected"
        };
        return Err(Box::new(
            error_response(StatusCode::BAD_REQUEST, "invalid_request", message).into_response(),
        ));
    }
    for server_id in &requested {
        if !state.mcp_servers.contains_key(server_id) {
            return Err(Box::new(
                error_response(
                    StatusCode::NOT_FOUND,
                    "not_found",
                    format!("mcp server not found: {server_id}"),
                )
                .into_response(),
            ));
        }
    }
    Ok(requested)
}

fn resolve_requested_mcp_servers_jsonrpc(
    state: &GatewayHttpState,
    headers: &HeaderMap,
    selector: Option<&str>,
) -> Result<Vec<String>, String> {
    let selector = selector
        .and_then(|value| value.split('/').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());

    let header_selector = extract_header(headers, "x-mcp-servers");

    let mut requested = if let Some(selector) = selector.or(header_selector) {
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
        let message = if state.mcp_servers.is_empty() {
            "no MCP servers configured"
        } else {
            "no MCP servers selected"
        };
        return Err(message.to_string());
    }
    for server_id in &requested {
        if !state.mcp_servers.contains_key(server_id) {
            return Err(format!("mcp server not found: {server_id}"));
        }
    }
    Ok(requested)
}

async fn mcp_list_tools(
    state: &GatewayHttpState,
    server_ids: &[String],
    cursor: Option<String>,
) -> Result<Value, GatewayError> {
    if cursor.is_some() && server_ids.len() != 1 {
        return Err(GatewayError::InvalidRequest {
            reason: "cursor is only supported when selecting a single MCP server".to_string(),
        });
    }
    let prefix_names = server_ids.len() > 1;

    let mut futures = Vec::with_capacity(server_ids.len());
    for server_id in server_ids {
        let server = state.mcp_servers.get(server_id).ok_or_else(|| {
            GatewayError::InvalidRequest {
                reason: format!("unknown MCP server: {server_id}"),
            }
        })?;
        let server = server.clone();
        let server_id = server_id.clone();
        let cursor = cursor.clone();
        futures.push(async move {
            let result = server.list_tools_page_cached(cursor).await?;
            Ok::<_, GatewayError>((server_id, result))
        });
    }

    let mut next_cursor_out: Option<String> = None;
    let mut tools_out: Vec<Value> = Vec::new();
    for (server_id, result) in futures_util::future::try_join_all(futures).await? {
        if server_ids.len() == 1 {
            next_cursor_out = result.next_cursor;
        }
        for tool in result.tools {
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

    let mut out = serde_json::Map::new();
    out.insert("tools".to_string(), Value::Array(tools_out));
    if let Some(next_cursor) = next_cursor_out {
        out.insert("nextCursor".to_string(), Value::String(next_cursor));
    }

    Ok(Value::Object(out))
}

async fn mcp_call_tool(
    state: &GatewayHttpState,
    server_ids: &[String],
    name: &str,
    arguments: Value,
) -> Result<Value, GatewayError> {
    let (server_id, tool_name) = if server_ids.len() == 1 {
        (server_ids[0].as_str(), name)
    } else {
        let mut best_match: Option<(&str, &str, usize)> = None;
        for server_id in server_ids {
            let Some(tool_name) = name
                .strip_prefix(server_id.as_str())
                .and_then(|tail| tail.strip_prefix('-'))
            else {
                continue;
            };
            let candidate_len = server_id.len();
            match best_match {
                Some((_, _, best_len)) if candidate_len <= best_len => {}
                _ => best_match = Some((server_id.as_str(), tool_name, candidate_len)),
            }
        }

        let Some((server_id, tool_name, _)) = best_match else {
            return Err(GatewayError::InvalidRequest {
                reason: "ambiguous tool name; expected <server_id>-<tool_name>".to_string(),
            });
        };
        if tool_name.is_empty() {
            return Err(GatewayError::InvalidRequest {
                reason: "ambiguous tool name; expected <server_id>-<tool_name>".to_string(),
            });
        }
        (server_id, tool_name)
    };
    if tool_name.trim().is_empty() {
        return Err(GatewayError::InvalidRequest {
            reason: "invalid tool name; expected non-empty name".to_string(),
        });
    }

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
