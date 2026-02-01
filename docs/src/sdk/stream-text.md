# 文本流式：stream_text

`stream_text` 对标 AI SDK 的 `streamText`：它把底层 provider 的 streaming 统一成两个可消费的 stream：

- `text_stream`: `Stream<Item = Result<String>>`（只输出文本增量）
- `full_stream`: `Stream<Item = Result<StreamChunk>>`（完整 chunk，包括 warnings/usage/tool deltas 等）

并在内部使用 `StreamCollector` 维护最终 `GenerateResponse`，供你在 stream 结束后读取 `final_text()` / `final_summary()`。

## 最小示例：消费 text deltas

```rust
use futures_util::StreamExt;
use ditto_llm::{GenerateRequest, LanguageModelTextExt, Message, OpenAI};

#[tokio::main]
async fn main() -> ditto_llm::Result<()> {
    let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
        ditto_llm::DittoError::InvalidResponse("missing OPENAI_API_KEY".into())
    })?;
    let llm = OpenAI::new(api_key).with_model("gpt-4o-mini");

    let req = GenerateRequest::from(vec![
        Message::system("You are a helpful assistant."),
        Message::user("Stream a short poem about Rust."),
    ]);

    let mut result = llm.stream_text(req).await?;

    while let Some(delta) = result.text_stream.next().await {
        print!("{}", delta?);
    }

    let final_text = result.final_text()?.unwrap_or_default();
    println!("\nfinal={final_text}");
    Ok(())
}
```

## 进阶：消费 full stream（StreamChunk）

当你需要：

- 观察 `Warning` 的出现时机
- 统计 usage（某些 provider 会在 stream 末尾给出）
- 处理 tool calling（streaming tool deltas）

可以消费 `full_stream`：

```rust
use futures_util::StreamExt;
use ditto_llm::{GenerateRequest, LanguageModelTextExt, Message, OpenAI, StreamChunk};

let mut result = llm.stream_text(GenerateRequest::from(vec![Message::user("hi")])).await?;
while let Some(chunk) = result.full_stream.next().await {
    match chunk? {
        StreamChunk::TextDelta { text } => print!("{text}"),
        StreamChunk::Warnings { warnings } => eprintln!("warnings: {warnings:?}"),
        StreamChunk::Usage(usage) => eprintln!("usage: {usage:?}"),
        StreamChunk::FinishReason(_) => break,
        _ => {}
    }
}
```

## 取消与资源释放

- **默认行为**：丢弃/Drop `StreamTextResult` 或其中任一 stream，会触发内部 task abort（通过 `AbortOnDrop`）。
- **显式 abort**：如果你想拿到一个可调用的 handle，请使用 `abortable_stream` 包装底层 `llm.stream(...)`（见 README 的 “Streaming Cancellation”）。

## 注意：无界缓冲的内存增长风险

当前实现内部使用 `mpsc::unbounded_channel` fan-out `text_stream` 与 `full_stream`。

含义：

- 如果你的消费者**处理很慢**或**完全不消费**，内存可能持续增长（更像 backpressure 问题，而不是“泄漏”）。
- 生产环境建议为上游请求设置合理的超时/并发控制，并确保下游持续消费或及时 abort。
