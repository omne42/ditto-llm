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

- **长输出会占用内存**：如果你选择“收集所有 chunk”，内存会随输出增长；Ditto 对部分内部聚合器/缓冲区设置了上限，超限会发出 `Warning` 并截断最终聚合结果。
- **慢消费会影响吞吐**：`stream_text` / `stream_object` 的 fan-out 使用有界缓冲；当你同时启用多条 stream 但其中一条消费很慢时，会对上游施加 backpressure（表现为等待/吞吐降低），而不是无界占用内存。
- **异常/恶意 SSE 会打爆内存**：Ditto 会对 SSE 单行与单事件设置上限；超限会以可控错误终止 stream。

本仓库倾向于把风险暴露给调用方，而不是静默丢数据。
