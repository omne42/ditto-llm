# CLI 选项（ditto-gateway）

本页是 `ditto-gateway` 的运行参数速查（实现见 `src/bin/ditto-gateway.rs`）。

> 当前 CLI 采用轻量参数解析：**没有 `--help`**。运行时缺少必填参数会打印 usage（并退出）。

---

## 1) 基本用法

`ditto-gateway` 的第一个参数必须是配置文件路径：

```bash
ditto-gateway <gateway.json> [flags...]
```

开发期常见用法：

```bash
cargo run --features gateway --bin ditto-gateway -- ./gateway.json --listen 0.0.0.0:8080
```

---

## 2) 配置与运行（Core）

- `--listen HOST:PORT`（或 `--addr`）：监听地址（默认 `127.0.0.1:8080`）
- `--dotenv PATH`：加载 dotenv 文件（供 `${ENV_VAR}` 展开与 `*-env` 选项读取）
- `--json-logs`：输出 Ditto 自定义的 JSON 行事件日志（stderr）

---

## 3) Admin（管理面）

- `--admin-token TOKEN`：启用 `/admin/*` 并设置 admin token
- `--admin-token-env ENV`：从环境变量读取 admin token（可配合 `--dotenv`）

约束：

- `--admin-token` 与 `--admin-token-env` 互斥

---

## 4) 存储（state / sqlite / redis）

三选一（超过一个会直接报错）：

- `--state PATH`：JSON state file（只持久化 virtual keys）
- `--sqlite PATH`：sqlite store（需要 `--features gateway-store-sqlite`）
- `--redis URL`：redis store（需要 `--features gateway-store-redis`）

redis 相关：

- `--redis-env ENV`：从环境变量读取 redis url（可配合 `--dotenv`；需要 `gateway-store-redis`）
- `--redis-prefix PREFIX`：设置 redis key prefix（需要 `--redis`/`--redis-env`）

约束：

- `--redis` 与 `--redis-env` 互斥

---

## 5) Proxy cache（可选）

需要编译启用 `gateway-proxy-cache`：

- `--proxy-cache`：启用
- `--proxy-cache-ttl SECS`：TTL（隐式启用 cache；默认 60；最小 1）
- `--proxy-cache-max-entries N`：内存缓存容量（隐式启用 cache；默认 1024；最小 1）
- `--proxy-cache-max-body-bytes N`：单条响应最大 body bytes（隐式启用 cache；默认 1048576；最小 1）
- `--proxy-cache-max-total-body-bytes N`：内存缓存总 body budget（隐式启用 cache；默认 67108864；最小 1）

如果同时启用 redis store，cache 会作为 L2 写入 redis（共享）。

---

## 6) Proxy backpressure（强烈建议）

- `--proxy-max-in-flight N`：限制同时代理的请求数（超限 429；N 必须 > 0）

此外，`gateway.json.backends[].max_in_flight` 也会对单 backend 限并发（更细粒度）。

---

## 7) Proxy routing advanced（可选）

需要编译启用 `gateway-routing-advanced`：

- Retry：
  - `--proxy-retry`
  - `--proxy-retry-status-codes CODES`（逗号分隔，如 `429,500,502`）
  - `--proxy-retry-max-attempts N`
- Circuit breaker：
  - `--proxy-circuit-breaker`
  - `--proxy-cb-failure-threshold N`
  - `--proxy-cb-cooldown-secs SECS`
- Health checks：
  - `--proxy-health-checks`
  - `--proxy-health-check-path PATH`
  - `--proxy-health-check-interval-secs SECS`
  - `--proxy-health-check-timeout-secs SECS`

---

## 8) Costing（可选）

需要编译启用 `gateway-costing`：

- `--pricing-litellm PATH`：加载 LiteLLM 风格 pricing JSON（用于 cost budgets）

---

## 9) Prometheus（可选）

需要编译启用 `gateway-metrics-prometheus`：

- `--prometheus-metrics`：启用 `GET /metrics/prometheus`
- `--prometheus-max-key-series N`
- `--prometheus-max-model-series N`
- `--prometheus-max-backend-series N`
- `--prometheus-max-path-series N`

---

## 10) Devtools（可选）

需要编译启用 `gateway-devtools`（或 `gateway + sdk`）：

- `--devtools PATH`：输出 JSONL（用于调试/重放/离线分析）

---

## 11) OpenTelemetry（可选）

需要编译启用 `gateway-otel`：

- `--otel`：启用 tracing export
- `--otel-endpoint URL`：覆盖 OTLP HTTP endpoint
- `--otel-json`：把 tracing logs 输出成 JSON

`RUST_LOG` 控制日志级别（默认 `info`）。

---

## 12) 临时覆盖后端（高级）

这两组参数用于“运行时注入/覆盖”一部分后端配置：

- `--upstream name=base_url`：注入一个 OpenAI-compatible passthrough upstream（ProxyBackend）
- `--backend name=url`：给 `POST /v1/gateway` demo 注册一个 HttpBackend

它们主要用于快速试验；生产建议以 `gateway.json` 为准。
