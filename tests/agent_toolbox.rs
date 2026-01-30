#![cfg(feature = "agent")]

use httpmock::Method::GET;
use httpmock::MockServer;
use serde_json::json;

use ditto_llm::Result;
use ditto_llm::agent::{
    FsToolExecutor, HttpToolExecutor, TOOL_FS_LIST_DIR, TOOL_FS_READ_FILE, TOOL_FS_WRITE_FILE,
    TOOL_HTTP_FETCH, ToolCall, ToolExecutor,
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
