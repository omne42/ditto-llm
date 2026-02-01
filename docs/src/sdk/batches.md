# Batches

Ditto 提供 `BatchClient` trait 对齐 OpenAI/OpenAI-compatible 的 `/v1/batches` 能力（feature `batches`）。

Batch 适合把大量请求离线提交给 provider，由 provider 异步执行并产出输出文件。

## 需要的 features

- OpenAI：`openai` + `batches`
- OpenAI-compatible：`openai-compatible` + `batches`

对应的 client（crate re-exports）：

- `OpenAIBatches`
- `OpenAICompatibleBatches`

## 最小示例：上传 JSONL 并创建 batch

参考 `examples/batches.rs`，核心步骤是：

1) 读取 `requests.jsonl`  
2) 通过 files API 上传（`purpose = "batch"`，`media_type = "application/jsonl"`）  
3) 创建 batch（指定 endpoint 与 completion window）  

```rust
use ditto_llm::{BatchClient, BatchCreateRequest, OpenAICompatible, OpenAICompatibleBatches};

#[tokio::main]
async fn main() -> ditto_llm::Result<()> {
    let api_key = std::env::var("OPENAI_COMPAT_API_KEY")
        .or_else(|_| std::env::var("OPENAI_API_KEY"))
        .map_err(|_| {
            ditto_llm::DittoError::InvalidResponse(
                "missing OPENAI_COMPAT_API_KEY (or fallback OPENAI_API_KEY)".into(),
            )
        })?;

    let base_url = std::env::var("OPENAI_COMPAT_BASE_URL")
        .or_else(|_| std::env::var("OPENAI_BASE_URL"))
        .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());

    let bytes = std::fs::read("requests.jsonl")?;

    let uploader = OpenAICompatible::new(api_key.clone()).with_base_url(base_url.clone());
    let input_file_id = uploader
        .upload_file_with_purpose(
            "requests.jsonl",
            bytes,
            "batch",
            Some("application/jsonl"),
        )
        .await?;

    let batches = OpenAICompatibleBatches::new(api_key).with_base_url(base_url);
    let resp = batches
        .create(BatchCreateRequest {
            input_file_id,
            endpoint: "/v1/chat/completions".to_string(),
            completion_window: "24h".to_string(),
            metadata: None,
            provider_options: None,
        })
        .await?;

    println!("batch_id={} status={:?}", resp.batch.id, resp.batch.status);
    Ok(())
}
```

## 常用操作

- 查询：`retrieve(batch_id)`
- 取消：`cancel(batch_id)`
- 列表：`list(limit, after)`

## provider_options

`BatchCreateRequest.provider_options` 支持与 `GenerateRequest.provider_options` 类似的“按 provider bucket”结构，用于传递特定 provider 的额外字段（不会被 Ditto 强类型化）。

建议：

- 只对你明确理解的字段使用 `provider_options`
- 并在审计日志/观测里记录（注意脱敏）
