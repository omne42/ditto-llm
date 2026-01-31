pub const TOOL_FS_DELETE_FILE: &str = "fs_delete_file";
pub const TOOL_FS_MKDIR: &str = "fs_mkdir";
pub const TOOL_FS_MOVE: &str = "fs_move";
pub const TOOL_FS_COPY_FILE: &str = "fs_copy_file";

pub fn fs_delete_file_tool() -> Tool {
    Tool {
        name: TOOL_FS_DELETE_FILE.to_string(),
        description: Some(
            "Delete a file or directory under the configured root directory.".to_string(),
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
        description: Some("Create a directory under the configured root directory.".to_string()),
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
            "Move (rename) a file or directory under the configured root directory.".to_string(),
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
        description: Some("Copy a file under the configured root directory.".to_string()),
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

        let rel = match Self::validate_relative_path(&args.path) {
            Ok(rel) => {
                let is_root = rel
                    .components()
                    .all(|component| matches!(component, std::path::Component::CurDir));
                if is_root {
                    return Ok(ToolResult {
                        tool_call_id: call.id,
                        content: "refusing to delete the root directory".to_string(),
                        is_error: Some(true),
                    });
                }
                rel.to_path_buf()
            }
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: err,
                    is_error: Some(true),
                });
            }
        };

        let recursive = args.recursive.unwrap_or(false);
        let ignore_missing = args.ignore_missing.unwrap_or(false);

        let root = self.root.clone();
        let delete = tokio::task::spawn_blocking(move || {
            fs_delete_path_blocking(&root, &rel, recursive, ignore_missing)
        })
        .await
        .map_err(|err| {
            DittoError::Io(std::io::Error::other(format!(
                "fs_delete_file join error: {err}"
            )))
        })?;

        let (deleted, kind) = match delete {
            Ok(ok) => ok,
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
            "deleted": deleted,
            "type": kind,
        });

        Ok(ToolResult {
            tool_call_id: call.id,
            content: out.to_string(),
            is_error: None,
        })
    }
}

fn fs_delete_path_blocking(
    root: &Path,
    rel: &Path,
    recursive: bool,
    ignore_missing: bool,
) -> std::io::Result<(bool, String)> {
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
    let canonical_parent = match std::fs::canonicalize(&joined_parent) {
        Ok(canonical) => canonical,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound && ignore_missing => {
            return Ok((false, "missing".to_string()));
        }
        Err(err) => return Err(err),
    };
    if !canonical_parent.starts_with(&root) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "path escapes root",
        ));
    }

    let target = canonical_parent.join(file_name);
    if !target.starts_with(&root) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "path escapes root",
        ));
    }

    let meta = match std::fs::symlink_metadata(&target) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound && ignore_missing => {
            return Ok((false, "missing".to_string()));
        }
        Err(err) => return Err(err),
    };

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

    if file_type.is_dir() {
        if !recursive {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "path is a directory; set recursive=true to delete directories",
            ));
        }
        std::fs::remove_dir_all(&target)?;
        return Ok((true, kind.to_string()));
    }

    std::fs::remove_file(&target)?;
    Ok((true, kind.to_string()))
}

impl FsToolExecutor {
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

        let raw_path = args.path.clone();
        let rel = match Self::validate_relative_path(&args.path) {
            Ok(rel) => {
                let is_root = rel
                    .components()
                    .all(|component| matches!(component, std::path::Component::CurDir));
                if is_root {
                    return Ok(ToolResult {
                        tool_call_id: call.id,
                        content: "refusing to create the root directory".to_string(),
                        is_error: Some(true),
                    });
                }
                rel.to_path_buf()
            }
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: err,
                    is_error: Some(true),
                });
            }
        };

        let create_parents = args.create_parents;
        let ignore_existing = args.ignore_existing;

        let root = self.root.clone();
        let mkdir = tokio::task::spawn_blocking(move || {
            fs_mkdir_blocking(&root, &rel, create_parents, ignore_existing)
        })
        .await
        .map_err(|err| DittoError::Io(std::io::Error::other(format!("fs_mkdir join error: {err}"))))?;

        let (created, created_path) = match mkdir {
            Ok(ok) => ok,
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: format!("fs_mkdir failed: {err}"),
                    is_error: Some(true),
                });
            }
        };

        let out = serde_json::json!({
            "path": raw_path,
            "created": created,
            "absolute_path": created_path.to_string_lossy(),
        });

        Ok(ToolResult {
            tool_call_id: call.id,
            content: out.to_string(),
            is_error: None,
        })
    }
}

