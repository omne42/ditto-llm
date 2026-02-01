# Streaming

## 为什么 Ditto 使用 StreamChunk

不同 provider 的 streaming 协议差异很大：

- OpenAI：SSE events（Responses / Chat Completions）
- Anthropic：SSE events（Messages）
- Google：SSE events（GenAI）
- OpenAI-compatible：兼容但存在细微差异

Ditto 的策略是把这些协议统一为 `StreamChunk` 序列，然后由上层决定如何：

- 显示给用户（SSE/NDJSON/WebSocket/CLI）
- 聚合成最终 `GenerateResponse`（`collect_stream` / `StreamCollector`）
- 记录审计与指标

## 常见坑

- **长输出会占用内存**：如果你选择“收集所有 chunk”，内存会随输出增长。
- **慢消费会导致缓冲增长**：若中间层用无界队列，慢消费可能导致内存增长（见「内存风险审查」）。

本仓库倾向于把风险暴露给调用方，而不是静默丢数据。
