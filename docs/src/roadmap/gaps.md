# Gap Analysis（对标 LiteLLM + AI SDK）

本页回答一个问题：**Ditto-LLM 距离“更好用、更快、更内存安全、企业级超集”还差什么？**

口径说明：

- 对标对象：
  - **LiteLLM Proxy**：OpenAI-compatible AI Gateway（多租户、鉴权、限流、预算、路由、观测、运维）
  - **Vercel AI SDK（Core）**：应用侧开发体验（generate/stream/object/tools/agents/middleware）
- 目标不是 1:1 复刻 UI/前端生态（那是 AI SDK UI 的强项），而是提供：
  - Rust 侧可测试/可审计/可控依赖的 SDK
  - 一个更偏“控制面/治理”的网关（更像 infra 组件）
  - 默认安全、默认节制（避免在无意间把内存/Redis 打爆）

---

## 1) 对标 AI SDK：还缺什么？

AI SDK 的优势不在“接口形式”，而在“生态 + 工程化体验”。Ditto-LLM 当前已覆盖 Core 主干（generate/stream/tools/structured output/middleware/telemetry/devtools/MCP），仍有差距：

### 1.1 开发体验（DX）

- **模板与样例不足**：AI SDK 有大量 cookbook/模板；Ditto 需要：
  - Rust examples（SDK、agent loop、middleware、stream protocol v1）
  - 多语言客户端调用 `ditto-gateway` 的最小工程模板（Node/Python/Go）
- **UI/前端生态（可选超集）**：AI SDK 的强项是 UI hooks（React/Vue/Svelte）与 RSC/Generative UI；Ditto 不追求 1:1 复刻，但可以通过：
  - 官方“stream protocol v1”客户端（JS/TS）+ 最小 hooks（React）降低接入成本
  - 端到端模板（Next.js/Node 调 `ditto-gateway`）把“工程化路径”补齐
- **缓存与回放（应用侧）**：AI SDK 有成熟的 caching/backpressure 示例；Ditto 可以补齐：
  - 基于 `LanguageModelLayer` 的缓存 middleware（含流式回放）
  - 可复制的性能/稳定性 recipe（超时、并发、重试、背压）
- **生态适配器**：AI SDK 有 LangChain/LlamaIndex 等 adapters；Ditto 需要明确“协议级桥接”与最小适配层（否则迁移成本高）。
- **调试/重放链路需要更“开箱即用”**：
  - devtools JSONL 的字段稳定化（版本/事件类型 taxonomy）
  - 更容易把一次失败请求最小化复现出来（request_id → 事件切片）

### 1.2 “正确性默认值”

- **stream abort / backpressure 的默认策略**：AI SDK 的用户很少会踩到“慢消费导致缓冲增长”的坑；Ditto 需要进一步把这些坑变成“默认不会出事”的路径（见第 3 节）。

---

## 2) 对标 LiteLLM：还缺什么？

LiteLLM 的强项是“平台化能力 + 企业功能覆盖”。Ditto Gateway 的核心能力（virtual keys / budgets / routing / proxy cache / OTel/Prometheus / redis store）已具备，但距离企业平台仍有缺口：

### 2.1 企业身份与权限（P0）

- **RBAC/SSO/SCIM**：仍缺组织/角色/权限模型。
- ✅ 已支持（RBAC-lite 切片）：admin token 分为 **read-only** 与 **write** 两类（`--admin-read-token*` / `--admin-token*`），便于把 dashboard/只读审计与写操作分离。
- 推荐承接方式（现实主义）：外层 API gateway / IAM 做 OIDC/mTLS/WAF，Ditto 先专注模型治理；当交易需要时，再逐步补齐更细粒度的 RBAC（只读/运维/审计/密钥管理员）与 tenant 隔离边界。

### 2.2 多租户隔离（P0→P1）

- ✅ 已支持 tenant 维度的归因与配额桶：`tenant_id` + `tenant_budget` / `tenant_limits`（与 project/user 同语义；启用 Redis store 时多副本全局一致）。
- 仍缺：tenant 级别的权限与隔离边界（例如 tenant 独立 keys 管理、跨 tenant 查询默认拒绝、审计/导出按 tenant 隔离、RBAC/审批流）。

