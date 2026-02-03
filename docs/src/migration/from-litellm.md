# 从 LiteLLM Proxy 迁移

Ditto Gateway 的目标是覆盖 LiteLLM Proxy 的核心使用路径（OpenAI-compatible `/v1/*` + 控制面治理），并在 Rust-first 的基础上把“路由/预算/审计/观测”做成可组合、可部署的积木。

本页给一个务实的迁移路线：**先并行、再替换**。

---

## 0) 先做一个现实判断：你当前依赖 LiteLLM 的哪些能力？

把能力分三类：

1) **HTTP 兼容面**：你主要用 `/v1/chat/completions`、`/v1/embeddings`、`/v1/models` 等。
2) **治理面**：virtual keys / budgets / rate limit / caching / routing / audit。
3) **生态面**：特定 provider 的适配与一堆“快捷开关”。

Ditto 当前覆盖度：

- (1) passthrough：覆盖（`ANY /v1/*`）
- (1) translation：覆盖一批（见 README 的 translation endpoints 列表；需要 `gateway-translation`）
- (2) 治理面：覆盖一部分（virtual keys / budgets / routing / cache / audit / metrics），但仍缺一些企业治理项（见 Roadmap）
- (3) provider 生态：以“原生 adapter + OpenAI-compatible adapter”为主（见 `PROVIDERS.md`）

---

## 1) 迁移路线 A：把 Ditto 放在 LiteLLM 前面（最稳）

目的：

- 你不改变现有 upstream（仍是 LiteLLM）
- 先把“鉴权/预算/路由/观测”收拢到 Ditto

配置思路：

- Ditto 的 `backends[].base_url` 指向 LiteLLM 的 `/v1`
- Ditto 自己启用 `virtual_keys`（对外发放 key）
- LiteLLM 的真实 token 仍由 Ditto backend header 注入（对客户端不可见）

示例（片段）：

```json
{
  "backends": [
    {
      "name": "litellm",
      "base_url": "http://litellm:4000/v1",
      "headers": { "authorization": "Bearer ${LITELLM_MASTER_KEY}" }
    }
  ],
  "virtual_keys": [
    { "id": "vk-dev", "token": "${DITTO_VK_DEV}", "enabled": true, "limits": {}, "budget": {}, "cache": {}, "guardrails": {}, "passthrough": { "allow": true, "bypass_cache": true }, "route": null }
  ],
  "router": { "default_backend": "litellm", "default_backends": [], "rules": [] }
}
```

优势：

- 迁移风险小：只改一个入口地址
- 你可以逐步打开 Ditto 的缓存/预算/路由功能

---

## 2) 迁移路线 B：让 Ditto 直接接 upstream（替换 LiteLLM）

如果你的 upstream 本身就是 OpenAI-compatible（或你只用 OpenAI），你可以让 Ditto passthrough 直接打到 upstream：

- OpenAI：`https://api.openai.com/v1`
- 某些厂商：提供了兼容 OpenAI 的 `/v1`

如果你的 upstream 不是 OpenAI-compatible（例如 Anthropic/Google 原生），可以考虑：

- 启用 `gateway-translation`，用 Ditto 的 native adapters 做 translation backends
- 或者继续用 LiteLLM 作为兼容层，直到 Ditto translation 覆盖到你的 endpoint 集合

---

## 3) 迁移最容易踩坑的点（差异说明）

### 3.1 配置格式与 env 展开

- Ditto 配置默认是 JSON（`gateway.json`）；如需 YAML（`gateway.yaml`），需要编译启用 feature `gateway-config-yaml`
- 支持 `${ENV_VAR}` 占位符展开，并且 env 缺失会启动失败（避免 silent misconfig）
- 兼容性补充：当启用 `gateway-config-yaml` 时，`ditto-gateway` 也支持直接读取 LiteLLM 的 `proxy_config.yaml` / `proxy_server_config.yaml`（会将 `model_list` 与 `general_settings.master_key` 转为 Ditto 配置）

### 3.2 Virtual keys 的行为差异

启用 virtual keys 后：

- 客户端的 `Authorization` 会被视为 virtual key，不会转发 upstream
- upstream 的鉴权必须由 Ditto backend headers/query 注入

这是为了防止“把虚拟 key 泄露给上游”。

### 3.3 分布式一致性

- 启用 redis store（`gateway-store-redis` + `--redis`）后：virtual keys / budgets（预留/结算）/ audit logs / rpm/tpm limits 都可做到多副本一致（按 virtual key 维度）
- 不启用 redis store：以上能力大多是进程内/本地存储（多副本不一致）

### 3.4 Costing（pricing）

Ditto 支持加载 LiteLLM 风格 pricing JSON（`--pricing-litellm`）用于 cost budgets。

注意：

- 这只用于“预算/估算”，不是“完整 billing 系统”

### 3.5 HTTP surface（兼容性补充）

- Ditto 的 passthrough proxy 主入口是 `ANY /v1/*`，但也接受 LiteLLM 常用的无 `/v1` 前缀别名（例如 `/chat/completions`、`/models/*`、`/files/*`、`/responses/*`）。
- Ditto 提供 LiteLLM 风格的 `/key/*` endpoints（`/key/generate|update|regenerate|delete|info|list`），用于迁移过程中减少改动；它们由 Ditto admin auth 控制（未配置 admin token 时不可用）。

### 3.6 MCP tools（兼容性补充）

如果你在 LiteLLM 里已经用了 MCP（`/mcp` + `tools: [{"type":"mcp", ...}]`），Ditto Gateway 也支持相同方向：

- ✅ `/mcp*` MCP JSON-RPC proxy（`tools/list` / `tools/call`）
- ✅ `POST /v1/chat/completions` 的 MCP tools 集成（把 MCP tools 转成 OpenAI `function` tools；可选 `require_approval: "never"` 自动执行）

迁移口径（最小映射）：

- LiteLLM 的 `mcp_servers: { <label>: { url: ... } }` → Ditto 的 `mcp_servers: [{ "server_id": "<label>", "url": "..." }]`
- `server_url` 选择器：Ditto 支持 LiteLLM 常见形式 `litellm_proxy/mcp/<servers>`，也支持 path/header 选择（`/mcp/<servers>`、`/<servers>/mcp`、`x-mcp-servers`）

差异/注意：

- Ditto 当前不覆盖 LiteLLM 那种更细粒度的 MCP 权限控制面（例如 per-key/tool permissions、`allowed_params` 等）；可以先用请求级 `allowed_tools` + 多实例隔离承接。
- 当前只拦截 `/v1/chat/completions` 的 MCP tools；`/v1/responses` 的 MCP tools 暂未接入。

完整说明见「Gateway → MCP Gateway（/mcp + tools）」。

---

## 4) 推荐的“最小可用替换”清单

如果你希望尽快替换 LiteLLM，但又要稳定：

- 先把 Ditto 部署在 LiteLLM 前（路线 A）
- 开启 `gateway-store-redis` + `--redis`（多副本一致）
- 开启 `--proxy-max-in-flight` + `backends[].max_in_flight`（背压）
- 逐步启用：routing-advanced / proxy-cache / prometheus/otel

当这一层稳定后，再考虑路线 B。
