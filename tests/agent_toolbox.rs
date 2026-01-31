#![cfg(feature = "agent")]

use httpmock::Method::GET;
use httpmock::MockServer;
use serde_json::json;

use ditto_llm::Result;
use ditto_llm::agent::{
    FsToolExecutor, HttpToolExecutor, ShellToolExecutor, TOOL_FS_FIND, TOOL_FS_GREP,
    TOOL_FS_LIST_DIR, TOOL_FS_READ_FILE, TOOL_FS_STAT, TOOL_FS_WRITE_FILE, TOOL_HTTP_FETCH,
    TOOL_SHELL_EXEC, ToolCall, ToolExecutor,
};

#[tokio::test]
async fn http_fetch_tool_executes_get() -> Result<()> {
    let upstream = MockServer::start();
    upstream.mock(|when, then| {
        when.method(GET).path("/hello");
        then.status(200)
            .header("content-type", "text/plain")
            .body("world");
    });

    let url = format!("{}/hello", upstream.base_url());
    let executor = HttpToolExecutor::new().with_max_response_bytes(1024);
    let call = ToolCall {
        id: "call_1".to_string(),
        name: TOOL_HTTP_FETCH.to_string(),
        arguments: json!({
            "url": url,
            "method": "GET"
        }),
    };

    let result = executor.execute(call).await?;
    assert_eq!(result.tool_call_id, "call_1");
    assert_eq!(result.is_error, None);

    let value: serde_json::Value = serde_json::from_str(&result.content)?;
    assert_eq!(value.get("status").and_then(|v| v.as_u64()), Some(200));
    assert_eq!(value.get("body").and_then(|v| v.as_str()), Some("world"));
    Ok(())
}

#[tokio::test]
async fn fs_read_file_tool_reads_within_root() -> Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("hello.txt"), "hi")?;

    let executor = FsToolExecutor::new(dir.path())?;
    let call = ToolCall {
        id: "call_1".to_string(),
        name: TOOL_FS_READ_FILE.to_string(),
        arguments: json!({
            "path": "hello.txt"
        }),
    };

    let result = executor.execute(call).await?;
    assert_eq!(result.tool_call_id, "call_1");
    assert_eq!(result.is_error, None);

    let value: serde_json::Value = serde_json::from_str(&result.content)?;
    assert_eq!(value.get("content").and_then(|v| v.as_str()), Some("hi"));
    assert_eq!(
        value.get("truncated").and_then(|v| v.as_bool()),
        Some(false)
    );
    Ok(())
}

#[tokio::test]
async fn fs_read_file_tool_rejects_path_escape() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let parent = dir.path().parent().unwrap();
    std::fs::write(parent.join("secret.txt"), "nope")?;

    let executor = FsToolExecutor::new(dir.path())?;
    let call = ToolCall {
        id: "call_1".to_string(),
        name: TOOL_FS_READ_FILE.to_string(),
        arguments: json!({
            "path": "../secret.txt"
        }),
    };

    let result = executor.execute(call).await?;
    assert_eq!(result.tool_call_id, "call_1");
    assert_eq!(result.is_error, Some(true));
    Ok(())
}

#[tokio::test]
async fn fs_write_file_tool_writes_and_respects_overwrite() -> Result<()> {
    let dir = tempfile::tempdir()?;

    let executor = FsToolExecutor::new(dir.path())?;
    let write_1 = ToolCall {
        id: "call_1".to_string(),
        name: TOOL_FS_WRITE_FILE.to_string(),
        arguments: json!({
            "path": "sub/hello.txt",
            "content": "hi",
            "create_parents": true
        }),
    };
    let result_1 = executor.execute(write_1).await?;
    assert_eq!(result_1.tool_call_id, "call_1");
    assert_eq!(result_1.is_error, None);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("sub").join("hello.txt"))?,
        "hi"
    );

    let write_2 = ToolCall {
        id: "call_2".to_string(),
        name: TOOL_FS_WRITE_FILE.to_string(),
        arguments: json!({
            "path": "sub/hello.txt",
            "content": "bye"
        }),
    };
    let result_2 = executor.execute(write_2).await?;
    assert_eq!(result_2.tool_call_id, "call_2");
    assert_eq!(result_2.is_error, Some(true));

    let write_3 = ToolCall {
        id: "call_3".to_string(),
        name: TOOL_FS_WRITE_FILE.to_string(),
        arguments: json!({
            "path": "sub/hello.txt",
            "content": "bye",
            "overwrite": true
        }),
    };
    let result_3 = executor.execute(write_3).await?;
    assert_eq!(result_3.tool_call_id, "call_3");
    assert_eq!(result_3.is_error, None);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("sub").join("hello.txt"))?,
        "bye"
    );

    Ok(())
}

