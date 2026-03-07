# CLI 选项（ditto-gateway）

本页是 `ditto-gateway` 的运行参数速查（实现见 `src/bin/ditto_gateway/cli.rs` + `src/bin/ditto-gateway.rs`）。

> 当前 CLI 采用轻量参数解析：**没有 `--help`**。运行时缺少必填参数会打印 usage（并退出）。

---

## 1) 基本用法

`ditto-gateway` 的第一个参数必须是配置文件路径：

```bash
ditto-gateway <gateway.(json|yaml)> [flags...]
```

> YAML 配置需要编译启用 feature `gateway-config-yaml`（否则只支持 JSON）。

开发期常见用法：

```bash
cargo run --features gateway --bin ditto-gateway -- ./gateway.json --listen 0.0.0.0:8080
```

### 配置子命令（provider/model add）

当启用 `gateway` feature 时，`ditto-gateway` 也支持配置写入子命令（不启动网关进程）：

```bash
# `add` 与 `set` 等价（互为别名）
# 增量更新 provider（不会整文件覆盖）
cargo run --features gateway --bin ditto-gateway -- \
  provider add openrouter \
  --namespace google \
  --upstream-api gemini_generate_content \
  --auth-type api_key_env \
  --auth-key OPENROUTER_API_KEY \
  --base-url https://openrouter.ai/api/v1 \
  --scope workspace

# 增量更新 model（不会整文件覆盖）
cargo run --features gateway --bin ditto-gateway -- \
  model add gemini-3.1-pro \
  --provider google.providers.openrouter \
  --set-default \
  --scope workspace

# 列表 / 查看 / 删除
cargo run --features gateway --bin ditto-gateway -- provider list --namespace google
cargo run --features gateway --bin ditto-gateway -- provider show openrouter --namespace google
cargo run --features gateway --bin ditto-gateway -- provider delete openrouter --namespace google
cargo run --features gateway --bin ditto-gateway -- model list
cargo run --features gateway --bin ditto-gateway -- model show gemini-3.1-pro
cargo run --features gateway --bin ditto-gateway -- model delete gemini-3.1-pro

# 多步交互默认开启（参数可预填，剩余项交互补全）
cargo run --features gateway --bin ditto-gateway -- \
  provider add openrouter \
  --namespace google \
  --upstream-api gemini_generate_content

# 非交互（脚本场景）
cargo run --features gateway --bin ditto-gateway -- \
  provider add openrouter \
  --namespace google \
  --upstream-api gemini_generate_content \
  --no-interactive
```

说明：

- `provider/model add` 的底层实现在 `ditto-llm` 库（`upsert_provider_config` / `upsert_model_config`），Omne 也复用同一套方法。
- 默认是 merge update（增量更新）；不会清空已有无关配置。

---

## 2) 配置与运行（Core）

- `--listen HOST:PORT`（或 `--addr`）：监听地址（默认 `127.0.0.1:8080`）
- `--dotenv PATH`：加载 dotenv 文件（供 `${ENV_VAR}` 展开与 `*-env` 选项读取）
- `--json-logs`：输出 Ditto 自定义的 JSON 行事件日志（stderr）

---

## 3) Admin（管理面）

- `--admin-token TOKEN`：启用 `/admin/*` 并设置 **write admin token**（可执行写操作）
- `--admin-token-env ENV`：从环境变量读取 write admin token（可配合 `--dotenv`）
- `--admin-read-token TOKEN`：启用 `/admin/*` 并设置 **read-only admin token**（只读）
- `--admin-read-token-env ENV`：从环境变量读取 read-only admin token（可配合 `--dotenv`）
- `--admin-tenant-token TENANT_ID=TOKEN`：启用 `/admin/*` 并设置 **tenant-scoped write token**（只能管理该 tenant 的 keys/budgets/costs/audit）
- `--admin-tenant-token-env TENANT_ID=ENV`：从环境变量读取 tenant-scoped write token（可配合 `--dotenv`）
- `--admin-tenant-read-token TENANT_ID=TOKEN`：启用 `/admin/*` 并设置 **tenant-scoped read-only token**
- `--admin-tenant-read-token-env TENANT_ID=ENV`：从环境变量读取 tenant-scoped read-only token（可配合 `--dotenv`）

说明：

- `TOKEN` 也可以是 `secret://...`（见「Gateway 安全与加固」的 secret 管理章节）。

约束：

- `--admin-token` 与 `--admin-token-env` 互斥
- `--admin-read-token` 与 `--admin-read-token-env` 互斥

说明：

- 如果只配置 `--admin-read-token*`（不配置 `--admin-token*`），则写端点不会挂载（404）。

---

## 4) 存储（state / sqlite / pg / mysql / redis）

可以并行启用多个持久层（例如 `--sqlite` + `--pg` 做双写）。

- `--state PATH`：JSON state file（持久化 `virtual_keys` + `router`）
- `--sqlite PATH`：sqlite store（需要 `--features gateway-store-sqlite`）
- `--pg URL` / `--pg-env ENV`：postgres store（需要 `--features gateway-store-postgres`）
- `--mysql URL` / `--mysql-env ENV`：mysql store（需要 `--features gateway-store-mysql`）
- `--redis URL`：redis store（需要 `--features gateway-store-redis`）

可选（适用于持久层）：

- `--audit-retention-secs SECS`：审计日志保留期（只保留最近 `SECS` 秒；启用持久层时默认 30 天；设置为 `0` 表示不做清理）
- `--db-doctor`：只执行存储层 schema 自检并退出（任一已配置 store 自检失败即进程失败）

redis 相关：

- `--redis-env ENV`：从环境变量读取 redis url（可配合 `--dotenv`；需要 `gateway-store-redis`）
- `--redis-prefix PREFIX`：设置 redis key prefix（需要 `--redis`/`--redis-env`）

约束：

- `--redis` 与 `--redis-env` 互斥
- `--pg`/`--postgres` 与 `--pg-env`/`--postgres-env` 互斥
- `--mysql` 与 `--mysql-env` 互斥

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
- `--proxy-max-body-bytes N`：限制 `/v1/*` 入口请求体最大 bytes（默认 64MiB；N 必须 > 0）
- `--proxy-usage-max-body-bytes N`：限制为了解析 `usage` 而缓冲的 **非 streaming JSON 响应**最大 bytes（默认 1MiB；`0` 表示禁用 usage 缓冲并回退到估算）

此外，`gateway.json.backends[].max_in_flight` 也会对单 backend 限并发（更细粒度）。

---

## 7) Proxy routing advanced（可选）

需要编译启用 `gateway-routing-advanced`：

- Retry：
  - `--proxy-retry`
  - `--proxy-retry-status-codes CODES`（逗号分隔，如 `429,500,502`）
  - `--proxy-fallback-status-codes CODES`（命中状态码时直接 fallback 到下一个 backend；不依赖 `--proxy-retry`）
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
