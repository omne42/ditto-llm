# 核心概念

Ditto-LLM 的设计目标是：在 **不隐藏差异** 的前提下，让多 provider 的调用体验尽可能一致。

你需要先理解几个贯穿 SDK 与 Gateway 的概念：

- **Provider**：一个具体的大模型服务提供方（OpenAI / Anthropic / Google / OpenAI-compatible / Bedrock / Vertex / ...）。
- **Model**：provider 里的具体模型（例如 `gpt-4.1` / `claude-3-5-sonnet-20241022`）。
- **Request/Response**：Ditto 的统一请求/响应类型（`GenerateRequest` / `GenerateResponse` 等）。
- **Streaming**：以 `StreamChunk` 的形式把生成过程增量输出，并允许上层决定如何消费。
- **Warnings**：显式记录“兼容性降级”与“字段被忽略/被钳制”的原因，避免 silent fallback。

建议按顺序阅读本章的其他页面。