#[tokio::test]
async fn fs_list_dir_lists_entries() -> Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("a.txt"), "a")?;
    std::fs::create_dir_all(dir.path().join("sub"))?;

    let executor = FsToolExecutor::new(dir.path())?;
    let call = ToolCall {
        id: "call_1".to_string(),
        name: TOOL_FS_LIST_DIR.to_string(),
        arguments: json!({}),
    };
    let result = executor.execute(call).await?;
    assert_eq!(result.tool_call_id, "call_1");
    assert_eq!(result.is_error, None);

    let value: serde_json::Value = serde_json::from_str(&result.content)?;
    let entries = value
        .get("entries")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let names: Vec<String> = entries
        .iter()
        .filter_map(|entry| {
            entry
                .get("name")
                .and_then(|n| n.as_str())
                .map(|s| s.to_string())
        })
        .collect();
    assert!(names.contains(&"a.txt".to_string()));
    assert!(names.contains(&"sub".to_string()));
    Ok(())
}

#[tokio::test]
async fn fs_find_tool_finds_nested_entries() -> Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::create_dir_all(dir.path().join("sub").join("nested"))?;
    std::fs::write(dir.path().join("root.txt"), "hi")?;
    std::fs::write(
        dir.path().join("sub").join("nested").join("hello.rs"),
        "fn main() {}",
    )?;

    let executor = FsToolExecutor::new(dir.path())?;
    let call = ToolCall {
        id: "call_1".to_string(),
        name: TOOL_FS_FIND.to_string(),
        arguments: json!({
            "pattern": "hello",
            "extensions": ["rs"],
        }),
    };
    let result = executor.execute(call).await?;
    assert_eq!(result.tool_call_id, "call_1");
    assert_eq!(result.is_error, None);

    let value: serde_json::Value = serde_json::from_str(&result.content)?;
    let entries = value
        .get("entries")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert_eq!(entries.len(), 1);
    let path = entries[0]
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(path.ends_with("sub/nested/hello.rs"));
    Ok(())
}

#[tokio::test]
async fn fs_grep_tool_finds_matching_lines() -> Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::create_dir_all(dir.path().join("sub"))?;
    std::fs::write(
        dir.path().join("sub").join("hello.txt"),
        "first line\nneedle here\nlast line\n",
    )?;
    std::fs::write(dir.path().join("sub").join("other.md"), "needle too\n")?;

    let executor = FsToolExecutor::new(dir.path())?;
    let call = ToolCall {
        id: "call_1".to_string(),
        name: TOOL_FS_GREP.to_string(),
        arguments: json!({
            "pattern": "needle",
            "extensions": ["txt"],
        }),
    };
    let result = executor.execute(call).await?;
    assert_eq!(result.tool_call_id, "call_1");
    assert_eq!(result.is_error, None);

    let value: serde_json::Value = serde_json::from_str(&result.content)?;
    let matches = value
        .get("matches")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert_eq!(matches.len(), 1);
    assert_eq!(
        matches[0].get("line_number").and_then(|v| v.as_u64()),
        Some(2)
    );
    assert_eq!(
        matches[0].get("line").and_then(|v| v.as_str()),
        Some("needle here")
    );
    let path = matches[0]
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(path.ends_with("sub/hello.txt"));
    Ok(())
}

#[tokio::test]
async fn fs_stat_tool_reports_metadata() -> Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("a.txt"), "hi")?;

    let executor = FsToolExecutor::new(dir.path())?;
    let call = ToolCall {
        id: "call_1".to_string(),
        name: TOOL_FS_STAT.to_string(),
        arguments: json!({
            "path": "a.txt"
        }),
    };
    let result = executor.execute(call).await?;
    assert_eq!(result.tool_call_id, "call_1");
    assert_eq!(result.is_error, None);

    let value: serde_json::Value = serde_json::from_str(&result.content)?;
    let stat = value.get("stat").cloned().unwrap_or_default();
    assert_eq!(stat.get("type").and_then(|v| v.as_str()), Some("file"));
    assert_eq!(stat.get("size_bytes").and_then(|v| v.as_u64()), Some(2));
    assert!(stat.get("modified_ms").is_some());
    Ok(())
}

