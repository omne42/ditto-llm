# 运行网关

`ditto-gateway` 是一个可选启用的 HTTP 服务（feature `gateway`）。它提供：

- OpenAI-compatible passthrough proxy：`ANY /v1/*`
- 可选控制面：virtual keys / limits / budgets / guardrails / caching / routing
- 可选 translation：OpenAI in/out → native providers（feature `gateway-translation`）

如果你想要“像 LiteLLM Proxy 一样”的落地方式（发放 virtual keys + 管理面动态增删改 + 多副本共享），建议直接从「Gateway Recipes → Recipe 1」开始。

## 0) 准备环境变量（两种方式）

方式 A：直接 export（最小）

```bash
export OPENAI_API_KEY=...
```

方式 B：写入 `.env`（推荐，便于本地/容器复用）

```bash
# .env
OPENAI_API_KEY=...
```

> 注意：如果你的 `gateway.json` 使用了 `${OPENAI_API_KEY}`，但运行时 env 缺失或为空，网关会在启动时直接报错退出（避免 silent misconfig）。

## 1) 选择 features（建议用“套餐”）

最小网关（仅转发 /v1/*）：

```bash
cargo build --features gateway --bin ditto-gateway
```

常见套餐：

- 本地试玩：`gateway`
- 单机可持久化（管理 keys / 预算 ledger / 审计）：`gateway + gateway-store-sqlite`
- 多副本/分布式（推荐）：`gateway + gateway-store-redis`（可选再叠加 proxy-cache / routing-advanced / prometheus / otel）

> `ditto-gateway` 的完整 CLI 参数见「参考 → CLI 选项（ditto-gateway）」。目前 CLI 采用轻量参数解析：不提供 `--help`，但启动时缺少必填参数会打印 usage。

## 2) 准备最小 gateway.json（两种鉴权模式）

下面示例把所有 `/v1/*` 转发到 OpenAI（注意 `base_url` 一般以 `/v1` 结尾）。

### 模式 A：网关持有 upstream key（最常见）

特点：

- 你把 upstream 的真实 API key 配在 `backends[].headers`（或 `.env`）
- 客户端不需要提供 OpenAI key（但这意味着“网关本身就是一个能力入口”，生产请务必加 virtual keys 或外层网关鉴权）

```json
{
  "backends": [
    {
      "name": "primary",
      "base_url": "https://api.openai.com/v1",
      "max_in_flight": 64,
      "timeout_seconds": 60,
      "headers": { "authorization": "Bearer ${OPENAI_API_KEY}" },
      "query_params": {}
    }
  ],
  "virtual_keys": [],
  "router": { "default_backends": [{ "backend": "primary", "weight": 1.0 }], "rules": [] }
}
```

> 细节：当 `virtual_keys` 为空时，Ditto 不会把客户端 `Authorization` 当作 virtual key；但如果 backend 配了 `authorization`，它会覆盖同名 header（因此建议客户端不要再传 `Authorization`，避免误解）。

### 模式 B：客户端持有 upstream key（纯反向代理）

特点：

- 你不在 `backends[].headers` 注入 `authorization`
- 客户端请求时必须自带 `Authorization: Bearer <upstream_key>`（Ditto 会原样转发）

```json
{
  "backends": [
    {
      "name": "primary",
      "base_url": "https://api.openai.com/v1",
      "max_in_flight": 64,
      "timeout_seconds": 60,
      "headers": {},
      "query_params": {}
    }
  ],
  "virtual_keys": [],
  "router": { "default_backends": [{ "backend": "primary", "weight": 1.0 }], "rules": [] }
}
```

## 3) 启动（第一个参数必须是 config 路径）

`ditto-gateway` 的第一个参数必须是 `gateway.json` 路径；如果你想用 `gateway.yaml`，需要编译启用 feature `gateway-config-yaml`：

```bash
cargo run --features gateway --bin ditto-gateway -- ./gateway.json --listen 0.0.0.0:8080
```

如果你把 token 放在 `.env` 文件里（推荐），可以：

```bash
cargo run --features gateway --bin ditto-gateway -- ./gateway.json --dotenv .env --listen 0.0.0.0:8080
```

## 4) 验证

```bash
curl -sS http://127.0.0.1:8080/health
curl -sS http://127.0.0.1:8080/v1/models | head
```

最小对话（Chat Completions）：

```bash
curl -sS http://127.0.0.1:8080/v1/chat/completions \
  -H "content-type: application/json" \
  -d '{"model":"gpt-4o-mini","messages":[{"role":"user","content":"Say hello."}]}' | head
```

> 如果你使用的是「模式 B：客户端持有 upstream key」，请额外加上 `-H "authorization: Bearer <UPSTREAM_API_KEY>"`。

如果你测试的是 `POST /v1/responses`：

- upstream 支持时：直接透传
- upstream 不支持时：Ditto 会 fallback 到 `POST /v1/chat/completions` 并返回 best-effort Responses-like（响应头带 `x-ditto-shim: responses_via_chat_completions`）

## 5) 下一步

- 想加鉴权与配额：看「鉴权：Virtual Keys 与 Admin Token」与「预算与成本」。
- 想做多 backend 路由与故障切换：看「路由：Weighted / Fallback / Retry」。
- 想多副本部署：看「部署：多副本与分布式」。
- 想用容器/模板快速跑起来：看「Docker Compose（本地模板）」与「Kubernetes（多副本模板）」。
- 想直接复制一套可跑配置：看「Gateway Recipes（可复制落地）」。
