# Ditto-LLM TODO（目标：成为 LiteLLM + AI SDK 的能力超集）

本文是 `ditto-llm` 的能力口径 + 全量待办清单。

目标（Superset）：

- **SDK（AI SDK-like）**：覆盖 Vercel AI SDK 的核心开发体验（generate/stream/tools/structured output/agent loop），并保持 Rust 侧的可测试/可审计特性（warnings、严格的错误边界、最小依赖）。
- **Gateway（LiteLLM-like）**：覆盖 LiteLLM Proxy 的核心平台能力（OpenAI-compatible HTTP surface、virtual keys、limits/budget/cache/routing、日志/指标），并提供“passthrough/translation”两种模式。
- **不变形直通（OpenAI Responses）**：对 OpenAI `/responses`（含 `/responses/compact`）提供 raw passthrough，保证 items round-trip。

分层路线（L0/L1/L2）：

- **L0（本仓库）**：互转内核与直接调用（SDK/adapters/protocol）。
- **L1（本仓库）**：Gateway/Proxy 平台能力（API/routing/control-plane + 轻量治理补充）。
- **L2（独立仓库）**：企业闭环平台（prompt/eval/agent eval/组织治理与审批流）。
- **边界约束**：L2 依赖 L1 稳定契约；L1 可独立部署运行，不依赖 L2。

本仓库内相关文档：

- `README.md`（概览 + 用法）
- `PROVIDERS.md`（provider/capability matrix）
- `COMPARED_TO_LITELLM_AI_SDK.md`（对比口径）
- `docs/src/roadmap/module-boundaries.md`（代码目录边界与迁移顺序）

---

## 架构重构状态（2026-03-09）

- [x] `foundation` / `contracts` / `llm_core` 已稳定落位到 `crates/ditto-core`；拆分前顶层 `src/` facade 已删除。
- [x] `capabilities/` 已从单文件 facade 改成真实目录模块。
- [x] config editing 已迁入 `crates/ditto-server/src/config_editing.rs`，`crates/ditto-core/src/config/` 仅保留 schema / parsing / auth / routing 语义边界。
- [x] 旧 `profile` / `compat/profile` 兼容层已退休；当前 provider truth 以 `crates/ditto-core/src/catalog/`、`crates/ditto-core/src/runtime_registry/` 与 `crates/ditto-core/src/providers/openai_compat_profile.rs` 为准。
- [x] `catalog` 的共享契约已下沉到 `crates/ditto-core/src/contracts/`，`catalog <-> config` 的直接环依赖已被打断。
- [x] `gateway` 已物理落位到 `domain/`、`application/`、`adapters/`、`transport/http/`；根目录旧模块仅保留兼容 shim。
- [x] `gateway` 已不再依赖 `include!` 做模块拼装。
- [x] `gateway` 根导出已改为优先指向新层级路径，旧平铺模块以 `#[doc(hidden)]` 兼容形式保留。

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
- [x] Raw OpenAI Responses passthrough + `/responses/compact`（items round-trip）
- [x] SDK utilities（feature `sdk`）：stream protocol v1、telemetry、devtools JSONL logger、MCP tool adapter

### 1.2 Gateway（LiteLLM-like，feature `gateway`）

