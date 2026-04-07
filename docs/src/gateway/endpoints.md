# HTTP Endpoints

Gateway 的 HTTP 路由见 `crates/ditto-server/src/gateway/transport/http/router.rs`。

如果你要做跨仓库/跨语言对接（例如 rust-ui），可优先参考冻结的 v0.1 契约产物：

- OpenAPI：`contracts/gateway-contract-v0.1.openapi.yaml`
- Rust 类型包：`crates/ditto-gateway-contract-types`

> 说明：本页重点描述 Ditto Gateway 自己暴露的端点与语义；对于 `/v1/*` passthrough 的具体请求/响应格式，请参考 OpenAI-compatible API（Ditto 尽量不变形）。

## Health

- `GET /health` → `{ "status": "ok" }`
- `GET /ready` → `200 {"status":"ready",...}` 或 `503 {"status":"not_ready",...}`
  - `/health` 只做 liveness。
  - `/ready` 负责 readiness，会检查已配置的 state/store backend；若启用了主动 backend health check，也会纳入 readiness 判定。

## Core metrics（JSON）

- `GET /metrics` → `ObservabilitySnapshot`（简单计数器：requests/cache_hits/rate_limited/...）

## Prometheus metrics（可选）

需要启用 feature `gateway-metrics-prometheus` 并传 `--prometheus-metrics`：

- `GET /metrics/prometheus`

## OpenAI-compatible proxy（passthrough）

- `ANY /v1/*`

行为要点：

- `/v1/*` passthrough 始终 fail-close：client 必须提供 virtual key（`Authorization: Bearer <vk>` / `x-ditto-virtual-key` / `x-api-key`）。
- 如果 `virtual_keys` 为空，Ditto 不会退化成匿名 relay；而是返回 `401`，直到你显式配置可用 key。
- client 的 `Authorization` 被视为 virtual key，不会转发到 upstream。
- upstream 的鉴权由 backend 的 `headers` / `query_params` 决定；这些字段始终会注入，并可覆盖 client 同名 header。

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
- `POST /v1/responses`、`POST /v1/responses/compact`、`POST /v1/responses/input_tokens`、`GET /v1/responses/*`、`GET /v1/responses/*/input_items`、`DELETE /v1/responses/*`
- `POST /v1/embeddings`
- `POST /v1/moderations`
- `POST /v1/images/generations`
- `/v1/videos`（create/list）以及 `/v1/videos/*`（retrieve/delete/content/remix）
- `POST /v1/audio/transcriptions`、`POST /v1/audio/translations`、`POST /v1/audio/speech`
- `POST /v1/rerank`
- `/v1/batches`（以及 retrieve/cancel）

当请求由 translation backend 处理时，响应会包含：

- `x-ditto-translation: <backend>`

补充说明：

- `GET /v1/models`、`GET /v1/models/*` 只暴露“当前 virtual key 经过 router 规则后实际可路由到”的 translation models；没有被当前 key 命中的 translation backend 不会出现在模型列表里。
- `POST /v1/responses/input_tokens` 是 best-effort 估算：启用 `gateway-tokenizer` 时尽量按模型计数，否则显式返回 `unsupported_endpoint`，不会发起上游 provider 调用。
- `GET /v1/responses/*`、`GET /v1/responses/*/input_items`、`DELETE /v1/responses/*` 当前走 best-effort local store。这个 surface 不是跨实例、跨进程、跨重启的持久化 response store。
- 它只保证读写“同一 gateway instance 内由 translation `POST /v1/responses` create 生成”的 response（含 streaming create），并要求调用方使用该 gateway 返回的 gateway-scoped response id。
- 这个 local store 目前是进程内内存 LRU，最多保留 128 条 translated responses；进程重启、跨实例访问或超过容量被淘汰后，旧的 response id 都可能变成不可读/不可删。
- translated `/v1/files*`、`/v1/videos*`、`/v1/batches*` 的 list/retrieve/delete/content/remix/cancel 现在默认按 gateway-local owner tracking fail-closed：只暴露“同一 gateway instance 内由同一个 virtual key 创建或派生出来”的资源 id。
- 这意味着它们不会再直接透传共享上游资源空间；但如果进程重启、切换实例，或者资源不是经由当前 gateway 创建，后续 retrieve/list 可能返回空或 `*_not_found`。

## Anthropic Messages（compat）

Ditto Gateway 提供 Anthropic Messages API 的兼容端点（请求/响应为 Anthropic 口径），内部会转成 OpenAI-compatible 的 `/v1/chat/completions` 进行代理，然后再翻回 Anthropic message（含 streaming）。

