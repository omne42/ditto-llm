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

## Stream protocol v1（可选）

如果你需要把 Ditto 的 `StreamChunk` 接入你自己的 HTTP API（例如输出 NDJSON 或 SSE），可以启用 feature `sdk` 并使用 Ditto 的 stream protocol v1：

- `StreamEventV1`：`chunk` / `error` / `done`（带版本号 envelope）
- `stream_v1_ndjson` / `stream_v1_sse`：把 `StreamResult` 转成 HTTP 友好的字节流

详见「SDK → Stream Protocol v1（NDJSON / SSE）」。

## 常见坑

- **长输出会占用内存**：如果你选择“收集所有 chunk”，内存会随输出增长。
- **慢消费会导致缓冲增长**：例如 `stream_text` 为了简化使用，内部用无界 channel 做 fan-out；当消费端非常慢时，缓冲可能增长。
- **异常/恶意 SSE 会打爆内存**：Ditto 会对 SSE 单行与单事件设置上限；超限会以可控错误终止 stream。

本仓库倾向于把风险暴露给调用方，而不是静默丢数据。