- [x] Control-plane primitives：virtual keys、rpm/tpm limits、token budget、simple cache、routing rules、guardrails
- [x] Routing (basic)：weighted backends（`default_backends` / `rules[].backends`）+ proxy network-error fallback
- [x] HTTP server：`/health`、`/metrics`、`/admin/keys`、`POST /v1/gateway`
- [x] OpenAI-compatible passthrough proxy：`ANY /v1/*`（含 SSE streaming）+ per-backend header/query-param injection
- [x] LiteLLM 兼容：`/key/*` key management endpoints + 常用 OpenAI 路由无 `/v1` 前缀别名（例如 `/chat/completions`）
- [x] OpenAI `/v1/responses` shim：当 upstream 不支持 `/v1/responses` 时，自动 fallback 到 `/v1/chat/completions` 并返回“Responses-like”输出（含 streaming + tool_calls）
- [x] State file persistence：`--state <path>` 持久化 admin virtual-key mutations（`GatewayStateFile`）
- [x] Optional sqlite persistence：`--sqlite <path>`（feature `gateway-store-sqlite`）
- [x] Optional redis persistence：`--redis <url>`（feature `gateway-store-redis`）
- [x] Optional devtools JSONL logging（`--devtools`，feature `gateway-devtools`）
- [x] Optional JSON logs（`--json-logs`）
- [x] Optional proxy cache for OpenAI-compatible passthrough（`--proxy-cache*`，feature `gateway-proxy-cache`）
- [x] Optional Prometheus metrics（`--prometheus-metrics`，feature `gateway-metrics-prometheus`）
- [x] Optional proxy retry/circuit breaker/health checks（`--proxy-retry*`/`--proxy-circuit-breaker*`/`--proxy-health-check*`，feature `gateway-routing-advanced`）
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
- [x] “UI/HTTP 适配层”（AI SDK UI-like）：提供 `sdk::http::{stream_v1_sse, stream_v1_ndjson}`，把 Ditto 的 stream protocol v1 以 SSE/NDJSON 输出（Rust 侧提供 primitives，而非 React hooks）
- [x] 常用工具 wrappers（可选模块）：shell/fs/http 等“本地工具”封装（对齐 AI SDK `ToolLoopAgent` 的可组合体验）
  - [x] `http_fetch` tool + executor（feature `agent`）
  - [x] `fs_read_file` tool + executor（feature `agent`，`safe-fs-tools`，限制 root）
  - [x] `fs_find` tool + executor（feature `agent`，`safe-fs-tools` glob；files-only）
  - [x] `fs_grep` tool + executor（feature `agent`，`safe-fs-tools` grep）
  - [x] `fs_write_file` tool + executor（feature `agent`，`safe-fs-tools` `write_file`；支持创建新文件；支持 `create_parents`；需要 `overwrite=true` 才会覆盖已存在文件）
  - [x] `fs_delete_file` tool + executor（feature `agent`，`safe-fs-tools` `delete`；支持 `recursive=true` 删除目录；支持 `ignore_missing=true`）
  - [x] `fs_list_dir` tool + executor（feature `agent`，`safe-fs-tools` `list_dir`）
  - [x] `fs_stat` tool + executor（feature `agent`，`safe-fs-tools` `stat`）
  - [x] `fs_mkdir` tool + executor（feature `agent`，`safe-fs-tools` `mkdir`）
  - [x] `fs_move` tool + executor（feature `agent`，`safe-fs-tools` `move_path`）
  - [x] `fs_copy_file` tool + executor（feature `agent`，`safe-fs-tools` `copy_file`）
  - [x] `shell_exec` tool（feature `agent`，allowlist + cwd 限制 root）

### 2.2 Gateway：LiteLLM parity（OpenAI HTTP surface）

- [x] Passthrough proxy endpoints：`ANY /v1/*`（含 `/v1/responses`、`/v1/chat/completions`、`/v1/embeddings`、`/v1/models`）
- [x] `/v1/responses` shim（OpenAI-compatible upstream）：当 upstream 仅支持 `/v1/chat/completions` 时，gateway 自动 fallback 并返回“Responses-like”（best-effort，仍属于变形路径）
- [x] Translation proxy endpoints：用 Ditto provider adapters 实现“OpenAI in/out”的 `GET /v1/models` + `GET /v1/models/*` + `POST /v1/responses` + `POST /v1/responses/compact` + `POST /v1/chat/completions` + `POST /v1/completions` + `POST /v1/embeddings` + `POST /v1/moderations` + `POST /v1/images/generations` + `POST /v1/audio/transcriptions` + `POST /v1/audio/translations` + `POST /v1/audio/speech` + `POST /v1/rerank` + `/v1/batches`（feature `gateway-translation`；不依赖上游 OpenAI-compatible 服务）
- [x] 路由（basic）：weighted backends（seeded）+ network-error fallback
- [x] 路由（advanced）：retry + circuit breaker + active health checks（feature `gateway-routing-advanced`）
- [x] 成本口径：真实 token 计数（tiktoken 等价；feature `gateway-tokenizer`，失败回退估算）
- [x] 成本口径：usage-based settle（非 streaming 响应优先使用 `usage`；否则回退预估）
- [x] 存储（basic）：virtual keys 持久化（`--state` file / `--sqlite`）
- [x] 存储（advanced）：budgets / audit logs 持久化（sqlite/redis 可选，支持多进程/多副本）
- [x] 观测（core）：request_id 贯穿（`x-ditto-request-id`/`x-request-id`）
- [x] 观测（extended）
  - [x] structured JSON logs（`--json-logs`）
  - [x] OpenTelemetry traces（feature `gateway-otel`）
  - [x] per-key metrics 标签（Prometheus counters by `virtual_key_id`/`model`）
  - [x] per-backend metrics（Prometheus per-backend inflight gauge + request latency histogram）
