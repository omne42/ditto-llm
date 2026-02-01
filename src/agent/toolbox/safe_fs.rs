use safe_fs_tools::{
    CopyFileRequest, DeleteRequest, GlobRequest, GrepRequest, ListDirRequest, MkdirRequest,
    MovePathRequest, ReadRequest, RootMode, SandboxPolicy, StatRequest, WriteFileRequest,
};

const SAFE_FS_ROOT_ID: &str = "root";

#[derive(Debug, Deserialize)]
struct FsStatArgs {
    path: String,
}

fn safe_fs_ctx(
    root: PathBuf,
    max_read_bytes: u64,
    max_write_bytes: u64,
    max_results: usize,
) -> std::result::Result<safe_fs_tools::Context, safe_fs_tools::Error> {
    let mut policy = SandboxPolicy::single_root(SAFE_FS_ROOT_ID, root, RootMode::ReadWrite);
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
    safe_fs_tools::Context::new(policy)
}

fn path_depth_under(base: &Path, path: &Path) -> usize {
    let rel = if base.as_os_str().is_empty() {
        path
    } else {
        path.strip_prefix(base).unwrap_or(path)
    };
    rel.components().count().saturating_sub(1)
}

#[async_trait]
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
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: format!("invalid args: {err}"),
                    is_error: Some(true),
                });
            }
        };

        let root = self.root.clone();
        let max_bytes = self.max_bytes as u64;
        let max_results = self.max_list_entries;
        let raw_path = args.path.clone();

        let read = tokio::task::spawn_blocking(move || {
            let ctx = safe_fs_ctx(root, max_bytes, max_bytes, max_results)?;
            ctx.read_file(ReadRequest {
                root_id: SAFE_FS_ROOT_ID.to_string(),
                path: PathBuf::from(raw_path),
                start_line: None,
                end_line: None,
            })
        })
        .await
        .map_err(|err| DittoError::Io(std::io::Error::other(format!("fs_read_file join error: {err}"))))?;

        let resp = match read {
            Ok(resp) => resp,
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
            "content": resp.content,
            "truncated": false,
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

        let root = self.root.clone();
        let max_bytes = self.max_bytes as u64;
        let max_results = self.max_list_entries;
        let raw_path = args.path.clone();
        let desired = args.content.clone();
        let overwrite = args.overwrite;
        let create_parents = args.create_parents;

        let write = tokio::task::spawn_blocking(move || {
            let ctx = safe_fs_ctx(root, max_bytes, max_bytes, max_results)?;
            ctx.write_file(WriteFileRequest {
                root_id: SAFE_FS_ROOT_ID.to_string(),
                path: PathBuf::from(&raw_path),
                content: desired,
                overwrite,
                create_parents,
            })
        })
        .await
        .map_err(|err| DittoError::Io(std::io::Error::other(format!("fs_write_file join error: {err}"))))?;

        let resp = match write {
            Ok(resp) => resp,
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
            "bytes_written": resp.bytes_written,
            "created": resp.created,
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

        let requested = args.path.unwrap_or_else(|| ".".to_string());
        let max_entries = args
            .max_entries
            .unwrap_or(self.max_list_entries)
            .min(self.max_list_entries)
            .max(1);

        let root = self.root.clone();
        let max_bytes = self.max_bytes as u64;
        let max_results = self.max_list_entries;
        let raw_path = requested.clone();

        let list = tokio::task::spawn_blocking(move || {
            let ctx = safe_fs_ctx(root, max_bytes, max_bytes, max_results)?;
            ctx.list_dir(ListDirRequest {
                root_id: SAFE_FS_ROOT_ID.to_string(),
                path: PathBuf::from(raw_path),
                max_entries: Some(max_entries),
            })
        })
        .await
        .map_err(|err| DittoError::Io(std::io::Error::other(format!("fs_list_dir join error: {err}"))))?;

        let resp = match list {
            Ok(resp) => resp,
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: format!("fs_list_dir failed: {err}"),
                    is_error: Some(true),
                });
            }
        };

        let entries: Vec<Value> = resp
            .entries
            .into_iter()
            .map(|entry| {
                serde_json::json!({
                    "path": entry.path.to_string_lossy(),
                    "name": entry.name,
                    "type": entry.kind,
                    "size_bytes": entry.size_bytes,
                })
            })
            .collect();

        let out = serde_json::json!({
            "path": requested,
            "entries": entries,
            "truncated": resp.truncated,
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
        if !include_files {
            let out = serde_json::json!({
                "path": args.path.as_deref().unwrap_or("."),
                "entries": [],
                "truncated": false,
            });
            return Ok(ToolResult {
                tool_call_id: call.id,
                content: out.to_string(),
                is_error: None,
            });
        }

        if args.include_dirs {
            return Ok(ToolResult {
                tool_call_id: call.id,
                content: "fs_find include_dirs=true is not supported (safe-fs-tools glob only returns files)"
                    .to_string(),
                is_error: Some(true),
            });
        }

        let raw_dir = args.path.as_deref().unwrap_or("").trim().trim_matches('/');
        let dir_prefix = if raw_dir.is_empty() {
            None
        } else {
            Some(raw_dir.to_string())
        };

        let max_entries = args
            .max_entries
            .unwrap_or(self.max_list_entries)
            .min(self.max_list_entries)
            .max(1);

        let max_depth = args.max_depth.unwrap_or(10).min(64);

        let substring = args
            .pattern
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());

        let extensions: Vec<String> = args
            .extensions
            .unwrap_or_default()
            .into_iter()
            .filter_map(|value| {
                let trimmed = value.trim().trim_start_matches('.').to_ascii_lowercase();
                (!trimmed.is_empty()).then_some(trimmed)
            })
            .collect();

        let file_glob = match (dir_prefix.as_deref(), extensions.as_slice()) {
            (None, []) => "**/*".to_string(),
            (Some(dir), []) => format!("{dir}/**/*"),
            (None, [ext]) => format!("**/*.{ext}"),
            (Some(dir), [ext]) => format!("{dir}/**/*.{ext}"),
            (None, exts) => format!("**/*.{{{}}}", exts.join(",")),
            (Some(dir), exts) => format!("{dir}/**/*.{{{}}}", exts.join(",")),
        };

        let root = self.root.clone();
        let max_bytes = self.max_bytes as u64;
        let max_results = self.max_list_entries;
        let base = dir_prefix.clone();

        let glob = tokio::task::spawn_blocking(move || {
            let ctx = safe_fs_ctx(root, max_bytes, max_bytes, max_results)?;
            ctx.glob_paths(GlobRequest {
                root_id: SAFE_FS_ROOT_ID.to_string(),
                pattern: file_glob,
            })
        })
        .await
        .map_err(|err| DittoError::Io(std::io::Error::other(format!("fs_find join error: {err}"))))?;

        let resp = match glob {
            Ok(resp) => resp,
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: format!("fs_find failed: {err}"),
                    is_error: Some(true),
                });
            }
        };

        let base_path = base.as_deref().map(PathBuf::from).unwrap_or_default();
        let mut entries = Vec::<Value>::new();

        for path in resp.matches {
            if let Some(substring) = substring.as_ref() {
                if !path.to_string_lossy().contains(substring) {
                    continue;
                }
            }
            if path_depth_under(&base_path, &path) > max_depth {
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
        if entries.len() > max_entries {
            entries.truncate(max_entries);
            truncated = true;
        }

        let out = serde_json::json!({
            "path": args.path.as_deref().unwrap_or("."),
            "entries": entries,
            "truncated": truncated,
        });

        Ok(ToolResult {
            tool_call_id: call.id,
            content: out.to_string(),
            is_error: None,
        })
    }

    async fn execute_grep(&self, call: ToolCall) -> Result<ToolResult> {
        let args: FsGrepArgs = match serde_json::from_value(call.arguments.clone()) {
            Ok(args) => args,
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: format!("invalid args: {err}"),
                    is_error: Some(true),
                });
            }
        };

        let pattern = args.pattern.trim();
        if pattern.is_empty() {
            return Ok(ToolResult {
                tool_call_id: call.id,
                content: "pattern is empty".to_string(),
                is_error: Some(true),
            });
        }

        let raw_dir = args.path.as_deref().unwrap_or("").trim().trim_matches('/');
        let dir_prefix = if raw_dir.is_empty() {
            None
        } else {
            Some(raw_dir.to_string())
        };

        let max_entries = args
            .max_entries
            .unwrap_or(self.max_list_entries)
            .min(self.max_list_entries)
            .max(1);

        let max_depth = args.max_depth.unwrap_or(10).min(64);

        let max_read_bytes = args
            .max_file_bytes
            .unwrap_or(self.max_bytes)
            .min(self.max_bytes)
            .max(1) as u64;

        let case_sensitive = args.case_sensitive.unwrap_or(true);

        let extensions: Vec<String> = args
            .extensions
            .unwrap_or_default()
            .into_iter()
            .filter_map(|value| {
                let trimmed = value.trim().trim_start_matches('.').to_ascii_lowercase();
                (!trimmed.is_empty()).then_some(trimmed)
            })
            .collect();

        let file_glob = match (dir_prefix.as_deref(), extensions.as_slice()) {
            (None, []) => "**/*".to_string(),
            (Some(dir), []) => format!("{dir}/**/*"),
            (None, [ext]) => format!("**/*.{ext}"),
            (Some(dir), [ext]) => format!("{dir}/**/*.{ext}"),
            (None, exts) => format!("**/*.{{{}}}", exts.join(",")),
            (Some(dir), exts) => format!("{dir}/**/*.{{{}}}", exts.join(",")),
        };

        let (query, regex) = if case_sensitive {
            (pattern.to_string(), false)
        } else {
            (format!("(?i){}", regex::escape(pattern)), true)
        };

        let root = self.root.clone();
        let max_write_bytes = self.max_bytes as u64;
        let max_results = self.max_list_entries;
        let base = dir_prefix.clone();

        let grep = tokio::task::spawn_blocking(move || {
            let ctx = safe_fs_ctx(root, max_read_bytes, max_write_bytes, max_results)?;
            ctx.grep(GrepRequest {
                root_id: SAFE_FS_ROOT_ID.to_string(),
                query,
                regex,
                glob: Some(file_glob),
            })
        })
        .await
        .map_err(|err| DittoError::Io(std::io::Error::other(format!("fs_grep join error: {err}"))))?;

        let resp = match grep {
            Ok(resp) => resp,
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: format!("fs_grep failed: {err}"),
                    is_error: Some(true),
                });
            }
        };

        let base_path = base.as_deref().map(PathBuf::from).unwrap_or_default();
        let mut matches = Vec::<Value>::new();
        for item in resp.matches {
            if path_depth_under(&base_path, &item.path) > max_depth {
                continue;
            }
            matches.push(serde_json::json!({
                "path": item.path.to_string_lossy(),
                "line_number": item.line,
                "line": item.text,
            }));
        }

        let mut truncated = resp.truncated;
        if matches.len() > max_entries {
            matches.truncate(max_entries);
            truncated = true;
        }

        let out = serde_json::json!({
            "path": args.path.as_deref().unwrap_or("."),
            "matches": matches,
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

        let root = self.root.clone();
        let max_bytes = self.max_bytes as u64;
        let max_results = self.max_list_entries;
        let raw_path = args.path.clone();

        let stat_value = tokio::task::spawn_blocking(move || {
            let ctx = safe_fs_ctx(root, max_bytes, max_bytes, max_results)?;
            ctx.stat(StatRequest {
                root_id: SAFE_FS_ROOT_ID.to_string(),
                path: PathBuf::from(raw_path),
            })
        })
        .await
        .map_err(|err| DittoError::Io(std::io::Error::other(format!("fs_stat join error: {err}"))))?;

        let resp = match stat_value {
            Ok(resp) => resp,
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
            "stat": {
                "type": resp.kind,
                "size_bytes": resp.size_bytes,
                "modified_ms": resp.modified_ms,
            }
        });

        Ok(ToolResult {
            tool_call_id: call.id,
            content: out.to_string(),
            is_error: None,
        })
    }
}
