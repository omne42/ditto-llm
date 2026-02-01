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

    let (handle, mut text_stream) = llm.stream_text(req).await?.into_text_stream();

    while let Some(delta) = text_stream.next().await {
        print!("{}", delta?);
    }

    let final_text = handle.final_text()?.unwrap_or_default();
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

let (handle, mut full_stream) =
    llm.stream_text(GenerateRequest::from(vec![Message::user("hi")])).await?.into_full_stream();

while let Some(chunk) = full_stream.next().await {
    match chunk? {
        StreamChunk::TextDelta { text } => print!("{text}"),
        StreamChunk::Warnings { warnings } => eprintln!("warnings: {warnings:?}"),
        StreamChunk::Usage(usage) => eprintln!("usage: {usage:?}"),
        StreamChunk::FinishReason(_) => break,
        _ => {}
    }
}

let _final = handle.final_summary()?;
```

## 取消与资源释放

- **默认行为**：丢弃/Drop `StreamTextResult` 或其中任一 stream，会触发内部 task abort（通过 `AbortOnDrop`）。
- **显式 abort**：如果你想拿到一个可调用的 handle，请使用 `abortable_stream` 包装底层 `llm.stream(...)`（见 README 的 “Streaming Cancellation”）。

## 注意：有界 fan-out 与背压

`stream_text` 内部使用**有界** `mpsc::channel` fan-out 到 `text_stream` / `full_stream`，避免“未消费的 stream 导致无界缓冲”的 OOM 风险。

含义：

- 建议使用 `into_text_stream()` / `into_full_stream()` / `into_streams()` 明确你要消费哪条/哪些 stream（只启用被消费的 fan-out）。
- 如果你启用了两条 stream（`into_streams()`），请确保两者都被持续消费（例如分别 `tokio::spawn`），否则慢的一侧会通过有界缓冲对上游施加 backpressure（表现为吞吐降低/等待），而不是内存持续增长。
- 只想拿最终 `GenerateResponse` 时，优先用 `collect_stream()` / `generate_text()`。
