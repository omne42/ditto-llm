use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::Value;

use crate::contracts::Tool;
use crate::error::{DittoError, Result};

use super::{ToolCall, ToolExecutor, ToolResult};

pub const TOOL_HTTP_FETCH: &str = "http_fetch";
pub const TOOL_FS_READ_FILE: &str = "fs_read_file";
pub const TOOL_FS_WRITE_FILE: &str = "fs_write_file";
pub const TOOL_FS_LIST_DIR: &str = "fs_list_dir";
pub const TOOL_FS_FIND: &str = "fs_find";
pub const TOOL_FS_GREP: &str = "fs_grep";
pub const TOOL_FS_STAT: &str = "fs_stat";
pub const TOOL_SHELL_EXEC: &str = "shell_exec";

pub fn toolbox_tools() -> Vec<Tool> {
    vec![
        http_fetch_tool(),
        fs_read_file_tool(),
        fs_write_file_tool(),
        fs_list_dir_tool(),
        fs_delete_file_tool(),
        fs_mkdir_tool(),
        fs_move_tool(),
        fs_copy_file_tool(),
        fs_find_tool(),
        fs_grep_tool(),
        fs_stat_tool(),
        shell_exec_tool(),
    ]
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
                "timeout_ms": { "type": "integer", "description": "Optional per-request timeout in milliseconds." },
                "max_response_bytes": { "type": "integer", "description": "Optional max response size override in bytes (clamped by the host)." },
                "parse_json": { "type": "boolean", "description": "If true, attempt to parse the response body as JSON (best-effort)." }
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

pub fn fs_write_file_tool() -> Tool {
    Tool {
        name: TOOL_FS_WRITE_FILE.to_string(),
        description: Some(
            "Write a file under the configured root directory.\n\n\
Implemented via `safe-fs-tools`. When `overwrite=false`, this tool refuses to modify existing files.\n\
When `create_parents=true`, it creates missing parent directories."
                .to_string(),
        ),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path relative to the configured root directory." },
                "content": { "type": "string", "description": "File contents (UTF-8 string)." },
                "overwrite": { "type": "boolean", "description": "Overwrite existing files (default: false)." },
                "create_parents": { "type": "boolean", "description": "Create missing parent directories (default: false)." }
            },
            "required": ["path", "content"]
        }),
        strict: Some(true),
    }
}

pub fn fs_list_dir_tool() -> Tool {
    Tool {
        name: TOOL_FS_LIST_DIR.to_string(),
        description: Some(
            "List directory entries under the configured root directory.".to_string(),
        ),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Directory path relative to the configured root directory. Defaults to root." },
                "max_entries": { "type": "integer", "description": "Maximum number of entries to return (default: 200)." }
            }
        }),
        strict: Some(true),
    }
}

pub fn fs_find_tool() -> Tool {
    Tool {
        name: TOOL_FS_FIND.to_string(),
        description: Some(
            "Recursively find files and directories under the configured root directory."
                .to_string(),
        ),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Directory path relative to the configured root directory. Defaults to root." },
                "pattern": { "type": "string", "description": "Optional substring match on the relative path." },
                "extensions": { "type": "array", "description": "Optional file extension filter (e.g., [\"rs\",\"md\"]).", "items": { "type": "string" } },
                "max_entries": { "type": "integer", "description": "Maximum number of entries to return (default: 200)." },
                "max_depth": { "type": "integer", "description": "Maximum recursion depth (default: 10)." },
                "include_dirs": { "type": "boolean", "description": "Include directories in results (default: false)." },
                "include_files": { "type": "boolean", "description": "Include files in results (default: true)." }
            }
        }),
        strict: Some(true),
    }
}

pub fn fs_grep_tool() -> Tool {
    Tool {
        name: TOOL_FS_GREP.to_string(),
        description: Some(
            "Search for matching lines in files under the configured root directory.".to_string(),
        ),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Directory path relative to the configured root directory. Defaults to root." },
                "pattern": { "type": "string", "description": "Substring match on each line." },
                "case_sensitive": { "type": "boolean", "description": "Case sensitive match (default: true)." },
                "extensions": { "type": "array", "description": "Optional file extension filter (e.g., [\"rs\",\"md\"]).", "items": { "type": "string" } },
                "max_entries": { "type": "integer", "description": "Maximum number of matches to return (default: 200)." },
                "max_depth": { "type": "integer", "description": "Maximum recursion depth (default: 10)." },
                "max_file_bytes": { "type": "integer", "description": "Maximum bytes to read per file (default: 256 KiB)." }
            },
            "required": ["pattern"]
        }),
        strict: Some(true),
    }
}

