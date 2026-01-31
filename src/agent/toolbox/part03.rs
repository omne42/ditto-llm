pub const TOOL_FS_DELETE_FILE: &str = "fs_delete_file";
pub const TOOL_FS_MKDIR: &str = "fs_mkdir";
pub const TOOL_FS_MOVE: &str = "fs_move";
pub const TOOL_FS_COPY_FILE: &str = "fs_copy_file";

pub fn fs_delete_file_tool() -> Tool {
    Tool {
        name: TOOL_FS_DELETE_FILE.to_string(),
        description: Some(
            "Delete a file under the configured root directory.\n\n\
Implemented via `safe-fs-tools`.\n\
Note: directory deletion (recursive or not) is not supported yet."
                .to_string(),
        ),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path relative to the configured root directory." },
                "recursive": { "type": "boolean", "description": "Not supported yet: safe-fs-tools delete does not support directories." },
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
Not supported yet: `safe-fs-tools` currently has no mkdir API."
                .to_string(),
        ),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path relative to the configured root directory." },
                "create_parents": { "type": "boolean", "description": "Not supported yet." },
                "ignore_existing": { "type": "boolean", "description": "Not supported yet." },
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
            "Move (rename) a file under the configured root directory.\n\n\
Not supported yet: `safe-fs-tools` currently has no move/rename API."
                .to_string(),
        ),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "from": { "type": "string", "description": "Source path relative to the configured root directory." },
                "to": { "type": "string", "description": "Destination path relative to the configured root directory." },
                "overwrite": { "type": "boolean", "description": "Not supported yet." },
                "create_parents": { "type": "boolean", "description": "Not supported yet." },
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
Not supported yet: `safe-fs-tools` currently has no copy API (and cannot create new files)."
                .to_string(),
        ),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "from": { "type": "string", "description": "Source path relative to the configured root directory." },
                "to": { "type": "string", "description": "Destination path relative to the configured root directory." },
                "overwrite": { "type": "boolean", "description": "Not supported yet." },
                "create_parents": { "type": "boolean", "description": "Not supported yet." },
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
            ctx.delete_file(safe_fs_tools::DeleteRequest {
                root_id: SAFE_FS_ROOT_ID.to_string(),
                path: PathBuf::from(raw_path),
            })
        })
        .await
        .map_err(|err| {
            DittoError::Io(std::io::Error::other(format!(
                "fs_delete_file join error: {err}"
            )))
        })?;

        match delete {
            Ok(_) => {
                let out = serde_json::json!({
                    "path": args.path,
                    "deleted": true,
                    "type": "file",
                });

                Ok(ToolResult {
                    tool_call_id: call.id,
                    content: out.to_string(),
                    is_error: None,
                })
            }
            Err(err) if ignore_missing && safe_fs_is_not_found(&err) => {
                let out = serde_json::json!({
                    "path": args.path,
                    "deleted": false,
                    "type": "missing",
                });

                Ok(ToolResult {
                    tool_call_id: call.id,
                    content: out.to_string(),
                    is_error: None,
                })
            }
            Err(err) => {
                let msg = if recursive {
                    format!(
                        "fs_delete_file failed: {err} (note: safe-fs-tools delete cannot delete directories yet)"
                    )
                } else {
                    format!("fs_delete_file failed: {err}")
                };
                Ok(ToolResult {
                    tool_call_id: call.id,
                    content: msg,
                    is_error: Some(true),
                })
            }
        }
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

        Ok(ToolResult {
            tool_call_id: call.id,
            content: format!(
                "fs_mkdir is not supported yet (safe-fs-tools has no mkdir API): path={:?} create_parents={} ignore_existing={}",
                args.path, args.create_parents, args.ignore_existing
            ),
            is_error: Some(true),
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

        Ok(ToolResult {
            tool_call_id: call.id,
            content: format!(
                "fs_move is not supported yet (safe-fs-tools has no move/rename API): from={:?} to={:?} overwrite={} create_parents={}",
                args.from, args.to, args.overwrite, args.create_parents
            ),
            is_error: Some(true),
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

        Ok(ToolResult {
            tool_call_id: call.id,
            content: format!(
                "fs_copy_file is not supported yet (safe-fs-tools has no copy API): from={:?} to={:?} overwrite={} create_parents={}",
                args.from, args.to, args.overwrite, args.create_parents
            ),
            is_error: Some(true),
        })
    }
}
