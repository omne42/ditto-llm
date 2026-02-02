# Admin API

Admin API 用于“管理与观测控制面状态”：

- virtual keys：list / upsert / delete
- proxy cache：purge（可选）
- backend health：list / reset（可选）
- audit / budgets / costs：查询（可选，需要 store）

实现位置：

- 路由挂载：`src/gateway/http/core.rs`
- 鉴权：`src/gateway/http/admin/auth.rs`
- handlers：`src/gateway/http/admin/handlers.rs`

仓库内也提供一个最小 Admin UI（React）用于快速试用与演示：

- `apps/admin-ui`

---

## 0) 启用条件与鉴权方式

### 0.1 必须启用 admin token（read 或 write）

只有在启动时设置了：

- `--admin-token <TOKEN>` 或
- `--admin-token-env <ENV>`（write admin token）
- `--admin-read-token <TOKEN>` 或
- `--admin-read-token-env <ENV>`（read-only admin token）

才会挂载 `/admin/*` 路由。

写端点（例如 upsert/delete keys、purge cache、reset backend）需要 **write admin token**；当你只配置 read-only token 时，这些写端点不会挂载（404）。

### 0.2 如何携带 admin token

两种等价方式：

- `Authorization: Bearer <admin_token>`
- `x-admin-token: <admin_token>`

---

## 1) Keys：管理 virtual keys

### 1.1 `GET /admin/keys`

默认会把 `token` 字段替换为 `"redacted"`。

权限：read-only admin token 或 write admin token。

常用 query 参数：

- `include_tokens=true`：返回真实 token（谨慎使用）。
- `tenant_id` / `project_id` / `user_id`：按归因字段过滤。
- `enabled=true|false`：按启用状态过滤。
- `id_prefix=...`：按 key id 前缀过滤。
- `limit` / `offset`：分页（默认不限制；`limit` 最大 10000）。为了稳定分页，返回结果会按 `id` 排序。

### 1.2 `POST /admin/keys`：upsert

请求体是完整的 `VirtualKeyConfig` JSON。

- 若 id 不存在：创建（201）
- 若 id 已存在：更新（200）

权限：需要 write admin token。

### 1.3 `PUT /admin/keys/:id`：upsert（id 在 path）

与 `POST /admin/keys` 类似，但以 path 的 `:id` 覆盖 body 的 `id`。

权限：需要 write admin token。

### 1.4 `DELETE /admin/keys/:id`

- 成功：204
- 不存在：404

权限：需要 write admin token。

### 1.5 keys 的持久化（重要）

upsert/delete 后 Ditto 会尝试持久化 keys：

- `--sqlite`：写入 sqlite
- `--redis`：写入 redis
- `--state`：写入 state file
- 都没有：只在内存生效（重启丢失）

---

## 2) Proxy cache：清理缓存（可选）

启用条件：

- admin token 已启用（read 或 write）
- proxy cache 已启用（`--proxy-cache` 且编译启用 `gateway-proxy-cache`）

### 2.1 `POST /admin/proxy_cache/purge`

权限：需要 write admin token。

请求体二选一：

- `{ "all": true }`
- `{ "cache_key": "ditto-proxy-cache-v1-..." }`

响应：

```json
{ "cleared_memory": true, "deleted_redis": 123 }
```

- `deleted_redis` 仅在启用 redis store 时出现

实现细节与运维提示：

- `{ "all": true }` 在启用 redis store 时会使用 `SCAN + DEL` 按批删除（不会把所有 key 一次性读进内存），但仍然是 O(N) 操作；大规模缓存场景建议优先依赖 TTL、并避免频繁 purge-all。

---

## 3) Backends：查看/重置健康状态（可选）

启用条件：

- 编译启用 `gateway-routing-advanced`
- admin token 已启用（read 或 write）

### 3.1 `GET /admin/backends`

权限：read-only admin token 或 write admin token。

返回每个 backend 的 `BackendHealthSnapshot`，字段包括：

- `consecutive_failures`
- `unhealthy_until_epoch_seconds`
- `health_check_healthy` / `health_check_last_error`

### 3.2 `POST /admin/backends/:name/reset`

清除某个 backend 的健康状态（把它恢复为默认健康）。

权限：需要 write admin token。

---

## 4) Audit：查询审计日志（可选，需要 store）

启用条件：

- 编译启用 `gateway-store-sqlite` 或 `gateway-store-redis`
- 运行时启用 `--sqlite` 或 `--redis`
- admin token 已启用（read 或 write）

### 4.1 `GET /admin/audit`

权限：read-only admin token 或 write admin token。

Query 参数：

- `limit`（默认 100，最大 1000）
- `since_ts_ms`（可选）

返回 `AuditLogRecord[]`：

```json
{
  "id": 1,
  "ts_ms": 1738368000000,
  "kind": "proxy.blocked",
  "payload": { "...": "..." }
}
```

### 4.2 `GET /admin/audit/export`

返回带防篡改 hash-chain 的审计导出流（JSONL/CSV）。

权限：read-only admin token 或 write admin token。

Query 参数：

- `format=jsonl|csv`（默认 `jsonl`；`ndjson` 视为 `jsonl`）
- `limit`（默认 1000；最大 10000）
- `since_ts_ms`（可选）
- `before_ts_ms`（可选）

