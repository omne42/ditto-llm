use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
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
pub const TOOL_FS_WRITE_FILE: &str = "fs_write_file";
pub const TOOL_FS_LIST_DIR: &str = "fs_list_dir";
pub const TOOL_FS_FIND: &str = "fs_find";
pub const TOOL_FS_STAT: &str = "fs_stat";
pub const TOOL_SHELL_EXEC: &str = "shell_exec";

pub fn toolbox_tools() -> Vec<Tool> {
    vec![
        http_fetch_tool(),
        fs_read_file_tool(),
        fs_write_file_tool(),
        fs_list_dir_tool(),
        fs_find_tool(),
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

pub fn fs_write_file_tool() -> Tool {
    Tool {
        name: TOOL_FS_WRITE_FILE.to_string(),
        description: Some(
            "Write a file under the configured root directory and return the written path."
                .to_string(),
        ),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path relative to the configured root directory." },
                "content": { "type": "string", "description": "File contents (UTF-8 string)." },
                "overwrite": { "type": "boolean", "description": "Overwrite existing files (default: false)." },
                "create_parents": { "type": "boolean", "description": "Create parent directories if missing (default: false)." }
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

#[async_trait]
impl ToolExecutor for ToolboxExecutor {
    async fn execute(&self, call: ToolCall) -> Result<ToolResult> {
        match call.name.as_str() {
            TOOL_HTTP_FETCH => self.http.execute(call).await,
            TOOL_SHELL_EXEC => self.shell.execute(call).await,
            TOOL_FS_READ_FILE | TOOL_FS_WRITE_FILE | TOOL_FS_LIST_DIR | TOOL_FS_FIND
            | TOOL_FS_STAT => self.fs.execute(call).await,
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
pub struct ShellToolExecutor {
    root: PathBuf,
    allowed_programs: BTreeSet<String>,
    max_output_bytes: usize,
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
    cwd: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[async_trait]
impl ToolExecutor for ShellToolExecutor {
    async fn execute(&self, call: ToolCall) -> Result<ToolResult> {
        if call.name != TOOL_SHELL_EXEC {
            return Ok(ToolResult {
                tool_call_id: call.id,
                content: format!("unknown tool: {}", call.name),
                is_error: Some(true),
            });
        }

        let args: ShellExecArgs = match serde_json::from_value(call.arguments.clone()) {
            Ok(args) => args,
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: format!("invalid args: {err}"),
                    is_error: Some(true),
                });
            }
        };

        let program = match Self::validate_program(&args.program) {
            Ok(program) => program.to_string(),
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: err,
                    is_error: Some(true),
                });
            }
        };

        if !self.allowed_programs.contains(&program) {
            return Ok(ToolResult {
                tool_call_id: call.id,
                content: format!("program not allowed: {program}"),
                is_error: Some(true),
            });
        }

        if args.args.len() > 128 {
            return Ok(ToolResult {
                tool_call_id: call.id,
                content: "too many args (max: 128)".to_string(),
                is_error: Some(true),
            });
        }
        for arg in &args.args {
            if arg.len() > 8 * 1024 {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: "arg too large (max: 8192 bytes)".to_string(),
                    is_error: Some(true),
                });
            }
        }

        let cwd = match self.resolve_existing_dir(args.cwd.as_deref().unwrap_or("")) {
            Ok(dir) => dir,
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: err,
                    is_error: Some(true),
                });
            }
        };

        let timeout = args
            .timeout_ms
            .map(Duration::from_millis)
            .unwrap_or(self.timeout);
        let timeout = timeout.min(self.timeout).max(Duration::from_millis(1));

        let mut command = tokio::process::Command::new(&program);
        command.args(&args.args);
        command.current_dir(&cwd);
        command.stdin(Stdio::null());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        command.kill_on_drop(true);

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: format!("failed to spawn: {err}"),
                    is_error: Some(true),
                });
            }
        };

        let stdout = child.stdout.take().ok_or_else(|| {
            DittoError::Io(std::io::Error::other("shell_exec missing stdout pipe"))
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            DittoError::Io(std::io::Error::other("shell_exec missing stderr pipe"))
        })?;

        let max_output_bytes = self.max_output_bytes;
        let stdout_task = tokio::spawn(read_async_limited_bytes(stdout, max_output_bytes));
        let stderr_task = tokio::spawn(read_async_limited_bytes(stderr, max_output_bytes));

        let mut timed_out = false;
        let status = match tokio::time::timeout(timeout, child.wait()).await {
            Ok(status) => match status {
                Ok(status) => Some(status),
                Err(err) => {
                    return Ok(ToolResult {
                        tool_call_id: call.id,
                        content: format!("failed to wait for process: {err}"),
                        is_error: Some(true),
                    });
                }
            },
            Err(_) => {
                timed_out = true;
                let _ = child.kill().await;
                let _ = child.wait().await;
                None
            }
        };

        let (stdout_bytes, stdout_truncated) = match stdout_task.await {
            Ok(Ok(ok)) => ok,
            Ok(Err(err)) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: format!("failed to read stdout: {err}"),
                    is_error: Some(true),
                });
            }
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: format!("stdout join error: {err}"),
                    is_error: Some(true),
                });
            }
        };
        let (stderr_bytes, stderr_truncated) = match stderr_task.await {
            Ok(Ok(ok)) => ok,
            Ok(Err(err)) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: format!("failed to read stderr: {err}"),
                    is_error: Some(true),
                });
            }
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: format!("stderr join error: {err}"),
                    is_error: Some(true),
                });
            }
        };

        let exit_code = status.as_ref().and_then(|status| status.code());
        let ok = exit_code == Some(0) && !timed_out;

        let out = serde_json::json!({
            "program": program,
            "args": args.args,
            "cwd": args.cwd.unwrap_or_else(|| ".".to_string()),
            "ok": ok,
            "exit_code": exit_code,
            "stdout": String::from_utf8_lossy(&stdout_bytes).to_string(),
            "stderr": String::from_utf8_lossy(&stderr_bytes).to_string(),
            "stdout_truncated": stdout_truncated,
            "stderr_truncated": stderr_truncated,
            "timed_out": timed_out,
        });

        Ok(ToolResult {
            tool_call_id: call.id,
            content: out.to_string(),
            is_error: if timed_out { Some(true) } else { None },
        })
    }
}

