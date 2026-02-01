# SDK（AI SDK-like）

Ditto-LLM 的 SDK 侧对标 Vercel AI SDK 的 “Core” 部分：提供统一的模型接口与常用高层 helper，但不复刻前端 UI hooks。

你会反复遇到三个关键词：

- **统一类型**：`GenerateRequest` / `GenerateResponse` / `ContentPart` / `Tool` / `Usage`
- **Streaming primitives**：`StreamChunk`，以及 `stream_text` / `stream_object` 等 fan-out helpers
- **Warnings**：把 provider 差异显式暴露给调用方（避免 silent fallback）

本章结构建议按顺序阅读：

1) 先看「安装与最小用法」把第一个请求跑通。  
2) 再看 text/stream/structured outputs/tool calling，理解 Ditto 的主线能力。  
3) 再看 Agents/Middleware，把多步 tool calling 与可组合扩展补齐。  
4) 最后再看 ProviderConfig、错误处理与测试策略，把工程化收尾。  

如果你来自 AI SDK，可以先读「迁移 → 从 Vercel AI SDK 迁移（概念对照）」。
