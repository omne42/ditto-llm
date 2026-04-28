use omne_fs::ops::{
    Context as OmneFsContext, CopyFileRequest, DeleteRequest, GlobRequest, GrepRequest,
    ListDirRequest, MkdirRequest, MovePathRequest, ReadRequest, StatRequest, WriteFileRequest,
};
use omne_fs::policy::SandboxPolicy;
use policy_meta::WriteScope;
use serde::Serialize;

const SAFE_FS_ROOT_ID: &str = "root";
type SafeFsOp<T> = std::result::Result<T, String>;

// Own the omne-fs policy/request surface inside runtime so fs_* executors only
// translate tool args/results instead of impersonating a lower-level SDK.
#[derive(Clone)]
struct OmneFsToolRuntime {
    root: PathBuf,
    max_bytes: u64,
    max_results: usize,
}

#[derive(Debug, Deserialize)]
struct FsStatArgs {
    path: String,
}

impl OmneFsToolRuntime {
    fn new(root: PathBuf, max_bytes: usize, max_results: usize) -> Self {
        Self {
            root,
            max_bytes: max_bytes.max(1) as u64,
            max_results: max_results.max(1),
        }
    }

    fn with_max_bytes(mut self, max_bytes: usize) -> Self {
        self.max_bytes = max_bytes.max(1) as u64;
        self
    }

    fn with_max_results(mut self, max_results: usize) -> Self {
        self.max_results = max_results.max(1);
        self
    }

    fn clamp_max_results(&self, requested: Option<usize>) -> usize {
        requested.unwrap_or(self.max_results).min(self.max_results).max(1)
    }

    fn clamp_max_depth(&self, requested: Option<usize>) -> usize {
        requested.unwrap_or(10).min(64)
    }

    fn clamp_max_file_bytes(&self, requested: Option<usize>) -> u64 {
        requested
            .unwrap_or(self.max_bytes as usize)
            .min(self.max_bytes as usize)
            .max(1) as u64
    }

    fn context(
        &self,
        max_read_bytes: u64,
        max_write_bytes: u64,
        max_results: usize,
    ) -> SafeFsOp<OmneFsContext> {
        let mut policy = SandboxPolicy::single_root(
            SAFE_FS_ROOT_ID,
            self.root.clone(),
            WriteScope::WorkspaceWrite,
        );
        policy.paths.allow_absolute = false;
        policy.permissions.read = true;
        policy.permissions.glob = true;
        policy.permissions.grep = true;
        policy.permissions.delete = true;
        policy.permissions.list_dir = true;
        policy.permissions.stat = true;
        policy.permissions.mkdir = true;
        policy.permissions.write = true;
        policy.permissions.move_path = true;
        policy.permissions.copy_file = true;
        policy.limits.max_read_bytes = max_read_bytes;
        policy.limits.max_write_bytes = max_write_bytes;
        policy.limits.max_results = max_results.max(1);
        OmneFsContext::new(policy).map_err(|err| err.to_string())
    }

