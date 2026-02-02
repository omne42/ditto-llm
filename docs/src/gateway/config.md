# 配置文件（gateway.json / gateway.yaml）

Gateway 配置的核心类型是 `GatewayConfig`（见 `src/gateway/config.rs`）：

Ditto 支持用 **JSON** 表达配置；如果你希望用 YAML，也可以启用 `gateway-config-yaml` feature（字段完全一致）；下文以 JSON 为例。

```json
{
  "backends": [ ... ],
  "virtual_keys": [ ... ],
  "router": { ... }
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
  "project_id": null,
  "user_id": null,
  "project_budget": null,
  "user_budget": null,
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

### Enterprise：Project/User shared budgets & limits（可选）

如果你希望把多个 virtual keys 归并到同一个“配额桶”（企业常见：按 project 或 user 做聚合），可以启用：

- `project_id` + `project_budget` / `project_limits`
- `user_id` + `user_budget` / `user_limits`

聚合语义：

- `project:*` / `user:*` scope 会被多个 key 共享
- 任意一个 scope 超额都会被拒绝（见「预算与成本」）
- 当启用 Redis store 时，shared limits/budgets 在多副本下也会保持全局一致（见「部署：多副本与分布式」与「预算与成本」）

## router：按模型路由到 backend

`RouterConfig` 支持：

- `default_backend`：默认 backend
- `default_backends`：按 weight 选择主 backend（并返回 fallback 顺序）
- `rules[]`：按 `model_prefix` 覆盖路由（也可写 weighted backends）

示例：

```json
{
  "default_backend": "primary",
  "default_backends": [],
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

若 env 缺失或为空，启动会失败（避免 silent misconfig）。