### 2.3 分布式限流（P0）

- 已支持：启用 redis store（`gateway-store-redis` + `--redis`）时，rpm/tpm 通过 Redis 原子计数实现 **全局一致**（按 virtual key id；窗口=分钟；计数 key 带 TTL），并支持可选的 tenant/project/user shared limits。
- ✅ 已支持：按 route 分组的分布式限流（Redis 加权滑动窗口 60s；适合多副本一致）。
- 仍缺：更丰富的策略（令牌桶、分级限流、IP/地理维度等）与更完整的可观测性/告警配套。

### 2.4 审计合规（P1→P2）

- 当前审计可写入 sqlite/redis，并支持基础保留期（`--audit-retention-secs`，默认 30 天）：
  - ✅ admin 写操作（例如 key upsert/delete、backend reset、cache purge）在启用 sqlite/redis store 时也会写入 audit log（作为 taxonomy 的一部分）。
  - ✅ 防篡改导出：`GET /admin/audit/export` 提供 hash-chain（含 `ditto-audit-verify` 校验工具）。
  - ✅ 对象存储导出：`ditto-audit-export` 可将导出文件上传到 S3/GCS，并生成 manifest（含文件 sha256、最后一个 hash-chain 值等）；WORM 建议在对象存储侧开启（例如 S3 Object Lock）。
  - 仍缺：全链路脱敏策略（logs/audit/devtools/metrics）与更完整的合规导出流程（审批/分批/追踪）。

### 2.5 运维资产（P1）

- LiteLLM 有成熟的部署资产与运维说明；Ditto 需要补齐：
  - ✅ 已提供：`deploy/docker-compose.yml`（本地模板）、`deploy/k8s/*`（多副本模板）、Helm chart（`deploy/helm/ditto-gateway`）、Grafana dashboard 模板与 PrometheusRule 模板。
  - 仍缺：Kustomize overlays、以及“带监控栈”的组合模板（redis、OTel collector、prometheus + dashboards）与更完整的 SLO/告警体系。

### 2.6 “平台扩展项”（P2）

- ✅ A2A agent gateway（LiteLLM-like）：已支持 `/a2a/*` 的 JSON-RPC 代理端点（beta；需要配置 `a2a_agents`）。
- 仍缺：MCP gateway（LiteLLM 已提供相关方向）。Ditto 当前更偏 SDK 工具适配（MCP schema）与本地 tool loop；要成为“企业超集”，后续可以把这些能力扩展到网关侧（但建议按真实客户需求推进）。
- Provider 覆盖面：LiteLLM 的优势是“海量 providers”；Ditto 需要平衡“可维护的 native adapters”与“更强的 OpenAI-compatible 兼容层”。
- Guardrails/告警/日志目的地生态：LiteLLM 提供大量集成；Ditto 需要优先补齐“通用扩展点 + 官方 adapter（Langfuse/Datadog/S3 等）”。
- ✅ Secret 管理：已支持 `secret://...` 解析（env/file/Vault/AWS SM/GCP SM/Azure KV），并已接入 gateway/SDK 配置与 CLI flags。
- ✅ 管理 UI：已提供最小 Admin UI（`apps/admin-ui`），用于演示 keys/budgets/costs/audit 等控制面能力。

---

## 3) 性能与内存安全：还缺什么？

### 3.1 已改进（降低 OOM 风险）

