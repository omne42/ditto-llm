# 存储：state / sqlite / pg / mysql / redis

Ditto Gateway 的“存储”主要用于两类数据：

- **配置态（virtual keys + router）**：允许用 Admin API 动态改配置，并在重启后保留。
- **运行态（审计/预算/成本/缓存）**：用于运维观测与多副本一致性（视 feature 而定）。

启动参数解析见 `src/bin/ditto-gateway.rs`。当前可以同时启用多个持久层（例如 `--sqlite` + `--pg` 双写）。

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

## 2) `--state <path>`：JSON state file（存 `virtual_keys` + `router`）

启用方式：

```bash
cargo run --features gateway --bin ditto-gateway -- ./gateway.json --state ./gateway.state.json
```

行为（启动时）：

- 若文件存在：读取 `GatewayStateFile.virtual_keys` 与可选 `router` 覆盖 `gateway.json`
- 若文件不存在：用 `gateway.json` 初始化并写入文件

特性：

- 持久化 **virtual keys + router**
- 不持久化预算 ledger / 审计 / proxy cache
- 不支持多副本共享（每个实例各写各的，容易冲突）

适用：

- 单机部署，想要“Admin API 修改 key 可持久化”的最小方案

---

## 3) `--sqlite <path>`：单机持久化（config + ledger + audit）

前置：

- 编译启用 feature `gateway-store-sqlite`

启用方式：

```bash
cargo run --features "gateway gateway-store-sqlite" --bin ditto-gateway -- ./gateway.json \
  --sqlite ./ditto-gateway.sqlite
```

行为（启动时）：

- 如果 sqlite 文件已存在：从 sqlite 载入 `virtual_keys` + `router` 覆盖 `gateway.json`
- 如果 sqlite 文件不存在：用 `gateway.json` 初始化 sqlite

可存内容（视功能而定）：

- virtual keys + router（Admin API config 变更的来源与落盘）
- audit logs（`/admin/audit`）
- token budgets ledger（`/admin/budgets*`）
- cost budgets ledger（`/admin/costs*`，需要 `gateway-costing`）
  - 建议根据合规需求配置 `--audit-retention-secs`（默认 30 天），避免审计日志无限增长（见下文）

限制：

- 不支持多副本共享（仍是单机）

适用：

- 单机部署但需要“预算/审计可追溯”

---

## 4) `--pg <url>`：Postgres 持久化（config + ledger + audit）

前置：

- 编译启用 feature `gateway-store-postgres`

启用方式：

```bash
cargo run --features "gateway gateway-store-postgres" --bin ditto-gateway -- ./gateway.json \
  --pg postgresql://user:pass@localhost:5432/ditto
```

行为（启动时）：

- 若 Postgres 中已有配置：载入 `virtual_keys` + `router` 覆盖 `gateway.json`
- 若无配置：用 `gateway.json` 初始化

当前能力：

- 持久化 `virtual_keys` + `router`
- 审计日志（`/admin/audit*`）
- token budgets ledger（`/admin/budgets*`）
- cost budgets ledger（`/admin/costs*`，需要 `gateway-costing`）
- reservations 回收（`POST /admin/reservations/reap`）
- 可与其他存储同时启用，作为双写/迁移目标
- schema 优化：
  - 配置与审计 payload 使用 `JSONB`
  - ledger/reservation 增加非负约束（`CHECK >= 0`）
  - 审计索引覆盖 `ts_ms` 与 `(kind, ts_ms)`

---

## 5) `--mysql <url>`：MySQL 持久化（config + ledger + audit）

前置：

- 编译启用 feature `gateway-store-mysql`

启用方式：

```bash
cargo run --features "gateway gateway-store-mysql" --bin ditto-gateway -- ./gateway.json \
  --mysql mysql://user:pass@localhost:3306/ditto
```

行为（启动时）：

- 若 MySQL 中已有配置：载入 `virtual_keys` + `router` 覆盖 `gateway.json`
- 若无配置：用 `gateway.json` 初始化

当前能力：

- 持久化 `virtual_keys` + `router`
- 审计日志（`/admin/audit*`）
- token budgets ledger（`/admin/budgets*`）
- cost budgets ledger（`/admin/costs*`，需要 `gateway-costing`）
- reservations 回收（`POST /admin/reservations/reap`）
- 可与其他存储同时启用，作为双写/迁移目标
- schema 优化：
  - 配置与审计 payload 使用 `JSON`
  - `id/key_id/request_id/key` 使用 `utf8mb4_bin`（大小写敏感、字节级一致）
  - ledger/reservation 增加非负约束（`CHECK >= 0`）
  - 审计索引覆盖 `ts_ms` 与 `(kind, ts_ms)`

---

## 6) `--redis <url>`：分布式共享存储（推荐）

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

- virtual keys + router（共享）
- audit logs（共享）
- token/cost budgets ledger（共享，支持多副本预算一致）
- proxy cache（若启用 `gateway-proxy-cache`，会作为 L2 共享缓存）
  - 建议根据合规需求配置 `--audit-retention-secs`（默认 30 天），避免审计日志无限增长（见下文）

适用：

- 多副本/高可用部署（同一套 keys/budgets/audit 需要全局一致）

---

## 7) 选型建议

- 只想“Admin API 配置可持久化”：`--state`（最小）
- 单机且需要 ledger/audit：`--sqlite`
- 想先上关系型配置落盘：`--sqlite` + `--pg`（优先）或 `--mysql`
- 多副本/分布式：`--redis`（推荐）

下一步：

- 「部署：多副本与分布式」：如何把 redis store 用在实际部署拓扑里

---

## 8) 审计日志保留期（强烈建议）

当你启用 sqlite/pg/mysql/redis store 并且开启审计（`/admin/audit`）时，建议配置：

- `--audit-retention-secs SECS`：只保留最近 `SECS` 秒的审计日志（sqlite/pg/mysql/redis 都会按时间戳清理）

默认会按 30 天清理；如果你禁用清理（`--audit-retention-secs 0`），审计日志会随时间增长（取决于你的 QPS），可能导致 sqlite 文件或 redis 数据集不断变大。

实现细节：

- 清理不是每次 `append_audit_log` 都执行，而是按周期触发（当前实现约每 30 秒最多一次）
- 这样可以显著降低高 QPS 下的清理开销，同时维持 retention 上界

---

## 9) 数据模型与跨库策略

- Ditto 在 `sqlite/pg/mysql/redis` 上保持统一的**逻辑模型**（virtual key/router/audit/ledger/reservations）。
- 各数据库会做**物理层优化**（类型、索引、约束、排序规则），这不会改变 API 语义。
- 当前不承诺自动跨库迁移（例如 sqlite -> pg、pg -> mysql）；如果切库，迁移由使用方自行处理。

---

## 10) `db doctor` / 启动自检

为避免“库连上了但 schema 漂移”的隐性故障，Gateway 现在做两层校验：

- 启动时：对已配置 store 执行 schema 自检；不符合预期直接启动失败。
- 手动检查：可用 `--db-doctor` 仅运行自检并退出（适合发布前 smoke check）。

示例：

```bash
cargo run --features "gateway gateway-store-postgres" --bin ditto-gateway -- ./gateway.json \
  --pg postgresql://user:pass@localhost:5432/ditto \
  --db-doctor
```
