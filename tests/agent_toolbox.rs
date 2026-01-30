#![cfg(feature = "agent")]

use httpmock::Method::GET;
use httpmock::MockServer;
use serde_json::json;

use ditto_llm::Result;
use ditto_llm::agent::{
    FsToolExecutor, HttpToolExecutor, TOOL_FS_READ_FILE, TOOL_HTTP_FETCH, ToolCall, ToolExecutor,
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
