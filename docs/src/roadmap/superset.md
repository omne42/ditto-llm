# Superset Roadmap（对标 AI SDK + LiteLLM 的可执行切片）

本页是对 `docs/src/roadmap/gaps.md` 的“落地版本”：把“差距”翻译成 **可以直接实现/验收** 的切片清单。

约束：

- 不依赖外网文档；以仓库现状为准（`TODO.md`、`COMPARED_TO_LITELLM_AI_SDK.md`、`docs/`）。
- 目标是成为 **LiteLLM Proxy + Vercel AI SDK Core 的能力超集**，但默认构建保持小（通过 feature gating）。
- “无内存泄漏风险”的工程口径：避免**无界增长**与**不可回收的持有**；在需要缓存/队列的地方，必须有上限/TTL/清理路径/背压。

---

## P0：企业可上线（替换 LiteLLM 的最低可行超集）

> 关键词：多副本一致性、治理能力、可运维性、默认不炸内存/Redis。

### P0.1 预算预留的“可回收性”（避免 reserved 泄漏）

**问题**：持久化预算（sqlite/redis）依赖 request reservation（预留→commit/rollback）。在进程崩溃/超时/客户端断连等场景，可能留下“陈旧预留”，导致：

- 配额被永久占用（`reserved_*` 不归零）
- 记录无界增长（尤其是 redis key churn / sqlite 表增长）
- 运维难以自愈

**切片**：

- 提供 Admin 维护端点：按“创建时间阈值”回收陈旧 reservations（dry-run + limit）。
- 对 redis：不要让 reservation key 静默过期导致 ledger 无法修复；保留信息以便运维回收。

**涉及模块**：

- store：`src/gateway/redis_store/budget.rs`、`src/gateway/redis_store/store.rs`（以及后续的 `sqlite_store`）
- admin：`src/gateway/http/*`

**验收/验证**：

- `cargo test -p ditto-llm`（包含 reaper 的单测；redis 部分可用 `DITTO_REDIS_URL` 跑）
- 启用 `gateway-store-redis` 后，通过 `POST /admin/...` dry-run 可看到将回收的数量；非 dry-run 会减少 ledger 的 `reserved_*`。

---

### P0.2 Tenant 隔离边界（从“归因”走向“隔离”）

**现状**：已支持 `tenant_id/project_id/user_id` 归因 + shared budgets/limits，但 admin 查询仍是“全局视角”。

**切片**（推荐从最小安全边界开始）：

- tenant-scoped admin token（只允许管理/查看某个 tenant 的 keys/ledgers/audit）
- 默认拒绝跨 tenant 查询（显式 opt-in 才允许全局 admin）

**涉及模块**：

- admin auth：`src/gateway/http/admin/auth.rs`
- admin handlers：`src/gateway/http/admin/*`
- 配置/启动参数：`src/bin/ditto-gateway.rs`（注意文件行数门禁；建议继续拆分 include）

**验收/验证**：

- tenant-scoped token 对 `/admin/keys?tenant_id=...` 允许；对其他 tenant 返回 403（不是 200+空数组）。

---

### P0.3 分布式限流的“更强策略”（按 route 分组 + 更强算法）

**现状**：redis store 下 rpm/tpm 已全局一致（按 key + 可选 tenant/project/user）；窗口=分钟。

**切片**：

- 增加 route 维度：`/v1/chat/completions`、`/v1/responses`、`/v1/embeddings` 等分组限流
- 支持更强算法（滑窗/令牌桶）与可观测性（backpressure/lag 指标）

**验收/验证**：

- 多副本下同一路由的限流一致；不同路由互不影响。

---

### P0.4 审计合规（导出 + 防篡改）

**现状**：sqlite/redis audit log + retention；admin 写操作已纳入审计 taxonomy。

**切片**：

- audit 导出（JSONL/CSV）+ 分片分页（按时间范围）
- hash-chain（最小可交付：每条记录包含前序哈希），并提供校验工具

**验收/验证**：

- 导出可在大规模数据下稳定运行（分页、流式输出、不一次性读入内存）。

---

### P0.5 运维资产（让“分布式部署”更开箱即用）

**切片**：

- Helm chart / Kustomize overlays
- Prometheus 规则 + Grafana dashboard（SLO/告警模板）
- 推荐的 redis keyspace/内存规划与压测指南

---

## P1：开发体验超集（对标 AI SDK 的“工程化路径”）

### P1.1 可复制模板（多语言调用 gateway）

**切片**：

- Node/Python/Go 最小模板：带重试、超时、streaming、request_id 传递
- 端到端 examples：从 SDK → gateway → upstream 的最短路径

---

### P1.2 应用侧缓存与回放（middleware）

**切片**：

- `LanguageModelLayer` 缓存（包含流式回放）
- cache key 规范与可观测性（命中/跳过原因）

---

### P1.3 JS/TS client（基于 stream protocol v1）

**口径**：不复刻 AI SDK UI 全套，但提供“最小可用 client + hooks（可选）”，让前端/Node 调用 Ditto 更接近 AI SDK 体验。

---

## P2：覆盖面（providers / endpoints / 生态）

- 扩充 providers（平衡 native adapter 与 OpenAI-compatible 兼容层）
- 扩充 translation 端点覆盖
- LangChain/LlamaIndex 等桥接（优先协议级、低耦合）

