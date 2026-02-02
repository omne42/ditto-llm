# 预算与成本（Tokens / USD）

本页解释 Ditto Gateway 的“配额治理”三件套：

1) **Limits（速率限制）**：rpm / tpm（`LimitsConfig`，内存实现）。  
2) **Budget（token 预算）**：`BudgetConfig.total_tokens`。  
3) **Cost budget（美元预算，可选）**：`BudgetConfig.total_usd_micros` + pricing table（feature `gateway-costing`）。

实现位置：

- `src/gateway/limits.rs`
- `src/gateway/budget.rs`
- `src/gateway/costing.rs`（可选）
- `/v1/*` proxy 预算预留：`src/gateway/http/openai_compat_proxy.rs` + `src/gateway/http/proxy/budget_reservations.rs`

---

## 1) Limits：rpm / tpm（默认进程内；Redis 可分布式）

每个 `VirtualKeyConfig` 都有 `limits`：

```json
{
  "limits": { "rpm": 60, "tpm": 20000 }
}
```

- `rpm`：每分钟请求数上限
- `tpm`：每分钟 token 上限（token 计算见下文）

实现有两种模式：

- **不启用 store / 使用 sqlite**：进程内计数（按分钟窗口滚动），单实例可用；多副本下每个副本各算各的，不等价于全局限流。
- **使用 redis store**（`gateway-store-redis` + `--redis`）：通过 Redis 原子计数实现 **全局一致** 的 rpm/tpm（按 virtual key id 维度；窗口=分钟；计数 key 带 TTL，避免无界增长）。

> 如果你需要更复杂的策略（滑动窗口、按 IP、按 route 分组等），仍建议外层 API gateway 承接；Ditto 也会在后续里程碑继续扩面（见 Roadmap）。

### 1.1 Tenant/Project/User shared limits（可选）

除了 key 自身的 `limits` 外，你还可以配置“聚合限流”（多个 key 共享一个限流桶）：

- `tenant_id` + `tenant_limits`
- `project_id` + `project_limits`
- `user_id` + `user_limits`

当启用 redis store 时，上述 shared limits 也会变成 **多副本全局一致** 的限流。

示例：

```json
{
  "id": "vk-1",
  "token": "${VK_1}",
  "enabled": true,
  "tenant_id": "tenant-a",
  "tenant_limits": { "rpm": 600, "tpm": 200000 },
  "project_id": "proj-a",
  "project_limits": { "rpm": 120, "tpm": 40000 },
  "user_id": "user-42",
  "user_limits": { "rpm": 30, "tpm": 8000 },
  "limits": { "rpm": 60, "tpm": 20000 }
}
```

---

## 2) Token Budget：`total_tokens`

`VirtualKeyConfig.budget.total_tokens` 控制“总 token 额度”：

```json
{
  "budget": { "total_tokens": 5000000 }
}
```

一条请求的 **charge_tokens**（计费 token）在 `/v1/*` proxy 中按以下方式估算：

- `input_tokens_estimate`：
  - 若启用 `gateway-tokenizer`：尽量用 tiktoken 对 OpenAI 请求做 token 估算
  - 否则：fallback 为 `body_bytes_len / 4` 的粗估
- `charge_tokens = input_tokens_estimate + max_output_tokens`

其中 `max_output_tokens` 会从请求 JSON 中抽取（若缺失则按内部默认值处理）。

### 2.1 Tenant/Project/User 预算（可选）

除了 key 自身预算外，Ditto 支持额外的“聚合预算”：

- `tenant_id` + `tenant_budget`
- `project_id` + `project_budget`
- `user_id` + `user_budget`

示例：

```json
{
  "id": "vk-1",
  "token": "${VK_1}",
  "enabled": true,
  "tenant_id": "tenant-a",
  "project_id": "proj-a",
  "user_id": "user-42",
  "tenant_budget": { "total_tokens": 5000000 },
  "project_budget": { "total_tokens": 1000000 },
  "user_budget": { "total_tokens": 200000 },
  "budget": { "total_tokens": 5000000 },
  "limits": {},
  "cache": {},
  "guardrails": {},
  "passthrough": { "allow": true, "bypass_cache": true },
  "route": null
}
```

一条请求会同时消耗：

- key 预算（scope：`<key.id>`）
- tenant 预算（scope：`tenant:<tenant_id>`）
- project 预算（scope：`project:<project_id>`）
- user 预算（scope：`user:<user_id>`）

只要任意一个 scope 超额，就会被拒绝（OpenAI 风格错误：HTTP 402 `insufficient_quota`）。

---

## 3) 持久化预算：sqlite / redis（推荐用于生产）

当你启用 store（`--sqlite` 或 `--redis`）时，`/v1/*` proxy 会切换到“预算预留 + 结算”模式：

- **预留（reserve）**：在请求进入 upstream 前，按 `charge_tokens` 先把额度预留起来（避免并发穿透）。
- **结算（commit/rollback）**：请求结束后，根据是否成功、以及是否能观测到真实 usage，提交或回滚预留。

为什么需要预留？

- streaming 场景下，最终 usage 可能要到最后一个 chunk 才出现
- 并发场景下，不预留会导致“同时通过检查 → 同时超额”

实现细节：

- reservation id 基于 `request_id`：
  - key 预算：`<request_id>`
  - tenant/project/user 预算：`<request_id>::budget::<scope>`
- redis 预留记录带 TTL，避免异常中断导致永久占用（见 `DEFAULT_RESERVATION_TTL_SECS`）。

### 3.1 sqlite vs redis 的选择

- `--sqlite <path>`：单机持久化（重启不丢），不支持多副本共享。
- `--redis <url> [--redis-prefix p]`：多副本共享（推荐用于分布式部署）。

---

## 4) Cost Budget：`total_usd_micros`（可选）

### 4.1 启用条件

要启用“美元预算”，需要同时满足：

- 编译启用 feature `gateway-costing`
- 运行时通过 `--pricing-litellm <path>` 加载 LiteLLM 风格的 pricing JSON
- 你的 key/tenant/project/user budget 中至少一个设置了 `total_usd_micros`

示例（1 美元 = 1_000_000 micros）：

```json
{ "budget": { "total_usd_micros": 1000000 } }
```

如果配置了 cost budget 但没有 pricing table，Ditto 会返回 500（`pricing_not_configured`），避免 silently 不计费。

### 4.2 估算策略（best-effort）

Ditto 会对请求做 cost 估算：

- 以请求的 `model` 为主
- 若某个 backend 配置了 `model_map`，并且 pricing 表里存在映射后的 model，则会取“更保守”的估算（在多个 backend 候选时取 max）
- 支持 LiteLLM 的 prompt-cache 成本字段（若响应 usage 提供）
- 支持 `service_tier`（若请求带该字段且 pricing 支持）

与 token 预算类似，cost 预算在启用 store 后也会“预留 + 结算”，并可在 `/admin/costs*` 查看 ledger。

---

## 5) 建议的生产配置组合

- 需要稳定的 token 预算：启用 `gateway-store-redis` 或 `gateway-store-sqlite`
- 需要多副本一致：优先 `gateway-store-redis`
- 需要美元预算：再加 `gateway-costing` + `--pricing-litellm`
- 需要更准确 token 估算：再加 `gateway-tokenizer`

下一步：

- 「缓存」：理解预算与缓存（cache hit 是否计费）之间的边界
- 「Admin API」：如何查看 ledger / audit / 动态管理 keys
