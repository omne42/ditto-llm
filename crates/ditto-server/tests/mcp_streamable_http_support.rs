use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

const TEST_MCP_PROTOCOL_VERSION: &str = "2025-06-18";

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct RecordedRequest {
    pub method: String,
    pub path: String,
    pub headers: BTreeMap<String, String>,
    pub json_body: Option<Value>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) enum ResponseSpec {
    JsonResult(Value),
    JsonError {
        code: i64,
        message: String,
    },
    RawHttp {
        status: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
}

#[allow(dead_code)]
impl ResponseSpec {
    pub(crate) fn json_result(result: Value) -> Self {
        Self::JsonResult(result)
    }

    pub(crate) fn raw_http(
        status: impl Into<String>,
        headers: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
        body: impl Into<Vec<u8>>,
    ) -> Self {
        Self::RawHttp {
            status: status.into(),
            headers: headers
                .into_iter()
                .map(|(name, value)| (name.into(), value.into()))
                .collect(),
            body: body.into(),
        }
    }
}

#[derive(Debug)]
struct State {
    queued_responses: Mutex<VecDeque<ResponseSpec>>,
    recorded_requests: Mutex<Vec<RecordedRequest>>,
}

impl State {
    fn push_response(&self, response: ResponseSpec) {
        self.queued_responses.lock().unwrap().push_back(response);
    }

    fn pop_response(&self) -> Option<ResponseSpec> {
        self.queued_responses.lock().unwrap().pop_front()
    }

    fn record(&self, request: RecordedRequest) {
        self.recorded_requests.lock().unwrap().push(request);
    }

    fn recorded_requests(&self) -> Vec<RecordedRequest> {
        self.recorded_requests.lock().unwrap().clone()
    }
}

pub(crate) struct TestMcpStreamableHttpServer {
    base_url: String,
    state: Arc<State>,
    shutdown_tx: watch::Sender<bool>,
}

impl TestMcpStreamableHttpServer {
    pub(crate) async fn start() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind test MCP listener");
        let addr = listener.local_addr().expect("listener address");
        let state = Arc::new(State {
            queued_responses: Mutex::new(VecDeque::new()),
            recorded_requests: Mutex::new(Vec::new()),
        });
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let accept_state = state.clone();
        tokio::spawn(async move {
            run_accept_loop(listener, accept_state, shutdown_rx).await;
        });

        Self {
            base_url: format!("http://{addr}/mcp"),
            state,
            shutdown_tx,
        }
    }

    pub(crate) fn url(&self) -> String {
        self.base_url.clone()
    }

    pub(crate) fn enqueue(&self, response: ResponseSpec) {
        self.state.push_response(response);
    }

    pub(crate) fn recorded_requests(&self) -> Vec<RecordedRequest> {
        self.state.recorded_requests()
    }

    pub(crate) fn requests_for_method(&self, method: &str) -> Vec<RecordedRequest> {
        self.recorded_requests()
            .into_iter()
            .filter(|request| {
                request
                    .json_body
                    .as_ref()
                    .and_then(|body| body.get("method"))
                    .and_then(|value| value.as_str())
                    == Some(method)
            })
            .collect()
    }
}

impl Drop for TestMcpStreamableHttpServer {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.send(true);
    }
}

async fn run_accept_loop(
    listener: TcpListener,
    state: Arc<State>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            changed = shutdown_rx.changed() => {
                if changed.is_ok() && *shutdown_rx.borrow() {
                    break;
                }
            }
            accept = listener.accept() => {
                let Ok((stream, _)) = accept else {
                    break;
                };
                let connection_state = state.clone();
                let connection_shutdown = shutdown_rx.clone();
                tokio::spawn(async move {
                    let _ = handle_connection(stream, connection_state, connection_shutdown).await;
                });
            }
        }
    }
}

