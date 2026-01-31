pub const TOOL_FS_DELETE_FILE: &str = "fs_delete_file";
pub const TOOL_FS_MKDIR: &str = "fs_mkdir";
pub const TOOL_FS_MOVE: &str = "fs_move";
pub const TOOL_FS_COPY_FILE: &str = "fs_copy_file";

pub fn fs_delete_file_tool() -> Tool {
    Tool {
        name: TOOL_FS_DELETE_FILE.to_string(),
        description: Some(
            "Delete a file or directory under the configured root directory.\n\n\
Implemented via `safe-fs-tools`.\n\
Directory deletion is supported with `recursive=true`."
                .to_string(),
        ),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path relative to the configured root directory." },
                "recursive": { "type": "boolean", "description": "If true, allow deleting directories recursively." },
                "ignore_missing": { "type": "boolean", "description": "If true, succeed when the path does not exist." },
            },
            "required": ["path"]
        }),
        strict: Some(true),
    }
}

pub fn fs_mkdir_tool() -> Tool {
    Tool {
        name: TOOL_FS_MKDIR.to_string(),
        description: Some(
            "Create a directory under the configured root directory.\n\n\
Implemented via `safe-fs-tools`."
                .to_string(),
        ),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path relative to the configured root directory." },
                "create_parents": { "type": "boolean", "description": "If true, create missing parent directories." },
                "ignore_existing": { "type": "boolean", "description": "If true, succeed when the directory already exists." },
            },
            "required": ["path"]
        }),
        strict: Some(true),
    }
}

pub fn fs_move_tool() -> Tool {
    Tool {
        name: TOOL_FS_MOVE.to_string(),
        description: Some(
            "Move (rename) a file or directory under the configured root directory.\n\n\
Implemented via `safe-fs-tools`."
                .to_string(),
        ),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "from": { "type": "string", "description": "Source path relative to the configured root directory." },
                "to": { "type": "string", "description": "Destination path relative to the configured root directory." },
                "overwrite": { "type": "boolean", "description": "If true, allow overwriting an existing destination file." },
                "create_parents": { "type": "boolean", "description": "If true, create missing parent directories for the destination path." },
            },
            "required": ["from", "to"]
        }),
        strict: Some(true),
    }
}

pub fn fs_copy_file_tool() -> Tool {
    Tool {
        name: TOOL_FS_COPY_FILE.to_string(),
        description: Some(
            "Copy a file under the configured root directory.\n\n\
Implemented via `safe-fs-tools`."
                .to_string(),
        ),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "from": { "type": "string", "description": "Source path relative to the configured root directory." },
                "to": { "type": "string", "description": "Destination path relative to the configured root directory." },
                "overwrite": { "type": "boolean", "description": "If true, allow overwriting an existing destination file." },
                "create_parents": { "type": "boolean", "description": "If true, create missing parent directories for the destination path." },
            },
            "required": ["from", "to"]
        }),
        strict: Some(true),
    }
}

