# Language Model Middleware（Layer）

AI SDK 的一个核心能力是“中间件/钩子”式的可组合扩展。Ditto-LLM 在 Rust 里提供等价的抽象：`LanguageModelLayer`（以及 `LayeredLanguageModel`）。

实现位置：`src/layer.rs`。

---

## 1) 什么时候用 Layer？

适合放进 Layer 的逻辑（横切关注点）：

- 统一日志 / 指标 / tracing
- 请求参数规范化（例如：限制 max_tokens、追加 system prompt）
- 失败重试（注意 streaming 的语义）
- 路由/多模型 fallback（如果你在 SDK 侧做，而不是 Gateway）

不建议放进 Layer：

- 业务强相关的 prompt 组装（更适合在调用方做）
- tool loop（推荐用 `ToolLoopAgent` 或显式循环）

---

## 2) 最小示例：记录 warnings

```rust
use async_trait::async_trait;
use ditto_llm::{LanguageModel, LanguageModelLayer, GenerateRequest, GenerateResponse, StreamResult};

struct WarningLoggerLayer;

#[async_trait]
impl LanguageModelLayer for WarningLoggerLayer {
    async fn generate(
        &self,
        inner: &dyn LanguageModel,
        request: GenerateRequest,
    ) -> ditto_llm::Result<GenerateResponse> {
        let resp = inner.generate(request).await?;
        if !resp.warnings.is_empty() {
            eprintln!("warnings: {:?}", resp.warnings);
        }
        Ok(resp)
    }

    async fn stream(
        &self,
        inner: &dyn LanguageModel,
        request: GenerateRequest,
    ) -> ditto_llm::Result<StreamResult> {
        inner.stream(request).await
    }
}
```

使用方式：

```rust
use ditto_llm::{LanguageModelLayerExt, OpenAI};

let api_key = std::env::var("OPENAI_API_KEY")
    .map_err(|_| ditto_llm::DittoError::InvalidResponse("missing OPENAI_API_KEY".into()))?;
let llm = OpenAI::new(api_key)
    .with_model("gpt-4o-mini")
    .layer(WarningLoggerLayer);
```

---

## 3) 组合多层 Layer

`LayeredLanguageModel::with_layer` 支持链式叠加：

```rust
let llm = base.layer(L1).with_layer(L2).with_layer(L3);
```

顺序建议：

- 最外层：观测/日志（确保覆盖所有后续行为）
- 中间层：参数规范化/策略
- 最内层：provider client（OpenAI/Anthropic/...）

---

## 4) 内置：缓存 Layer（含流式回放）

Ditto 提供一个轻量的 `CacheLayer`（feature `sdk`）：用于缓存 `generate()` 的响应，以及缓存 `stream()` 的 chunk 序列并在命中时回放（replay）。

```rust
use std::time::Duration;

use ditto_llm::{CacheLayer, LanguageModelLayerExt, OpenAI};

let llm = OpenAI::new(std::env::var("OPENAI_API_KEY")?)
    .with_model("gpt-4o-mini")
    .layer(CacheLayer::new().with_ttl(Duration::from_secs(60)));
```

默认策略：

- 只做进程内缓存（不会落盘/跨进程共享）
- 命中时不会再次调用 provider
- 对单条缓存设置体积上限与 streaming chunk 上限（超过上限会跳过缓存，避免无界内存增长）