fn fs_mkdir_blocking(
    root: &Path,
    rel: &Path,
    create_parents: bool,
    ignore_existing: bool,
) -> std::io::Result<(bool, PathBuf)> {
    let root = std::fs::canonicalize(root)?;

    for component in rel.components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "parent dir segments are not allowed",
            ));
        }
    }

    let dir_name = rel.file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no filename")
    })?;
    let rel_parent = rel.parent().unwrap_or_else(|| Path::new(""));

    let canonical_parent = ensure_dir_under_root(&root, rel_parent, create_parents)?;

    let target = canonical_parent.join(dir_name);
    if !target.starts_with(&root) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "path escapes root",
        ));
    }

    let meta = match std::fs::symlink_metadata(&target) {
        Ok(meta) => Some(meta),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(err),
    };

    if let Some(meta) = meta {
        if meta.file_type().is_symlink() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "refusing to create directory through symlink",
            ));
        }
        if !meta.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "path exists and is not a directory",
            ));
        }
        if ignore_existing {
            return Ok((false, target));
        }
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "directory exists",
        ));
    }

    std::fs::create_dir(&target)?;
    Ok((true, target))
}

impl FsToolExecutor {
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

        let raw_from = args.from.clone();
        let raw_to = args.to.clone();

        let from_rel = match Self::validate_relative_path(&raw_from) {
            Ok(rel) => {
                let is_root = rel
                    .components()
                    .all(|component| matches!(component, std::path::Component::CurDir));
                if is_root {
                    return Ok(ToolResult {
                        tool_call_id: call.id,
                        content: "refusing to move the root directory".to_string(),
                        is_error: Some(true),
                    });
                }
                rel.to_path_buf()
            }
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: err,
                    is_error: Some(true),
                });
            }
        };

        let to_rel = match Self::validate_relative_path(&raw_to) {
            Ok(rel) => {
                let is_root = rel
                    .components()
                    .all(|component| matches!(component, std::path::Component::CurDir));
                if is_root {
                    return Ok(ToolResult {
                        tool_call_id: call.id,
                        content: "refusing to move the root directory".to_string(),
                        is_error: Some(true),
                    });
                }
                rel.to_path_buf()
            }
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: err,
                    is_error: Some(true),
                });
            }
        };

        let overwrite = args.overwrite;
        let create_parents = args.create_parents;

        let root = self.root.clone();
        let move_result = tokio::task::spawn_blocking(move || {
            fs_move_path_blocking(&root, &from_rel, &to_rel, overwrite, create_parents)
        })
        .await
        .map_err(|err| DittoError::Io(std::io::Error::other(format!("fs_move join error: {err}"))))?;

        let (moved, kind) = match move_result {
            Ok(ok) => ok,
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: format!("fs_move failed: {err}"),
                    is_error: Some(true),
                });
            }
        };

        let out = serde_json::json!({
            "from": raw_from,
            "to": raw_to,
            "moved": moved,
            "type": kind,
        });

        Ok(ToolResult {
            tool_call_id: call.id,
            content: out.to_string(),
            is_error: None,
        })
    }
}

fn fs_move_path_blocking(
    root: &Path,
    from: &Path,
    to: &Path,
    overwrite: bool,
    create_parents: bool,
) -> std::io::Result<(bool, String)> {
    let root = std::fs::canonicalize(root)?;

    for component in from.components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "parent dir segments are not allowed",
            ));
        }
    }

    for component in to.components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "parent dir segments are not allowed",
            ));
        }
    }

    let from_name = from.file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "from path has no filename")
    })?;
    let to_name = to.file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "to path has no filename")
    })?;

    let from_parent_rel = from.parent().unwrap_or_else(|| Path::new(""));
    let to_parent_rel = to.parent().unwrap_or_else(|| Path::new(""));

    let from_parent = ensure_dir_under_root(&root, from_parent_rel, false)?;
    let to_parent = ensure_dir_under_root(&root, to_parent_rel, create_parents)?;

    let source = from_parent.join(from_name);
    if !source.starts_with(&root) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "source escapes root",
        ));
    }

    let destination = to_parent.join(to_name);
    if !destination.starts_with(&root) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "destination escapes root",
        ));
    }

    let meta = std::fs::symlink_metadata(&source)?;
    let kind = if meta.file_type().is_file() {
        "file"
    } else if meta.file_type().is_dir() {
        "dir"
    } else if meta.file_type().is_symlink() {
        "symlink"
    } else {
        "other"
    };

    if source == destination {
        return Ok((false, kind.to_string()));
    }

    match std::fs::symlink_metadata(&destination) {
        Ok(meta) => {
            if meta.is_dir() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "destination exists and is a directory",
                ));
            }
            if !overwrite {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "destination exists",
                ));
            }
            std::fs::remove_file(&destination)?;
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }

    std::fs::rename(&source, &destination)?;
    Ok((true, kind.to_string()))
}

