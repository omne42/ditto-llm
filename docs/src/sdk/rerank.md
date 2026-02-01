# Rerank

Rerank 用于对候选文档/段落按相关性重新排序，常见于 RAG 管线的“重排”阶段。

Ditto 通过 `RerankModel` trait 统一接口，目前主要实现为 Cohere Rerank（feature `cohere` + `rerank`）。

## 最小示例（Cohere）

```rust
use ditto_llm::{CohereRerank, RerankDocument, RerankRequest};

#[tokio::main]
async fn main() -> ditto_llm::Result<()> {
    let rerank = CohereRerank::new(std::env::var("COHERE_API_KEY")?)
        .with_model("rerank-english-v3.0");

    let resp = rerank
        .rerank(RerankRequest {
            query: "best rust async runtime".to_string(),
            documents: vec![
                RerankDocument::Text("Tokio is widely used.".to_string()),
                RerankDocument::Text("Rayon is for data parallelism.".to_string()),
            ],
            model: None,
            top_n: Some(2),
            provider_options: None,
        })
        .await?;

    for r in resp.ranking {
        println!("idx={} score={}", r.index, r.relevance_score);
    }
    Ok(())
}
```

## provider_options（Cohere）

在 Cohere rerank 中，`provider_options` 支持（按 `cohere` bucket）：

- `max_tokens_per_doc`
- `priority`

这些字段是 Cohere 特有的，不会被 Ditto 强类型化；传错字段可能会得到 `DittoError::InvalidResponse(...)`。

## 常见坑

- **documents 类型**：Ditto 的 `RerankDocument` 支持 `Text` 或 `Json`；Cohere 侧会把 object document 转成字符串，并产生 `Warning`（避免 silent coercion）。
- **top_n**：如果不填，provider 可能返回全部排序结果；生产建议显式限制，避免大结果集带来的成本与延迟。