JSONL 输出每行是一个 `AuditExportRecord`（包含 `prev_hash`/`hash`，用 SHA-256 串起来；用于离线校验与合规留存）。

### 4.3 离线校验与对象存储导出

仓库内提供两个 CLI：

- `ditto-audit-verify`：校验 JSONL 导出的 hash-chain。
- `ditto-audit-export`：从 gateway 拉取 `/admin/audit/export`，写到本地文件，并可选上传到对象存储（S3/GCS）+ 生成 manifest（含文件 sha256、最后一个 hash-chain 值等）。

示例：

```bash
# 1) 导出到本地文件 + 生成 manifest
cargo run --bin ditto-audit-export --features gateway -- \
  --base-url http://127.0.0.1:8080 \
  --admin-token-env DITTO_ADMIN_TOKEN \
  --output audit.jsonl

# 2) 校验 hash-chain
cargo run --bin ditto-audit-verify --features gateway -- --input audit.jsonl

# 3) 上传到 S3（需要本机 aws cli + 凭证）
cargo run --bin ditto-audit-export --features gateway -- \
  --base-url http://127.0.0.1:8080 \
  --admin-token-env DITTO_ADMIN_TOKEN \
  --output audit.jsonl \
  --upload s3://my-bucket/ditto/audit.jsonl

# 4) 上传到 GCS（需要本机 gsutil + 凭证）
cargo run --bin ditto-audit-export --features gateway -- \
  --base-url http://127.0.0.1:8080 \
  --admin-token-env DITTO_ADMIN_TOKEN \
  --output audit.jsonl \
  --upload gs://my-bucket/ditto/audit.jsonl
```

WORM（不可变/保留期）建议在对象存储侧开启（例如 S3 Object Lock）。如需在上传时设置 S3 Object Lock 参数，可使用 `ditto-audit-export` 的 `--s3-object-lock-*` 选项（详见 `--help`）。

---

## 5) Budgets：查看 token 预算 ledger（可选，需要 store）

### 5.1 `GET /admin/budgets`

返回 `BudgetLedgerRecord[]`，其中 `key_id` 既可能是：

- virtual key id（例如 `vk-dev`）
- 也可能是 scope（例如 `tenant:tenant-a`、`project:proj-a`、`user:user-42`）

常用 query 参数：

- `key_prefix=tenant:` / `key_prefix=project:` / `key_prefix=user:`：按 ledger `key_id` 前缀过滤（便于大规模部署按 scope 查看）。
- `limit` / `offset`：分页（默认不限制；`limit` 最大 10000）。

### 5.2 `GET /admin/budgets/tenants` / `GET /admin/budgets/projects` / `GET /admin/budgets/users`

这是“按 virtual key 的 `tenant_id` / `project_id` / `user_id` 字段做聚合”的视图：

- 它主要用于“按 key 归因汇总”
- 如果你想直接查看 `tenant:*` / `project:*` / `user:*` scope 的 ledger，请用 `GET /admin/budgets` 自行筛选

---

## 6) Costs：查看美元预算 ledger（可选，需要 store + costing）

启用条件：

- 编译启用 `gateway-costing`
- 启用 sqlite/redis store

端点与 budgets 类似：

- `GET /admin/costs`
- `GET /admin/costs/tenants`
- `GET /admin/costs/projects`
- `GET /admin/costs/users`

常用 query 参数：

- `key_prefix=tenant:` / `key_prefix=project:` / `key_prefix=user:`：按 ledger `key_id` 前缀过滤。
- `limit` / `offset`：分页（默认不限制；`limit` 最大 10000）。

---

## 7) Maintenance：回收陈旧预算预留（可选，需要 store）

> 用途：当进程崩溃/异常中断导致“预留未结算”时，ledger 的 `reserved_*` 可能长期不归零。该端点用于运维回收陈旧预留。

权限：需要 write admin token。

### 7.1 `POST /admin/reservations/reap`

请求体：

```json
{
  "older_than_secs": 86400,
  "limit": 1000,
  "dry_run": true
}
```

- `older_than_secs`：只回收“创建时间早于 now-older_than_secs”的 reservations（默认 24h）。
- `limit`：最多回收多少条（默认 1000；最大 100000）。
- `dry_run=true`：只统计，不实际修改。

响应体：

```json
{
  "store": "redis",
  "dry_run": true,
  "cutoff_ts_ms": 1738368000000,
  "budget": { "scanned": 0, "reaped": 0, "released": 0 },
  "cost": { "scanned": 0, "reaped": 0, "released": 0 }
}
```

实现与注意事项：

- 当前仅支持 redis store（`--redis` + feature `gateway-store-redis`）；sqlite store 会返回 501（后续会补齐）。
- 该操作会扫描 `redis-prefix` 下的 reservation keys（O(N)）；建议在离峰时以较小 `limit` 分批执行。
- 建议把 `older_than_secs` 设得足够保守，避免误伤超长 streaming 请求。

---

## 8) 常见错误与排障

- 401 `unauthorized`：admin token 未配置或不匹配
- 404：
  - 未启用 admin token（/admin 路由不会挂载）
  - 或者未启用对应 feature（例如 proxy cache / routing-advanced）
- 400 `not_configured`：依赖 store/feature 的端点未启用（例如 budgets/audit）
