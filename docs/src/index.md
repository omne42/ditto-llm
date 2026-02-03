# Ditto-LLM Docs

Ditto-LLM 的目标是成为 **LiteLLM Proxy + Vercel AI SDK 的能力超集**，但以 Rust-first 的方式交付：

- **Rust SDK（AI SDK Core-like）**：在 Rust 中以统一类型调用多个大模型提供方（providers），并显式暴露兼容性差异（Warnings）。
- **Gateway（LiteLLM-like）**：可选启用的 OpenAI-compatible HTTP gateway，支持路由/限流/预算/缓存/审计/观测等控制面能力，并可选启用 MCP（`/mcp*`）/ A2A（`/a2a/*`）等协议端点。
- **JS/React Clients（AI SDK UI-like）**：面向前端/JS 侧的最小 stream 协议解析与 hook（不试图复刻完整 AI SDK UI 生态）。
- **Passthrough vs Translation**：既支持 OpenAI `/v1/*` passthrough（不变形），也支持将 OpenAI 输入/输出翻译到 native providers（translation）。

本 docs 目录定位为“可复制落地”的工程手册：你可以从快速开始直接跑起来，然后按需深入 SDK / Clients / Gateway 的各个主题。

## 为什么用 Ditto-LLM？

LLM 集成的复杂度通常来自三件事：

1) **Provider 差异**：请求字段/返回字段/流式协议/tool calling/多模态的支持程度都不同。  
2) **工程化落地**：需要可测试、可审计、可观测、可控依赖的代码与接口边界。  
3) **平台能力**：当调用方变多时，网关侧需要 keys/limits/budgets/routing/audit/metrics。

Ditto-LLM 的取舍是：

- Rust 侧以统一 traits/types 提供 AI SDK 风格的“语义一致性”，并用 `Warning` 显式暴露降级与忽略字段（避免 silent fallback）。
- 网关侧以 LiteLLM 风格的 OpenAI-compatible surface 做“平台化控制面”，并保留 passthrough 与 translation 两条路径。

## 3 个入口（按你的使用方式选）

### 入口 1：Rust SDK（AI SDK Core-like）

最小示例（文本生成）：

```rust
use ditto_llm::{LanguageModelTextExt, Message, OpenAI};

#[tokio::main]
async fn main() -> ditto_llm::Result<()> {
    let api_key = std::env::var("OPENAI_API_KEY").expect("missing OPENAI_API_KEY");
    let llm = OpenAI::new(api_key).with_model("gpt-4o-mini");
    let req = vec![Message::user("Say hello in one sentence.")].into();
    let out = llm.generate_text(req).await?;
    println!("{}", out.text);
    Ok(())
}
```

下一步：

- 「快速开始 → 方式 1：作为 Rust SDK」跑通第一个请求。
- 「SDK → 安装与最小用法」进入 SDK 主线。

### 入口 2：Gateway（LiteLLM-like）

启动本地网关：

```bash
cargo run --features gateway --bin ditto-gateway -- ./gateway.json --listen 0.0.0.0:8080
```

验证：

```bash
curl -sS http://127.0.0.1:8080/health
curl -sS http://127.0.0.1:8080/v1/models | head
```

下一步：

- 「Gateway → 运行网关」与「Gateway → 配置文件」。
- 如果你想直接复制一套可跑配置：看「Gateway → Gateway Recipes」与「模板与示例」。

### 入口 3：客户端（JS/React，AI SDK UI-like）

Ditto 提供最小 JS/React 客户端用于解析 **Stream Protocol v1**（SSE/NDJSON）并快速接入 UI。

下一步：

- 「客户端（JS/React）→ JS：Stream Protocol v1 解析」
- 「客户端（JS/React）→ React：useStreamV1」

## Provider 支持与兼容性

- 「参考 → Providers 能力矩阵」
- 如果你更关心“对标差异/缺口”：看仓库根目录 `COMPARED_TO_LITELLM_AI_SDK.md` 与 docs 的「迁移」章节。

## 模板与示例（像 AI SDK 的 templates 一样可复制）

如果你想要“拿来就跑”的参考：

- 「模板与示例」：Rust examples、Gateway docker-compose/k8s/helm、以及多语言客户端示例。

## LLM-friendly：llms.txt

如果你在用 LLM 辅助理解 Ditto（例如让它帮你改配置/写集成代码），可以直接把仓库根目录的 `llms.txt` 丢给它作为上下文入口。

`llms.txt` 包含一段“手写入口 + 约定”，并在末尾追加 **自动聚合的文档全文**（来自 `docs/src/SUMMARY.md`）。如需刷新：

```bash
cargo run --bin ditto-llms-txt -- --out llms.txt
```

## 本地构建（推荐）

本目录采用 `mdBook`（Rust 生态、零前端依赖）组织导航。

```bash
cargo install mdbook
mdbook serve docs
```

> 不想安装 mdBook 也没关系：`docs/src` 下的 Markdown 直接在 GitHub 上阅读同样可用。

## 读者路径

- 只想把模型调用集成到 Rust：从「SDK → 安装与最小用法」开始。
- 需要一个 LiteLLM 风格的 HTTP 网关：从「Gateway → 运行网关」开始。
- 关心“到底比 LiteLLM / AI SDK 多什么”：看 `COMPARED_TO_LITELLM_AI_SDK.md` 与本 docs 的「迁移」章节。
