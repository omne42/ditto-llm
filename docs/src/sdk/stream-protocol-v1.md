# Stream Protocol v1（NDJSON / SSE）

Ditto 的底层 streaming 统一输出为 `StreamChunk` 序列（见「核心概念 → Streaming」）。

如果你需要把 Ditto 的 streaming 接入你自己的 HTTP 服务（而不是直接用 Gateway 的 `/v1/*` passthrough），`sdk` feature 提供了一个很轻量的“线协议”：**stream protocol v1**。

实现位置：

- 协议：`src/sdk/protocol.rs`（`StreamEventV1` / `encode_v1` / `decode_v1`）
- HTTP 编码：`src/sdk/http/stream_v1.rs`（NDJSON / SSE）

---

## 1) 事件模型：StreamEventV1

每个 event 都会被包裹在一个 envelope 里：

```json
{ "v": 1, "type": "chunk", "data": { "...StreamChunk..." } }
```

事件类型：

- `chunk`：一个 `StreamChunk`
- `error`：`{ "message": "..." }`
- `done`：流结束（Ditto 保证一定会发）

---

## 2) 两种 HTTP 输出格式

Ditto 提供两种等价但适配不同生态的输出：

### 2.1 NDJSON（`<json>\n`）

- 每行一个 JSON（`encode_line_v1`）
- 适合 CLI、日志管道、部分 Web 框架的 streaming response

入口函数：

- `ditto_llm::sdk::http::stream_v1_ndjson(stream)`

### 2.2 SSE（`data: <json>\n\n`）

- 每个 event 是一条 SSE `data:`（payload 仍是 stream protocol v1 JSON）
- 适合浏览器 EventSource、以及更接近 OpenAI 的 streaming 体验

入口函数：

- `ditto_llm::sdk::http::stream_v1_sse(stream)`

---

## 3) 错误语义（重要）

如果底层 `StreamResult` 产生 `Err(DittoError)`：

- 先发 `error`
- 再发 `done`
- 然后结束

这比“直接断流”更利于客户端做可观测的错误处理。

---

## 4) 什么时候该用它？

- 你在自建 HTTP API（非 OpenAI-compatible）但希望统一流式协议
- 你需要在网关之外把 `StreamChunk` 安全地传到另一个服务/进程

如果你只需要 OpenAI-compatible `/v1/*`，优先用 Gateway（它直接 passthrough upstream 的 SSE）。
