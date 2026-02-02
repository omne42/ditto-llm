# 存储：state / sqlite / redis

Ditto Gateway 的“存储”主要用于两类数据：

- **配置态（virtual keys）**：允许用 Admin API 动态增删改 key，并在重启后保留。
- **运行态（审计/预算/成本/缓存）**：用于运维观测与多副本一致性（视 feature 而定）。

启动参数解析见 `src/bin/ditto-gateway.rs`，核心约束是：

> **只能选择一种存储后端**：`--state`、`--sqlite`、`--redis` 三选一。

---

## 1) 不启用存储（默认）

行为：

- virtual keys 只来自 `gateway.json`
- Admin API（如果启用）对 key 的变更只存在于内存，重启会丢
- `/v1/*` proxy 的预算：
  - 没有 store 时使用内存预算（并发下可能穿透；不会跨实例共享）

适用：

- 本地开发
- 单实例、无治理需求的简易 passthrough

---

## 2) `--state <path>`：JSON state file（只存 keys）

启用方式：

```bash
cargo run --features gateway --bin ditto-gateway -- ./gateway.json --state ./gateway.state.json
```

行为（启动时）：

- 若文件存在：读取 `GatewayStateFile.virtual_keys` 覆盖 `gateway.json` 里的 keys
- 若文件不存在：用 `gateway.json` 里的 keys 初始化并写入文件

特性：

- 只持久化 **virtual keys**
- 不持久化预算 ledger / 审计 / proxy cache
- 不支持多副本共享（每个实例各写各的，容易冲突）

适用：

- 单机部署，想要“Admin API 修改 key 可持久化”的最小方案

---

## 3) `--sqlite <path>`：单机持久化（keys + ledger + audit）

前置：

- 编译启用 feature `gateway-store-sqlite`

启用方式：

```bash
cargo run --features "gateway gateway-store-sqlite" --bin ditto-gateway -- ./gateway.json \
  --sqlite ./ditto-gateway.sqlite
```

行为（启动时）：

- 如果 sqlite 文件已存在：从 sqlite 载入 keys 覆盖 `gateway.json`
- 如果 sqlite 文件不存在：用 `gateway.json` 的 keys 初始化 sqlite

可存内容（视功能而定）：

- virtual keys（Admin API upsert/delete 的来源与落盘）
- audit logs（`/admin/audit`）
- token budgets ledger（`/admin/budgets*`）
- cost budgets ledger（`/admin/costs*`，需要 `gateway-costing`）
  - 建议根据合规需求配置 `--audit-retention-secs`（默认 30 天），避免审计日志无限增长（见下文）

限制：

- 不支持多副本共享（仍是单机）

适用：

- 单机部署但需要“预算/审计可追溯”

---

## 4) `--redis <url>`：分布式共享存储（推荐）

前置：

- 编译启用 feature `gateway-store-redis`

启用方式：

```bash
cargo run --features "gateway gateway-store-redis" --bin ditto-gateway -- ./gateway.json \
  --redis redis://127.0.0.1:6379 --redis-prefix ditto
```

也可以从 env 读取（配合 `--dotenv`）：

```bash
cargo run --features "gateway gateway-store-redis" --bin ditto-gateway -- ./gateway.json \
  --dotenv .env --redis-env REDIS_URL --redis-prefix ditto
```

行为（启动时）：

- Ditto 会先 `PING` redis
- 若 redis 里已有 keys：以 redis 为准覆盖 `gateway.json`
- 若 redis 里没有 keys：用 `gateway.json` 初始化写入 redis

可存内容：

- virtual keys（共享）
- audit logs（共享）
- token/cost budgets ledger（共享，支持多副本预算一致）
- proxy cache（若启用 `gateway-proxy-cache`，会作为 L2 共享缓存）
  - 建议根据合规需求配置 `--audit-retention-secs`（默认 30 天），避免审计日志无限增长（见下文）

适用：

- 多副本/高可用部署（同一套 keys/budgets/audit 需要全局一致）

---

## 5) 选型建议

- 只想“Admin API 修改 key 可持久化”：`--state`（最小）
- 单机但想要 ledger/audit：`--sqlite`
- 多副本/分布式：`--redis`（推荐）

下一步：

- 「部署：多副本与分布式」：如何把 redis store 用在实际部署拓扑里

---

## 6) 审计日志保留期（强烈建议）

当你启用 sqlite/redis store 并且开启审计（`/admin/audit`）时，建议配置：

- `--audit-retention-secs SECS`：只保留最近 `SECS` 秒的审计日志（sqlite/redis 都会按时间戳清理）

默认会按 30 天清理；如果你禁用清理（`--audit-retention-secs 0`），审计日志会随时间增长（取决于你的 QPS），可能导致 sqlite 文件或 redis 数据集不断变大。
