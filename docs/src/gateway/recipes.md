# Gateway Recipes（可复制落地）

本页的定位类似 LiteLLM Proxy docs 里的“how-to / recipes”：每个 recipe 都给出 **可复制的配置 + 启动命令 + 验证 curl**。

---

## Recipe 1：OpenAI upstream + Virtual Keys + Admin API（最常见）

目标：

- 对外发放 virtual keys（客户端用）
- 对内用 OpenAI API key 访问 upstream（不暴露给客户端）
- 用 Admin API 动态增删改 key（并持久化到 Redis，支持多副本）

### 1) `.env`（示例）

```bash
OPENAI_API_KEY=...
DITTO_ADMIN_TOKEN=...
DITTO_VK_BOOTSTRAP=...
REDIS_URL=redis://127.0.0.1:6379
```

### 2) `gateway.json`（最小骨架）

```json
{
  "backends": [
    {
      "name": "openai",
      "base_url": "https://api.openai.com/v1",
      "max_in_flight": 64,
      "timeout_seconds": 60,
      "headers": { "authorization": "Bearer ${OPENAI_API_KEY}" },
      "query_params": {}
    }
  ],
  "virtual_keys": [
    {
      "id": "vk-bootstrap",
      "token": "${DITTO_VK_BOOTSTRAP}",
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
      "limits": { "rpm": 60, "tpm": 20000 },
      "budget": { "total_tokens": 5000000, "total_usd_micros": null },
      "cache": { "enabled": false, "ttl_seconds": null, "max_entries": 1024, "max_body_bytes": 1048576, "max_total_body_bytes": 67108864 },
      "guardrails": { "block_pii": true, "validate_schema": true },
      "passthrough": { "allow": true, "bypass_cache": true },
      "route": null
    }
  ],
  "router": { "default_backends": [{ "backend": "openai", "weight": 1.0 }], "rules": [] }
}
```

> 说明：virtual key 一旦启用，客户端的 `Authorization` 不会被转发 upstream；upstream 鉴权由 `backends[].headers` 注入。

### 3) 启动

```bash
cargo run --features "gateway gateway-store-redis" --bin ditto-gateway -- ./gateway.json \
  --listen 0.0.0.0:8080 \
  --dotenv .env \
  --admin-token-env DITTO_ADMIN_TOKEN \
  --redis-env REDIS_URL --redis-prefix ditto \
  --proxy-max-in-flight 256
```

### 4) 验证（客户端调用）

```bash
curl -sS http://127.0.0.1:8080/health
curl -sS http://127.0.0.1:8080/v1/models -H "Authorization: Bearer ${DITTO_VK_BOOTSTRAP}" | head
```

多语言最小模板（包含 request id 传递）：

- Node：`examples/clients/node/stream_chat_completions.mjs`
- Python：`examples/clients/python/chat_completions.py`
- Go：`examples/clients/go/chat_completions.go`

### 5) 验证（Admin API）

```bash
curl -sS http://127.0.0.1:8080/admin/keys -H "Authorization: Bearer ${DITTO_ADMIN_TOKEN}" | jq .
```

---

## Recipe 2：Weighted 路由 + fallback（主备）

目标：

- `primary`/`backup` 两个 upstream
- 按权重选择主 backend，并在失败时 fallback

关键配置：`router.default_backends` 或 `router.rules[].backends`（见「路由」）。

> 重要语义：\n> - 网络错误会自动尝试下一个 backend。\n> - 想在 `429/5xx` 等“可重试状态码”时 fallback，需要启用 `gateway-routing-advanced` 并打开 `--proxy-retry`。

示例（片段）：

```json
{
  "backends": [
    { "name": "primary", "base_url": "https://api.openai.com/v1", "headers": { "authorization": "Bearer ${OPENAI_API_KEY}" } },
    { "name": "backup", "base_url": "http://litellm:4000/v1", "headers": { "authorization": "Bearer ${LITELLM_MASTER_KEY}" } }
  ],
  "router": {
    "default_backends": [
      { "backend": "primary", "weight": 9 },
      { "backend": "backup", "weight": 1 }
    ],
    "rules": []
  }
}
```

---

## Recipe 3：Proxy cache（非 streaming）+ Redis L2（多副本）

目标：

- 缓存 `GET/POST` 的非 streaming 成功响应
- 多副本共享缓存（Redis）

启用条件：

- 编译启用 `gateway-proxy-cache` + `gateway-store-redis`
- 运行时加 `--proxy-cache --proxy-cache-ttl ...`

并确保你已启用 `--redis ...`（否则只有本机内存缓存）。

示例：

```bash
cargo run --features "gateway gateway-proxy-cache gateway-store-redis" --bin ditto-gateway -- ./gateway.json \
  --dotenv .env --redis-env REDIS_URL --redis-prefix ditto \
  --proxy-cache --proxy-cache-ttl 60 --proxy-cache-max-entries 2048
```

验证（命中会带响应头）：

- `x-ditto-cache: hit`
- `x-ditto-cache-key: ...`
- `x-ditto-cache-source: memory|redis`