#[tokio::test]
async fn shell_exec_tool_denies_disallowed_program() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let executor = ShellToolExecutor::new(dir.path())?;
    let call = ToolCall {
        id: "call_1".to_string(),
        name: TOOL_SHELL_EXEC.to_string(),
        arguments: json!({
            "program": "rustc",
            "args": ["--version"]
        }),
    };
    let result = executor.execute(call).await?;
    assert_eq!(result.tool_call_id, "call_1");
    assert_eq!(result.is_error, Some(true));
    Ok(())
}

#[tokio::test]
async fn shell_exec_tool_runs_allowlisted_program() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let executor = ShellToolExecutor::new(dir.path())?.with_allowed_programs(["rustc"]);
    let call = ToolCall {
        id: "call_1".to_string(),
        name: TOOL_SHELL_EXEC.to_string(),
        arguments: json!({
            "program": "rustc",
            "args": ["--version"]
        }),
    };
    let result = executor.execute(call).await?;
    assert_eq!(result.tool_call_id, "call_1");
    assert_eq!(result.is_error, None);

    let value: serde_json::Value = serde_json::from_str(&result.content)?;
    assert_eq!(value.get("exit_code").and_then(|v| v.as_i64()), Some(0));
    let stdout = value.get("stdout").and_then(|v| v.as_str()).unwrap_or("");
    let stderr = value.get("stderr").and_then(|v| v.as_str()).unwrap_or("");
    assert!(stdout.contains("rustc") || stderr.contains("rustc"));
    Ok(())
}

#[tokio::test]
async fn shell_exec_tool_writes_stdin() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let executor = ShellToolExecutor::new(dir.path())?
        .with_allowed_programs(["cat"])
        .with_max_output_bytes(1024);
    let call = ToolCall {
        id: "call_1".to_string(),
        name: TOOL_SHELL_EXEC.to_string(),
        arguments: json!({
            "program": "cat",
            "stdin": "hello"
        }),
    };

    let result = executor.execute(call).await?;
    assert_eq!(result.tool_call_id, "call_1");
    assert_eq!(result.is_error, None);

    let value: serde_json::Value = serde_json::from_str(&result.content)?;
    assert_eq!(value.get("ok").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(value.get("stdout").and_then(|v| v.as_str()), Some("hello"));
    Ok(())
}

#[tokio::test]
async fn shell_exec_tool_rejects_cwd_escape() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let executor = ShellToolExecutor::new(dir.path())?.with_allowed_programs(["rustc"]);
    let call = ToolCall {
        id: "call_1".to_string(),
        name: TOOL_SHELL_EXEC.to_string(),
        arguments: json!({
            "program": "rustc",
            "args": ["--version"],
            "cwd": "../"
        }),
    };
    let result = executor.execute(call).await?;
    assert_eq!(result.tool_call_id, "call_1");
    assert_eq!(result.is_error, Some(true));
    Ok(())
}

#[tokio::test]
async fn shell_exec_tool_truncates_large_stdout() -> Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::create_dir_all(dir.path().join("sub"))?;

    let executor = ShellToolExecutor::new(dir.path())?
        .with_allowed_programs(["rustc"])
        .with_max_output_bytes(64);
    let call = ToolCall {
        id: "call_1".to_string(),
        name: TOOL_SHELL_EXEC.to_string(),
        arguments: json!({
            "program": "rustc",
            "args": ["--print", "target-list"],
            "cwd": "sub"
        }),
    };
    let result = executor.execute(call).await?;
    assert_eq!(result.tool_call_id, "call_1");
    assert_eq!(result.is_error, None);

    let value: serde_json::Value = serde_json::from_str(&result.content)?;
    assert_eq!(value.get("ok").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(
        value.get("stdout_truncated").and_then(|v| v.as_bool()),
        Some(true)
    );
    let stdout = value.get("stdout").and_then(|v| v.as_str()).unwrap_or("");
    assert!(stdout.len() <= 64);
    Ok(())
}

#[tokio::test]
async fn http_fetch_tool_executor_errors_on_unknown_tool() -> Result<()> {
    let executor = HttpToolExecutor::new();
    let call = ToolCall {
        id: "call_1".to_string(),
        name: "not_http_fetch".to_string(),
        arguments: json!({"url": "http://example.com"}),
    };
    let result = executor.execute(call).await?;
    assert_eq!(result.is_error, Some(true));
    Ok(())
}

#[tokio::test]
async fn fs_read_file_tool_executor_errors_on_unknown_tool() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let executor = FsToolExecutor::new(dir.path())?;
    let call = ToolCall {
        id: "call_1".to_string(),
        name: "not_fs_read_file".to_string(),
        arguments: json!({"path": "hello.txt"}),
    };
    let result = executor.execute(call).await?;
    assert_eq!(result.is_error, Some(true));
    Ok(())
}
