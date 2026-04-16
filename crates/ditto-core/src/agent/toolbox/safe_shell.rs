use std::io::{Read, Write};
use std::ffi::{OsStr, OsString};
use std::time::Instant;

use omne_execution_gateway::{
    ExecGateway, ExecRequest, ExecutionOutcome, GatewayPolicy, PreparedChild,
    resolve_bare_program_path_for_execution,
};

#[::async_trait::async_trait]
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

        let stdin = args.stdin.clone();
        if let Some(stdin) = stdin.as_deref()
            && stdin.len() > self.max_stdin_bytes
        {
            return Ok(ToolResult {
                tool_call_id: call.id,
                content: format!("stdin exceeds max_stdin_bytes ({})", self.max_stdin_bytes),
                is_error: Some(true),
            });
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

        let resolved_program = match self.resolve_allowlisted_program_path(&program) {
            Ok(path) => path,
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

        let gateway = self.exec_gateway_for_program(&resolved_program);
        let capability = gateway.capability_report();
        let request = ExecRequest::new(
            &resolved_program,
            args.args.iter().cloned().map(OsString::from),
            &cwd,
            capability.policy_default_isolation,
            &self.root,
        )
        .with_declared_mutation(false);

        let (_event, prepared) = gateway.prepare_command(&request);
        let prepared = match prepared {
            Ok(prepared) => {
                let prepared = prepared.with_piped_stdout().with_piped_stderr();
                if stdin.is_some() {
                    prepared.with_piped_stdin()
                } else {
                    prepared
                }
            }
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: err.to_string(),
                    is_error: Some(true),
                });
            }
        };

        let mut child = match prepared.spawn() {
            Ok(child) => child,
            Err(err) => {
                return Ok(ToolResult {
                    tool_call_id: call.id,
                    content: format!("failed to spawn: {err}"),
                    is_error: Some(true),
                });
            }
        };

        let stdin_task = stdin.map(|stdin| {
            let child_stdin = match child.take_stdin() {
                Some(stdin_handle) => stdin_handle,
                None => {
                    return tokio::task::spawn_blocking(|| {
                        Err::<(), std::io::Error>(std::io::Error::other(
                            "shell_exec missing stdin pipe",
                        ))
                    });
                }
            };

            tokio::task::spawn_blocking(move || write_sync_stdin(child_stdin, stdin))
        });

        let stdout = child.take_stdout().ok_or_else(|| {
            DittoError::Io(std::io::Error::other("shell_exec missing stdout pipe"))
        })?;
        let stderr = child.take_stderr().ok_or_else(|| {
            DittoError::Io(std::io::Error::other("shell_exec missing stderr pipe"))
        })?;

        let max_output_bytes = self.max_output_bytes;
        let stdout_task =
            tokio::task::spawn_blocking(move || read_sync_limited_bytes(stdout, max_output_bytes));
        let stderr_task =
            tokio::task::spawn_blocking(move || read_sync_limited_bytes(stderr, max_output_bytes));

        let (exit_code, wait_error, timed_out) = wait_for_prepared_child(child, timeout).await;

        let stdin_error = match stdin_task {
            Some(task) => match task.await {
                Ok(Ok(())) => None,
                Ok(Err(err)) => Some(format!("failed to write stdin: {err}")),
                Err(err) => Some(format!("stdin join error: {err}")),
            },
            None => None,
        };

        let (stdout_result, stderr_result) = tokio::join!(stdout_task, stderr_task);
        let (stdout_bytes, stdout_truncated) = match stdout_result {
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
        let (stderr_bytes, stderr_truncated) = match stderr_result {
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

        let ok =
            exit_code == Some(0) && !timed_out && wait_error.is_none() && stdin_error.is_none();

        let is_error = !ok || timed_out || wait_error.is_some() || stdin_error.is_some();

        let mut out = serde_json::json!({
            "program": program,
            "args": args.args,
            "cwd": args.cwd.unwrap_or_else(|| ".".to_string()),
            "stdin_provided": args.stdin.is_some(),
            "ok": ok,
            "exit_code": exit_code,
            "stdout": String::from_utf8_lossy(&stdout_bytes).to_string(),
            "stderr": String::from_utf8_lossy(&stderr_bytes).to_string(),
            "stdout_truncated": stdout_truncated,
            "stderr_truncated": stderr_truncated,
            "timed_out": timed_out,
        });
        if let Some(obj) = out.as_object_mut() {
            if let Some(wait_error) = wait_error {
                obj.insert("wait_error".to_string(), Value::String(wait_error));
            }
            if let Some(stdin_error) = stdin_error {
                obj.insert("stdin_error".to_string(), Value::String(stdin_error));
            }
        }

        Ok(ToolResult {
            tool_call_id: call.id,
            content: out.to_string(),
            is_error: if is_error { Some(true) } else { None },
        })
    }
}

