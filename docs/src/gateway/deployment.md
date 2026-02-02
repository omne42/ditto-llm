# 部署：多副本与分布式

Ditto Gateway 的部署目标是“尽量无状态”，把状态放到外部 store（尤其是 Redis），从而支持多副本与滚动升级。

本页的建议以当前实现为准（见 `src/bin/ditto-gateway.rs`、`src/gateway/http/core.rs`、`src/gateway/redis_store/*`）。

---

## 1) 最小部署拓扑（单实例）

- 1 个 `ditto-gateway` 实例
- 1 份 `gateway.json`
- 上游是 OpenAI-compatible upstream（OpenAI / LiteLLM / 其他兼容网关）

启动：

```bash
cargo run --features gateway --bin ditto-gateway -- ./gateway.json --listen 0.0.0.0:8080
```

健康检查：

- `GET /health` → `{"status":"ok"}`

---

## 2) 多副本（stateless）会遇到什么问题？

把 `ditto-gateway` 横向扩容到 N 个副本后，如果你不配置共享 store，会出现：

- virtual keys：每个副本只读自己的 `gateway.json`；Admin API 修改也不会同步。
- token/cost budgets：每个副本各算各的，无法形成“全局预算”。
- proxy cache：每个副本各有一份内存缓存，命中率低且不一致。

因此“多副本”要想具备“企业级一致性”，核心是引入共享存储。

---

## 3) 推荐：Redis 作为共享状态（分布式）

启用条件：

- 编译启用 feature `gateway-store-redis`
- 启动时传 `--redis <url>`（建议配置 `--redis-prefix`）

```bash
cargo run --features "gateway gateway-store-redis" --bin ditto-gateway -- ./gateway.json \
  --redis redis://redis:6379 --redis-prefix ditto \
  --listen 0.0.0.0:8080
```

有了 redis store 后，你可以做到：

- virtual keys 全局一致（Admin API 修改后立刻对所有副本生效）
- token/cost budgets 预留/结算全局一致（避免并发穿透）
- audit logs 全局收集
- proxy cache 可选写入 redis（作为 L2，见「缓存」）

---

## 4) 运行时保护：并发与超时（防止内存被打爆）

Ditto 的 proxy 会在一些位置“把内容读入内存”：

- `/v1/*` 请求体会先读入内存（默认上限 64MiB；可用 `--proxy-max-body-bytes` 调整）
- 非 streaming 响应会**尽量流式转发**；当响应体积可确定且较小（用于从 JSON 提取 `usage` 做更准的结算，或写入 proxy cache）时才会缓冲读取；其中 `usage` 缓冲上限由 `--proxy-usage-max-body-bytes`（默认 1MiB）控制，与 `--proxy-cache-max-body-bytes` 解耦；无 `content-length` 或超过上限时会跳过缓冲/缓存并直接流式转发

生产建议至少打开两类“背压”：

### 4.1 全局并发：`--proxy-max-in-flight`

限制“同时在代理中的请求数”，超限会 429。

```bash
... --proxy-max-in-flight 256
```

### 4.2 后端并发：`backends[].max_in_flight`

对某个 backend 单独限并发：

```json
{
  "name": "primary",
  "base_url": "https://api.openai.com/v1",
  "max_in_flight": 64
}
```

### 4.3 后端超时：`backends[].timeout_seconds`

避免某个 upstream 挂死导致连接长期占用：

```json
{ "timeout_seconds": 60 }
```

---

## 5) 多副本下的配置发布

建议把配置拆成两部分：

- `gateway.json`：非敏感配置（backends/router/策略骨架）
- `.env` / Secret：敏感信息（upstream token / virtual key / admin token / redis url）

启动时通过 `--dotenv`（开发）或容器环境变量（生产）注入 `${ENV_VAR}`：

```bash
... --dotenv .env
```

---

## 6) 企业落地的现实边界（当前 vs 需要补齐）

当前 Ditto Gateway 已经具备多副本运行所需的关键积木（redis store + 预算预留 + 可选共享缓存），但“企业级调用”通常还需要：

- 分布式限流：使用 redis store 时 rpm/tpm 已全局一致（按 virtual key）；更细粒度维度（project/user/route）仍在 Roadmap
- RBAC/SSO、多租户隔离、权限模型
- 配置中心/灰度发布、不可变审计、告警

这些条目见「Roadmap → 企业与合规能力清单」。