async fn handle_connection(
    mut stream: TcpStream,
    state: Arc<State>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> io::Result<()> {
    loop {
        let request = match read_http_request(&mut stream).await {
            Ok(request) => request,
            Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(err) => return Err(err),
        };

        let json_body = if request.body.is_empty() {
            None
        } else {
            serde_json::from_slice::<Value>(&request.body).ok()
        };

        state.record(RecordedRequest {
            method: request.method.clone(),
            path: request.path.clone(),
            headers: request.headers.clone(),
            json_body: json_body.clone(),
        });

        if request.method == "GET" {
            write_chunked_response_headers(
                &mut stream,
                "200 OK",
                &[
                    ("content-type", "text/event-stream"),
                    ("cache-control", "no-cache"),
                    ("mcp-session-id", "test-session"),
                ],
            )
            .await?;
            let _ = shutdown_rx.changed().await;
            finish_chunked_response(&mut stream).await?;
            return Ok(());
        }

        let method = json_body
            .as_ref()
            .and_then(|body| body.get("method"))
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let request_id = json_body
            .as_ref()
            .and_then(|body| body.get("id"))
            .cloned()
            .unwrap_or(Value::Null);

        match method {
            "initialize" => {
                let body = json!({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "result": {
                        "protocolVersion": TEST_MCP_PROTOCOL_VERSION,
                        "serverInfo": {
                            "name": "test-mcp-server",
                            "version": "0.1.0",
                        },
                        "capabilities": {
                            "tools": {},
                        },
                    },
                });
                write_json_response(&mut stream, "200 OK", &body).await?;
            }
            "notifications/initialized" => {
                write_fixed_response(&mut stream, "202 Accepted", &[], &[]).await?;
            }
            _ => {
                let response = state
                    .pop_response()
                    .unwrap_or_else(|| ResponseSpec::JsonError {
                        code: -32000,
                        message: format!("no queued response for method {method}"),
                    });
                write_response_spec(&mut stream, request_id, response).await?;
            }
        }
    }
}

async fn write_response_spec(
    stream: &mut TcpStream,
    request_id: Value,
    response: ResponseSpec,
) -> io::Result<()> {
    match response {
        ResponseSpec::JsonResult(result) => {
            let body = json!({
                "jsonrpc": "2.0",
                "id": request_id,
                "result": result,
            });
            write_json_response(stream, "200 OK", &body).await
        }
        ResponseSpec::JsonError { code, message } => {
            let body = json!({
                "jsonrpc": "2.0",
                "id": request_id,
                "error": {
                    "code": code,
                    "message": message,
                },
            });
            write_json_response(stream, "200 OK", &body).await
        }
        ResponseSpec::RawHttp {
            status,
            headers,
            body,
        } => {
            let borrowed_headers: Vec<(&str, &str)> = headers
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str()))
                .collect();
            write_fixed_response(stream, &status, &borrowed_headers, &body).await
        }
    }
}

async fn write_json_response(stream: &mut TcpStream, status: &str, body: &Value) -> io::Result<()> {
    write_fixed_response(
        stream,
        status,
        &[("content-type", "application/json")],
        body.to_string().as_bytes(),
    )
    .await
}

#[derive(Debug, Clone)]
struct HttpRequest {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

async fn read_http_request(stream: &mut TcpStream) -> io::Result<HttpRequest> {
    let mut buf = Vec::new();
    let header_end = loop {
        let mut byte = [0_u8; 1];
        stream.read_exact(&mut byte).await?;
        buf.push(byte[0]);
        if buf.len() >= 4 && buf[buf.len() - 4..] == *b"\r\n\r\n" {
            break buf.len() - 4;
        }
    };

    let header_text = std::str::from_utf8(&buf[..header_end]).map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("http request header was not valid utf-8: {err}"),
        )
    })?;
    let mut lines = header_text.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing http request line"))?;
    let mut request_line_parts = request_line.split_whitespace();
    let method = request_line_parts
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing http method"))?
        .to_string();
    let path = request_line_parts
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing http path"))?
        .to_string();

    let mut headers = BTreeMap::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid http header line: {line}"),
            ));
        };
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
    }

    let content_length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0_u8; content_length];
    if content_length > 0 {
        stream.read_exact(&mut body).await?;
    }

    Ok(HttpRequest {
        method,
        path,
        headers,
        body,
    })
}

async fn write_fixed_response(
    stream: &mut TcpStream,
    status: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> io::Result<()> {
    let mut response = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\n", body.len());
    for (name, value) in headers {
        response.push_str(name);
        response.push_str(": ");
        response.push_str(value);
        response.push_str("\r\n");
    }
    response.push_str("\r\n");
    stream.write_all(response.as_bytes()).await?;
    if !body.is_empty() {
        stream.write_all(body).await?;
    }
    stream.flush().await
}

async fn write_chunked_response_headers(
    stream: &mut TcpStream,
    status: &str,
    headers: &[(&str, &str)],
) -> io::Result<()> {
    let mut response = format!("HTTP/1.1 {status}\r\nTransfer-Encoding: chunked\r\n");
    for (name, value) in headers {
        response.push_str(name);
        response.push_str(": ");
        response.push_str(value);
        response.push_str("\r\n");
    }
    response.push_str("\r\n");
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await
}

async fn finish_chunked_response(stream: &mut TcpStream) -> io::Result<()> {
    stream.write_all(b"0\r\n\r\n").await?;
    stream.flush().await
}
