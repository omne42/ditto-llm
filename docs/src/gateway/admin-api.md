# Admin API

Admin API 用于“管理与观测控制面状态”：

- virtual keys：list / upsert / delete
- config versions：current / history / detail / diff / export / validate / router upsert / rollback
- proxy cache：purge（可选）
- backend health：list / reset（可选）
- audit / budgets / costs：查询（可选，需要 store）

实现位置：

- 路由挂载：`crates/ditto-server/src/gateway/transport/http/router.rs`
- 鉴权：`crates/ditto-server/src/gateway/transport/http/admin_auth.rs`
- config versions handlers：`crates/ditto-server/src/gateway/transport/http/config_versions.rs`
- other handlers：`crates/ditto-server/src/gateway/transport/http/admin.rs`

仓库内还保留一个可选 Admin UI（React）资产用于快速试用与演示；它不属于默认核心交付或默认 CI 路径：

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

- `include_tokens=true`：只对 write admin token 或 tenant-write admin token 放行；read-only token 会返回 403。
- 如果当前 key 已经是从 `--state` / `--sqlite` / `--pg` / `--mysql` / `--redis` 的 `sha256:` 持久化结果重载进来，Ditto 会返回 `409 secret_unavailable`，因为原始 secret 已不可逆恢复。
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
- `--pg` / `--pg-env`：写入 postgres
- `--mysql` / `--mysql-env`：写入 mysql
- `--redis`：写入 redis
- `--state`：写入 state file
- 都没有：只在内存生效（重启丢失）

持久化时，virtual key token 会被写成单向 `sha256:` 哈希；重启后仍可继续校验来访 token，但不能再从 state/store 中反解出原始 secret。

这也意味着：

- `include_tokens=true` 只在当前进程里仍持有原始 token 时可用。
- 一旦进程是从持久化的 `sha256:` token 重载起来，`GET /admin/keys`、`GET /admin/config/versions/:version_id`、`GET /admin/config/export`、`GET /admin/config/diff`、`/key/list?include_tokens=true` 这类 secret 导出会返回 `409 secret_unavailable`，而不是把哈希伪装成 token。

---

## 2) Config Versions：配置版本与回滚（virtual keys）

### 2.1 `GET /admin/config/version`

返回当前配置版本（virtual keys 维度），包括：

- `version_id`
- `created_at_ms`
- `reason`
- `virtual_key_count`
- `virtual_keys_sha256`
- `router_default_backend_count`
- `router_rule_count`
- `router_sha256`

权限：read-only admin token 或 write admin token。

### 2.2 `GET /admin/config/versions`

返回版本历史（新 -> 旧）。

注意：这份 history 是当前 gateway 进程内的版本历史，不会单独持久化。重启后 Ditto 会基于已加载的当前 `virtual_keys + router` 重新建立一个新的 `bootstrap` 快照，因此旧进程里的 history 不会跨重启保留。

Query 参数：

- `limit`（默认不限制，最大 1000）
- `offset`

权限：read-only admin token 或 write admin token。

### 2.3 `GET /admin/config/versions/:version_id`

返回指定版本的 virtual keys 快照。

返回体还包含 `router`（`RouterConfig`）。

Query 参数：

- `include_tokens=true`：只对 write admin token 或 tenant-write admin token 放行；read-only token 会返回 403。
- 如果该版本里的 key 已经只剩持久化后的 `sha256:` 哈希，Ditto 会返回 `409 secret_unavailable`。

权限：read-only admin token 或 write admin token。

### 2.4 `GET /admin/config/diff`

对比两个版本的 virtual keys 差异，返回：

- `summary`：`added`/`removed`/`changed`/`unchanged` 计数
- `added`：仅在目标版本存在的 key
- `removed`：仅在起始版本存在的 key
- `changed`：同一个 `id` 下内容变化的 key（包含 `before` / `after`）
- `summary.router_changed`：router 是否变化
- `router_before` / `router_after`：仅在 router 变化时返回

Query 参数：

- `from_version_id`（必填）
- `to_version_id`（必填）
- `include_tokens=true`：只对 write admin token 或 tenant-write admin token 放行；read-only token 会返回 403。
- 如果参与对比的任一版本已经只剩持久化后的 `sha256:` 哈希，Ditto 会返回 `409 secret_unavailable`。

权限：read-only admin token 或 write admin token。

### 2.5 `GET /admin/config/export`