async fn read_async_limited_bytes(
    mut reader: impl tokio::io::AsyncRead + Unpin,
    max_bytes: usize,
) -> std::io::Result<(Vec<u8>, bool)> {
    use tokio::io::AsyncReadExt;

    let mut out = Vec::<u8>::new();
    let mut truncated = false;
    let mut buf = [0u8; 8192];

    loop {
        let read = reader.read(&mut buf).await?;
        if read == 0 {
            break;
        }

        if out.len() < max_bytes {
            let remaining = max_bytes.saturating_sub(out.len());
            let take = read.min(remaining);
            out.extend_from_slice(&buf[..take]);
            if take < read {
                truncated = true;
            }
        } else {
            truncated = true;
        }
    }

    Ok((out, truncated))
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

    fn validate_relative_path(raw: &str) -> std::result::Result<&Path, String> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err("path is empty".to_string());
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
        Ok(rel)
    }

    fn resolve_existing_path(&self, raw: &str) -> std::result::Result<PathBuf, String> {
        let rel = Self::validate_relative_path(raw)?;
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
struct FsStatArgs {
    path: String,
}

#[async_trait]
impl ToolExecutor for FsToolExecutor {
    async fn execute(&self, call: ToolCall) -> Result<ToolResult> {
        match call.name.as_str() {
            TOOL_FS_READ_FILE => self.execute_read_file(call).await,
            TOOL_FS_WRITE_FILE => self.execute_write_file(call).await,
            TOOL_FS_LIST_DIR => self.execute_list_dir(call).await,
            TOOL_FS_FIND => self.execute_find(call).await,
            TOOL_FS_STAT => self.execute_stat(call).await,
            other => Ok(ToolResult {
                tool_call_id: call.id,
                content: format!("unknown tool: {other}"),
                is_error: Some(true),
            }),
        }
    }
}

impl FsToolExecutor {
    async fn execute_read_file(&self, call: ToolCall) -> Result<ToolResult> {
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

        let path = match self.resolve_existing_path(&args.path) {
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

    async fn execute_write_file(&self, call: ToolCall) -> Result<ToolResult> {
        let args: FsWriteFileArgs = match serde_json::from_value(call.arguments.clone()) {
            Ok(args) => args,
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: format!("invalid args: {err}"),
                    is_error: Some(true),
                });
            }
        };

        if args.content.len() > self.max_bytes {
            return Ok(ToolResult {
                tool_call_id: call.id,
                content: format!("content exceeds max_bytes ({})", self.max_bytes),
                is_error: Some(true),
            });
        }

        let rel = match Self::validate_relative_path(&args.path) {
            Ok(rel) => rel.to_path_buf(),
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: err,
                    is_error: Some(true),
                });
            }
        };

        let root = self.root.clone();
        let max_bytes = self.max_bytes;
        let write = tokio::task::spawn_blocking(move || {
            fs_write_file_blocking(
                &root,
                &rel,
                &args.content,
                args.overwrite,
                args.create_parents,
                max_bytes,
            )
        })
        .await
        .map_err(|err| {
            DittoError::Io(std::io::Error::other(format!(
                "fs_write_file join error: {err}"
            )))
        })?;

        let written_path = match write {
            Ok(path) => path,
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: format!("fs_write_file failed: {err}"),
                    is_error: Some(true),
                });
            }
        };

        let out = serde_json::json!({
            "path": args.path,
            "written": true,
            "absolute_path": written_path.to_string_lossy(),
        });

        Ok(ToolResult {
            tool_call_id: call.id,
            content: out.to_string(),
            is_error: None,
        })
    }

    async fn execute_list_dir(&self, call: ToolCall) -> Result<ToolResult> {
        let args: FsListDirArgs = match serde_json::from_value(call.arguments.clone()) {
            Ok(args) => args,
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: format!("invalid args: {err}"),
                    is_error: Some(true),
                });
            }
        };

        let raw_path = args.path.as_deref().unwrap_or("");
        let dir = if raw_path.trim().is_empty() {
            self.root.clone()
        } else {
            match self.resolve_existing_path(raw_path) {
                Ok(path) => path,
                Err(err) => {
                    return Ok(ToolResult {
                        tool_call_id: call.id,
                        content: err,
                        is_error: Some(true),
                    });
                }
            }
        };

        let max_entries = args
            .max_entries
            .unwrap_or(self.max_list_entries)
            .min(self.max_list_entries)
            .max(1);

        let root = self.root.clone();
        let list =
            tokio::task::spawn_blocking(move || fs_list_dir_blocking(&root, &dir, max_entries))
                .await
                .map_err(|err| {
                    DittoError::Io(std::io::Error::other(format!(
                        "fs_list_dir join error: {err}"
                    )))
                })?;

        let (entries, truncated) = match list {
            Ok(ok) => ok,
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: format!("fs_list_dir failed: {err}"),
                    is_error: Some(true),
                });
            }
        };

        let out = serde_json::json!({
            "path": if raw_path.trim().is_empty() { "." } else { raw_path },
            "entries": entries,
            "truncated": truncated,
        });

        Ok(ToolResult {
            tool_call_id: call.id,
            content: out.to_string(),
            is_error: None,
        })
    }

    async fn execute_find(&self, call: ToolCall) -> Result<ToolResult> {
        let args: FsFindArgs = match serde_json::from_value(call.arguments.clone()) {
            Ok(args) => args,
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: format!("invalid args: {err}"),
                    is_error: Some(true),
                });
            }
        };

        let include_files = args.include_files.unwrap_or(true);

        let raw_path = args.path.as_deref().unwrap_or("");
        let dir = if raw_path.trim().is_empty() {
            self.root.clone()
        } else {
            match self.resolve_existing_path(raw_path) {
                Ok(path) => path,
                Err(err) => {
                    return Ok(ToolResult {
                        tool_call_id: call.id,
                        content: err,
                        is_error: Some(true),
                    });
                }
            }
        };

        let max_entries = args
            .max_entries
            .unwrap_or(self.max_list_entries)
            .min(self.max_list_entries)
            .max(1);

        let max_depth = args.max_depth.unwrap_or(10).min(64);

        let pattern = args
            .pattern
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());

        let extensions = args.extensions.map(|values| {
            values
                .into_iter()
                .filter_map(|value| {
                    let trimmed = value.trim().trim_start_matches('.');
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_ascii_lowercase())
                    }
                })
                .collect::<BTreeSet<_>>()
        });

        let options = FsFindOptions {
            pattern,
            extensions,
            max_entries,
            max_depth,
            include_files,
            include_dirs: args.include_dirs,
        };

        let root = self.root.clone();
        let find = tokio::task::spawn_blocking(move || fs_find_blocking(&root, &dir, options))
            .await
            .map_err(|err| {
                DittoError::Io(std::io::Error::other(format!("fs_find join error: {err}")))
            })?;

        let (entries, truncated) = match find {
            Ok(ok) => ok,
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: format!("fs_find failed: {err}"),
                    is_error: Some(true),
                });
            }
        };

        let out = serde_json::json!({
            "path": if raw_path.trim().is_empty() { "." } else { raw_path },
            "entries": entries,
            "truncated": truncated,
        });

        Ok(ToolResult {
            tool_call_id: call.id,
            content: out.to_string(),
            is_error: None,
        })
    }

    async fn execute_stat(&self, call: ToolCall) -> Result<ToolResult> {
        let args: FsStatArgs = match serde_json::from_value(call.arguments.clone()) {
            Ok(args) => args,
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: format!("invalid args: {err}"),
                    is_error: Some(true),
                });
            }
        };

        let path = match self.resolve_existing_path(&args.path) {
            Ok(path) => path,
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: err,
                    is_error: Some(true),
                });
            }
        };

        let stat = tokio::task::spawn_blocking(move || fs_stat_blocking(&path))
            .await
            .map_err(|err| {
                DittoError::Io(std::io::Error::other(format!("fs_stat join error: {err}")))
            })?;

        let value = match stat {
            Ok(value) => value,
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: format!("fs_stat failed: {err}"),
                    is_error: Some(true),
                });
            }
        };

        let out = serde_json::json!({
            "path": args.path,
            "stat": value,
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

fn fs_write_file_blocking(
    root: &Path,
    rel: &Path,
    content: &str,
    overwrite: bool,
    create_parents: bool,
    max_bytes: usize,
) -> std::io::Result<PathBuf> {
    let root = std::fs::canonicalize(root)?;

    for component in rel.components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "parent dir segments are not allowed",
            ));
        }
    }

    let file_name = rel.file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no filename")
    })?;
    let rel_parent = rel.parent().unwrap_or_else(|| Path::new(""));

    let joined_parent = root.join(rel_parent);
    if create_parents {
        std::fs::create_dir_all(&joined_parent)?;
    }
    let canonical_parent = std::fs::canonicalize(&joined_parent)?;
    if !canonical_parent.starts_with(&root) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "path escapes root",
        ));
    }

    if content.len() > max_bytes {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "content exceeds max_bytes",
        ));
    }

    let target = canonical_parent.join(file_name);
    if !target.starts_with(&root) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "path escapes root",
        ));
    }

    if target.exists() && !overwrite {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "file exists",
        ));
    }

    std::fs::write(&target, content)?;
    Ok(target)
}