---

## Recipe 4：预算（tokens / USD）+ pricing

目标：

- 用 `BudgetConfig.total_tokens` 限制 token 额度
- 用 `BudgetConfig.total_usd_micros` 限制美元额度（需要 pricing）

关键点：

- cost budgets 需要编译启用 `gateway-costing`
- 需要 `--pricing-litellm <path>` 加载 LiteLLM 风格 pricing JSON

示例（片段）：

```json
{
  "virtual_keys": [
    {
      "id": "vk-paid",
      "token": "${VK_PAID}",
      "enabled": true,
      "limits": {},
      "budget": { "total_tokens": 5000000, "total_usd_micros": 1000000 },
      "cache": {},
      "guardrails": {},
      "passthrough": { "allow": true, "bypass_cache": true },
      "route": null
    }
  ]
}
```

启动（示意）：

```bash
cargo run --features "gateway gateway-costing gateway-store-redis" --bin ditto-gateway -- ./gateway.json \
  --pricing-litellm ./pricing.json \
  --redis redis://127.0.0.1:6379 --redis-prefix ditto
```

---

## Recipe 5：重试/熔断/健康检查（routing-advanced）

目标：

- 在 upstream 不稳定时提高可用性

启用方式：

- 编译启用 `gateway-routing-advanced`
- 运行时打开 `--proxy-retry` / `--proxy-circuit-breaker` / `--proxy-health-checks`

并使用 `/admin/backends` 查看健康状态（需 admin token）。

示例：

```bash
cargo run --features "gateway gateway-routing-advanced" --bin ditto-gateway -- ./gateway.json \
  --proxy-retry --proxy-retry-max-attempts 2 \
  --proxy-circuit-breaker --proxy-cb-failure-threshold 3 --proxy-cb-cooldown-secs 30 \
  --proxy-health-checks --proxy-health-check-path /v1/models --proxy-health-check-interval-secs 10
```

---

## Recipe 6：OpenAI-compatible upstream → Claude Code CLI + Gemini CLI（指向 localhost）

目标：

- 上游是 **OpenAI-compatible**（例如 LiteLLM 的 `/v1`），模型为 `glm-4.7`
- 本地启动 `ditto-gateway`，同时对外提供：
  - OpenAI：`/v1/*`
  - Claude（Anthropic Messages）：`/v1/messages`
  - Gemini（Google GenAI）：`/v1beta/models/*:generateContent` / `:streamGenerateContent`
- 让 **Claude Code CLI** / **Gemini CLI** 都能通过 `localhost` 直接使用

### 1) 配置文件（示例）

直接用仓库内示例：`deploy/gateway.litellm.talesofai.cn.glm47.json`。

这个示例做了两件事：

- 上游鉴权从环境变量注入：`TALES_LITELLM_API_KEY`
- 任意下游 `model` 都会被改写到上游 `glm-4.7`：`model_map: { "*": "glm-4.7" }`

### 2) 启动

```bash
export TALES_LITELLM_API_KEY='...'
export DITTO_VK='vk-local'

cargo run --features gateway --bin ditto-gateway -- ./deploy/gateway.litellm.talesofai.cn.glm47.json \
  --listen 127.0.0.1:18080
```

### 3) 验证（curl）

```bash
# OpenAI
curl -sS http://127.0.0.1:18080/v1/models \
  -H "Authorization: Bearer ${DITTO_VK}" | head

# Claude (Anthropic Messages)
curl -sS http://127.0.0.1:18080/v1/messages \
  -H "x-api-key: ${DITTO_VK}" \
  -H "content-type: application/json" \
  -d '{"model":"claude-local","max_tokens":16,"messages":[{"role":"user","content":[{"type":"text","text":"Reply with ok"}]}]}' | jq -r '.content[0].text'

# Gemini (Google GenAI)
curl -sS http://127.0.0.1:18080/v1beta/models/gemini-pro:generateContent \
  -H "x-goog-api-key: ${DITTO_VK}" \
  -H "content-type: application/json" \
  -d '{"contents":[{"role":"user","parts":[{"text":"Reply with ok"}]}]}' | jq -r '.candidates[0].content.parts[0].text'
```

### 4) Claude Code CLI（Anthropic Base URL 指向 localhost）

```bash
export ANTHROPIC_BASE_URL='http://127.0.0.1:18080'
export ANTHROPIC_API_KEY="${DITTO_VK}"

claude -p 'Reply with ok'
```

### 5) Gemini CLI（Gemini Base URL 指向 localhost）

```bash
export GOOGLE_GEMINI_BASE_URL='http://127.0.0.1:18080'
export GEMINI_API_KEY="${DITTO_VK}"

# 建议显式选一个模型（避免 auto 模式走 classifier / generateJson）
gemini -m flash -p 'Reply with ok'
```

---

下一步：

- 想要更完整的端点/参数查表：看「参考 → CLI」与「Gateway → HTTP Endpoints」
- 想要生产部署注意事项：看「部署：多副本与分布式」与「安全与加固」