pub fn fs_stat_tool() -> Tool {
    Tool {
        name: TOOL_FS_STAT.to_string(),
        description: Some(
            "Get file or directory metadata under the configured root directory.".to_string(),
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

pub fn shell_exec_tool() -> Tool {
    Tool {
        name: TOOL_SHELL_EXEC.to_string(),
        description: Some(
            "Execute an allowlisted program (no shell parsing) and return exit status and output."
                .to_string(),
        ),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "program": { "type": "string", "description": "Program name (must be allowlisted by the host)." },
                "args": {
                    "type": "array",
                    "description": "Optional program arguments.",
                    "items": { "type": "string" }
                },
                "stdin": { "type": "string", "description": "Optional stdin (UTF-8) to write to the program." },
                "cwd": { "type": "string", "description": "Working directory relative to the configured root directory (default: root)." },
                "timeout_ms": { "type": "integer", "description": "Optional timeout override in milliseconds (clamped by the host)." }
            },
            "required": ["program"]
        }),
        strict: Some(true),
    }
}

#[derive(Clone)]
pub struct ToolboxExecutor {
    http: HttpToolExecutor,
    shell: ShellToolExecutor,
    fs: FsToolExecutor,
}

impl ToolboxExecutor {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        Ok(Self {
            http: HttpToolExecutor::new(),
            shell: ShellToolExecutor::new(root.clone())?,
            fs: FsToolExecutor::new(root)?,
        })
    }

    pub fn with_http(mut self, http: HttpToolExecutor) -> Self {
        self.http = http;
        self
    }

    pub fn with_shell(mut self, shell: ShellToolExecutor) -> Self {
        self.shell = shell;
        self
    }

    pub fn with_fs(mut self, fs: FsToolExecutor) -> Self {
        self.fs = fs;
        self
    }
}