fn fs_list_dir_blocking(
    root: &Path,
    dir: &Path,
    max_entries: usize,
) -> std::io::Result<(Vec<Value>, bool)> {
    let meta = std::fs::metadata(dir)?;
    if !meta.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "path is not a directory",
        ));
    }

    let mut entries = Vec::<Value>::new();
    let mut truncated = false;

    let mut rows: Vec<_> = std::fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
    rows.sort_by_key(|entry| entry.file_name());

    for entry in rows.into_iter().take(max_entries + 1) {
        if entries.len() >= max_entries {
            truncated = true;
            break;
        }

        let path = entry.path();
        let file_type = entry.file_type()?;
        let kind = if file_type.is_file() {
            "file"
        } else if file_type.is_dir() {
            "dir"
        } else if file_type.is_symlink() {
            "symlink"
        } else {
            "other"
        };

        let rel_path = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();

        let size_bytes = if file_type.is_file() {
            entry.metadata().map(|m| m.len()).unwrap_or(0)
        } else {
            0
        };

        entries.push(serde_json::json!({
            "path": rel_path,
            "name": entry.file_name().to_string_lossy(),
            "type": kind,
            "size_bytes": size_bytes,
        }));
    }

    Ok((entries, truncated))
}

#[derive(Clone, Debug)]
struct FsFindOptions {
    pattern: Option<String>,
    extensions: Option<BTreeSet<String>>,
    max_entries: usize,
    max_depth: usize,
    include_files: bool,
    include_dirs: bool,
}