- [x] Proxy caching（non-streaming deterministic requests；streaming 默认不开启）
- [x] 更丰富的 guardrails/策略扩展（regex、PII、schema 校验、per-route policy）
  - [x] model allow/deny lists（exact match 或 `prefix*`）
  - [x] banned regex patterns（feature `gateway`，配置 `guardrails.banned_regexes`）
  - [x] basic PII blocking（email/ssn；配置 `guardrails.block_pii`）
  - [x] per-route guardrails override（router rules by `model_prefix`）
  - [x] schema validation（request shape）

### 2.3 Interop：不变形与多协议互转

- [x] OpenAI `/responses` raw passthrough items round-trip（含 `/responses/compact`）
- [x] OpenAI-compatible ↔ Claude Code / Gemini CLI 格式互转
- [x] Gateway “passthrough vs translation” per-route 策略（同一个 gateway 可混用两种模式；以 backend config 的 `base_url` vs `provider` 区分）

---

## 3) Roadmap（按优先级推进）

### Backlog（未完成项，必须可追踪）

- [x] 观测：统一的采样/脱敏策略（logs/audit/devtools/metrics）
  - DoD：
    - 提供可配置的脱敏规则（headers/query/json-path/正则），并为敏感字段提供默认规则
    - JSON logs、audit export、devtools JSONL、Prometheus labels 均遵守同一套脱敏策略（默认不泄漏 token/密钥/Authorization）
    - 为脱敏策略补充单测与文档（含“默认会脱什么、不脱什么”的契约）
  - 验证：
    - `cargo test --all-targets --all-features`
- [x] 观测：补齐更细粒度指标维度（按 model/provider 聚合、streaming 特有指标）
  - DoD：
    - Prometheus 支持按 `model`/`provider` 聚合的 latency/错误/限流指标，并提供基数上限参数
    - streaming 侧提供关键指标（例如 SSE 连接数、stream bytes、abort/timeout 计数）
    - 文档更新（指标表/labels/基数上限/含义）
  - 验证：
    - `cargo test --all-targets --all-features`
- [x] 路由：更丰富的策略（分级 fallback、按错误类型熔断、按路由 backpressure）
  - [x] 新增按状态码 fallback 策略（`--proxy-fallback-status-codes`，可独立于 retry 启用）
  - [x] 新增 transport action 策略（`--proxy-network-error-action` / `--proxy-timeout-error-action`）与按错误类型熔断配置（`--proxy-cb-failure-status-codes` / `--proxy-cb-no-*`）
  - [x] 决策日志补齐 `action` / `failure_kind` / `reason` / `will_attempt_next_backend`
  - DoD：
    - 路由策略可以表达“哪些错误才允许 fallback / retry”，并提供可解释的决策日志
    - 为路由策略补充回归测试（至少覆盖：网络错误、429/5xx、超时、熔断恢复）
  - 验证：
    - `cargo test --all-targets --all-features`
- [x] 代理缓存：支持 streaming cache 与更细粒度 invalidation
  - [x] 非流式缓存已支持强类型 metadata + selector-based purge（memory/redis/admin API 同步生效）
  - [x] streaming cache 已支持显式开关、独立体积上限、完整 SSE 回放、默认禁用与共享 purge 语义
  - DoD：
    - streaming 场景可选择性缓存（可控上限/TTL/回放），并具备明确的禁用/绕过机制
    - 提供按 key/model/path 的 purge/invalidation 策略与运维端点
  - 验证：
    - `cargo test --all-targets --all-features`
