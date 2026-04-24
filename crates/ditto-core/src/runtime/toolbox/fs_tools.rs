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
Directory deletion is supported with `recursive=true`.\n\
When `ignore_missing=true`, deleting a missing path succeeds and returns `deleted=false`."
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
            Err(err) => return Ok(invalid_args_tool_result(&call, err)),
        };

        Ok(finish_fs_tool_result(
            call.id,
            self.runtime
                .delete(
                    args.path,
                    args.recursive.unwrap_or(false),
                    args.ignore_missing.unwrap_or(false),
                )
                .await?,
        ))
    }

    async fn execute_mkdir(&self, call: ToolCall) -> Result<ToolResult> {
        let args: FsMkdirArgs = match serde_json::from_value(call.arguments.clone()) {
            Ok(args) => args,
            Err(err) => return Ok(invalid_args_tool_result(&call, err)),
        };

        Ok(finish_fs_tool_result(
            call.id,
            self.runtime
                .mkdir(args.path, args.create_parents, args.ignore_existing)
                .await?,
        ))
    }

    async fn execute_move(&self, call: ToolCall) -> Result<ToolResult> {
        let args: FsMoveArgs = match serde_json::from_value(call.arguments.clone()) {
            Ok(args) => args,
            Err(err) => return Ok(invalid_args_tool_result(&call, err)),
        };

        Ok(finish_fs_tool_result(
            call.id,
            self.runtime
                .move_path(args.from, args.to, args.overwrite, args.create_parents)
                .await?,
        ))
    }

    async fn execute_copy_file(&self, call: ToolCall) -> Result<ToolResult> {
        let args: FsCopyFileArgs = match serde_json::from_value(call.arguments.clone()) {
            Ok(args) => args,
            Err(err) => return Ok(invalid_args_tool_result(&call, err)),
        };

        Ok(finish_fs_tool_result(
            call.id,
            self.runtime
                .copy_file(args.from, args.to, args.overwrite, args.create_parents)
                .await?,
        ))
    }
}
