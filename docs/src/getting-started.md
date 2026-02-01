# 快速开始

Ditto-LLM 有两种常见使用方式：

1) **作为 Rust SDK**：在你的服务里直接调用 providers（OpenAI/Anthropic/Google/OpenAI-compatible/...）。  
2) **作为 HTTP Gateway（可选 feature）**：对外提供 OpenAI-compatible 的 `/v1/*` API，并在内部做路由/缓存/预算/审计等控制面逻辑。

## 方式 1：作为 Rust SDK

添加依赖（示例）：

```toml
[dependencies]
ditto-llm = "0.1"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

最小用法（以 OpenAI 为例）：

```rust
use ditto_llm::{LanguageModelTextExt, Message, OpenAI};

#[tokio::main]
async fn main() -> ditto_llm::Result<()> {
    let llm = OpenAI::new(std::env::var("OPENAI_API_KEY")?);
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

## 方式 2：作为 HTTP Gateway（LiteLLM-like）

启动一个本地 gateway：

```bash
cargo run --features gateway --bin ditto-gateway -- ./gateway.json --listen 0.0.0.0:8080
```

然后你可以用 OpenAI-compatible 的方式调用：

```bash
curl -sS http://127.0.0.1:8080/v1/models
```

下一步建议阅读：

- 「Gateway → 配置文件（gateway.json）」了解如何配置 upstream backends / virtual keys / router。
- 「Gateway → 运行网关」了解常用 CLI 选项与部署建议。