- [ ] Translation：扩面 OpenAI 端点覆盖（保持 feature gating）
  - [x] OpenAI `/v1/images/edits`：multipart 解析、`ImageEditModel` runtime 绑定、`stream=true` 显式拒绝与回归测试
  - [x] OpenAI `/v1/videos` 资源 API：typed `VideoGenerationModel` translation 绑定，覆盖 create(JSON/multipart) + list + retrieve + delete + `/:id/content` + `/:id/remix`，并显式拒绝 `stream=true`
  - [x] OpenAI `/v1/responses` 资源读取/删除：best-effort local translation response store，覆盖 `GET /v1/responses/:id` + `GET /v1/responses/:id/input_items` + `DELETE /v1/responses/:id`（当前仅保证本进程内由 non-stream translation create 生成的响应可读写）
  - [x] OpenAI `/v1/responses/input_tokens`：best-effort token 估算，复用 `gateway-tokenizer`，未启用该 feature 时显式返回 `unsupported_endpoint`
  - DoD：
    - 明确每个端点的“best-effort”语义与错误边界（不 silent downgrade）
    - 为新增端点补充 fixture/回归测试（含 streaming 与 files/multipart）
  - 验证：
    - `cargo test --all-targets --all-features`
- [ ] 企业平台：配置版本化/灰度/回滚（以运维可控为第一优先级）
  - DoD：
    - [x] L1 最小切片：virtual keys + router 配置版本历史、差异对比、导出校验与回滚（`GET /admin/config/version`、`GET /admin/config/versions`、`GET /admin/config/versions/:version_id`、`GET /admin/config/diff`、`GET /admin/config/export`、`POST /admin/config/validate`、`PUT /admin/config/router`、`POST /admin/config/rollback`，支持 `dry_run`）
    - [ ] 配置灰度发布（按租户/流量/路由分批）与一键回滚
    - [ ] 扩展版本覆盖面（keys 之外的 budgets/router/policy）与跨维度复现能力
  - 验证：
    - `cargo test --all-targets --all-features`

### P0（让 Gateway 达到 LiteLLM 的“可替换”）

- [x] Gateway 代理路径：基础持久化（virtual keys via `--state` or `--sqlite`）
- [x] Gateway 代理路径：持久化存储（virtual keys / budgets / audit logs；sqlite/redis 可选）
- [x] 路由：retry/fallback + weighted load balancing + passive health（circuit breaker）
- [x] 路由：主动健康检查/探活（active probing）
- [x] 路由：backpressure（`--proxy-max-in-flight`）
- [x] 成本：token 计数 + pricing + spend + 预算控制（USD 口径）
  - [x] token 计数（feature `gateway-tokenizer`；失败回退估算）
  - [x] pricing table（LiteLLM JSON；feature `gateway-costing`）
  - [x] spend ledger by virtual key（sqlite/redis + `/admin/budgets` + `/admin/costs`）
  - [x] spend aggregation by tenant/project/user（`virtual_keys[].tenant_id/project_id/user_id` + `/admin/budgets/tenants|projects|users` + `/admin/costs/tenants|projects|users`）
  - [x] shared budgets/limits by tenant/project/user（`tenant_budget/tenant_limits` 等；与 project/user 同语义）
