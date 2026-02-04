# 配置文件（gateway.json / gateway.yaml）

Gateway 配置的核心类型是 `GatewayConfig`（见 `src/gateway/config.rs`）：

Ditto 支持用 **JSON** 表达配置；如果你希望用 YAML，也可以启用 `gateway-config-yaml` feature（字段完全一致）；下文以 JSON 为例。

```json
{
  "backends": [ ... ],
  "virtual_keys": [ ... ],
  "router": { ... },
  "a2a_agents": [ ... ],
  "mcp_servers": [ ... ]
}
```

## backends：两种模式（passthrough vs translation）

一个 backend 可以是两种形态之一：

### A) Passthrough proxy backend（OpenAI-compatible upstream）

使用 `base_url` + `headers` + `query_params` 描述一个 OpenAI-compatible upstream（例如 OpenAI、LiteLLM、Azure 网关等）。

```json
{
  "name": "primary",
  "base_url": "http://127.0.0.1:4000/v1",
  "headers": { "authorization": "Bearer ${OPENAI_COMPAT_API_KEY}" },
  "query_params": { "api-version": "${AZURE_API_VERSION}" }
}
```

### B) Translation backend（native provider）

当你启用 feature `gateway-translation` 后，backend 也可以写成：

```json
{
  "name": "anthropic",
  "provider": "anthropic",
  "provider_config": {
    "auth": { "type": "api_key_env", "keys": ["ANTHROPIC_API_KEY"] },
    "default_model": "claude-3-5-sonnet-20241022"
  }
}
```

此时 gateway 会把 OpenAI in/out 翻译为对应 provider 的 native 请求（并尽量保持 OpenAI shape）。

## backend 字段说明

`BackendConfig` 常用字段：

- `name`：唯一 backend 名称（用于路由选择与观测标签）
- `base_url`：passthrough upstream 根地址（通常以 `/v1` 结尾）
- `headers` / `query_params`：注入到 upstream 请求的默认 headers/query
- `max_in_flight`：该 backend 的并发上限（超限返回 429）
- `timeout_seconds`：该 backend 的请求超时（默认 300s）
- `provider` / `provider_config`：translation backend 配置（详见「SDK → ProviderConfig 与 Profile」）
- `model_map`：按 key/value 重写 `model`
  - 在 passthrough proxy 中：重写 JSON body 的 `model`
  - 在 translation 中：作为 `TranslationBackend.model_map` 使用

## virtual_keys：鉴权/限流/预算/策略的单位

当 `virtual_keys` 非空时：

- 客户端必须提供 virtual key（`Authorization: Bearer <virtual_key>` 或 `x-api-key` 等）
- 客户端的 `Authorization` 不会被透传到 upstream（避免把虚拟 key 泄露给上游）
- upstream 的鉴权由 backend 的 `headers/query_params` 或 translation backend 的 `provider_config.auth` 决定

virtual key 的字段很多，建议先从最小配置开始：

```json
{
  "id": "vk-dev",
  "token": "${DITTO_VK_DEV}",
  "enabled": true,
  "tenant_id": null,
  "project_id": null,
  "user_id": null,
  "tenant_budget": null,
  "project_budget": null,
  "user_budget": null,
  "tenant_limits": null,
  "project_limits": null,
  "user_limits": null,
  "limits": {},
  "budget": {},
  "cache": { "enabled": false },
  "guardrails": {},
  "passthrough": { "allow": true, "bypass_cache": true },
  "route": null
}
```

详细解释见「鉴权」「预算与成本」「缓存」「安全与加固」。

### Enterprise：Tenant/Project/User shared budgets & limits（可选）

如果你希望把多个 virtual keys 归并到同一个“配额桶”（企业常见：按 tenant/project/user 做聚合），可以启用：

- `tenant_id` + `tenant_budget` / `tenant_limits`
- `project_id` + `project_budget` / `project_limits`
- `user_id` + `user_budget` / `user_limits`

聚合语义：