impl FsToolExecutor {
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

        let raw_from = args.from.clone();
        let raw_to = args.to.clone();

        let from_rel = match Self::validate_relative_path(&raw_from) {
            Ok(rel) => {
                let is_root = rel
                    .components()
                    .all(|component| matches!(component, std::path::Component::CurDir));
                if is_root {
                    return Ok(ToolResult {
                        tool_call_id: call.id,
                        content: "refusing to copy the root directory".to_string(),
                        is_error: Some(true),
                    });
                }
                rel.to_path_buf()
            }
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: err,
                    is_error: Some(true),
                });
            }
        };

        let to_rel = match Self::validate_relative_path(&raw_to) {
            Ok(rel) => {
                let is_root = rel
                    .components()
                    .all(|component| matches!(component, std::path::Component::CurDir));
                if is_root {
                    return Ok(ToolResult {
                        tool_call_id: call.id,
                        content: "refusing to copy the root directory".to_string(),
                        is_error: Some(true),
                    });
                }
                rel.to_path_buf()
            }
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: err,
                    is_error: Some(true),
                });
            }
        };

        let overwrite = args.overwrite;
        let create_parents = args.create_parents;

        let root = self.root.clone();
        let copy_result = tokio::task::spawn_blocking(move || {
            fs_copy_file_blocking(&root, &from_rel, &to_rel, overwrite, create_parents)
        })
        .await
        .map_err(|err| {
            DittoError::Io(std::io::Error::other(format!(
                "fs_copy_file join error: {err}"
            )))
        })?;

        let (copied, bytes) = match copy_result {
            Ok(ok) => ok,
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: format!("fs_copy_file failed: {err}"),
                    is_error: Some(true),
                });
            }
        };

        let out = serde_json::json!({
            "from": raw_from,
            "to": raw_to,
            "copied": copied,
            "bytes": bytes,
        });

        Ok(ToolResult {
            tool_call_id: call.id,
            content: out.to_string(),
            is_error: None,
        })
    }
}

fn fs_copy_file_blocking(
    root: &Path,
    from: &Path,
    to: &Path,
    overwrite: bool,
    create_parents: bool,
) -> std::io::Result<(bool, u64)> {
    let root = std::fs::canonicalize(root)?;

    for component in from.components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "parent dir segments are not allowed",
            ));
        }
    }

    for component in to.components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "parent dir segments are not allowed",
            ));
        }
    }

    let from_name = from.file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "from path has no filename")
    })?;
    let to_name = to.file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "to path has no filename")
    })?;

    let from_parent_rel = from.parent().unwrap_or_else(|| Path::new(""));
    let to_parent_rel = to.parent().unwrap_or_else(|| Path::new(""));

    let from_parent = ensure_dir_under_root(&root, from_parent_rel, false)?;
    let to_parent = ensure_dir_under_root(&root, to_parent_rel, create_parents)?;

    let source = from_parent.join(from_name);
    if !source.starts_with(&root) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "source escapes root",
        ));
    }

    let destination = to_parent.join(to_name);
    if !destination.starts_with(&root) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "destination escapes root",
        ));
    }

    if source == destination {
        return Ok((false, 0));
    }

    let meta = std::fs::symlink_metadata(&source)?;
    if meta.file_type().is_symlink() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "refusing to copy symlinks",
        ));
    }
    if meta.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "path is a directory",
        ));
    }
    if !meta.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "path is not a file",
        ));
    }

    match std::fs::symlink_metadata(&destination) {
        Ok(meta) => {
            if meta.is_dir() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "destination exists and is a directory",
                ));
            }
            if !overwrite {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "destination exists",
                ));
            }
            std::fs::remove_file(&destination)?;
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }

    let bytes = std::fs::copy(&source, &destination)?;
    Ok((true, bytes))
}