    async fn run<T, F>(&self, op_name: &'static str, task: F) -> Result<SafeFsOp<T>>
    where
        T: Send + 'static,
        F: FnOnce(Self) -> SafeFsOp<T> + Send + 'static,
    {
        let runtime = self.clone();
        tokio::task::spawn_blocking(move || task(runtime))
            .await
            .map_err(|err| DittoError::Io(std::io::Error::other(format!(
                "{op_name} join error: {err}"
            ))))
    }

    async fn read_file(&self, path: String) -> Result<SafeFsOp<Value>> {
        self.run("fs_read_file", move |runtime| {
            let resp = runtime.context(runtime.max_bytes, runtime.max_bytes, runtime.max_results)?
                .read_file(ReadRequest {
                    root_id: SAFE_FS_ROOT_ID.to_string(),
                    path: PathBuf::from(&path),
                    start_line: None,
                    end_line: None,
                })
                .map_err(|err| err.to_string())?;

            Ok(serde_json::json!({
                "path": path,
                "content": resp.content,
                "truncated": false,
            }))
        })
        .await
    }

    async fn write_file(
        &self,
        path: String,
        content: String,
        overwrite: bool,
        create_parents: bool,
    ) -> Result<SafeFsOp<Value>> {
        if content.len() > self.max_bytes as usize {
            return Ok(Err(format!(
                "content exceeds max_bytes ({})",
                self.max_bytes
            )));
        }

        self.run("fs_write_file", move |runtime| {
            let resp = runtime.context(runtime.max_bytes, runtime.max_bytes, runtime.max_results)?
                .write_file(WriteFileRequest {
                    root_id: SAFE_FS_ROOT_ID.to_string(),
                    path: PathBuf::from(&path),
                    content,
                    overwrite,
                    create_parents,
                })
                .map_err(|err| err.to_string())?;

            Ok(serde_json::json!({
                "path": path,
                "written": true,
                "bytes_written": resp.bytes_written,
                "created": resp.created,
            }))
        })
        .await
    }

    async fn list_dir(&self, path: Option<String>, max_entries: Option<usize>) -> Result<SafeFsOp<Value>> {
        let requested_path = path.unwrap_or_else(|| ".".to_string());
        let clamped_max_entries = self.clamp_max_results(max_entries);

        self.run("fs_list_dir", move |runtime| {
            let resp = runtime.context(runtime.max_bytes, runtime.max_bytes, runtime.max_results)?
                .list_dir(ListDirRequest {
                    root_id: SAFE_FS_ROOT_ID.to_string(),
                    path: PathBuf::from(&requested_path),
                    max_entries: Some(clamped_max_entries),
                })
                .map_err(|err| err.to_string())?;

            let entries: Vec<Value> = resp
                .entries
                .into_iter()
                .map(|entry| {
                    serde_json::json!({
                        "path": entry.path.to_string_lossy(),
                        "name": entry.name,
                        "type": serialize_kind(entry.kind),
                        "size_bytes": entry.size_bytes,
                    })
                })
                .collect();

            Ok(serde_json::json!({
                "path": requested_path,
                "entries": entries,
                "truncated": resp.truncated,
            }))
        })
        .await
    }

    async fn find(
        &self,
        path: Option<String>,
        pattern: Option<String>,
        extensions: Vec<String>,
        max_entries: Option<usize>,
        max_depth: Option<usize>,
        include_dirs: bool,
        include_files: bool,
    ) -> Result<SafeFsOp<Value>> {
        let requested_path = path.unwrap_or_else(|| ".".to_string());
        if !include_files {
            return Ok(Ok(serde_json::json!({
                "path": requested_path,
                "entries": [],
                "truncated": false,
            })));
        }
        if include_dirs {
            return Ok(Err(
                "fs_find include_dirs=true is not supported (safe-fs-tools glob only returns files)"
                    .to_string(),
            ));
        }

        let raw_dir = requested_path.trim().trim_matches('/');
        let dir_prefix = if raw_dir.is_empty() {
            None
        } else {
            Some(raw_dir.to_string())
        };
        let file_glob = glob_pattern(dir_prefix.as_deref(), &extensions);
        let substring = pattern
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());
        let clamped_max_entries = self.clamp_max_results(max_entries);
        let clamped_max_depth = self.clamp_max_depth(max_depth);

        self.run("fs_find", move |runtime| {
            let resp = runtime.context(runtime.max_bytes, runtime.max_bytes, runtime.max_results)?
                .glob_paths(GlobRequest {
                    root_id: SAFE_FS_ROOT_ID.to_string(),
                    pattern: file_glob,
                })
                .map_err(|err| err.to_string())?;

            let base_path = dir_prefix.as_deref().map(PathBuf::from).unwrap_or_default();
            let mut entries = Vec::<Value>::new();

            for path in resp.matches {
                if let Some(substring) = substring.as_ref() {
                    if !path.to_string_lossy().contains(substring) {
                        continue;
                    }
                }
                if path_depth_under(&base_path, &path) > clamped_max_depth {
                    continue;
                }
                let name = path
                    .file_name()
                    .map(|value| value.to_string_lossy().to_string())
                    .unwrap_or_default();
                entries.push(serde_json::json!({
                    "path": path.to_string_lossy(),
                    "name": name,
                    "type": "file",
                }));
            }

            let mut truncated = resp.truncated;
            if entries.len() > clamped_max_entries {
                entries.truncate(clamped_max_entries);
                truncated = true;
            }

            Ok(serde_json::json!({
                "path": requested_path,
                "entries": entries,
                "truncated": truncated,
            }))
        })
        .await
    }

