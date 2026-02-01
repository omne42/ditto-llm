# 运行网关

`ditto-gateway` 是一个可选启用的 HTTP 服务（feature `gateway`）。它提供：

- OpenAI-compatible passthrough proxy：`ANY /v1/*`
- 可选控制面：virtual keys / limits / budgets / guardrails / caching / routing
- 可选 translation：OpenAI in/out → native providers（feature `gateway-translation`）

## 1) 选择 features

最小网关（仅转发 /v1/*）：

```bash
cargo build --features gateway --bin ditto-gateway
```

分布式/多副本常用组合（建议）：

- `gateway`（HTTP server）
- `gateway-store-redis`（共享存储）
- `gateway-proxy-cache`（可选：非 streaming 缓存）
- `gateway-routing-advanced`（可选：retry/circuit breaker/health checks）
- `gateway-metrics-prometheus` / `gateway-otel`（可选：观测）

> `ditto-gateway` 的完整 CLI 参数见「参考 → CLI 选项（ditto-gateway）」。目前 CLI 采用轻量参数解析：不提供 `--help`，但启动时缺少必填参数会打印 usage。

## 2) 准备最小 gateway.json

下面示例把所有 `/v1/*` 转发到 OpenAI（注意 `base_url` 一般以 `/v1` 结尾）：

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
  "router": { "default_backend": "primary", "rules": [] }
}
```

## 3) 启动

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

## 5) 下一步

- 想加鉴权与配额：看「鉴权：Virtual Keys 与 Admin Token」与「预算与成本」。
- 想做多 backend 路由与故障切换：看「路由：Weighted / Fallback / Retry」。
- 想多副本部署：看「部署：多副本与分布式」。
