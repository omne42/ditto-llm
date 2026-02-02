# 路由：Weighted / Fallback / Retry

本页覆盖两类“路由”：

1) **RouterConfig（静态路由）**：按 `model` 选择 backend，支持 rule + 权重 + fallback（见 `src/gateway/router.rs`）。  
2) **Proxy routing（运行时鲁棒性）**：重试 / 熔断 / 健康检查（feature `gateway-routing-advanced`，见 `src/gateway/proxy_routing.rs` 与 `src/gateway/http/proxy/core.rs`）。

---

## 1) RouterConfig：按模型选择 backend

### 配置结构

`gateway.json` 的 `router` 是 `RouterConfig`：

```json
{
  "router": {
    "default_backend": "primary",
    "default_backends": [],
    "rules": []
  }
}
```

- `default_backend`：最简单的默认 backend（必填，除非你用 `default_backends`）。
- `default_backends`：支持权重的默认候选集（会选出主 backend，并附带 fallback 顺序）。
- `rules[]`：按 `model_prefix` 覆盖路由；每条 rule 也可以用单 backend 或 weighted backends。

### Rule 匹配规则

- `model_prefix` 是前缀匹配：`model.starts_with(model_prefix)`。
- 第一条匹配的 rule 生效（按 `rules` 数组顺序）。

### Weighted 选择与 deterministic fallback 顺序

当你配置 `backends: [{backend, weight}, ...]` 时：

- Ditto 会对候选集做 **确定性**的加权选择（FNV-1a hash + weight 区间）。
- 返回结果不是“只返回一个 backend”，而是一个 **有序列表**：
  - 第一个是选中的“主 backend”
  - 后面的元素是去重后的 fallback 顺序（用于失败时尝试下一个）

这使得你可以写出 “9:1” 的主备分流，同时在主后端故障时有自然的 fallback。

示例：

```json
{
  "router": {
    "default_backend": "",
    "default_backends": [
      { "backend": "primary", "weight": 9 },
      { "backend": "backup", "weight": 1 }
    ],
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
}
```

### VirtualKeyConfig.route：固定路由（绕过规则）

如果某个 virtual key 设置了 `route: "<backend_name>"`：

- 该 key 的请求会直接路由到这个 backend
- router 的 `rules` 不再对它生效（包括 per-route 的 guardrails 覆盖）

这适合做：

- “VIP key” 固定走高配 backend
- 灰度 key 固定走新 upstream

---

## 2) Per-route Guardrails：按 model_prefix 覆盖策略

`RouteRule` 可以携带 `guardrails`：

```json
{
  "model_prefix": "gpt-4",
  "backends": [{ "backend": "primary", "weight": 1 }],
  "guardrails": {
    "allow_models": ["gpt-4*"],
    "deny_models": ["gpt-4o-realtime*"],
    "block_pii": true,
    "validate_schema": true
  }
}
```

实际生效逻辑：

- 先匹配到 rule 后，如果 rule 有 `guardrails`，则以它为准
- 否则回退到 `VirtualKeyConfig.guardrails`

---

## 3) Retry / Circuit Breaker / Health Checks（可选）

需要启用 feature `gateway-routing-advanced`，并在启动时通过 CLI 打开对应开关：

- `--proxy-retry` / `--proxy-retry-status-codes` / `--proxy-retry-max-attempts`
- `--proxy-circuit-breaker` / `--proxy-cb-failure-threshold` / `--proxy-cb-cooldown-secs`
- `--proxy-health-checks` / `--proxy-health-check-*`

这些选项会影响 `/v1/*` passthrough 的“backend 候选集如何被尝试/过滤”。

### 3.1 Retry（按状态码/网络错误）

默认 retry 状态码（可覆盖）：`429, 500, 502, 503, 504`（见 `ProxyRetryConfig`）。

重要细节：

- `max_attempts` 的默认值是 “候选 backend 数量”（即最多尝试一遍 fallback 列表）
- 只有当启用 retry 时才会对“失败”继续尝试下一个 backend

### 3.2 Circuit Breaker（按连续失败）

熔断器默认配置（可覆盖）：

- `failure_threshold = 3`
- `cooldown_seconds = 30`

计数策略（见 `FailureKind`）：

- 网络错误会计入失败
- retryable status 里 **只有 5xx** 会计入失败（例如 429 不计入熔断）

### 3.3 Health Checks（主动探活）

健康检查默认配置（可覆盖）：

- `path = /v1/models`
- `interval_seconds = 10`
- `timeout_seconds = 2`

实现细节：

- 健康检查以后台任务形式运行；当 `ditto-gateway` 退出/被 drop 时会自动取消，避免进程内残留任务。

启用后，gateway 会对每个 backend 定期发起 `GET <path>`：

- 2xx → healthy
- 非 2xx 或请求错误 → unhealthy

不健康的 backend 会在候选集中被过滤（若过滤后为空，则仍会回退到原候选集，避免“全拒绝”）。

### 3.4 运维接口：查看/重置 backend health

开启 admin token + `gateway-routing-advanced` 后：

- `GET /admin/backends`：返回每个 backend 的 health snapshot（连续失败、熔断到期、上次健康检查等）
- `POST /admin/backends/:name/reset`：清除某个 backend 的 health 状态（立刻视为健康）

---

## 4) 常见路由策略（建议）

- **主备（9:1）+ fallback**：用 weighted backends；失败自动落到 backup。
- **按模型族路由**：用 `rules[].model_prefix` 把 `gpt-4*`、`claude-*` 分流到不同 upstream。
- **按 key 固定路由**：用 `VirtualKeyConfig.route` 做灰度/专线。

下一步建议：

- 「预算与成本」：理解“路由之后如何计费/预算/预留”
- 「部署：多副本与分布式」：理解多副本下路由与 store 的关系