导出配置快照（默认导出当前版本），响应结构与 `GET /admin/config/versions/:version_id` 一致（顶层包含 `version_id` / `created_at_ms` / `reason` / `virtual_key_count` / `virtual_keys_sha256` / `router_default_backend_count` / `router_rule_count` / `router_sha256`、以及 `virtual_keys` 与 `router`）。

Query 参数：

- `version_id`（可选；不传时导出当前版本）
- `include_tokens=true`：只对 write admin token 或 tenant-write admin token 放行；read-only token 会返回 403。
- 如果导出的 key 已经只剩持久化后的 `sha256:` 哈希，Ditto 会返回 `409 secret_unavailable`。

权限：read-only admin token 或 write admin token。

### 2.6 `POST /admin/config/validate`

用于离线导入前校验配置 payload（`virtual_keys` + 可选 `router`），不修改当前配置。

请求体：

```json
{
  "virtual_keys": [],
  "router": {
    "default_backends": [{ "backend": "primary", "weight": 1.0 }],
    "rules": []
  },
  "expected_virtual_keys_sha256": "optional-hash",
  "expected_router_sha256": "optional-hash"
}
```

返回：

- `valid`：是否通过校验
- `virtual_key_count`
- `computed_virtual_keys_sha256`
- `router_default_backend_count`（仅传入 `router` 时返回）
- `router_rule_count`（仅传入 `router` 时返回）
- `computed_router_sha256`（仅传入 `router` 时返回）
- `issues[]`：`invalid_id` / `invalid_token` / `duplicate_id` / `duplicate_token` / `hash_mismatch` / `invalid_router` / `router_hash_mismatch`

说明：

- `router` 校验会基于当前 gateway 已注册 backend 名称检查引用合法性。

权限：read-only admin token 或 write admin token。

### 2.7 `PUT /admin/config/router`

更新 router 配置（`RouterConfig`），会生成新的 config version。

请求体：

```json
{
  "router": {
    "default_backends": [{ "backend": "primary", "weight": 1.0 }],
    "rules": []
  },
  "dry_run": false
}
```

行为：

- 校验 router 中引用的 backend 是否存在
- `dry_run=true` 时只返回预览，不修改配置

当前限制：

- 仅 router 会更新；virtual keys 不变
- router 会写入已启用的持久层（`--state` / `--sqlite` / `--pg` / `--mysql` / `--redis`）

权限：需要 write admin token。

### 2.8 `POST /admin/config/rollback`

请求体：

```json
{ "version_id": "cfgv-00000000000000000001" }
```

可选 dry-run：

```json
{ "version_id": "cfgv-00000000000000000001", "dry_run": true }
```

行为：

- 将当前 virtual keys 恢复到指定历史版本
- 同时恢复该版本的 router 配置
- 成功后会生成一个新的“回滚结果版本”（方便继续前滚/回滚）
- `dry_run=true` 时只返回预览，不修改当前配置

权限：需要 write admin token。

---

## 3) Proxy cache：清理缓存（可选）

启用条件：

- admin token 已启用（read 或 write）
- proxy cache 已启用（`--proxy-cache` 且编译启用 `gateway-proxy-cache`）

### 3.1 `POST /admin/proxy_cache/purge`

权限：需要 write admin token。

请求体支持两类模式：

- 全量清理：`{ "all": true }`
- 选择器清理：`{ "cache_key"?: "...", "scope"?: "vk:key-1", "method"?: "POST", "path"?: "/v1/responses", "model"?: "gpt-4o-mini" }`

选择器语义：

- 所有已提供字段按 AND 语义匹配
- `cache_key` 单独使用时走精确删除
- `path` 会按规范化后的请求路径匹配，不带 query string（例如 `/v1/responses?foo=1` 会归一为 `/v1/responses`）
- `method` 会先归一为大写再匹配

响应：

```json
{ "cleared_memory": true, "deleted_memory": 1, "deleted_redis": 1 }
```

- `deleted_memory` 表示本机内存缓存删除条数
- `deleted_redis` 仅在启用 redis store 时出现，表示共享 redis 缓存删除条数
- selector purge 同时适用于非流式缓存与已启用的 streaming SSE 缓存条目

实现细节与运维提示：

- `{ "all": true }` 在启用 redis store 时会使用 `SCAN + DEL` 按批删除（不会把所有 key 一次性读进内存），但仍然是 O(N) 操作；大规模缓存场景建议优先依赖 TTL、并避免频繁 purge-all。
- selector purge 在 redis 中会使用 `SCAN` 逐条读取缓存记录的 metadata 后再选择性删除，因此它是运维操作，不应该放在热点路径里高频调用。

