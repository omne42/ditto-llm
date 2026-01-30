use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::Value;

use crate::types::Tool;
use crate::{DittoError, Result};

use super::{ToolCall, ToolExecutor, ToolResult};

pub const TOOL_HTTP_FETCH: &str = "http_fetch";
pub const TOOL_FS_READ_FILE: &str = "fs_read_file";

pub fn toolbox_tools() -> Vec<Tool> {
    vec![http_fetch_tool(), fs_read_file_tool()]
}

pub fn http_fetch_tool() -> Tool {
    Tool {
        name: TOOL_HTTP_FETCH.to_string(),
        description: Some(
            "Fetch an HTTP resource and return status, headers, and body.".to_string(),
        ),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "The URL to fetch." },
                "method": { "type": "string", "description": "HTTP method (default: GET)." },
                "headers": {
                    "type": "object",
                    "description": "Optional HTTP headers.",
                    "additionalProperties": { "type": "string" }
                },
                "body": { "type": "string", "description": "Optional request body (string)." },
                "json": { "description": "Optional request JSON body. If set, takes precedence over body." },
                "timeout_ms": { "type": "integer", "description": "Optional per-request timeout in milliseconds." }
            },
            "required": ["url"]
        }),
        strict: Some(true),
    }
}

pub fn fs_read_file_tool() -> Tool {
    Tool {
        name: TOOL_FS_READ_FILE.to_string(),
        description: Some(
            "Read a file from the configured root directory and return its contents.".to_string(),
        ),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path relative to the configured root directory." }
            },
            "required": ["path"]
        }),
        strict: Some(true),
    }
}

#[derive(Clone)]
pub struct ToolboxExecutor {
    http: HttpToolExecutor,
    fs: FsToolExecutor,
}

impl ToolboxExecutor {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        Ok(Self {
            http: HttpToolExecutor::new(),
            fs: FsToolExecutor::new(root)?,
        })
    }

    pub fn with_http(mut self, http: HttpToolExecutor) -> Self {
        self.http = http;
        self
    }

    pub fn with_fs(mut self, fs: FsToolExecutor) -> Self {
        self.fs = fs;
        self
    }
}

#[async_trait]
impl ToolExecutor for ToolboxExecutor {
    async fn execute(&self, call: ToolCall) -> Result<ToolResult> {
        match call.name.as_str() {
            TOOL_HTTP_FETCH => self.http.execute(call).await,
            TOOL_FS_READ_FILE => self.fs.execute(call).await,
            other => Ok(ToolResult {
                tool_call_id: call.id,
                content: format!("unknown tool: {other}"),
                is_error: Some(true),
            }),
        }
    }
}

#[derive(Clone)]
pub struct HttpToolExecutor {
    client: reqwest::Client,
    max_response_bytes: usize,
    timeout: Duration,
}

impl Default for HttpToolExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpToolExecutor {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            max_response_bytes: 256 * 1024,
            timeout: Duration::from_secs(20),
        }
    }

    pub fn with_client(mut self, client: reqwest::Client) -> Self {
        self.client = client;
        self
    }

    pub fn with_max_response_bytes(mut self, max_response_bytes: usize) -> Self {
        self.max_response_bytes = max_response_bytes.max(1);
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

#[derive(Debug, Deserialize)]
struct HttpFetchArgs {
    url: String,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    json: Option<Value>,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[async_trait]
impl ToolExecutor for HttpToolExecutor {
    async fn execute(&self, call: ToolCall) -> Result<ToolResult> {
        if call.name != TOOL_HTTP_FETCH {
            return Ok(ToolResult {
                tool_call_id: call.id,
                content: format!("unknown tool: {}", call.name),
                is_error: Some(true),
            });
        }

        let args: HttpFetchArgs = match serde_json::from_value(call.arguments.clone()) {
            Ok(args) => args,
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: format!("invalid args: {err}"),
                    is_error: Some(true),
                });
            }
        };

        let url = match reqwest::Url::parse(&args.url) {
            Ok(url) => url,
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: format!("invalid url: {err}"),
                    is_error: Some(true),
                });
            }
        };
        if url.scheme() != "http" && url.scheme() != "https" {
            return Ok(ToolResult {
                tool_call_id: call.id,
                content: format!("unsupported url scheme: {}", url.scheme()),
                is_error: Some(true),
            });
        }

        let method = args.method.unwrap_or_else(|| "GET".to_string());
        let method = method.trim().to_ascii_uppercase();
        let method = match reqwest::Method::from_bytes(method.as_bytes()) {
            Ok(method) => method,
            Err(_) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: format!("invalid http method: {method}"),
                    is_error: Some(true),
                });
            }
        };

        let mut builder = self.client.request(method, url.clone());
        for (key, value) in args.headers {
            builder = builder.header(key, value);
        }
        if let Some(json) = args.json {
            builder = builder.json(&json);
        } else if let Some(body) = args.body {
            builder = builder.body(body);
        }

        let timeout = args
            .timeout_ms
            .map(Duration::from_millis)
            .unwrap_or(self.timeout);
        builder = builder.timeout(timeout);

        let response = match builder.send().await {
            Ok(response) => response,
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: format!("http request failed: {err}"),
                    is_error: Some(true),
                });
            }
        };

        let status = response.status().as_u16();
        let headers = response_headers_to_map(response.headers());
        let (body_bytes, truncated, read_error) =
            read_limited_bytes(response.bytes_stream(), self.max_response_bytes).await;
        let body = String::from_utf8_lossy(&body_bytes).to_string();

        let mut out = serde_json::json!({
            "url": url.as_str(),
            "ok": status < 400,
            "status": status,
            "headers": headers,
            "body": body,
            "truncated": truncated,
        });
        if let Some(read_error) = read_error {
            if let Some(obj) = out.as_object_mut() {
                obj.insert(
                    "read_error".to_string(),
                    serde_json::Value::String(read_error),
                );
            }
        }

        Ok(ToolResult {
            tool_call_id: call.id,
            content: out.to_string(),
            is_error: None,
        })
    }
}

