# HTTP Endpoints

Gateway 的 HTTP 路由见 `src/gateway/http/core.rs`。

> 说明：本页重点描述 Ditto Gateway 自己暴露的端点与语义；对于 `/v1/*` passthrough 的具体请求/响应格式，请参考 OpenAI-compatible API（Ditto 尽量不变形）。

## Health

- `GET /health` → `{ "status": "ok" }`

## Core metrics（JSON）

- `GET /metrics` → `ObservabilitySnapshot`（简单计数器：requests/cache_hits/rate_limited/...）

## Prometheus metrics（可选）

需要启用 feature `gateway-metrics-prometheus` 并传 `--prometheus-metrics`：

- `GET /metrics/prometheus`

## OpenAI-compatible proxy（passthrough）

- `ANY /v1/*`

行为要点：

- 如果 `virtual_keys` 为空：
  - Ditto 不会把 client 的 `Authorization` 当作 virtual key；它会随请求一起被转发到 upstream。
  - **backend 的 `headers/query_params` 永远会被注入**，并且同名 header 会覆盖 client header（例如 backend 配了 `authorization` 时，会覆盖 client `Authorization`）。
- 如果 `virtual_keys` 非空：
  - client 必须提供 virtual key（`Authorization: Bearer <vk>` / `x-ditto-virtual-key` / `x-api-key`）
  - client 的 `Authorization` 被视为 virtual key，不会转发到 upstream
  - upstream 的鉴权由 backend 的 `headers` / `query_params` 决定

### /v1/responses shim（重要）

当 upstream 不支持 `POST /v1/responses`（例如返回 404/405/501），Ditto 会自动 fallback 到 `POST /v1/chat/completions` 并返回 best-effort 的 “Responses-like” response/stream：

- 返回头会包含 `x-ditto-shim: responses_via_chat_completions`

注意：

- 非 streaming 的 shim 需要把 upstream 的 chat/completions JSON 响应完整读入内存后再转换；为避免 OOM，Ditto 对该缓冲设置了上限（当前 8MiB）。如果响应超过上限，Ditto 会返回 502，并建议改用 streaming 或直接调用 `POST /v1/chat/completions`。

这使得你可以在同一个 gateway 下兼容“只支持 chat/completions 的 OpenAI-compatible 服务”。

## OpenAI-compatible translation（可选）

启用 feature `gateway-translation` 后，以下端点可由 translation backend（配置了 `provider` 的 backend）直接服务：

- `GET /v1/models`、`GET /v1/models/*`
- `POST /v1/chat/completions`、`POST /v1/completions`
- `POST /v1/responses`、`POST /v1/responses/compact`
- `POST /v1/embeddings`
- `POST /v1/moderations`
- `POST /v1/images/generations`
- `POST /v1/audio/transcriptions`、`POST /v1/audio/translations`、`POST /v1/audio/speech`
- `POST /v1/rerank`
- `/v1/batches`（以及 retrieve/cancel）

当请求由 translation backend 处理时，响应会包含：

- `x-ditto-translation: <provider>`

## A2A Agents（LiteLLM-like，beta）

Ditto Gateway 支持 LiteLLM 风格的 A2A 协议端点（JSON-RPC 2.0），用于“通过网关调用已注册的 agent 服务”：

- `GET /a2a/:agent_id/.well-known/agent-card.json`
- `POST /a2a/:agent_id`
- `POST /a2a/:agent_id/message/send`
- `POST /a2a/:agent_id/message/stream`
- `POST /v1/a2a/:agent_id/message/send`
- `POST /v1/a2a/:agent_id/message/stream`

语义：

- 请求体是 JSON-RPC 2.0（`jsonrpc: "2.0"`），`method` 支持：
  - `message/send`（非流式）
  - `message/stream`（流式；NDJSON，一行一个 JSON）
- Ditto 会把请求代理到该 agent 的真实 `url`（来自配置 `a2a_agents[].agent_card_params.url`），并原样转发响应。
- `agent-card.json` 会将 `url` 重写为 Ditto 的 `/a2a/:agent_id`（让 A2A SDK 后续请求继续走 Ditto）。
- 当 `virtual_keys` 非空时，A2A 端点同样需要 virtual key（与 `/v1/*` 一致）。

## Control-plane demo endpoint

- `POST /v1/gateway`
  - 请求体：`GatewayRequest`（包含 `virtual_key/model/prompt/input_tokens/max_output_tokens/passthrough`）
  - 响应体：`GatewayResponse`（包含 `content/output_tokens/backend/cached`）

这个端点主要用于“演示控制面能力”（预算/限流/缓存/路由/策略）；实际生产更多使用 `/v1/*` passthrough 或 translation。

## Admin endpoints（可选）

当你通过 `--admin-token*`（write）或 `--admin-read-token*`（read-only）启用 admin token 后，会开放 `/admin/*`（只读或读写）。

常见端点：

- `GET /admin/keys`（read-only 或 write token）
- `POST /admin/keys`、`PUT|DELETE /admin/keys/:id`（需要 write token）
- `POST /admin/proxy_cache/purge`（需要 write token + proxy cache）
- `GET /admin/backends`（read-only 或 write token）
- `POST /admin/backends/:name/reset`（需要 write token + `gateway-routing-advanced`）
- `GET /admin/audit`、`GET /admin/budgets*`（需要启用 sqlite/redis store）
- `GET /admin/costs*`（需要启用 sqlite/redis store + `gateway-costing`）

详细见「Admin API」。

## 响应头（Observability）

Ditto 会尽量为每个响应附加以下头（便于排障/观测）：

- `x-ditto-backend`: 实际使用的 backend 名称
- `x-ditto-request-id`: request id（复用或生成 `x-request-id`）
- `x-ditto-cache`: `hit`（当 proxy cache 命中时）
- `x-ditto-cache-key`: cache key（当 cacheable 时）
- `x-ditto-cache-source`: `memory` 或 `redis`（当 cache 命中时）
