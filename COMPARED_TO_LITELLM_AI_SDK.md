# Ditto-LLM vs LiteLLM Proxy vs Vercel AI SDK（相同点/不同点/缺口）

目标口径：`ditto-llm` 要成为 **LiteLLM Proxy + Vercel AI SDK 的能力超集**，但通过“分层 + feature gating”保证默认构建保持小而清晰：

- **SDK（AI SDK-like）**：Rust 里直接调用 providers（统一类型、warnings、严格错误边界）
- **Gateway（LiteLLM-like）**：OpenAI-compatible HTTP surface + control-plane（virtual keys/limits/budget/cache/routing/logs）
- **Passthrough（不变形）**：OpenAI `/responses` raw passthrough（items round-trip + `/responses/compact`）
- **Translation（超集项）**：OpenAI in/out → native providers（减少必须先上 LiteLLM 的依赖）

本文件是“对比 + 缺口”的口径，具体 Roadmap 见 `TODO.md`。

---

## 1) SDK 侧（对标 Vercel AI SDK）

### 已对齐

- 统一抽象：`LanguageModel` / `EmbeddingModel` + `Message`/`ContentPart`/`Tool`/`Warning`
- `generate_text` / `stream_text`（AI SDK `generateText`/`streamText`）
- 结构化输出：`generate_object_json` / `stream_object`（JSON schema / tool-call enforced）
- 多模态输入（images/PDF）：统一为 `ContentPart`
- 可选 agent loop：`feature=agent`
- Stream protocol v1 HTTP 适配层：以 SSE/NDJSON 输出（feature `sdk`）
- 内存安全：stream fan-out 有界缓冲 + 聚合/缓冲区 max-bytes 上限（超限发出 `Warning`，避免 OOM）

### 主要差异

- AI SDK 的优势在于 JS/TS 生态 + UI hooks（React 等）；Ditto 的定位是 Rust 侧“可测试/可审计/可控依赖”的 SDK，并不复刻前端 hooks。
- Ditto 将 provider 差异通过 `Warning` 暴露，而不是静默降级。

### 可选超集项（部分已实现）

- 常用工具 wrappers（shell/fs/http 等）作为可选模块（✅ 已提供 `http_fetch` + `shell_exec` + `safe-fs-tools` 驱动的 `fs_read_file`/`fs_find`/`fs_grep`/`fs_write_file`/`fs_delete_file`/`fs_list_dir`/`fs_stat`/`fs_mkdir`/`fs_move`/`fs_copy_file`）
- “模板/脚手架”生态：AI SDK 的强项是大量可复制模板；Ditto 以 docs/工程化补齐（✅ `deploy/docker-compose.yml`、`deploy/k8s/*`、`deploy/helm/*` + Node/Python/Go 调用示例；仍可继续扩展更多模板）。
- AI SDK UI/RSC 生态：Ditto 不复刻 hooks/RSC，但提供基于 stream protocol v1 的最小 JS/TS client + React hooks（✅ `packages/ditto-client`、`packages/ditto-react`）。
- 应用侧缓存范式：基于 `LanguageModelLayer` 的缓存 middleware + 流式回放（✅ `CacheLayer`）。
- 生态适配器：LangChain/LlamaIndex 等协议级桥接（可选附加）

---

## 2) Gateway 侧（对标 LiteLLM Proxy）

### 已对齐（MVP）

- OpenAI-compatible proxy：`ANY /v1/*`（含 SSE streaming）+ per-backend header/query-param injection
- virtual keys（可选启用）+ rpm/tpm limits + token/USD budget + guardrails
- OpenAI `/v1/responses` shim：当 upstream 不支持 `/v1/responses` 时，自动 fallback 到 `/v1/chat/completions` 并返回“Responses-like”（含 streaming + tool_calls）
- Translation proxy：OpenAI in/out 的 `GET /v1/models` + `GET /v1/models/*` + `POST /v1/chat/completions` + `POST /v1/completions` + `POST /v1/responses` + `POST /v1/responses/compact` + `POST /v1/embeddings` + `POST /v1/moderations` + `POST /v1/images/generations` + `POST /v1/audio/transcriptions` + `POST /v1/audio/translations` + `POST /v1/audio/speech` + `/v1/files*` + `POST /v1/rerank` + `/v1/batches`（backend 配置 `provider`；feature `gateway-translation`）
- admin key 管理端点（可选启用）+ state/sqlite/redis 持久化 virtual keys + budgets/audit logs
- 可选 devtools JSONL 事件日志（`--features gateway-devtools`）+ 可选 JSON logs（`--json-logs`）
- 可选 proxy cache（`--features gateway-proxy-cache`）+ 可选 Prometheus metrics（`--features gateway-metrics-prometheus`）
- 默认内存安全：proxy 对非 SSE 响应默认流式转发；仅在体积可控时才有界缓冲用于 proxy cache 或 `usage` 结算；`usage` 缓冲上限由 `--proxy-usage-max-body-bytes` 控制并与 cache 上限解耦
- 可选 proxy retry/circuit breaker（`--features gateway-routing-advanced`）
- 可选 pricing table + USD budgets（`--features gateway-costing`）
- 可选 OpenTelemetry（`--features gateway-otel`）
- request id 贯穿：响应包含 `x-ditto-request-id`（复用/生成 `x-request-id`）

