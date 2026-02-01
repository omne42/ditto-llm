# 安装与最小用法

## Rust 版本要求

`ditto-llm` 使用 Rust 2024 edition，并声明 `rust-version = 1.85`（见 `Cargo.toml`）。

## 依赖引入

最简单的方式是直接使用默认 features（覆盖常见 provider + streaming/tools/embeddings）：

```toml
[dependencies]
ditto-llm = "0.1"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

如果你希望最小化依赖（适合生产镜像瘦身），可以关闭默认 features 并按需开启。

例如：只用 OpenAI + streaming：

```toml
[dependencies]
ditto-llm = { version = "0.1", default-features = false, features = ["openai", "streaming"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

例如：只用 OpenAI-compatible + streaming：

```toml
[dependencies]
ditto-llm = { version = "0.1", default-features = false, features = ["openai-compatible", "streaming"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

> 提示：更多 feature flags 见「核心概念 → Feature Flags」。

## 最小用法：OpenAI

```rust
use ditto_llm::{LanguageModelTextExt, Message, OpenAI};

#[tokio::main]
async fn main() -> ditto_llm::Result<()> {
    let api_key = std::env::var("OPENAI_API_KEY")
        .map_err(|_| ditto_llm::DittoError::InvalidResponse("missing OPENAI_API_KEY".into()))?;

    let llm = OpenAI::new(api_key).with_model("gpt-4o-mini");

    let messages = vec![
        Message::system("You are a helpful assistant."),
        Message::user("What is 2+2?"),
    ];

    let resp = llm.generate_text(messages.into()).await?;
    println!("{}", resp.text);
    Ok(())
}
```

## 最小用法：OpenAI-compatible（LiteLLM / Azure / DeepSeek / Qwen / …）

需要 features：`openai-compatible`（以及你是否需要 streaming/tools/embeddings 等能力）。

OpenAI-compatible 适配器的关键在于：

- `base_url` 指向兼容 OpenAI API 的 upstream（例如 LiteLLM proxy 的 `/v1`）
- 通过 `ProviderConfig.auth` 或环境变量传入 token
- 选择一个 upstream 支持的 `model`

示例（从 `ProviderConfig` 构建）：

```rust
use ditto_llm::{Env, OpenAICompatible, ProviderAuth, ProviderConfig};

#[tokio::main]
async fn main() -> ditto_llm::Result<()> {
    let env = Env::default();
    let config = ProviderConfig {
        base_url: Some("http://127.0.0.1:4000/v1".to_string()),
        default_model: Some("gpt-4o-mini".to_string()),
        auth: Some(ProviderAuth::ApiKeyEnv {
            keys: vec!["OPENAI_COMPAT_API_KEY".to_string()],
        }),
        ..Default::default()
    };

    let llm = OpenAICompatible::from_config(&config, &env).await?;
    let out = llm
        .generate(vec![ditto_llm::Message::user("Say hi.")] .into())
        .await?;
    println!("{}", out.text());
    Ok(())
}
```

## 用 ProviderConfig 统一管理配置

Ditto 提供 `ProviderConfig` + `Env` 的组合，让你可以：

- 把 provider 的 base_url/headers/query/auth 统一放进配置
- 在 Gateway/路由场景复用同一份配置结构
- 在本地用 dotenv 内容（或网关 `--dotenv`）注入敏感信息

常见模式：

- SDK：`OpenAI::from_config(&ProviderConfig, &Env)`（或其他 provider 的 `from_config`）
- 模型发现：`list_available_models(&ProviderConfig, &Env)`

详细字段解释见「SDK → ProviderConfig 与 Profile」。