- **Proxy cache 增加体积上限**：支持限制单条缓存 body 与总缓存体积，避免缓存把内存/Redis 打爆。
- **避免 key churn 导致无界增长**：对 rate limit / budget / control-plane cache 的 scope map 增加 `retain_scopes` 清理（当 virtual keys 变更时同步 prune），避免频繁 key 轮换时内存随 scope 数增长。
- **Admin 列表端点支持分页**：`/admin/keys`、`/admin/budgets`、`/admin/costs` 支持 `limit/offset`（最大 10000），避免大租户场景一次性返回超大 payload。
- **预算预留可运维回收**：提供 `POST /admin/reservations/reap` 回收陈旧预算预留；并避免 Redis reservation key 静默过期导致 ledger `reserved_*` 永久卡死。
- **Control-plane cache 增加体积上限**：`/v1/gateway` 的进程内缓存支持 `max_body_bytes` / `max_total_body_bytes`，避免 demo/control-plane 缓存导致内存增长。
- **Proxy 大响应默认不再整段缓冲**：passthrough proxy 对非 SSE 响应会尽量流式转发；仅在“体积较小”时才会缓冲读取（用于 usage 结算或写入 proxy cache）；即使 upstream 未提供 `content-length`，也只会最多预读到上限，超过上限会切换为流式转发并跳过缓存，降低大文件下载的 OOM 风险。
- **入口请求体上限可配置**：`/v1/*` 默认上限 64MiB，并提供 `--proxy-max-body-bytes` 便于企业按 JSON/multipart/上传策略做分级与收敛。
- **usage 缓冲上限与缓存上限解耦**：通过 `--proxy-usage-max-body-bytes` 单独限制“为解析 `usage` 而缓冲的非 streaming JSON 响应”，避免把 proxy cache 上限调大后导致 usage 缓冲也被动变大（默认 1MiB）。
- **错误响应体截断**：对 provider/backend 的非 2xx 错误体只读取有限字节（默认 64KiB），避免异常/恶意错误体导致 OOM，同时提升错误日志可读性。
- **Bedrock eventstream 有界解码**：对 eventstream 的 `total_len` 与内部缓冲区设置最大 bytes 上限，避免协议错位/恶意长度导致无界累积。
- **Responses shim 有界缓冲**：对非 streaming 的 `/v1/responses` shim（chat/completions → responses）设置最大 body bytes，上游响应超限时返回错误并建议改用 streaming 或直接调用 chat/completions。
- **“默认依赖安全”口径**：YAML 配置支持作为 opt-in（`gateway-config-yaml` feature），避免把不必要的解析依赖变成默认前置。
- **SSE parsing 增加行/事件大小上限**：异常/恶意 SSE 事件不会无限增长。
- **stream fan-out 可更安全使用**：提供 `StreamTextHandle`/`StreamObjectHandle` 与 `into_*_stream`，避免“只消费一条 stream 却保留另一条 receiver”的隐式积压。
- **聚合与缓冲区增加体积上限**：`StreamCollector` 与 `stream_object` 内部缓冲区设定 max bytes，避免“超大输出/异常输出”把进程内存打爆（超限发出 `Warning` 并截断）。

### 3.2 仍建议做（P0）

- **stream fan-out 的背压策略（仍可加强）**：`stream_text`/`stream_object` 已从“无界缓冲”升级为“有界缓冲 + 显式启用”，把慢消费从“内存增长”变成“吞吐降低/等待”；后续建议把 buffer 大小/策略做成可配置，并在 lag/backpressure 时打点或告警。
- **按 endpoint/内容类型细化 body 上限**：`/v1/*` 统一 64MiB 的上限对企业不够细；建议对 JSON/multipart/files/audio 分级限制并配合并发背压。

---

## 4) 推荐路线（M0/M1/M2）

- **M0（企业试点可上线，单租户）**：配置 schema 校验 + 脱敏策略 + 审计 taxonomy + 运维模板 + 内存安全 P0。
- **M1（多副本 + 多租户治理）**：Redis 全局限流（补齐更多分组维度） + tenant 模型 + RBAC-lite + 配置版本化/回滚。
- **M2（合规 + FinOps）**：防篡改审计 + 导出/保留期 + usage/cost 归因导出 + SLO/告警/dashboard 套件。

如果你愿意提供：

- 部署方式（单机/K8s/多 region）
- 峰值 QPS 与 streaming 占比
- 首个企业设计伙伴的 Top3 采购阻塞项

我可以把以上路线拆成可直接落地的 issue DAG（含依赖与验收命令）。