    async fn grep(
        &self,
        path: Option<String>,
        pattern: String,
        case_sensitive: bool,
        extensions: Vec<String>,
        max_entries: Option<usize>,
        max_depth: Option<usize>,
        max_file_bytes: Option<usize>,
    ) -> Result<SafeFsOp<Value>> {
        let requested_path = path.unwrap_or_else(|| ".".to_string());
        let raw_dir = requested_path.trim().trim_matches('/');
        let dir_prefix = if raw_dir.is_empty() {
            None
        } else {
            Some(raw_dir.to_string())
        };
        let file_glob = glob_pattern(dir_prefix.as_deref(), &extensions);
        let clamped_max_entries = self.clamp_max_results(max_entries);
        let clamped_max_depth = self.clamp_max_depth(max_depth);
        let clamped_max_file_bytes = self.clamp_max_file_bytes(max_file_bytes);
        let (query, regex) = if case_sensitive {
            (pattern, false)
        } else {
            (format!("(?i){}", regex::escape(&pattern)), true)
        };

        self.run("fs_grep", move |runtime| {
            let resp = runtime
                .context(clamped_max_file_bytes, runtime.max_bytes, runtime.max_results)?
                .grep(GrepRequest {
                    root_id: SAFE_FS_ROOT_ID.to_string(),
                    query,
                    regex,
                    glob: Some(file_glob),
                })
                .map_err(|err| err.to_string())?;

            let base_path = dir_prefix.as_deref().map(PathBuf::from).unwrap_or_default();
            let mut matches = Vec::<Value>::new();
            for item in resp.matches {
                if path_depth_under(&base_path, &item.path) > clamped_max_depth {
                    continue;
                }
                matches.push(serde_json::json!({
                    "path": item.path.to_string_lossy(),
                    "line_number": item.line,
                    "line": item.text,
                }));
            }

            let mut truncated = resp.truncated;
            if matches.len() > clamped_max_entries {
                matches.truncate(clamped_max_entries);
                truncated = true;
            }

            Ok(serde_json::json!({
                "path": requested_path,
                "matches": matches,
                "truncated": truncated,
            }))
        })
        .await
    }

    async fn stat(&self, path: String) -> Result<SafeFsOp<Value>> {
        self.run("fs_stat", move |runtime| {
            let resp = runtime.context(runtime.max_bytes, runtime.max_bytes, runtime.max_results)?
                .stat(StatRequest {
                    root_id: SAFE_FS_ROOT_ID.to_string(),
                    path: PathBuf::from(&path),
                })
                .map_err(|err| err.to_string())?;

            Ok(serde_json::json!({
                "path": path,
                "stat": {
                    "type": serialize_kind(resp.kind),
                    "size_bytes": resp.size_bytes,
                    "modified_ms": resp.modified_ms,
                }
            }))
        })
        .await
    }

    async fn delete(
        &self,
        path: String,
        recursive: bool,
        ignore_missing: bool,
    ) -> Result<SafeFsOp<Value>> {
        self.run("fs_delete_file", move |runtime| {
            let resp = runtime.context(runtime.max_bytes, runtime.max_bytes, runtime.max_results)?
                .delete(DeleteRequest {
                    root_id: SAFE_FS_ROOT_ID.to_string(),
                    path: PathBuf::from(&path),
                    recursive,
                    ignore_missing,
                })
                .map_err(|err| err.to_string())?;

            Ok(serde_json::json!({
                "path": path,
                "deleted": resp.deleted,
                "type": serialize_kind(resp.kind),
            }))
        })
        .await
    }

