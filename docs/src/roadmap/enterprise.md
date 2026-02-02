# 企业与合规能力清单

本页不是“营销列表”，而是一个工程化 checklist：帮助你评估 Ditto-LLM 距离“企业级平台能力”还差哪些积木，以及哪些可以由外部基础设施承接。

> 原则：Ditto 负责 **模型治理与路由控制面**；通用的“身份/网络/合规”尽量交给外层 API gateway / service mesh / IAM。

---

## 1) 当前已具备（可用于生产落地的部分）

### 1.1 多副本所需的共享状态（推荐）

- redis store（`gateway-store-redis` + `--redis`）
  - virtual keys 共享
  - budgets/cost 预留与 ledger 共享
  - audit logs 共享
  - proxy cache 可选 L2 共享
- 可复制部署模板
  - Docker Compose：`deploy/docker-compose.yml`
  - Kubernetes：`deploy/k8s/*`

### 1.2 API key 体系（简化版）

- Virtual Keys：每个 key 带 limits/budgets/guardrails/routing/cache 配置
- Admin API：动态 upsert/delete keys（需 admin token）

### 1.3 预算与审计（可观测）

- token budgets（可选持久化）
- cost budgets（需要 `gateway-costing` + pricing table）
- audit endpoints（需要 sqlite/redis store）

### 1.4 观测基础

- request id + 响应头（`x-ditto-*`）
- JSON metrics（`GET /metrics`）
- Prometheus（可选）
- OTel tracing（可选）

---

## 2) 企业级常见缺口（建议优先级从高到低）

### 2.1 身份与权限（RBAC/SSO）

目标能力：

- 管理面（Admin API）支持角色权限：只读/写入/审计/运维等
- 支持 SSO（OIDC/SAML）与团队目录同步（SCIM）

当前状态：

- 只有单一 admin token（等价于“root key”）

建议承接方式：

- 由外层 API gateway 做 OIDC/mTLS + 路由到 `/admin/*`
- Ditto 内部后续再引入 RBAC 模型（Roadmap）

### 2.2 多租户隔离（Multi-tenancy）

目标能力：

- 租户/项目/用户三级归因与隔离
- 每租户独立预算、独立 keys、独立审计与导出

当前状态：

- `VirtualKeyConfig` 支持 `project_id` / `user_id` 字段用于归因与分组预算
- 但没有“租户”这一层的权限与隔离模型

### 2.3 分布式限流（全局 rpm/tpm）

目标能力：

- 多副本下的全局 rpm/tpm（按 key / project / user / route 分组）

当前状态：

- 使用 redis store（`gateway-store-redis` + `--redis`）时：`/v1/*` proxy 的 rpm/tpm 已通过 Redis 原子计数实现 **全局一致**（按 virtual key id；窗口=分钟；计数 key 带 TTL），并支持可选的 project/user shared limits。
- 不使用 redis store 时：仍是进程内计数（单实例可用；多副本不一致）。

建议承接方式：

- 外层 API gateway/service mesh 仍适合承接更复杂维度（IP/route/tenant）与滑窗/令牌桶策略。
- Ditto 后续可补齐：按 route 分组的限流与更强策略（Roadmap）。

### 2.4 安全与合规（审计不可变、脱敏、保留期）

目标能力：

- 不可变审计（append-only + WORM 存储/签名）
- 可配置保留期与导出（S3/GCS）
- 全链路脱敏策略（日志/审计/metrics）

当前状态：

- audit log 可写入 sqlite/redis，并支持基础保留期（`--audit-retention-secs` 按时间戳清理；默认 30 天）。
- 但不可变/导出/签名仍需外部系统承接（或后续 Roadmap 补齐）。

### 2.5 配置中心与发布治理

目标能力：

- 配置版本化、灰度、回滚
- 动态调整路由权重、预算、策略（带审批）

当前状态：

- keys 可通过 Admin API 修改并持久化
- gateway.json 仍以文件分发为主

建议承接方式：

- 用 GitOps/配置中心管理 `gateway.json` 与 `.env`，通过滚动升级发布
- 未来可扩展为“路由/策略也可通过 Admin API 管理”

### 2.6 计费与对账（Billing）

目标能力：

- 以 usage 为准的计费与报表（按 tenant/project/user/key/model/backend）
- 导出到数据仓库（BigQuery/Snowflake）

当前状态：

- Ditto 提供 cost 预算与 ledger（面向配额治理），不是完整 billing 系统

---

## 3) 推荐落地姿势（现实主义）

如果你要在企业里落地，建议把“平台职责”分层：

- 外层（API gateway / IAM / WAF / mesh）：
  - SSO/RBAC
  - 全局限流
  - 网络边界与 TLS/mTLS
  - 配置发布与审计平台
- Ditto（模型治理控制面）：
  - virtual keys（细粒度策略单位）
  - budgets/costing（配额治理）
  - routing/caching（性能与鲁棒性）
  - provider adapters（统一语义与 warnings）

---

## 4) 如果你要我把它拆成里程碑

告诉我三件事，我可以按你的规模拆成可验收任务：

- 部署方式：单机 / K8s / 多 region？
- 调用规模：峰值 QPS、模型种类、是否大量 streaming？
- 治理目标：优先要 budgets、costing、还是审计/SSO？
