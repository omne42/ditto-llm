# 安装与最小用法

## Rust 版本要求

`ditto-core` 使用 Rust 2024 edition，并声明 `rust-version = 1.85`（见 `Cargo.toml`）。

## 依赖引入

最简单的方式是直接使用默认 features：

- 默认只启用稳定核心：`provider-openai-compatible + cap-llm`
- 这意味着默认构建只承诺“通用 OpenAI-compatible 文本生成/流式/工具调用”
- 其它 provider 与能力都需要显式打开

```toml
[dependencies]
ditto-core = "0.1"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

如果你希望最小化依赖，可以关闭默认 features 并按 provider pack / capability pack 精确启用。

例如：只用官方 OpenAI LLM：

```toml
[dependencies]
ditto-core = { version = "0.1", default-features = false, features = ["provider-openai", "cap-llm"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

例如：只用 OpenAI-compatible LLM：

```toml
[dependencies]
ditto-core = { version = "0.1", default-features = false, features = ["provider-openai-compatible", "cap-llm"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

> 提示：更多 feature flags 与 provider/capability 组合见「参考 → Providers 能力矩阵」。

## 最小用法：OpenAI

需要 features：`provider-openai + cap-llm`。

```rust
use ditto_core::capabilities::text::LanguageModelTextExt;
use ditto_core::contracts::Message;
use ditto_core::providers::OpenAI;

#[tokio::main]
async fn main() -> ditto_core::error::Result<()> {
    let api_key = std::env::var("OPENAI_API_KEY")
        .map_err(|_| ditto_core::error::DittoError::InvalidResponse("missing OPENAI_API_KEY".into()))?;

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

需要 features：`provider-openai-compatible + cap-llm`。

OpenAI-compatible 适配器的关键在于：

- `base_url` 指向兼容 OpenAI API 的具体 upstream node（例如 LiteLLM proxy 的 `/v1`）
- `ProviderConfig.auth` 描述这个 node 的鉴权
- `default_model` 只是这个 node 的默认模型

示例（从 `ProviderConfig` 构建）：

```rust
use ditto_core::config::{Env, ProviderAuth, ProviderConfig};
use ditto_core::providers::OpenAICompatible;

#[tokio::main]
async fn main() -> ditto_core::error::Result<()> {
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
        .generate(vec![ditto_core::contracts::Message::user("Say hi.")].into())
        .await?;
    println!("{}", out.text());
    Ok(())
}
```

## 用 ProviderConfig 统一管理 node 配置

Ditto 提供 `ProviderConfig` + `Env` 的组合，让你可以：

- 把一个上游 node 的 `base_url` / `headers` / `query` / `auth` / `default_model` 集中放进配置
- 在 SDK 与 Gateway/路由场景复用同一份 node 配置结构
- 在本地用 dotenv 内容（或网关 `--dotenv`）注入敏感信息

常见模式：

- SDK：`OpenAI::from_config(&ProviderConfig, &Env)`（或其他 provider 的 `from_config`）
- 模型发现：`list_available_models(&ProviderConfig, &Env)`

注意边界：

- `ProviderConfig` 不是 provider 品牌目录
- `ProviderConfig` 不是模型 catalog
- 请求级覆盖应使用 `GenerateRequest.provider_options`

详细字段解释见「SDK → ProviderConfig 与 Profile」。