---

## 4) Backends：查看/重置健康状态（可选）

启用条件：

- 编译启用 `gateway-routing-advanced`
- admin token 已启用（read 或 write）

### 4.1 `GET /admin/backends`

权限：read-only admin token 或 write admin token。

返回每个 backend 的 `BackendHealthSnapshot`，字段包括：

- `consecutive_failures`
- `unhealthy_until_epoch_seconds`
- `health_check_healthy` / `health_check_last_error`

### 4.2 `POST /admin/backends/:name/reset`

清除某个 backend 的健康状态（把它恢复为默认健康）。

权限：需要 write admin token。

---

## 5) Audit：查询审计日志（可选，需要 store）

启用条件：

- 编译启用 `gateway-store-sqlite` / `gateway-store-postgres` / `gateway-store-mysql` / `gateway-store-redis`
- 运行时启用任一对应 store（`--sqlite` / `--pg` / `--mysql` / `--redis`）
- admin token 已启用（read 或 write）

### 5.1 `GET /admin/audit`

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

### 5.2 `GET /admin/audit/export`

返回带防篡改 hash-chain 的审计导出流（JSONL/CSV）。

权限：read-only admin token 或 write admin token。

Query 参数：

- `format=jsonl|csv`（默认 `jsonl`；`ndjson` 视为 `jsonl`）
- `limit`（默认 1000；最大 10000）
- `since_ts_ms`（可选）
- `before_ts_ms`（可选）

JSONL 输出每行是一个 `AuditExportRecord`（包含 `prev_hash`/`hash`，用 SHA-256 串起来；用于离线校验与合规留存）。

### 5.3 离线校验与对象存储导出

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

## 6) Budgets：查看 token 预算 ledger（可选，需要 store）

### 6.1 `GET /admin/budgets`

返回 `BudgetLedgerRecord[]`，其中 `key_id` 既可能是：

- virtual key id（例如 `vk-dev`）
- 也可能是 scope（例如 `tenant:tenant-a`、`project:proj-a`、`user:user-42`）

常用 query 参数：

- `key_prefix=tenant:` / `key_prefix=project:` / `key_prefix=user:`：按 ledger `key_id` 前缀过滤（便于大规模部署按 scope 查看）。
- `limit` / `offset`：分页（默认不限制；`limit` 最大 10000）。

### 6.2 `GET /admin/budgets/tenants` / `GET /admin/budgets/projects` / `GET /admin/budgets/users`

这是“按 virtual key 的 `tenant_id` / `project_id` / `user_id` 字段做聚合”的视图：

- 它主要用于“按 key 归因汇总”
- 如果你想直接查看 `tenant:*` / `project:*` / `user:*` scope 的 ledger，请用 `GET /admin/budgets` 自行筛选

---

## 7) Costs：查看美元预算 ledger（可选，需要 store + costing）

启用条件：

- 编译启用 `gateway-costing`
- 启用 sqlite / pg / mysql / redis store

端点与 budgets 类似：

- `GET /admin/costs`
- `GET /admin/costs/tenants`
- `GET /admin/costs/projects`
- `GET /admin/costs/users`

常用 query 参数：

- `key_prefix=tenant:` / `key_prefix=project:` / `key_prefix=user:`：按 ledger `key_id` 前缀过滤。
- `limit` / `offset`：分页（默认不限制；`limit` 最大 10000）。

---

## 8) Maintenance：回收陈旧预算预留（可选，需要 store）

> 用途：当进程崩溃/异常中断导致“预留未结算”时，ledger 的 `reserved_*` 可能长期不归零。该端点用于运维回收陈旧预留。

权限：需要 write admin token。

### 8.1 `POST /admin/reservations/reap`

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

- 当前支持 sqlite / pg / mysql / redis（按 store 优先级选择一个执行）。
- redis 下该操作会扫描 `redis-prefix` 下的 reservation keys（O(N)）；建议在离峰时以较小 `limit` 分批执行。
- 建议把 `older_than_secs` 设得足够保守，避免误伤超长 streaming 请求。

---

## 9) 常见错误与排障

- 401 `unauthorized`：admin token 未配置或不匹配
- 404：
  - 未启用 admin token（/admin 路由不会挂载）
  - 或者未启用对应 feature（例如 proxy cache / routing-advanced）
- 400 `not_configured`：依赖 store/feature 的端点未启用（例如 budgets/audit）