impl ShellToolExecutor {
    fn resolve_allowlisted_program_path(&self, program: &str) -> std::result::Result<PathBuf, String> {
        let path = resolve_bare_program_path_for_execution(program.as_ref())
            .ok_or_else(|| format!("failed to resolve program in PATH: {program}"))?;
        let canonical_path = std::fs::canonicalize(&path)
            .map_err(|err| format!("failed to canonicalize program {}: {err}", path.display()))?;
        ensure_canonical_program_matches_request(program, &canonical_path)?;
        Ok(canonical_path)
    }

    fn exec_gateway_for_program(&self, program: &std::path::Path) -> ExecGateway {
        ExecGateway::with_policy(GatewayPolicy {
            allow_isolation_none: true,
            non_mutating_program_allowlist: vec![program.display().to_string()],
            ..GatewayPolicy::default()
        })
    }
}

fn ensure_canonical_program_matches_request(
    program: &str,
    canonical_path: &std::path::Path,
) -> std::result::Result<(), String> {
    match canonical_path.file_name().and_then(OsStr::to_str) {
        Some(file_name) if file_name == program => Ok(()),
        _ => Err(format!(
            "program resolution for {program} points at opaque command launchers or aliases: {}",
            canonical_path.display()
        )),
    }
}

async fn wait_for_prepared_child(
    mut child: PreparedChild,
    timeout: Duration,
) -> (Option<i32>, Option<String>, bool) {
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return (status.code(), None, false),
            Ok(None) => {}
            Err(err) => {
                return (
                    err.completed_status().and_then(std::process::ExitStatus::code),
                    Some(err.to_string()),
                    false,
                );
            }
        }

        if start.elapsed() >= timeout {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10).min(timeout.saturating_sub(start.elapsed())))
            .await;
    }

    let _ = child.kill();
    match tokio::task::spawn_blocking(move || child.wait()).await {
        Ok(outcome) => map_execution_outcome(outcome, true),
        Err(err) => (None, Some(format!("wait join error: {err}")), true),
    }
}

fn map_execution_outcome(
    outcome: ExecutionOutcome,
    timed_out: bool,
) -> (Option<i32>, Option<String>, bool) {
    let (_event, result) = outcome.into_parts();
    match result {
        Ok(status) => (status.code(), None, timed_out),
        Err(err) => (
            err.completed_status().and_then(std::process::ExitStatus::code),
            Some(err.to_string()),
            timed_out,
        ),
    }
}

fn write_sync_stdin(mut stdin: std::process::ChildStdin, input: String) -> std::io::Result<()> {
    stdin.write_all(input.as_bytes())?;
    stdin.flush()?;
    Ok(())
}

fn read_sync_limited_bytes(
    mut reader: impl Read,
    max_bytes: usize,
) -> std::io::Result<(Vec<u8>, bool)> {
    let mut out = Vec::<u8>::new();
    let mut truncated = false;
    let mut buf = [0u8; 8192];

    loop {
        let read = reader.read(&mut buf)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_program_name_must_match_requested_name() {
        assert!(ensure_canonical_program_matches_request("cat", std::path::Path::new("/bin/cat"))
            .is_ok());
        assert!(
            ensure_canonical_program_matches_request("rustc", std::path::Path::new("/bin/rustup"))
                .is_err()
        );
    }
}