- `POST /v1/messages`
- `POST /v1/messages/count_tokens`
- 兼容别名：
  - `POST /messages`
  - `POST /messages/count_tokens`

`/messages/count_tokens` 是 best-effort 估算（启用 `gateway-tokenizer` 时会尽量按模型计数，否则回退按 body 字节估算）。

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
- A2A 端点始终需要有效 virtual key（与 `/v1/*` 一致）；如果 `virtual_keys` 为空，则因为没有可用凭证而返回 `401`。

## MCP Gateway（LiteLLM-like）

Ditto Gateway 支持 MCP HTTP JSON-RPC proxy，并提供 `/v1/chat/completions` 与 `/v1/responses` 的 MCP tools 集成：

- MCP JSON-RPC：
  - `POST /mcp`
  - `POST /mcp/<servers>` 或 `POST /<servers>/mcp`（选择 server，例如 `local,github`）
- 便捷端点：
  - `ANY /mcp/tools/list`
  - `ANY /mcp/tools/call`
- server 选择（任选其一）：
  - header：`x-mcp-servers: local,github`
  - path：`/mcp/local,github` 或 `/local,github/mcp`
- `/mcp*` 端点始终需要有效 virtual key；如果 `virtual_keys` 为空，则因为没有可用凭证而返回 `401`。

完整说明与示例见「Gateway → MCP Gateway（/mcp + tools）」。

## Control-plane demo endpoint

- `POST /v1/gateway`
  - 请求体：`GatewayRequest`（包含 `virtual_key/model/prompt/input_tokens/max_output_tokens/passthrough`）
  - 响应体：`GatewayResponse`（包含 `content/output_tokens/backend/cached`）

这个端点主要用于“演示控制面能力”（预算/限流/缓存/路由/策略）；实际生产更多使用 `/v1/*` passthrough 或 translation。

## Admin endpoints（可选）

当你通过 `--admin-token*`（write）或 `--admin-read-token*`（read-only）启用 admin token 后，会开放 `/admin/*`（只读或读写）。`include_tokens=true` 一类 secret 导出只对 write / tenant-write admin token 放行；如果 gateway 是从持久化的 `sha256:` virtual key 状态重载起来，这些导出会返回 `409 secret_unavailable`。另外，`/admin/config/versions*` 提供的是当前进程内版本历史，重启后会从已加载配置重新建立一个新的 `bootstrap` 快照。

常见端点：

- `GET /admin/config/version`、`GET /admin/config/versions`、`GET /admin/config/versions/:version_id`（read-only 或 write token；这是当前进程内 config history；按版本明细支持 `?include_tokens=true`，但 secret 导出仅限 write / tenant-write token）
- `GET /admin/config/diff`（read-only 或 write token；`from_version_id` + `to_version_id` 对比版本差异；`include_tokens` 仅限 write / tenant-write token）
- `GET /admin/config/export`（read-only 或 write token；默认导出当前配置，支持 `version_id` + `include_tokens`；secret 导出仅限 write / tenant-write token）
- `POST /admin/config/validate`（read-only 或 write token；校验 `virtual_keys` 与可选 `router` payload（含可选 hash），不修改配置）
- `PUT /admin/config/router`（需要 write token；更新 router 配置并生成新版本；支持 `dry_run`）
- `POST /admin/config/rollback`（需要 write token；回滚 virtual keys + router 到指定版本；支持 `dry_run`）
- `GET /admin/keys`（read-only 或 write token；默认脱敏，`include_tokens=true` 仅限 write / tenant-write token）
- `POST /admin/keys`、`PUT|DELETE /admin/keys/:id`（需要 write token）
- `POST /admin/proxy_cache/purge`（需要 write token + proxy cache）
- `GET /admin/backends`（read-only 或 write token）
- `POST /admin/backends/:name/reset`（需要 write token + `gateway-routing-advanced`）
- `GET /admin/audit`、`GET /admin/budgets*`（需要启用 sqlite/pg/mysql/redis store）
- `GET /admin/costs*`（需要启用 sqlite/pg/mysql/redis store + `gateway-costing`）

详细见「Admin API」。

## 响应头（Observability）

Ditto 会尽量为每个响应附加以下头（便于排障/观测）：

- `x-ditto-backend`: 实际使用的 backend 名称
- `x-ditto-request-id`: request id（复用或生成 `x-request-id`）
- `x-ditto-cache`: `hit`（当 proxy cache 命中时）
- `x-ditto-cache-key`: cache key（当 cacheable 时）
- `x-ditto-cache-source`: `memory` 或 `redis`（当 cache 命中时）
