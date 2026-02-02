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

- **RBAC/SSO/SCIM**：当前是 “admin token + virtual keys”，没有组织/角色/权限模型。
- 推荐承接方式（现实主义）：外层 API gateway / IAM 做 OIDC/mTLS/WAF，Ditto 先专注模型治理；当交易需要时，再补 RBAC-lite（只读/运维/审计/密钥管理员）。

### 2.2 多租户隔离（P0→P1）

- 当前只有 `project_id/user_id` 归因字段；缺 tenant 这一层的隔离与权限边界（keys/budget/audit/export 都要按 tenant 隔离）。

### 2.3 分布式限流（P0）

- 已支持：启用 redis store（`gateway-store-redis` + `--redis`）时，rpm/tpm 通过 Redis 原子计数实现 **全局一致**（按 virtual key id；窗口=分钟；计数 key 带 TTL）。
- 仍缺：按 project/user/route 分组的限流、滑窗/令牌桶等更强策略。

### 2.4 审计合规（P1→P2）

- 当前审计可写入 sqlite/redis，并支持基础保留期（`--audit-retention-secs`），但仍缺：
  - 防篡改（hash-chain / WORM）
  - 保留期与导出（S3/GCS）
  - 全链路脱敏策略（logs/audit/devtools/metrics）

### 2.5 运维资产（P1）

- LiteLLM 有成熟的部署资产与运维说明；Ditto 需要补齐：
  - Docker/Helm/K8s manifests（含 redis、OTel collector、prometheus 样例）
  - SLO/告警规则与 Grafana dashboard 模板

### 2.6 “平台扩展项”（P2）

- A2A agent gateway / MCP gateway（LiteLLM 已提供相关方向）。Ditto 当前更偏 SDK 工具适配（MCP schema）与本地 tool loop；要成为“企业超集”，后续可以把这些能力扩展到网关侧（但建议按真实客户需求推进）。
- Provider 覆盖面：LiteLLM 的优势是“海量 providers”；Ditto 需要平衡“可维护的 native adapters”与“更强的 OpenAI-compatible 兼容层”。
- Guardrails/告警/日志目的地生态：LiteLLM 提供大量集成；Ditto 需要优先补齐“通用扩展点 + 官方 adapter（Langfuse/Datadog/S3 等）”。
- Secret 管理：企业落地常见要求是 Secret Manager（Vault/AWS/GCP/Azure）；Ditto 目前以 env/command 为主，后续可按需求补齐集成。
- 管理 UI：LiteLLM 有 admin UI；Ditto 当前以 CLI + Admin API 为主，可提供参考 UI 或与外部控制台对接规范。

---

## 3) 性能与内存安全：还缺什么？

### 3.1 已改进（降低 OOM 风险）

- **Proxy cache 增加体积上限**：支持限制单条缓存 body 与总缓存体积，避免缓存把内存/Redis 打爆。
- **Proxy 大响应默认不再整段缓冲**：passthrough proxy 对非 SSE 响应会尽量流式转发；仅在“体积较小”时才会缓冲读取（用于 usage 结算或写入 proxy cache）；即使 upstream 未提供 `content-length`，也只会最多预读到上限，超过上限会切换为流式转发并跳过缓存，降低大文件下载的 OOM 风险。
- **入口请求体上限可配置**：`/v1/*` 默认上限 64MiB，并提供 `--proxy-max-body-bytes` 便于企业按 JSON/multipart/上传策略做分级与收敛。
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
