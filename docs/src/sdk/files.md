# Files（upload / list / download）

Ditto 提供一个轻量的 `FileClient` trait，用于对齐 OpenAI/OpenAI-compatible 的 `/v1/files` 能力。

## 能力范围

当前 `FileClient` 覆盖：

- upload：`upload_file_with_purpose`
- list：`list_files`
- retrieve：`retrieve_file`
- delete：`delete_file`
- download content：`download_file_content`

对应的类型：

- `FileUploadRequest`
- `FileObject`
- `FileDeleteResponse`
- `FileContent`

## 最小示例：上传并下载

```rust
use ditto_llm::{FileClient, FileUploadRequest, OpenAI};

#[tokio::main]
async fn main() -> ditto_llm::Result<()> {
    let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
        ditto_llm::DittoError::InvalidResponse("missing OPENAI_API_KEY".into())
    })?;
    let client = OpenAI::new(api_key);

    let file_id = client
        .upload_file_with_purpose(FileUploadRequest {
            filename: "hello.txt".to_string(),
            bytes: b"hello".to_vec(),
            purpose: "assistants".to_string(),
            media_type: Some("text/plain".to_string()),
        })
        .await?;

    let obj = client.retrieve_file(&file_id).await?;
    println!("uploaded: id={} bytes={}", obj.id, obj.bytes);

    let content = client.download_file_content(&file_id).await?;
    println!("downloaded: {} bytes", content.bytes.len());
    Ok(())
}
```

## 目的字段（purpose）

OpenAI 的 files API 会要求一个 `purpose` 字段（例如 `assistants`）。Ditto 不对这个字段做强约束，按 provider 的要求透传。

## 内存注意事项

`download_file_content` 会把文件内容一次性读入 `Vec<u8>`。

如果你可能下载大文件，建议：

- 在业务层限制文件大小
- 或者扩展一层“流式下载到文件/Writer”的封装（避免一次性缓冲）
