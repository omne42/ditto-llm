# Ditto-LLM TODO（目标：成为 LiteLLM + AI SDK 的能力超集）

本文是 `ditto-llm` 的能力口径 + 全量待办清单。

目标（Superset）：

- **SDK（AI SDK-like）**：覆盖 Vercel AI SDK 的核心开发体验（generate/stream/tools/structured output/agent loop），并保持 Rust 侧的可测试/可审计特性（warnings、严格的错误边界、最小依赖）。
- **Gateway（LiteLLM-like）**：覆盖 LiteLLM Proxy 的核心平台能力（OpenAI-compatible HTTP surface、virtual keys、limits/budget/cache/routing、日志/指标），并提供“passthrough/translation”两种模式。
- **不变形直通（OpenAI Responses）**：对 OpenAI `/responses`（含 `/responses/compact`）提供 raw passthrough，保证 items round-trip（CodePM parity 要求）。

在 CodePM monorepo 内的相关背景文档（从本文件位置出发的相对路径）：

- `../../docs/ditto_llm.md`
- `../../docs/special_directives.md`
- `../../docs/model_routing.md`

本仓库内相关文档：

- `COMPARED_TO_LITELLM_AI_SDK.md`（对比口径）

---

## 0) 原则与范围（先钉死“超集”的口径）

“超集”不是指 1:1 复刻 UI/hooks 或某个云产品的全部企业功能，而是指 **能力覆盖 + 组合方式**：

- Ditto 必须能同时以 4 种形态工作：
  1. **SDK**：库内直接调用 provider adapters（OpenAI/Anthropic/Google/...）
  2. **Gateway**：提供 OpenAI-compatible HTTP 服务（面向多语言/多团队）
  3. **Passthrough Proxy**：payload 不变形直通（对接 OpenAI-compatible upstream）
  4. **Translation Proxy**：把 OpenAI 请求翻译到 non-OpenAI provider，再翻译回 OpenAI 响应（减少“必须先上 LiteLLM”依赖）

硬约束（长期不变）：

- OpenAI `/responses` 直连场景必须支持 raw passthrough + history compaction（用于 agent loop / prompt cache key / items replay）。
- 差异必须显式：SDK 层用 `Warning`；Gateway 层用 OpenAI-style error shape + `x-ditto-*` headers。

---

## 1) Done（当前仓库已具备的能力）

### 1.1 SDK（AI SDK-like）

- [x] Unified types + traits：`LanguageModel` / `EmbeddingModel` + `Message`/`ContentPart`/`Tool`/`StreamChunk`/`Warning`
- [x] Text helpers：`generate_text` / `stream_text`
- [x] Structured output：`generate_object_json` / `stream_object`
- [x] Tool loop agent（feature `agent`）
- [x] Providers：OpenAI `/responses`，OpenAI-compatible `/chat/completions`，Anthropic `/messages`，Google GenAI，Cohere（部分能力 feature-gated）
- [x] Raw OpenAI Responses passthrough + `/responses/compact`（items round-trip，CodePM parity）
- [x] SDK utilities（feature `sdk`）：stream protocol v1、telemetry、devtools JSONL logger、MCP tool adapter

### 1.2 Gateway（LiteLLM-like，feature `gateway`）

- [x] Control-plane primitives：virtual keys、rpm/tpm limits、token budget、simple cache、routing rules、guardrails
- [x] Routing (basic)：weighted backends（`default_backends` / `rules[].backends`）+ proxy network-error fallback
- [x] HTTP server：`/health`、`/metrics`、`/admin/keys`、`POST /v1/gateway`
- [x] OpenAI-compatible passthrough proxy：`ANY /v1/*`（含 SSE streaming）+ per-backend header injection
- [x] OpenAI `/v1/responses` shim：当 upstream 不支持 `/v1/responses` 时，自动 fallback 到 `/v1/chat/completions` 并返回“Responses-like”输出（含 streaming + tool_calls）
- [x] State file persistence：`--state <path>` 持久化 admin virtual-key mutations（`GatewayStateFile`）
- [x] Optional sqlite persistence：`--sqlite <path>`（feature `gateway-store-sqlite`）
- [x] Optional redis persistence：`--redis <url>`（feature `gateway-store-redis`）
- [x] Optional devtools JSONL logging（`--devtools`，feature `gateway-devtools`）
- [x] Optional JSON logs（`--json-logs`）
- [x] Optional proxy cache for OpenAI-compatible passthrough（`--proxy-cache*`，feature `gateway-proxy-cache`）
- [x] Optional Prometheus metrics（`--prometheus-metrics`，feature `gateway-metrics-prometheus`）
- [x] Optional proxy retry/circuit breaker（`--proxy-retry*`/`--proxy-circuit-breaker*`，feature `gateway-routing-advanced`）
- [x] Optional pricing table + USD budgets（`--pricing-litellm <path>`，feature `gateway-costing`）
- [x] Optional OpenTelemetry tracing（`--otel*`，feature `gateway-otel`）

---

## 2) 能力清单（“超集”路径拆开：SDK / Gateway / 互操作）

> checkbox 口径：**我们是否需要 + 是否已实现**。如果“不做”，就写清楚原因/替代方案，不留悬案。

### 2.1 SDK：AI SDK parity（Rust 口径）

