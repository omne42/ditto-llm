# 常见问题（FAQ）

## Q1：Ditto Gateway 现在算“企业级可用”了吗？

可以用于生产的部分：

- 多副本运行所需的核心积木（推荐：`gateway-store-redis`）
- virtual keys / budgets / audit / routing / 观测（取决于你启用的 feature）

尚缺的典型“企业治理项”：

- 分布式限流（rpm/tpm 全局一致）
- RBAC/SSO、多租户隔离、权限模型
- 配置中心、不可变审计、告警与运营面板

建议先按「部署：多副本与分布式」落地，再对照 Roadmap 逐步补齐。

---

## Q2：为什么我启用了多副本，但 budgets/limits 不一致？

- `limits`（rpm/tpm）目前是进程内计数，不会跨副本共享。
- `budgets` 如果没有启用 `--sqlite/--redis`，也不会持久化与共享。

解决：

- 多副本预算：用 `gateway-store-redis` + `--redis`
- 多副本限流：暂时建议外置（API gateway / service mesh）

---

## Q3：Ditto 会把我的 virtual key 转发给 upstream 吗？

不会（当启用 virtual keys 时）。

当 `gateway.json.virtual_keys` 非空时，Ditto 会把客户端 `authorization`/`x-api-key` 当作 virtual key，并在转发前移除，避免泄露。

上游鉴权应配置在 `backends[].headers/query_params` 或 translation backend 的 `provider_config.auth`。

---

## Q4：为什么 `--proxy-cache` 没有命中？

proxy cache 只缓存：

- `GET`/`POST`
- 非 streaming 响应
- 2xx 成功响应

并且以下情况会 bypass：

- 请求头有 `x-ditto-cache-bypass` / `x-ditto-bypass-cache`
- `Cache-Control` 包含 `no-store`/`no-cache`

---

## Q5：Ditto 的 token 预算是如何估算的？准确吗？

默认情况下（未启用 `gateway-tokenizer`），Ditto 用一个保守的粗估：`body_bytes_len / 4`。

如果你需要更准确的预算：

- 编译启用 `gateway-tokenizer`

即便启用了 tokenizer，预算仍然是 best-effort（不同 provider 的 token 规则不同），建议结合 usage 的最终统计做审计与校准。

---

## Q6：我能用 Ditto 做 OpenAI Responses 的兼容吗？

可以（passthrough proxy 的 shim 逻辑）。

当 upstream 不支持 `POST /v1/responses`（404/405/501），Ditto 会 fallback 到 `POST /v1/chat/completions` 并返回 best-effort Responses-like 结果，并附加：

- `x-ditto-shim: responses_via_chat_completions`

如果你遇到 502：

- 对非 streaming shim，Ditto 需要缓冲并转换 upstream 的 JSON 响应；为避免 OOM，存在最大缓冲上限（当前 8MiB）。超限时建议改用 streaming（SSE）或直接调用 `POST /v1/chat/completions`。

---

## Q7：我在 SDK 里如何发现模型列表？

Ditto 支持从 `ProviderConfig` 做模型发现（OpenAI-compatible 的 `/models`、以及部分 native provider 的能力）。

入口与字段解释见「SDK → ProviderConfig 与 Profile」。