#[derive(Debug, Deserialize)]
struct FsDeleteFileArgs {
    path: String,
    recursive: Option<bool>,
    ignore_missing: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct FsMkdirArgs {
    path: String,
    #[serde(default)]
    create_parents: bool,
    #[serde(default)]
    ignore_existing: bool,
}

#[derive(Debug, Deserialize)]
struct FsMoveArgs {
    from: String,
    to: String,
    #[serde(default)]
    overwrite: bool,
    #[serde(default)]
    create_parents: bool,
}

#[derive(Debug, Deserialize)]
struct FsCopyFileArgs {
    from: String,
    to: String,
    #[serde(default)]
    overwrite: bool,
    #[serde(default)]
    create_parents: bool,
}

impl FsToolExecutor {
    async fn execute_delete_file(&self, call: ToolCall) -> Result<ToolResult> {
        let args: FsDeleteFileArgs = match serde_json::from_value(call.arguments.clone()) {
            Ok(args) => args,
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: format!("invalid args: {err}"),
                    is_error: Some(true),
                });
            }
        };

        let recursive = args.recursive.unwrap_or(false);
        let ignore_missing = args.ignore_missing.unwrap_or(false);

        let root = self.root.clone();
        let max_bytes = self.max_bytes as u64;
        let max_results = self.max_list_entries;
        let raw_path = args.path.clone();

        let delete = tokio::task::spawn_blocking(move || {
            let ctx = safe_fs_ctx(root, max_bytes, max_bytes, max_results)?;
            ctx.delete_path(DeletePathRequest {
                root_id: SAFE_FS_ROOT_ID.to_string(),
                path: PathBuf::from(raw_path),
                recursive,
                ignore_missing,
            })
        })
        .await
        .map_err(|err| {
            DittoError::Io(std::io::Error::other(format!(
                "fs_delete_file join error: {err}"
            )))
        })?;

        let resp = match delete {
            Ok(resp) => resp,
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: format!("fs_delete_file failed: {err}"),
                    is_error: Some(true),
                });
            }
        };

        let out = serde_json::json!({
            "path": args.path,
            "deleted": resp.deleted,
            "type": resp.kind,
        });

        Ok(ToolResult {
            tool_call_id: call.id,
            content: out.to_string(),
            is_error: None,
        })
    }

    async fn execute_mkdir(&self, call: ToolCall) -> Result<ToolResult> {
        let args: FsMkdirArgs = match serde_json::from_value(call.arguments.clone()) {
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
        let create_parents = args.create_parents;
        let ignore_existing = args.ignore_existing;

        let mkdir_result = tokio::task::spawn_blocking(move || {
            let ctx = safe_fs_ctx(root, max_bytes, max_bytes, max_results)?;
            ctx.mkdir(MkdirRequest {
                root_id: SAFE_FS_ROOT_ID.to_string(),
                path: PathBuf::from(raw_path),
                create_parents,
                ignore_existing,
            })
        })
        .await
        .map_err(|err| DittoError::Io(std::io::Error::other(format!("fs_mkdir join error: {err}"))))?;

        let resp = match mkdir_result {
            Ok(resp) => resp,
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: format!("fs_mkdir failed: {err}"),
                    is_error: Some(true),
                });
            }
        };

        let out = serde_json::json!({
            "path": args.path,
            "created": resp.created,
        });

        Ok(ToolResult {
            tool_call_id: call.id,
            content: out.to_string(),
            is_error: None,
        })
    }

    async fn execute_move(&self, call: ToolCall) -> Result<ToolResult> {
        let args: FsMoveArgs = match serde_json::from_value(call.arguments.clone()) {
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
        let from = args.from.clone();
        let to = args.to.clone();
        let overwrite = args.overwrite;
        let create_parents = args.create_parents;

        let move_result = tokio::task::spawn_blocking(move || {
            let ctx = safe_fs_ctx(root, max_bytes, max_bytes, max_results)?;
            ctx.move_path(MovePathRequest {
                root_id: SAFE_FS_ROOT_ID.to_string(),
                from: PathBuf::from(from),
                to: PathBuf::from(to),
                overwrite,
                create_parents,
            })
        })
        .await
        .map_err(|err| DittoError::Io(std::io::Error::other(format!("fs_move join error: {err}"))))?;

        let resp = match move_result {
            Ok(resp) => resp,
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: format!("fs_move failed: {err}"),
                    is_error: Some(true),
                });
            }
        };

        let out = serde_json::json!({
            "from": args.from,
            "to": args.to,
            "moved": resp.moved,
            "type": resp.kind,
        });

        Ok(ToolResult {
            tool_call_id: call.id,
            content: out.to_string(),
            is_error: None,
        })
    }

    async fn execute_copy_file(&self, call: ToolCall) -> Result<ToolResult> {
        let args: FsCopyFileArgs = match serde_json::from_value(call.arguments.clone()) {
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
        let from = args.from.clone();
        let to = args.to.clone();
        let overwrite = args.overwrite;
        let create_parents = args.create_parents;

        let copy_result = tokio::task::spawn_blocking(move || {
            let ctx = safe_fs_ctx(root, max_bytes, max_bytes, max_results)?;
            ctx.copy_file(CopyFileRequest {
                root_id: SAFE_FS_ROOT_ID.to_string(),
                from: PathBuf::from(from),
                to: PathBuf::from(to),
                overwrite,
                create_parents,
            })
        })
        .await
        .map_err(|err| DittoError::Io(std::io::Error::other(format!("fs_copy_file join error: {err}"))))?;

        let resp = match copy_result {
            Ok(resp) => resp,
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: format!("fs_copy_file failed: {err}"),
                    is_error: Some(true),
                });
            }
        };

        let out = serde_json::json!({
            "from": args.from,
            "to": args.to,
            "copied": resp.copied,
            "bytes": resp.bytes,
        });

        Ok(ToolResult {
            tool_call_id: call.id,
            content: out.to_string(),
            is_error: None,
        })
    }
}