### 主要差异/缺口（P0：达到“可替换 LiteLLM”）

- 路由：已支持“主动健康检查/探活”（feature `gateway-routing-advanced`），仍缺更丰富的策略（更细粒度熔断、分级 fallback、更细粒度 backpressure）
- 成本：支持 **tiktoken-based token 计数**（best-effort；feature `gateway-tokenizer`）+ **usage-based settle**（非 streaming 响应优先按 `usage` 结算；否则回退预估）+ LiteLLM prompt cache 成本（read: `cache_read_input_token_cost` + `cached_tokens`；write: `cache_creation_input_token_cost` + `cache_creation_input_tokens`；tiered: `*_above_*_tokens`）+ LiteLLM service tier 成本（`*_priority`/`*_flex` + request `service_tier`）+ proxy `model_map` 计费对齐（按 backend 映射后的 model 选价）；支持按 tenant/project/user 归因与共享预算（`virtual_keys[].tenant_id/project_id/user_id` + `tenant_budget/project_budget/user_budget` + `/admin/budgets/tenants|projects|users` + `/admin/costs/tenants|projects|users`）
- 观测：Prometheus/OTel/JSON logs 已有，但缺更丰富的指标（latency histograms、per-route tags、采样/脱敏策略）
- 代理缓存：已有 best-effort in-memory cache（非流式）；支持在 `--redis` 场景下把 proxy cache 写入 Redis 以跨实例复用，并提供 admin purge（按 cache key 或全量）；仍缺 streaming cache 与更细粒度的 invalidation 策略
- Translation：当前覆盖 `GET /v1/models`/`GET /v1/models/*`/`POST /v1/chat/completions`/`POST /v1/completions`/`POST /v1/responses`/`POST /v1/responses/compact`/`POST /v1/embeddings`/`POST /v1/moderations`/`POST /v1/images/generations`/`POST /v1/audio/transcriptions`/`POST /v1/audio/translations`/`POST /v1/audio/speech`/`/v1/files*`/`POST /v1/rerank`/`/v1/batches`；其余 OpenAI 端点的 translation 仍需扩面
- 企业平台能力：仍缺完整 RBAC/SSO/SCIM 与更复杂的组织/审批流；但已具备可用的“平台 MVP”（✅ RBAC-lite：read-only/write admin token + tenant-scoped admin token；✅ tenant 维度归因与 shared budgets/limits；✅ per-route Redis 分布式限流（加权滑窗 60s）；✅ 审计保留期 + HTTP 导出（JSONL/CSV）+ hash-chain + verifier；✅ Docker Compose / K8s / Helm + Grafana dashboard + PrometheusRule）。仍缺：IP-based/令牌桶限流、对象存储审计导出（S3/GCS/WORM/签名）、配置版本化/灰度/回滚等更完整平台能力。
- 平台化生态：✅ Secret Manager 集成（`secret://...`）+ ✅ 最小 Admin UI；仍缺：更多 guardrails/alerting/logging destinations 的官方 adapters（可先做通用扩展点 + 少量官方适配）
- 平台扩展项：LiteLLM 侧还有 A2A agent gateway、MCP gateway 等方向；Ditto 当前偏 SDK 工具/协议适配，后续可以按真实企业需求扩到网关侧

---

## 3) “基础功能 vs 可选附加（默认不安装）”口径

以 Cargo features 表达：

- **默认构建（不含 Gateway）**：providers + streaming/tools/embeddings（面向 SDK/库调用）
- **可选：`gateway`**：HTTP server + control-plane + OpenAI-compatible proxy
- **可选：`gateway-devtools`**：在 `gateway` 基础上启用 JSONL devtools logging（依赖 `sdk`）
- **可选（默认不装）**：`gateway-proxy-cache`、`gateway-store-sqlite`、`gateway-store-redis`、`gateway-devtools`、`gateway-otel`、`gateway-metrics-prometheus`、`gateway-routing-advanced`、`gateway-costing`、`gateway-tokenizer`
- **后续可选（规划）**：`gateway-proxy-cache-redis`、translation proxy（OpenAI in/out → native providers）等

---

## 4) Codex CLI 对齐约束（为什么 Ditto 需要“不变形”模式）

对 OpenAI Responses 场景，Ditto 需要与 Codex CLI 对齐：`/responses` 原样 items 回放（含 encrypted compaction）+ `/responses/compact`。

因此 Ditto 需要同时支持两类路径：

1. **不变形 passthrough**：OpenAI `/responses` raw 直通（用于“完全对齐 Codex CLI”）
2. **统一抽象 SDK**：为非 OpenAI 或 OpenAI-compatible 服务提供 Ditto 的统一类型体验

这两者并行存在，是“超集”而不是二选一。