async fn read_limited_bytes(
    mut stream: impl futures_util::Stream<Item = std::result::Result<bytes::Bytes, reqwest::Error>>
    + Unpin,
    max_bytes: usize,
) -> (Vec<u8>, bool, Option<String>) {
    let mut out = Vec::<u8>::new();
    let mut truncated = false;
    let mut read_error = None;

    while let Some(item) = stream.next().await {
        match item {
            Ok(chunk) => {
                if out.len().saturating_add(chunk.len()) > max_bytes {
                    let remaining = max_bytes.saturating_sub(out.len());
                    if remaining > 0 {
                        out.extend_from_slice(&chunk[..remaining]);
                    }
                    truncated = true;
                    break;
                }
                out.extend_from_slice(&chunk);
            }
            Err(err) => {
                read_error = Some(err.to_string());
                break;
            }
        }
    }

    (out, truncated, read_error)
}

fn response_headers_to_map(headers: &reqwest::header::HeaderMap) -> BTreeMap<String, String> {
    let mut out = BTreeMap::<String, String>::new();
    for (key, value) in headers.iter() {
        let key = key.as_str().to_string();
        let value = value.to_str().unwrap_or_default().to_string();
        out.entry(key)
            .and_modify(|existing| {
                existing.push_str(", ");
                existing.push_str(&value);
            })
            .or_insert(value);
    }
    out
}

#[derive(Clone)]
pub struct FsToolExecutor {
    root: PathBuf,
    max_bytes: usize,
}

impl FsToolExecutor {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        let root = std::fs::canonicalize(&root).map_err(|err| {
            DittoError::Io(std::io::Error::new(
                err.kind(),
                format!("invalid fs tool root {}: {err}", root.display()),
            ))
        })?;
        Ok(Self {
            root,
            max_bytes: 256 * 1024,
        })
    }

    pub fn with_max_bytes(mut self, max_bytes: usize) -> Self {
        self.max_bytes = max_bytes.max(1);
        self
    }

    fn resolve_path(&self, raw: &str) -> std::result::Result<PathBuf, String> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err("path is empty".to_string());
        }
        let rel = Path::new(raw);
        if rel.is_absolute() {
            return Err("absolute paths are not allowed".to_string());
        }

        let joined = self.root.join(rel);
        let canonical = std::fs::canonicalize(&joined)
            .map_err(|err| format!("failed to resolve path {}: {err}", joined.display()))?;
        if !canonical.starts_with(&self.root) {
            return Err("path escapes root".to_string());
        }
        Ok(canonical)
    }
}

#[derive(Debug, Deserialize)]
struct FsReadFileArgs {
    path: String,
}

#[async_trait]
impl ToolExecutor for FsToolExecutor {
    async fn execute(&self, call: ToolCall) -> Result<ToolResult> {
        if call.name != TOOL_FS_READ_FILE {
            return Ok(ToolResult {
                tool_call_id: call.id,
                content: format!("unknown tool: {}", call.name),
                is_error: Some(true),
            });
        }

        let args: FsReadFileArgs = match serde_json::from_value(call.arguments.clone()) {
            Ok(args) => args,
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: format!("invalid args: {err}"),
                    is_error: Some(true),
                });
            }
        };

        let path = match self.resolve_path(&args.path) {
            Ok(path) => path,
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: err,
                    is_error: Some(true),
                });
            }
        };

        let max_bytes = self.max_bytes;
        let read = tokio::task::spawn_blocking(move || read_file_limited(&path, max_bytes))
            .await
            .map_err(|err| {
                DittoError::Io(std::io::Error::other(format!(
                    "fs_read_file join error: {err}"
                )))
            })?;

        let (content, truncated) = match read {
            Ok(ok) => ok,
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: format!("fs_read_file failed: {err}"),
                    is_error: Some(true),
                });
            }
        };

        let out = serde_json::json!({
            "path": args.path,
            "content": content,
            "truncated": truncated,
        });

        Ok(ToolResult {
            tool_call_id: call.id,
            content: out.to_string(),
            is_error: None,
        })
    }
}

fn read_file_limited(path: &Path, max_bytes: usize) -> std::io::Result<(String, bool)> {
    use std::io::Read;

    let file = std::fs::File::open(path)?;
    let mut buf = Vec::<u8>::new();
    let mut limited = file.take(max_bytes as u64 + 1);
    limited.read_to_end(&mut buf)?;

    let truncated = buf.len() > max_bytes;
    if truncated {
        buf.truncate(max_bytes);
    }

    Ok((String::from_utf8_lossy(&buf).to_string(), truncated))
}
