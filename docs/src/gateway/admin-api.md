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

---

## 0) 启用条件与鉴权方式

### 0.1 必须启用 admin token

只有在启动时设置了：

- `--admin-token <TOKEN>` 或
- `--admin-token-env <ENV>`

才会挂载 `/admin/*` 路由。

### 0.2 如何携带 admin token

两种等价方式：

- `Authorization: Bearer <admin_token>`
- `x-admin-token: <admin_token>`

---

## 1) Keys：管理 virtual keys

### 1.1 `GET /admin/keys`

默认会把 `token` 字段替换为 `"redacted"`。

- `GET /admin/keys?include_tokens=true`：返回真实 token（谨慎使用）。

### 1.2 `POST /admin/keys`：upsert

请求体是完整的 `VirtualKeyConfig` JSON。

- 若 id 不存在：创建（201）
- 若 id 已存在：更新（200）

### 1.3 `PUT /admin/keys/:id`：upsert（id 在 path）

与 `POST /admin/keys` 类似，但以 path 的 `:id` 覆盖 body 的 `id`。

### 1.4 `DELETE /admin/keys/:id`

- 成功：204
- 不存在：404

### 1.5 keys 的持久化（重要）

upsert/delete 后 Ditto 会尝试持久化 keys：

- `--sqlite`：写入 sqlite
- `--redis`：写入 redis
- `--state`：写入 state file
- 都没有：只在内存生效（重启丢失）

---

## 2) Proxy cache：清理缓存（可选）

启用条件：

- admin token 已启用
- proxy cache 已启用（`--proxy-cache` 且编译启用 `gateway-proxy-cache`）

### 2.1 `POST /admin/proxy_cache/purge`

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
- admin token 已启用

### 3.1 `GET /admin/backends`

返回每个 backend 的 `BackendHealthSnapshot`，字段包括：

- `consecutive_failures`
- `unhealthy_until_epoch_seconds`
- `health_check_healthy` / `health_check_last_error`

### 3.2 `POST /admin/backends/:name/reset`

清除某个 backend 的健康状态（把它恢复为默认健康）。

---

## 4) Audit：查询审计日志（可选，需要 store）

启用条件：

- 编译启用 `gateway-store-sqlite` 或 `gateway-store-redis`
- 运行时启用 `--sqlite` 或 `--redis`
- admin token 已启用

### 4.1 `GET /admin/audit`

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

---

## 5) Budgets：查看 token 预算 ledger（可选，需要 store）

### 5.1 `GET /admin/budgets`

返回 `BudgetLedgerRecord[]`，其中 `key_id` 既可能是：

- virtual key id（例如 `vk-dev`）
- 也可能是 scope（例如 `project:proj-a`、`user:user-42`）

### 5.2 `GET /admin/budgets/projects` / `GET /admin/budgets/users`

这是“按 virtual key 的 `project_id` / `user_id` 字段做聚合”的视图：

- 它主要用于“按 key 归因汇总”
- 如果你想直接查看 `project:*` / `user:*` scope 的 ledger，请用 `GET /admin/budgets` 自行筛选

---

## 6) Costs：查看美元预算 ledger（可选，需要 store + costing）

启用条件：

- 编译启用 `gateway-costing`
- 启用 sqlite/redis store

端点与 budgets 类似：

- `GET /admin/costs`
- `GET /admin/costs/projects`
- `GET /admin/costs/users`

---

## 7) 常见错误与排障

- 401 `unauthorized`：admin token 未配置或不匹配
- 404：
  - 未启用 admin token（/admin 路由不会挂载）
  - 或者未启用对应 feature（例如 proxy cache / routing-advanced）
- 400 `not_configured`：依赖 store/feature 的端点未启用（例如 budgets/audit）