    async fn mkdir(
        &self,
        path: String,
        create_parents: bool,
        ignore_existing: bool,
    ) -> Result<SafeFsOp<Value>> {
        self.run("fs_mkdir", move |runtime| {
            let resp = runtime.context(runtime.max_bytes, runtime.max_bytes, runtime.max_results)?
                .mkdir(MkdirRequest {
                    root_id: SAFE_FS_ROOT_ID.to_string(),
                    path: PathBuf::from(&path),
                    create_parents,
                    ignore_existing,
                })
                .map_err(|err| err.to_string())?;

            Ok(serde_json::json!({
                "path": path,
                "created": resp.created,
            }))
        })
        .await
    }

    async fn move_path(
        &self,
        from: String,
        to: String,
        overwrite: bool,
        create_parents: bool,
    ) -> Result<SafeFsOp<Value>> {
        self.run("fs_move", move |runtime| {
            let resp = runtime.context(runtime.max_bytes, runtime.max_bytes, runtime.max_results)?
                .move_path(MovePathRequest {
                    root_id: SAFE_FS_ROOT_ID.to_string(),
                    from: PathBuf::from(&from),
                    to: PathBuf::from(&to),
                    overwrite,
                    create_parents,
                })
                .map_err(|err| err.to_string())?;

            Ok(serde_json::json!({
                "from": from,
                "to": to,
                "moved": resp.moved,
                "type": serialize_kind(resp.kind),
            }))
        })
        .await
    }

    async fn copy_file(
        &self,
        from: String,
        to: String,
        overwrite: bool,
        create_parents: bool,
    ) -> Result<SafeFsOp<Value>> {
        self.run("fs_copy_file", move |runtime| {
            let resp = runtime.context(runtime.max_bytes, runtime.max_bytes, runtime.max_results)?
                .copy_file(CopyFileRequest {
                    root_id: SAFE_FS_ROOT_ID.to_string(),
                    from: PathBuf::from(&from),
                    to: PathBuf::from(&to),
                    overwrite,
                    create_parents,
                })
                .map_err(|err| err.to_string())?;

            Ok(serde_json::json!({
                "from": from,
                "to": to,
                "copied": resp.copied,
                "bytes": resp.bytes,
            }))
        })
        .await
    }
}

fn path_depth_under(base: &Path, path: &Path) -> usize {
    let rel = if base.as_os_str().is_empty() {
        path
    } else {
        path.strip_prefix(base).unwrap_or(path)
    };
    rel.components().count().saturating_sub(1)
}

fn serialize_kind<T>(kind: T) -> Value
where
    T: Serialize,
{
    serde_json::to_value(kind).unwrap_or(Value::Null)
}

fn glob_pattern(dir_prefix: Option<&str>, extensions: &[String]) -> String {
    match (dir_prefix, extensions) {
        (None, []) => "**/*".to_string(),
        (Some(dir), []) => format!("{dir}/**/*"),
        (None, [ext]) => format!("**/*.{ext}"),
        (Some(dir), [ext]) => format!("{dir}/**/*.{ext}"),
        (None, exts) => format!("**/*.{{{}}}", exts.join(",")),
        (Some(dir), exts) => format!("{dir}/**/*.{{{}}}", exts.join(",")),
    }
}

fn invalid_args_tool_result(call: &ToolCall, err: serde_json::Error) -> ToolResult {
    ToolResult {
        tool_call_id: call.id.clone(),
        content: format!("invalid args: {err}"),
        is_error: Some(true),
    }
}

fn finish_fs_tool_result(call_id: String, result: SafeFsOp<Value>) -> ToolResult {
    match result {
        Ok(content) => ToolResult {
            tool_call_id: call_id,
            content: content.to_string(),
            is_error: None,
        },
        Err(err) => ToolResult {
            tool_call_id: call_id,
            content: err,
            is_error: Some(true),
        },
    }
}