- `tenant:*` / `project:*` / `user:*` scope 会被多个 key 共享
- 任意一个 scope 超额都会被拒绝（见「预算与成本」）
- 当启用 Redis store 时，shared limits/budgets 在多副本下也会保持全局一致（见「部署：多副本与分布式」与「预算与成本」）

## router：按模型路由到 backend

`RouterConfig` 支持：

- `default_backends`：按 weight 选择主 backend（并返回 fallback 顺序）
- `rules[]`：按 `model_prefix` 覆盖路由（默认前缀匹配；可选 `exact=true` 精确匹配；也可写 weighted backends）

示例：

```json
{
  "default_backends": [{ "backend": "primary", "weight": 1.0 }],
  "rules": [
    {
      "model_prefix": "gpt-4",
      "backends": [
        { "backend": "primary", "weight": 9 },
        { "backend": "backup", "weight": 1 }
      ]
    }
  ]
}
```

## `${ENV_VAR}` 占位符展开

Gateway 支持在以下字段使用 `${ENV_VAR}`：

- `backends[].base_url` / `headers` / `query_params`
- `backends[].provider_config.*`（base_url/default_model/http_headers/http_query_params/model_whitelist）
- `virtual_keys[].token`
- `a2a_agents[].agent_card_params.url` / `headers` / `query_params`
- `mcp_servers[].url` / `headers` / `query_params`

## a2a_agents：A2A agent registry（LiteLLM-like，beta）

如果你希望“通过 Ditto Gateway 调用 A2A agents”（对齐 LiteLLM 的 `/a2a/*` 端点），可以在配置里注册 agents：

```json
{
  "a2a_agents": [
    {
      "agent_id": "hello-world",
      "agent_card_params": {
        "name": "Hello World Agent",
        "url": "http://127.0.0.1:9999/"
      }
    }
  ]
}
```

说明：

- `agent_id`：Ditto 的路由 id（对应 `/a2a/:agent_id`）。
- `agent_card_params`：Ditto 会在 `GET /a2a/:agent_id/.well-known/agent-card.json` 返回它，并把其中的 `url` 字段重写为 Ditto 自己的 `/a2a/:agent_id`（让 A2A SDK 后续请求继续走 Ditto）。
- `agent_card_params.url`：同时也是 Ditto 代理请求时实际要打到的 **agent 后端 URL**（Ditto 会将 JSON-RPC 请求转发到该 URL；如果后端只实现了 `/message/send` 或 `/message/stream`，Ditto 会自动 fallback 一次）。
- `headers` / `query_params` / `timeout_seconds`：可选，用于给 agent 后端注入鉴权与超时（与 `backends[]` 同语义）。

同样支持 env/secret 占位符：

- `${ENV_KEY}`
- `os.environ/ENV_KEY`
- `secret://...`


若 env 缺失或为空，启动会失败（避免 silent misconfig）。

兼容性补充（迁移 LiteLLM 配置时常见）：

- 如果某个字段的值是 `os.environ/ENV_KEY`（整段字符串），Ditto 会把它解析为环境变量引用并替换为对应的 env 值。

## mcp_servers：MCP server registry（LiteLLM-like）

如果你希望通过 Ditto Gateway 暴露 `/mcp*` 端点，并在 `/v1/chat/completions` / `/v1/responses` 中使用 `tools: [{"type":"mcp", ...}]`，可以在配置里注册 MCP servers：

```json
{
  "mcp_servers": [
    {
      "server_id": "local",
      "url": "http://127.0.0.1:3000/mcp",
      "headers": { "authorization": "Bearer ${MCP_TOKEN}" },
      "query_params": {},
      "timeout_seconds": 30
    }
  ]
}
```

字段说明：

- `server_id`：Ditto 内部标识（用于选择 servers，以及多 server 时给工具名加 `<server_id>-` 前缀）
- `url`：MCP server 的 HTTP endpoint（只支持 `http://` / `https://`）
- `headers` / `query_params`：转发时注入（可用于鉴权）
- `timeout_seconds`：覆盖默认超时（默认 300s）

兼容性补充：

- `url` 同时接受别名字段 `http_url`

使用方式与端点说明见「Gateway → MCP Gateway（/mcp + tools）」。