#[::async_trait::async_trait]
impl ToolExecutor for ToolboxExecutor {
    async fn execute(&self, call: ToolCall) -> Result<ToolResult> {
        match call.name.as_str() {
            TOOL_HTTP_FETCH => self.http.execute(call).await,
            TOOL_SHELL_EXEC => self.shell.execute(call).await,
            name if name.starts_with("fs_") => self.fs.execute(call).await,
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
    #[serde(default)]
    max_response_bytes: Option<usize>,
    #[serde(default)]
    parse_json: Option<bool>,
}

#[::async_trait::async_trait]
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

        let max_response_bytes = args
            .max_response_bytes
            .map(|max_response_bytes| max_response_bytes.max(1).min(self.max_response_bytes))
            .unwrap_or(self.max_response_bytes);
        let parse_json = args.parse_json.unwrap_or(false);

        let start = std::time::Instant::now();
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
            read_limited_bytes(response.bytes_stream(), max_response_bytes).await;
        let body = String::from_utf8_lossy(&body_bytes).to_string();
        let elapsed_ms = start.elapsed().as_millis();
        let elapsed_ms = u64::try_from(elapsed_ms).unwrap_or(u64::MAX);
        let body_json = if parse_json {
            serde_json::from_slice::<Value>(&body_bytes).ok()
        } else {
            None
        };
        let ok = status < 400 && read_error.is_none();

        let mut out = serde_json::json!({
            "url": url.as_str(),
            "ok": ok,
            "status": status,
            "headers": headers,
            "body": body,
            "truncated": truncated,
            "elapsed_ms": elapsed_ms,
        });
        if parse_json
            && let Some(obj) = out.as_object_mut() {
                obj.insert("body_json".to_string(), body_json.unwrap_or(Value::Null));
            }
        let is_error = !ok;
        if let Some(read_error) = read_error
            && let Some(obj) = out.as_object_mut() {
                obj.insert(
                    "read_error".to_string(),
                    serde_json::Value::String(read_error),
                );
            }

        Ok(ToolResult {
            tool_call_id: call.id,
            content: out.to_string(),
            is_error: if is_error { Some(true) } else { None },
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
pub struct ShellToolExecutor {
    root: PathBuf,
    allowed_programs: BTreeSet<String>,
    max_output_bytes: usize,
    max_stdin_bytes: usize,
    timeout: Duration,
}

impl ShellToolExecutor {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        let root = std::fs::canonicalize(&root).map_err(|err| {
            DittoError::Io(std::io::Error::new(
                err.kind(),
                format!("invalid shell tool root {}: {err}", root.display()),
            ))
        })?;
        Ok(Self {
            root,
            allowed_programs: BTreeSet::new(),
            max_output_bytes: 256 * 1024,
            max_stdin_bytes: 64 * 1024,
            timeout: Duration::from_secs(20),
        })
    }

    pub fn with_allowed_programs<I, S>(mut self, programs: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.allowed_programs = programs.into_iter().map(|p| p.into()).collect();
        self
    }

    pub fn with_max_output_bytes(mut self, max_output_bytes: usize) -> Self {
        self.max_output_bytes = max_output_bytes.max(1);
        self
    }

    pub fn with_max_stdin_bytes(mut self, max_stdin_bytes: usize) -> Self {
        self.max_stdin_bytes = max_stdin_bytes.max(1);
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn validate_program(raw: &str) -> std::result::Result<&str, String> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err("program is empty".to_string());
        }
        if raw.contains('/') || raw.contains('\\') || raw.contains(':') {
            return Err("program must be a bare name without path separators".to_string());
        }
        Ok(raw)
    }

    fn resolve_existing_dir(&self, raw: &str) -> std::result::Result<PathBuf, String> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Ok(self.root.clone());
        }

        let rel = Path::new(raw);
        if rel.is_absolute() {
            return Err("absolute paths are not allowed".to_string());
        }
        for component in rel.components() {
            if matches!(component, std::path::Component::ParentDir) {
                return Err("parent dir segments are not allowed".to_string());
            }
        }

        let joined = self.root.join(rel);
        let canonical = std::fs::canonicalize(&joined)
            .map_err(|err| format!("failed to resolve path {}: {err}", joined.display()))?;
        if !canonical.starts_with(&self.root) {
            return Err("path escapes root".to_string());
        }
        let meta = std::fs::metadata(&canonical).map_err(|err| format!("stat failed: {err}"))?;
        if !meta.is_dir() {
            return Err("path is not a directory".to_string());
        }
        Ok(canonical)
    }
}

#[derive(Debug, Deserialize)]
struct ShellExecArgs {
    program: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    stdin: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
}
#[derive(Clone)]
pub struct FsToolExecutor {
    root: PathBuf,
    max_bytes: usize,
    max_list_entries: usize,
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
            max_list_entries: 200,
        })
    }

    pub fn with_max_bytes(mut self, max_bytes: usize) -> Self {
        self.max_bytes = max_bytes.max(1);
        self
    }

    pub fn with_max_list_entries(mut self, max_list_entries: usize) -> Self {
        self.max_list_entries = max_list_entries.max(1);
        self
    }
}

#[derive(Debug, Deserialize)]
struct FsReadFileArgs {
    path: String,
}

#[derive(Debug, Deserialize)]
struct FsWriteFileArgs {
    path: String,
    content: String,
    #[serde(default)]
    overwrite: bool,
    #[serde(default)]
    create_parents: bool,
}

#[derive(Debug, Deserialize)]
struct FsListDirArgs {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    max_entries: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct FsFindArgs {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    pattern: Option<String>,
    #[serde(default)]
    extensions: Option<Vec<String>>,
    #[serde(default)]
    max_entries: Option<usize>,
    #[serde(default)]
    max_depth: Option<usize>,
    #[serde(default)]
    include_dirs: bool,
    #[serde(default)]
    include_files: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct FsGrepArgs {
    #[serde(default)]
    path: Option<String>,
    pattern: String,
    #[serde(default)]
    case_sensitive: Option<bool>,
    #[serde(default)]
    extensions: Option<Vec<String>>,
    #[serde(default)]
    max_entries: Option<usize>,
    #[serde(default)]
    max_depth: Option<usize>,
    #[serde(default)]
    max_file_bytes: Option<usize>,
}
