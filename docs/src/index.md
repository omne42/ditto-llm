# Ditto-LLM Docs

Ditto-LLM 的目标是成为 **LiteLLM Proxy + Vercel AI SDK 的能力超集**，但以 Rust-first 的方式交付：

- **SDK（AI SDK-like）**：在 Rust 中以统一类型调用多个大模型提供方（providers），并显式暴露兼容性差异（Warnings）。
- **Gateway（LiteLLM-like）**：可选启用的 OpenAI-compatible HTTP gateway，支持路由/限流/预算/缓存/审计/观测等控制面能力。
- **Passthrough vs Translation**：既支持 OpenAI `/v1/*` passthrough（不变形），也支持将 OpenAI 输入/输出翻译到 native providers（translation）。

本 docs 目录定位为“可复制落地”的工程手册：你可以从快速开始直接跑起来，然后按需深入 SDK 或 Gateway 的各个主题。

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
