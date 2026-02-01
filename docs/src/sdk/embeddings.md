# Embeddings

Ditto 通过 `EmbeddingModel` trait 统一 embeddings 调用，并提供 `EmbeddingModelExt` 作为 AI SDK 风格别名：

- `embed_many(texts)` → `embed(texts)`
- `embed_one(text)` → `embed_single(text)`

## 最小示例

```rust
use ditto_llm::EmbeddingModelExt;

let vectors = embeddings
    .embed_many(vec!["hello".to_string(), "world".to_string()])
    .await?;

let one = embeddings.embed_one("hi".to_string()).await?;
println!("n_vectors={} dim={}", vectors.len(), one.len());
```

## 如何构建 embeddings client

不同 provider 的 embeddings client 是 feature-gated 的：

- OpenAI：`OpenAIEmbeddings`（features: `openai` + `embeddings`）
- OpenAI-compatible：`OpenAICompatibleEmbeddings`（features: `openai-compatible` + `embeddings`）
- Google：`GoogleEmbeddings`（features: `google` + `embeddings`）
- Cohere：`CohereEmbeddings`（features: `cohere` + `embeddings`）

这些类型通常支持：

- `::new(api_key)`（或对应的 provider auth）
- `with_model(...)` / `with_base_url(...)`（不同实现略有差异）
- `::from_config(&ProviderConfig, &Env)`（当你希望统一配置管理）

建议在工程里用 `ProviderConfig` 做统一管理，避免把 base_url/token scattered 到代码里。

## 常见坑

- **向量维度与模型绑定**：维度不是 Ditto 决定的，取决于 provider 与 model。
- **大批量输入**：一次塞太多文本可能触发 provider 的 size limit 或导致较长延迟；建议你在上层做分批。
- **成本与 token 计数**：embeddings 的计费与 tokenization provider 相关；若你在 Gateway 场景要做成本控制，请配合 `gateway-costing` / `gateway-tokenizer`。