- [x] 观测：structured logs + OpenTelemetry + Prometheus metrics（含 per-path/per-backend latency histograms；request_id 已完成）
- [x] Proxy caching（非流式请求；并提供显式绕过）
- [x] 内存安全：proxy cache 增加体积上限（单条/总量）
- [x] 内存安全：SSE parsing 增加单行/单事件上限（防止异常上游导致 OOM）
- [x] 内存安全：`stream_text` fan-out 改为有界缓冲（避免未消费 stream 的无界增长）
- [x] 内存安全：`stream_object` fan-out 改为有界缓冲（替换 `mpsc::unbounded_channel`）
- [x] 内存安全：`StreamCollector` / `stream_object` 内部缓冲区增加 max-bytes 上限（超限发出 warning）
- [x] 企业：分布式限流（Redis 全局 rpm/tpm；按 virtual key id；窗口=分钟；计数 key 带 TTL；并支持可选的 tenant/project/user shared limits）
- [x] 企业：分布式限流（按 route 分组；Redis 加权滑动窗口（60s）算法；与外层 API gateway 可叠加）
- [x] 企业：RBAC-lite + tenant 隔离模型（keys/budgets/audit 的隔离边界：tenant 独立管理/跨 tenant 默认拒绝）
  - [x] RBAC-lite：admin token 支持 read-only（`--admin-read-token*`）与 write（`--admin-token*`）分离
  - [x] 审计 taxonomy：admin 写操作在启用 sqlite/redis store 时写入 audit log（用于合规与追踪）
  - [x] tenant 隔离：tenant 独立 keys 管理、跨 tenant 查询默认拒绝、审计/导出隔离（tenant-scoped admin token）
- [x] 企业：审计保留期（sqlite/redis；`--audit-retention-secs`）
- [x] 企业：审计导出（`GET /admin/audit/export` JSONL/CSV）+ 防篡改（SHA-256 hash-chain）+ verifier CLI（`ditto-audit-verify`）
- [x] 企业：审计导出到对象存储（S3/GCS）+ WORM/签名
- [x] 运维资产：Docker Compose / Helm / K8s manifests + Grafana dashboard + PrometheusRule 告警规则
- [x] 安全：Secret Manager 适配（`secret://...`：Vault/AWS SM/GCP SM/Azure KV；替代纯 env 明文）
- [x] 管理面：Admin UI（`apps/admin-ui`）

### P1（让 Ditto 成为“超集”，而不是“替代品”）

- [x] Translation proxy：把 `POST /v1/responses` / `POST /v1/responses/compact` / `POST /v1/chat/completions` 翻译到 native providers（Anthropic/Google/Bedrock/Vertex/Cohere；feature `gateway-translation`）
- [x] UI/HTTP 适配层：Rust 侧提供 AI SDK UI 类似的 streaming primitives（`sdk::http` 的 SSE/NDJSON 输出）
- [x] SDK：缓存 middleware + 流式回放（对齐 AI SDK caching 范式）
- [x] SDK：最小 JS/TS client + React hooks（基于 stream protocol v1；非 1:1 复刻 AI SDK UI）

### P2（扩面端点）

- [x] Gateway translation：`/audio/transcriptions` 与 `/audio/speech`
- [x] Gateway translation：`/batches`
- [x] Gateway translation：`/rerank`（`/images/generations` 与 `/moderations` 已完成）
- [x] 更强的策略/缓存/背压（backpressure）控制（适配高并发与长连接 streaming）

### 文档与结构说明（最近完成）