- [x] `generate` / `stream`：text delta + tool_call delta + finish_reason + usage + response_id + warnings
- [x] Abort/cancel primitives：`StreamAbortHandle`（`abortable_stream` / `LanguageModelExt::stream_abortable`）
- [x] stream 聚合器：`collect_stream(StreamResult) -> CollectedStream`
- [x] Structured output：OpenAI 原生 JSON schema；其它 provider 走 tool-call enforced JSON（并显式 warnings）
- [ ] “UI/HTTP 适配层”（AI SDK UI-like）：提供 `axum`/`tower` helpers，把 Ditto 的 stream protocol 以 SSE/NDJSON 形式输出（Rust 侧提供 primitives，而非 React hooks）
- [ ] 常用工具 wrappers（可选模块）：shell/fs/http 等“本地工具”封装（对齐 AI SDK `ToolLoopAgent` 的可组合体验）

### 2.2 Gateway：LiteLLM parity（OpenAI HTTP surface）

- [x] Passthrough proxy endpoints：`ANY /v1/*`（含 `/v1/responses`、`/v1/chat/completions`、`/v1/embeddings`、`/v1/models`）
- [x] `/v1/responses` shim（OpenAI-compatible upstream）：当 upstream 仅支持 `/v1/chat/completions` 时，gateway 自动 fallback 并返回“Responses-like”（best-effort，仍属于变形路径）
- [x] Translation proxy endpoints：用 Ditto provider adapters 实现“OpenAI in/out”的 `POST /v1/responses` + `POST /v1/chat/completions` + `POST /v1/embeddings` + `POST /v1/moderations` + `POST /v1/images/generations` + `POST /v1/audio/transcriptions` + `POST /v1/audio/speech` + `POST /v1/rerank` + `/v1/batches`（feature `gateway-translation`；不依赖上游 OpenAI-compatible 服务）
- [x] 路由（basic）：weighted backends（seeded）+ network-error fallback
- [x] 路由（advanced）：retry + circuit breaker（feature `gateway-routing-advanced`；health checks 暂不包含主动探测）
- [ ] 成本口径：真实 token 计数（tiktoken 等价）+ usage-based settle（目前为 bytes/token 预估 + 可选 pricing table + USD budget）
- [x] 存储（basic）：virtual keys 持久化（`--state` file / `--sqlite`）
- [x] 存储（advanced）：budgets / audit logs 持久化（sqlite/redis 可选，支持多进程/多副本）
- [x] 观测（core）：request_id 贯穿（`x-ditto-request-id`/`x-request-id`）
- [ ] 观测（extended）
  - [x] structured JSON logs（`--json-logs`）
  - [x] OpenTelemetry traces（feature `gateway-otel`）
  - [x] per-key metrics 标签（Prometheus counters by `virtual_key_id`/`model`）
- [x] Proxy caching（non-streaming deterministic requests；streaming 默认不开启）
- [ ] 更丰富的 guardrails/策略扩展（regex、PII、schema 校验、allow/deny lists、per-route policy）

### 2.3 Interop：不变形与多协议互转

- [x] OpenAI `/responses` raw passthrough items round-trip（含 `/responses/compact`）
- [ ] OpenAI-compatible ↔ Claude Code / Gemini CLI 格式互转（仅当 CodePM/上层需要；否则保持 scope 小）
- [x] Gateway “passthrough vs translation” per-route 策略（同一个 gateway 可混用两种模式；以 backend config 的 `base_url` vs `provider` 区分）

---

## 3) Roadmap（按优先级推进）

### P0（让 Gateway 达到 LiteLLM 的“可替换”）

- [x] Gateway 代理路径：基础持久化（virtual keys via `--state` or `--sqlite`）
- [x] Gateway 代理路径：持久化存储（virtual keys / budgets / audit logs；sqlite/redis 可选）
- [x] 路由：retry/fallback + weighted load balancing + passive health（circuit breaker）
- [ ] 路由：主动健康检查/探活（active probing）+ backpressure
- [ ] 成本：token 计数 + pricing + spend（按 project/user/key）+ 预算控制（USD 口径）
- [x] 观测：structured logs + OpenTelemetry + per-key metrics tags（request_id 已完成；logs/otel 已做）
- [x] Proxy caching（非流式请求；并提供显式绕过）

### P1（让 Ditto 成为“超集”，而不是“替代品”）

- [x] Translation proxy：把 `POST /v1/responses` / `POST /v1/chat/completions` 翻译到 native providers（Anthropic/Google/Bedrock/Vertex；feature `gateway-translation`）
- [ ] UI/HTTP 适配层：Rust 侧提供 AI SDK UI 类似的 streaming primitives（可独立 crate）

### P2（扩面端点）

- [x] Gateway translation：`/audio/transcriptions` 与 `/audio/speech`
- [x] Gateway translation：`/batches`
- [x] Gateway translation：`/rerank`（`/images/generations` 与 `/moderations` 已完成）
- [ ] 更强的策略/缓存/背压（backpressure）控制（适配高并发与长连接 streaming）

---

## 4) 验证（本仓库内可复制）

```bash
cd ditto-llm

cargo fmt -- --check
cargo test --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
```

跑 examples（需要相应环境变量）：

```bash
cargo run --example openai_compatible
cargo run --example multimodal -- <image_path> <pdf_path>
```