#[::async_trait::async_trait]
impl ToolExecutor for FsToolExecutor {
    async fn execute(&self, call: ToolCall) -> Result<ToolResult> {
        match call.name.as_str() {
            TOOL_FS_READ_FILE => self.execute_read_file(call).await,
            TOOL_FS_WRITE_FILE => self.execute_write_file(call).await,
            TOOL_FS_MOVE => self.execute_move(call).await,
            TOOL_FS_COPY_FILE => self.execute_copy_file(call).await,
            TOOL_FS_DELETE_FILE => self.execute_delete_file(call).await,
            TOOL_FS_MKDIR => self.execute_mkdir(call).await,
            TOOL_FS_LIST_DIR => self.execute_list_dir(call).await,
            TOOL_FS_FIND => self.execute_find(call).await,
            TOOL_FS_GREP => self.execute_grep(call).await,
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
            Err(err) => return Ok(invalid_args_tool_result(&call, err)),
        };

        Ok(finish_fs_tool_result(
            call.id,
            self.runtime.read_file(args.path).await?,
        ))
    }

    async fn execute_write_file(&self, call: ToolCall) -> Result<ToolResult> {
        let args: FsWriteFileArgs = match serde_json::from_value(call.arguments.clone()) {
            Ok(args) => args,
            Err(err) => return Ok(invalid_args_tool_result(&call, err)),
        };

        Ok(finish_fs_tool_result(
            call.id,
            self.runtime
                .write_file(args.path, args.content, args.overwrite, args.create_parents)
                .await?,
        ))
    }

    async fn execute_list_dir(&self, call: ToolCall) -> Result<ToolResult> {
        let args: FsListDirArgs = match serde_json::from_value(call.arguments.clone()) {
            Ok(args) => args,
            Err(err) => return Ok(invalid_args_tool_result(&call, err)),
        };

        Ok(finish_fs_tool_result(
            call.id,
            self.runtime.list_dir(args.path, args.max_entries).await?,
        ))
    }

    async fn execute_find(&self, call: ToolCall) -> Result<ToolResult> {
        let args: FsFindArgs = match serde_json::from_value(call.arguments.clone()) {
            Ok(args) => args,
            Err(err) => return Ok(invalid_args_tool_result(&call, err)),
        };

        let extensions: Vec<String> = args
            .extensions
            .unwrap_or_default()
            .into_iter()
            .filter_map(|value| {
                let trimmed = value.trim().trim_start_matches('.').to_ascii_lowercase();
                (!trimmed.is_empty()).then_some(trimmed)
            })
            .collect();

        Ok(finish_fs_tool_result(
            call.id,
            self.runtime
                .find(
                    args.path,
                    args.pattern,
                    extensions,
                    args.max_entries,
                    args.max_depth,
                    args.include_dirs,
                    args.include_files.unwrap_or(true),
                )
                .await?,
        ))
    }

    async fn execute_grep(&self, call: ToolCall) -> Result<ToolResult> {
        let args: FsGrepArgs = match serde_json::from_value(call.arguments.clone()) {
            Ok(args) => args,
            Err(err) => return Ok(invalid_args_tool_result(&call, err)),
        };

        let pattern = args.pattern.trim();
        if pattern.is_empty() {
            return Ok(ToolResult {
                tool_call_id: call.id,
                content: "pattern is empty".to_string(),
                is_error: Some(true),
            });
        }

        let extensions: Vec<String> = args
            .extensions
            .unwrap_or_default()
            .into_iter()
            .filter_map(|value| {
                let trimmed = value.trim().trim_start_matches('.').to_ascii_lowercase();
                (!trimmed.is_empty()).then_some(trimmed)
            })
            .collect();

        Ok(finish_fs_tool_result(
            call.id,
            self.runtime
                .grep(
                    args.path,
                    pattern.to_string(),
                    args.case_sensitive.unwrap_or(true),
                    extensions,
                    args.max_entries,
                    args.max_depth,
                    args.max_file_bytes,
                )
                .await?,
        ))
    }

    async fn execute_stat(&self, call: ToolCall) -> Result<ToolResult> {
        let args: FsStatArgs = match serde_json::from_value(call.arguments.clone()) {
            Ok(args) => args,
            Err(err) => return Ok(invalid_args_tool_result(&call, err)),
        };

        Ok(finish_fs_tool_result(
            call.id,
            self.runtime.stat(args.path).await?,
        ))
    }
}