fn fs_find_blocking(
    root: &Path,
    dir: &Path,
    options: FsFindOptions,
) -> std::io::Result<(Vec<Value>, bool)> {
    let meta = std::fs::metadata(dir)?;
    if !meta.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "path is not a directory",
        ));
    }

    let mut entries = Vec::<Value>::new();
    let mut truncated = false;
    fs_find_walk(root, dir, 0, &options, &mut entries, &mut truncated)?;
    Ok((entries, truncated))
}

fn fs_find_walk(
    root: &Path,
    dir: &Path,
    depth: usize,
    options: &FsFindOptions,
    out: &mut Vec<Value>,
    truncated: &mut bool,
) -> std::io::Result<()> {
    if *truncated {
        return Ok(());
    }
    if out.len() >= options.max_entries {
        *truncated = true;
        return Ok(());
    }

    let mut rows: Vec<_> = std::fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
    rows.sort_by_key(|entry| entry.file_name());

    for entry in rows {
        if out.len() >= options.max_entries {
            *truncated = true;
            break;
        }

        let path = entry.path();
        let file_type = entry.file_type()?;
        let name = entry.file_name().to_string_lossy().to_string();

        let kind = if file_type.is_file() {
            "file"
        } else if file_type.is_dir() {
            "dir"
        } else if file_type.is_symlink() {
            "symlink"
        } else {
            "other"
        };

        let rel_path = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();

        let matches_pattern = match options.pattern.as_ref() {
            None => true,
            Some(pattern) => rel_path.contains(pattern),
        };

        let matches_extension = match options.extensions.as_ref() {
            None => true,
            Some(allowed) => {
                if !(file_type.is_file() || file_type.is_symlink()) {
                    true
                } else {
                    match path.extension().and_then(|value| value.to_str()) {
                        Some(ext) => allowed.contains(&ext.to_ascii_lowercase()),
                        None => false,
                    }
                }
            }
        };

        let include = matches_pattern
            && matches_extension
            && ((options.include_dirs && file_type.is_dir())
                || (options.include_files && (file_type.is_file() || file_type.is_symlink())));

        if include {
            let size_bytes = if file_type.is_file() {
                entry.metadata().map(|m| m.len()).unwrap_or(0)
            } else {
                0
            };

            out.push(serde_json::json!({
                "path": rel_path,
                "name": name,
                "type": kind,
                "size_bytes": size_bytes,
            }));
        }

        if file_type.is_dir() && depth < options.max_depth {
            fs_find_walk(root, &path, depth + 1, options, out, truncated)?;
            if *truncated {
                break;
            }
        }
    }

    Ok(())
}

fn fs_stat_blocking(path: &Path) -> std::io::Result<Value> {
    let meta = std::fs::metadata(path)?;
    let file_type = meta.file_type();
    let kind = if file_type.is_file() {
        "file"
    } else if file_type.is_dir() {
        "dir"
    } else if file_type.is_symlink() {
        "symlink"
    } else {
        "other"
    };

    let size_bytes = if file_type.is_file() { meta.len() } else { 0 };

    let modified_ms = meta
        .modified()
        .ok()
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|value| value.as_millis().min(u128::from(u64::MAX)) as u64);

    Ok(serde_json::json!({
        "type": kind,
        "size_bytes": size_bytes,
        "modified_ms": modified_ms,
    }))
}