- [x] `ProviderConfig` 文档改为明确的 provider node 语义，并区分 node 配置、provider pack、catalog、request-level `provider_options`。
- [x] README 的 provider 说明改成“默认核心 + provider packs + capability packs”。
- [x] 根目录 `PROVIDERS.md` 追加 provider × capability × feature × runtime status 对照表。
- [x] 配置层新增显式 `provider` 与 `enabled_capabilities`，并由 registry 做 provider/capability 约束校验。
- [x] 配置编辑器默认值与提示改为 registry 驱动，并移除按节点名猜 `openrouter/yunwu/google` 等 provider 特例。
- [x] 默认 root pnpm 脚本与 CI Node 校验改为只覆盖 `packages/*`；`apps/admin-ui` 继续作为可选 workspace 资产保留。
- [x] README / docs / CHANGELOG 已把 `apps/admin-ui` 改写为可选资产，不再把它描述成默认核心交付面。
- [x] `cargo check` / `cargo clippy -D warnings` / provider feature matrix 已被明确写成新的结构演进 stop gate，并与默认 CI 口径对齐。
- [x] 新增 `ditto-catalog-dashboard` 与 `CATALOG_COMPLETENESS.md`，持续汇总 provider/capability/model completeness，并纳入 `llms.txt` 生成与 `--check` 校验。
- [x] 新增 `docs/src/roadmap/provider-runtime-rollout.md`，把剩余 runtime gap 收敛为按 capability 分组的落地顺序。
- [x] 已把 `OpenAiProviderFamily` / quirks 推断从 `profile` 兼容壳迁到 `providers` 共享层，避免继续把 provider 行为塞回 legacy config namespace。
- [x] 已把 `FileClient` 的 `OpenAI` / `OpenAICompatible` 实现移出 `crates/ditto-core/src/capabilities/file.rs`，能力层只保留 trait 与数据结构。
- [x] 已恢复 `cargo check --features agent`，并补齐 OpenAI-family 共享 HTTP/auth gate。
- [x] Rust provider catalog 生成结果已从单一 `crates/ditto-core/src/catalog/generated/providers/mod.rs` 拆成按 provider 的 `crates/ditto-core/src/catalog/generated/providers/*.rs`，并保留统一聚合入口 `crates/ditto-core/src/catalog/generated/providers/mod.rs`。
- [x] 已完成 `context.cache` 的第一阶段收口：新增 typed `ContextCacheModel`，并让 DeepSeek / MiniMax 的 runtime、dashboard、provider tests 同步对齐。
- [x] 已完成 OpenAI native `video.generation` 的第一阶段接入：新增 typed `VideoGenerationModel`，并补齐 `/videos` 资源型 builder、route、provider tests 与 dashboard。
- [x] 已完成 Google native `image.generation`、`realtime`、`video.generation` 的第一阶段接入：补齐 `predict`/`predictLongRunning`/live builder、typed session/polling、provider tests 与 dashboard；provider-level runtime gap 已清零。

---

## 4) 验证（本仓库内可复制）

```bash
cd ditto-llm

cargo fmt --all -- --check
cargo run -p ditto-core --all-features --bin ditto-catalog-dashboard -- --check
cargo run -p ditto-core --bin ditto-llms-txt -- --check
cargo test --all-targets                # default core: provider-openai-compatible + cap-llm
cargo check --examples                  # default examples must stay generic openai-compatible
cargo test --all-targets --all-features
cargo test --test openai_provider_capabilities --all-features -- --nocapture
cargo test --test deepseek_provider_capabilities --all-features -- --nocapture
cargo test --test anthropic_provider_capabilities --all-features -- --nocapture
cargo test --test google_provider_capabilities --all-features -- --nocapture
cargo test --test gateway_translation_custom_provider_resolution --all-features -- --nocapture
cargo test --all-features resolve_plan_accepts_response_only_model_for_completion -- --nocapture
cargo test --all-features resolve_plan_rejects_catalog_incompatible_model_for_completion -- --nocapture
cargo test --all-features builder_runtime_ -- --nocapture
cargo test --test gateway_translation gateway_translation_rejects_endpoint_without_bound_capability --all-features -- --nocapture
cargo test --test gateway_translation gateway_translation_rejects_model_capability_mismatch_before_builder_resolution --all-features -- --nocapture
cargo test --test gateway_translation_files --all-features -- --nocapture
cargo test --test gateway_translation_stream_options --all-features -- --nocapture
cargo clippy --all-targets --all-features -- -D warnings
# 外部 store 集成测试需要额外环境：DITTO_REDIS_URL / DITTO_MYSQL_URL / DITTO_POSTGRES_URL
cargo check --no-default-features
cargo clippy --no-default-features -- -D warnings

pnpm -r run typecheck
pnpm -r run build
```

跑 examples（需要相应环境变量）：

```bash
export OPENAI_COMPAT_BASE_URL="https://your-openai-compatible-endpoint/v1"
export OPENAI_COMPAT_MODEL="your-chat-model"
export OPENAI_COMPAT_API_KEY="sk-..."   # optional for local gateways

cargo run --example basic
cargo run --example streaming
cargo run --example tool_calling
cargo run --example openai_compatible
cargo run --example multimodal --features "provider-openai cap-llm base64" -- <image_path> <pdf_path>
```
