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

### 主要差异

- AI SDK 的优势在于 JS/TS 生态 + UI hooks（React 等）；Ditto 的定位是 Rust 侧“可测试/可审计/可控依赖”的 SDK，并不复刻前端 hooks。
- Ditto 将 provider 差异通过 `Warning` 暴露，而不是静默降级。

### 仍缺（但属于“超集可选项”）

- 常用工具 wrappers（shell/fs/http 等）作为可选模块

---

## 2) Gateway 侧（对标 LiteLLM Proxy）

### 已对齐（MVP）

- OpenAI-compatible proxy：`ANY /v1/*`（含 SSE streaming）+ per-backend header injection
- virtual keys（可选启用）+ rpm/tpm limits + token/USD budget + guardrails
- OpenAI `/v1/responses` shim：当 upstream 不支持 `/v1/responses` 时，自动 fallback 到 `/v1/chat/completions` 并返回“Responses-like”（含 streaming + tool_calls）
- Translation proxy：OpenAI in/out 的 `POST /v1/chat/completions` + `POST /v1/responses` + `POST /v1/embeddings` + `POST /v1/moderations` + `POST /v1/images/generations` + `POST /v1/audio/transcriptions` + `POST /v1/audio/speech` + `POST /v1/rerank` + `/v1/batches`（backend 配置 `provider`；feature `gateway-translation`）
- admin key 管理端点（可选启用）+ state/sqlite/redis 持久化 virtual keys + budgets/audit logs
- 可选 devtools JSONL 事件日志（`--features gateway-devtools`）+ 可选 JSON logs（`--json-logs`）
- 可选 proxy cache（`--features gateway-proxy-cache`）+ 可选 Prometheus metrics（`--features gateway-metrics-prometheus`）
- 可选 proxy retry/circuit breaker（`--features gateway-routing-advanced`）
- 可选 pricing table + USD budgets（`--features gateway-costing`）
- 可选 OpenTelemetry（`--features gateway-otel`）
- request id 贯穿：响应包含 `x-ditto-request-id`（复用/生成 `x-request-id`）

### 主要差异/缺口（P0：达到“可替换 LiteLLM”）

- 路由：已支持“主动健康检查/探活”（feature `gateway-routing-advanced`），仍缺更丰富的策略（更细粒度熔断、分级 fallback、backpressure）
- 成本：缺 **真实 token 计数**（tiktoken 等价）与 **usage-based settle**（目前可选 pricing table + USD budgets，但 token 仍是预估）
- 观测：Prometheus/OTel/JSON logs 已有，但缺更丰富的指标（latency histograms、per-route tags、采样/脱敏策略）
- 代理缓存：已有 best-effort in-memory cache（非流式）；缺 redis cache、streaming cache、cache invalidation 策略
- Translation：当前覆盖 `POST /v1/chat/completions`/`POST /v1/responses`/`POST /v1/embeddings`/`POST /v1/moderations`/`POST /v1/images/generations`/`POST /v1/audio/transcriptions`/`POST /v1/audio/speech`/`POST /v1/rerank`/`/v1/batches`；其余 OpenAI 端点的 translation 仍需扩面

---

## 3) “基础功能 vs 可选附加（默认不安装）”口径

以 Cargo features 表达：

- **默认构建（不含 Gateway）**：providers + streaming/tools/embeddings（面向 SDK/库调用）
- **可选：`gateway`**：HTTP server + control-plane + OpenAI-compatible proxy
- **可选：`gateway-devtools`**：在 `gateway` 基础上启用 JSONL devtools logging（依赖 `sdk`）
- **可选（默认不装）**：`gateway-proxy-cache`、`gateway-store-sqlite`、`gateway-store-redis`、`gateway-devtools`、`gateway-otel`、`gateway-metrics-prometheus`、`gateway-routing-advanced`、`gateway-costing`
- **后续可选（规划）**：`gateway-proxy-cache-redis`、translation proxy（OpenAI in/out → native providers）等

---

## 4) CodePM 的约束（为什么 Ditto 需要“不变形”模式）

对 OpenAI Responses 场景，CodePM 需要与 Codex CLI 对齐：`/responses` 原样 items 回放（含 encrypted compaction）+ `/responses/compact`。

因此 Ditto 需要同时支持两类路径：

1. **不变形 passthrough**：OpenAI `/responses` raw 直通（用于“完全对齐 Codex CLI”）
2. **统一抽象 SDK**：为非 OpenAI 或 OpenAI-compatible 服务提供 Ditto 的统一类型体验

这两者并行存在，是“超集”而不是二选一。
