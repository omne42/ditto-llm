# Moderations

Ditto 通过 `ModerationModel` trait 对齐 OpenAI/OpenAI-compatible 的 `/v1/moderations` 能力（feature `moderations`）。

## 需要的 features

- OpenAI：`openai` + `moderations`
- OpenAI-compatible：`openai-compatible` + `moderations`

对应的 client（crate re-exports）：

- `OpenAIModerations`
- `OpenAICompatibleModerations`

## 最小示例

```rust
use ditto_llm::{ModerationInput, ModerationModel, ModerationRequest, OpenAIModerations};

#[tokio::main]
async fn main() -> ditto_llm::Result<()> {
    let client = OpenAIModerations::new(std::env::var("OPENAI_API_KEY")?)
        .with_model("omni-moderation-latest");

    let resp = client
        .moderate(ModerationRequest {
            input: ModerationInput::Text("hi".to_string()),
            model: None,
            provider_options: None,
        })
        .await?;

    println!("results={:?}", resp.results);
    println!("warnings={:?}", resp.warnings);
    Ok(())
}
```

## 输入类型（ModerationInput）

`ModerationInput` 支持三种形态：

- `Text(String)`
- `TextArray(Vec<String>)`
- `Raw(serde_json::Value)`（当你需要传 provider 特有结构）

## 常见坑

- **模型选择**：moderations 通常有独立模型（例如 OpenAI 的 `omni-moderation-latest`）。你可以通过 `with_model(...)` 设置默认，也可以通过 `ModerationRequest.model` 覆盖。
- **provider_options**：仅在你明确知道 provider 支持的字段时使用；传错字段可能导致请求失败或产生 Warning。
