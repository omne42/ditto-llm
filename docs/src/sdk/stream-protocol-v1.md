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

---

## 5) JS/TS client（最小实现）

仓库内提供一个最小 JS/TS client（以及 React hook），用于在浏览器/Node 侧消费 stream protocol v1：

- `packages/ditto-client`：`streamV1FromSseResponse` / `streamV1FromNdjsonResponse`
- `packages/ditto-react`：`useStreamV1`（把 stream protocol v1 映射成 React state）

它们不追求 1:1 复刻 AI SDK UI，只提供“能用、易嵌入、易排障”的最小 DX。

---

## 6) AI SDK UI Message Stream（可选）

如果你希望直接对接 Vercel AI SDK UI（例如 `@ai-sdk/react` 的 `useChat`），你需要输出 **UI Message Stream** 的 SSE 协议（与 Ditto 的 stream protocol v1 不同）。

Ditto 提供一个最小适配器（feature `sdk`）：

- `ditto_llm::sdk::http::ui_message_stream_v1_sse(stream)`：把 Ditto `StreamResult` 转为 UI Message Stream SSE（末尾以 `data: [DONE]` 结束）

输出中会包含基础的 step 边界事件（`start-step` / `finish-step`），并在 Ditto 侧额外附带一些 `data-ditto-*` 的诊断事件（例如 usage/warnings；客户端可忽略）。

HTTP 响应侧需要额外设置：

- `x-vercel-ai-ui-message-stream: v1`
- 推荐同时设置其它常用 SSE headers（`content-type` / `cache-control` / `connection` / `x-accel-buffering`），Ditto 提供了一个常量便于复用：`ditto_llm::sdk::http::UI_MESSAGE_STREAM_V1_HEADERS`
