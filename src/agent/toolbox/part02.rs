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

        let max_file_bytes = args
            .max_file_bytes
            .unwrap_or(self.max_bytes)
            .min(self.max_bytes)
            .max(1);

        let case_sensitive = args.case_sensitive.unwrap_or(true);
        let pattern_lower = if case_sensitive {
            None
        } else {
            Some(pattern.to_lowercase())
        };

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

        let options = FsGrepOptions {
            pattern: pattern.to_string(),
            pattern_lower,
            max_entries,
            max_depth,
            max_file_bytes,
            extensions,
        };

        let root = self.root.clone();
        let grep = tokio::task::spawn_blocking(move || fs_grep_blocking(&root, &dir, options))
            .await
            .map_err(|err| {
                DittoError::Io(std::io::Error::other(format!("fs_grep join error: {err}")))
            })?;

        let (matches, truncated) = match grep {
            Ok(ok) => ok,
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: format!("fs_grep failed: {err}"),
                    is_error: Some(true),
                });
            }
        };

        let out = serde_json::json!({
            "path": if raw_path.trim().is_empty() { "." } else { raw_path },
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

#[derive(Clone, Debug)]
struct FsGrepOptions {
    pattern: String,
    pattern_lower: Option<String>,
    max_entries: usize,
    max_depth: usize,
    max_file_bytes: usize,
    extensions: Option<BTreeSet<String>>,
}

fn fs_grep_blocking(
    root: &Path,
    dir: &Path,
    options: FsGrepOptions,
) -> std::io::Result<(Vec<Value>, bool)> {
    let meta = std::fs::metadata(dir)?;
    if !meta.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "path is not a directory",
        ));
    }

    let mut matches = Vec::<Value>::new();
    let mut truncated = false;
    fs_grep_walk(root, dir, 0, &options, &mut matches, &mut truncated)?;
    Ok((matches, truncated))
}

fn fs_grep_walk(
    root: &Path,
    dir: &Path,
    depth: usize,
    options: &FsGrepOptions,
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

        if file_type.is_dir() {
            if depth < options.max_depth {
                fs_grep_walk(root, &path, depth + 1, options, out, truncated)?;
                if *truncated {
                    break;
                }
            }
            continue;
        }

        if !file_type.is_file() {
            continue;
        }

        if let Some(allowed) = options.extensions.as_ref() {
            let Some(ext) = path.extension().and_then(|value| value.to_str()) else {
                continue;
            };
            if !allowed.contains(&ext.to_ascii_lowercase()) {
                continue;
            }
        }

        let rel_path = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();

        let (content, _) = read_file_limited(&path, options.max_file_bytes)?;

        for (idx, line) in content.lines().enumerate() {
            let matches_line = if let Some(pattern_lower) = options.pattern_lower.as_ref() {
                line.to_lowercase().contains(pattern_lower)
            } else {
                line.contains(&options.pattern)
            };

            if matches_line {
                out.push(serde_json::json!({
                    "path": rel_path.clone(),
                    "line_number": idx + 1,
                    "line": line,
                }));

                if out.len() >= options.max_entries {
                    *truncated = true;
                    break;
                }
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
