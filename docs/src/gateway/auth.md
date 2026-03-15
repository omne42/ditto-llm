# 鉴权：Virtual Keys 与 Admin Token

Ditto Gateway 的鉴权分两层：

- **Virtual Keys**：给“业务调用方/客户端”使用，作用于 `POST /v1/gateway`、OpenAI-compatible `/v1/*` proxy surfaces、MCP `/mcp*` 与 A2A `/a2a/*`。
- **Admin Token**：给“运维/平台管理员”使用，作用于 `/admin/*`（仅在显式启用时挂载）。

本文以网关实现为准（见 `crates/ditto-server/src/gateway/config.rs`、`crates/ditto-server/src/gateway/transport/http/router.rs`、`crates/ditto-server/src/gateway/transport/http/openai_compat_proxy_handler.rs`、`crates/ditto-server/src/gateway/transport/http/admin_auth.rs`）。

---

## 1) Virtual Keys（面向客户端）

### 什么时候会启用？

对下列协议面，Ditto 默认 fail-close：

- OpenAI-compatible `/v1/*` passthrough proxy 会要求客户端提供 virtual key（否则 401）。
- MCP `/mcp*` 与 A2A `/a2a/*` 端点同样要求客户端提供 virtual key（否则 401）。
- 客户端提供的 key 会绑定到一个 `VirtualKeyConfig`，并作为 **策略单位**：limits/budget/cache/guardrails/routing/审计归因等。
- 如果 `virtual_keys` 为空，这些端点仍然不会放开匿名透传；而是返回 401，直到你显式配置可用 key。

兼容端点中，`/messages` 与 `/v1beta/models/*` 目前仍沿用“有 key 则校验、无 key 则不强制”的旧语义；它们还没有被提升成与 `/v1/*` / `/mcp*` / `/a2a/*` 完全一致的 fail-close 边界。

### Key 如何匹配？

- Key 的“秘密值”是 `VirtualKeyConfig.token`。
- 运行时按“完整 secret 是否匹配”校验；当 key 从 state/store 恢复时，Ditto 会对来访 secret 做同样的 `sha256` 归一化后再比较，因此持久化哈希不会影响认证。
- `enabled=false` 的 key 视为不可用（401）。

### 客户端如何携带 Virtual Key？

对 OpenAI-compatible passthrough：`ANY /v1/*`

- `Authorization: Bearer <virtual_key>`
- `x-litellm-api-key: Bearer <virtual_key>`（也接受不带 `Bearer` 的纯 token）
- `x-ditto-virtual-key: <virtual_key>`
- `x-api-key: <virtual_key>`

对控制面 demo：`POST /v1/gateway`

- JSON body 的 `virtual_key` 字段（可选）
- `Authorization: Bearer <virtual_key>`
- `x-litellm-api-key: Bearer <virtual_key>`（也接受不带 `Bearer` 的纯 token）
- `x-ditto-virtual-key: <virtual_key>`
- `x-api-key: <virtual_key>`

### 启用 Virtual Keys 后：Upstream 的真实鉴权怎么做？

当 `virtual_keys` 非空时，Ditto 会把客户端提供的 `authorization` / `x-api-key` / `x-litellm-api-key` 当作 virtual key 使用，并在转发 upstream 前做清理（见 `sanitize_proxy_headers`），以避免把虚拟 key 泄露到上游。

因此 upstream 的鉴权必须来自：

- `backends[].headers` / `backends[].query_params`（passthrough upstream）
- `backends[].provider_config.auth`（translation backend）

建议写成 `${ENV_VAR}`，并用 `--dotenv` 或运行环境变量注入：

```json
{
  "backends": [
    {
      "name": "openai",
      "base_url": "https://api.openai.com/v1",
      "headers": { "authorization": "Bearer ${OPENAI_API_KEY}" }
    }
  ],
  "virtual_keys": [
    { "id": "vk-dev", "token": "${DITTO_VK_DEV}", "enabled": true, "limits": {}, "budget": {}, "cache": {}, "guardrails": {}, "passthrough": { "allow": true, "bypass_cache": true }, "route": null }
  ],
  "router": { "default_backends": [{ "backend": "openai", "weight": 1.0 }], "rules": [] }
}
```

### 归因字段（可选，但推荐）

`VirtualKeyConfig` 还支持：

- `tenant_id` / `project_id` / `user_id`：为审计与预算分组提供归因信息（见「预算与成本」与「Admin API」）。
- `route`：把该 key 的请求 **固定路由**到某个 backend（绕过 router rules，见「路由」）。

---

## 2) Admin Token（面向管理员）

### 什么时候会启用？

只有当你在启动 `ditto-gateway` 时显式设置：

- `--admin-token <TOKEN>` 或
- `--admin-token-env <ENV_NAME>`（可配合 `--dotenv`）
- `--admin-read-token <TOKEN>` 或
- `--admin-read-token-env <ENV_NAME>`（可配合 `--dotenv`）

才会挂载 `/admin/*` 路由（见 `crates/ditto-server/src/gateway/transport/http/router.rs`）。

> 未配置 admin token 时，`/admin/*` 直接 404，这比“暴露出来但永远 401”更不容易被误用。

### 读写权限（RBAC-lite）

Ditto 目前不实现完整 RBAC/SSO，但提供一个“足够企业落地”的最小切片：

- **Write token（运维写）**：`--admin-token` / `--admin-token-env`
  - 允许所有 `/admin/*`（包含写操作）。
- **Read token（只读）**：`--admin-read-token` / `--admin-read-token-env`
  - 仅允许只读的 `/admin/*`（例如 `GET /admin/keys`、`GET /admin/audit`、`GET /admin/budgets*`、`GET /admin/costs*`、`GET /admin/backends`）。
  - `include_tokens=true` 这类 secret 导出不会放行；需要 write token 或 tenant-write token。
  - 当你只配置 read token（不配置 write token）时，写端点不会挂载（404）。

实践建议：把 read token 用于 dashboard/报表/审计查看，把 write token 只给少数运维人员或自动化发布系统。

### 管理请求如何携带 Admin Token？

支持两种等价方式（见 `crates/ditto-server/src/gateway/transport/http/admin_auth.rs`）：

- `Authorization: Bearer <admin_token>`
- `x-admin-token: <admin_token>`

---

## 3) 最佳实践（生产建议）

- 不要把 token 明文写进 `gateway.json`；用 `${ENV_VAR}` + `--dotenv` / K8s Secret 注入。
- `/admin/*` 建议只在内网开放，或由反向代理加一层 IP allowlist / mTLS。
- Virtual key 是“对外 API key”，应支持轮换：优先通过 Admin API 做 key 的 upsert/delete，并配合 `--state`/`--sqlite`/`--redis` 持久化。
- 不要把 virtual key/token 打进日志；Ditto 在 `GET /admin/keys` 默认会对 token 做 `redacted`，而 state/store 持久化只保留单向 `sha256:` 哈希。
