# 从 Vercel AI SDK 迁移（概念对照）

AI SDK（JS/TS）强调“语义统一”：`generateText`、`streamText`、`generateObject`、tool calling、provider abstraction 等。

Ditto-LLM 在 Rust 里走的是同一条路：

- 统一语义（traits/types）
- 显式暴露差异（`Warning`）
- 以 provider adapters 承接底层 API 差异

本页给你一张“概念映射表 + 最小迁移示例”。

---

## 1) 核心 API 映射

| AI SDK（概念） | Ditto-LLM（Rust） | 说明 |
| --- | --- | --- |
| `generateText` | `LanguageModelTextExt::generate_text` | 返回 `GenerateTextResponse { text, response }` |
| `streamText` | `LanguageModelTextExt::stream_text` | 返回 `StreamTextResult { text_stream, full_stream, final_* }` |
| `generateObject` | `generate_object_json` | 结构化输出（JSON），best-effort |
| `streamObject` | `stream_object` | 流式结构化输出（数组可 `element_stream`） |
| messages | `Vec<Message>` | `Message::system/user/assistant` + `ContentPart` |
| tool calling | `Tool` / `ToolChoice` / `ContentPart::ToolCall` | 不做自动 tool loop（除非用 `agent` feature） |
| model provider | `OpenAI` / `Anthropic` / `Google` / `OpenAICompatible`… | 也可从 `ProviderConfig` 构建 |
| provider options | `ProviderOptions` | 以 `provider_options` best-effort 传递/降级 |

---

## 2) 最小示例：generateText → generate_text

AI SDK（示意）：

```ts
// const result = await generateText({ model: openai('gpt-4o-mini'), messages: [...] })
```

Ditto（Rust）：

```rust
use ditto_llm::{LanguageModelTextExt, Message, OpenAI};

#[tokio::main]
async fn main() -> ditto_llm::Result<()> {
    let llm = OpenAI::new(std::env::var("OPENAI_API_KEY")?).with_model("gpt-4o-mini");

    let req = vec![
        Message::system("You are a helpful assistant."),
        Message::user("Say hello in one sentence."),
    ]
    .into();

    let out = llm.generate_text(req).await?;
    println!("{}", out.text);
    Ok(())
}
```

---

## 3) 最小示例：streamText → stream_text

Ditto 的 `stream_text` 返回两个 stream：

- `text_stream`：只输出增量文本（最常用）
- `full_stream`：输出 `StreamChunk`（包含 usage/warnings/tool-calls 等）

示例：

```rust
use futures_util::StreamExt;
use ditto_llm::{LanguageModelTextExt, Message, OpenAI};

#[tokio::main]
async fn main() -> ditto_llm::Result<()> {
    let llm = OpenAI::new(std::env::var("OPENAI_API_KEY")?).with_model("gpt-4o-mini");
    let req = vec![Message::user("Stream one sentence.")].into();

    let mut result = llm.stream_text(req).await?;
    while let Some(delta) = result.text_stream.next().await {
        print!("{}", delta?);
    }
    println!();

    let summary = result.final_summary()?;
    println!("summary: {summary:?}");
    Ok(())
}
```

---

## 4) 结构化输出：generateObject → generate_object_json

Ditto 的结构化输出以“尽量返回合法 JSON”为目标：

- 对支持原生 JSON schema / response_format 的 provider 会尽量走原生能力
- 不支持时会降级（可能通过 tool call 或 text-json），并通过 `Warning` 说明

建议：

- 在生产里把 `warnings` 打进日志或指标
- 对关键路径在 CI 断言“不允许出现某些 Warning”

---

## 5) Tool calling：Ditto 不自动跑 tool loop

AI SDK 中常见的“自动 tool calling 循环”，在 Ditto 里默认不自动进行：

- Ditto 会把 tool call 作为 `ContentPart::ToolCall` 返回给你
- 你决定是否执行工具、如何把 tool result 回填、以及循环策略

如果你需要一个可选的 tool loop，可以看 Ditto 的 `agent` feature（`ToolLoopAgent`）。

---

## 6) Provider 配置迁移：用 ProviderConfig 收拢配置

AI SDK 通常把 base_url/api_key 分散在不同 provider package 或环境变量里。

Ditto 推荐把它们收拢为：

- `ProviderConfig`：base_url/auth/headers/query/model_whitelist/…
- `Env`：dotenv/环境变量注入

这样同一份配置可以在：

- SDK 直接调用
- Gateway translation backend
- 模型发现（`/models`）

之间复用。
