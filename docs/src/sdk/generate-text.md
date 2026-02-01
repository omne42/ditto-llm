# 文本生成：generate_text

`generate_text` 是一个高层 helper，对标 AI SDK 的 `generateText`：

- 输入：`GenerateRequest`
- 输出：`GenerateTextResponse { text, response }`
- 语义：单次请求（不会自动执行工具循环）

## 最小示例

```rust
use ditto_llm::{GenerateRequest, LanguageModelTextExt, Message, OpenAI};

#[tokio::main]
async fn main() -> ditto_llm::Result<()> {
    let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
        ditto_llm::DittoError::InvalidResponse("missing OPENAI_API_KEY".into())
    })?;
    let llm = OpenAI::new(api_key).with_model("gpt-4o-mini");

    let req = GenerateRequest::from(vec![
        Message::system("You are a helpful assistant."),
        Message::user("Say hello in one sentence."),
    ]);

    let out = llm.generate_text(req).await?;
    println!("text: {}", out.text);
    println!("finish_reason: {:?}", out.response.finish_reason);
    println!("usage: {:?}", out.response.usage);
    println!("warnings: {:?}", out.response.warnings);
    Ok(())
}
```

## 何时用 `generate_text`，何时用 `generate`

- 用 `generate_text`：你只关心最终文本（同时保留完整 `GenerateResponse` 以获取 usage/warnings）。
- 用 `generate`：你需要处理多模态 `ContentPart`、tool calls、reasoning、或更细粒度的 output 结构。

`GenerateResponse.content` 是一个 `Vec<ContentPart>`，可能包含：

- `ContentPart::Text`
- `ContentPart::ToolCall` / `ContentPart::ToolResult`
- `ContentPart::Reasoning`
- `ContentPart::Image` / `ContentPart::File`

## 常用请求字段（GenerateRequest）

`GenerateRequest` 兼容 OpenAI-style 字段（不同 provider 支持度不同；不支持会产生 `Warning`）：

- `model`：请求级覆盖模型
- `temperature` / `top_p`
- `max_tokens`
- `seed`
- `presence_penalty` / `frequency_penalty`
- `logprobs` / `top_logprobs`
- `user`
- `stop_sequences`
- `tools` / `tool_choice`
- `provider_options`：细化到某个 provider 的额外能力（见「SDK → ProviderConfig 与 Profile」与「SDK → 工具调用」）

## Warnings 是“契约的一部分”

Ditto 的默认策略是 best-effort 映射，并通过 `Warning` 明确告诉你：

- 哪些字段被忽略/降级
- 哪些参数被 clamp 或被丢弃（例如非有限值）
- provider 的能力差异（tool calling / streaming / json schema）

生产建议：

- 将 `warnings` 写入日志（注意脱敏），用于定位“行为不一致”的根因。
- 在 CI 里对关键路径做断言（例如：不允许出现某类 Warning）。
