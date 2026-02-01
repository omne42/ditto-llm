# 安全与加固

本页只覆盖 Ditto Gateway **自身实现提供的**安全控制点，以及推荐的部署加固方式。

> 任何 API gateway 的安全都不是“单点开关”，建议把 Ditto 视为你平台的一部分：外层有网络边界/认证授权/审计，内层有预算/策略/路由治理。

---

## 1) Secret 管理：避免把 token 写进配置

推荐用 `${ENV_VAR}` 占位符，并通过 `--dotenv`（开发）或运行环境注入（生产）：

- `backends[].headers` / `query_params`
- `backends[].provider_config.*`
- `virtual_keys[].token`

缺失/为空的 env 会导致启动失败（避免 silent misconfig）。

---

## 2) Virtual Keys：把“对外 API key”当作一等公民

启用 virtual keys 后（`gateway.json.virtual_keys` 非空）：

- 所有 `/v1/*` 请求必须携带 virtual key
- Ditto 会把客户端的 `authorization` / `x-api-key` 当作 virtual key，并在转发 upstream 前移除，避免泄露

建议：

- key 默认最小权限：限制 `allow_models`、启用 `validate_schema`、设置预算与并发上限
- 定期轮换 key，并通过 Admin API 下线旧 key

---

## 3) Admin API：只在启用 admin token 时开放

只有当你显式设置 `--admin-token`/`--admin-token-env` 时，Ditto 才会挂载 `/admin/*`。

部署建议：

- 把 `/admin/*` 放在内网
- 或者由反向代理加一层 IP allowlist / mTLS / WAF

---

## 4) Guardrails：内容/模型/Schema 的“入口拦截”

每个 virtual key 都可以配置 `guardrails`（`GuardrailsConfig`）：

- `allow_models` / `deny_models`：模型 allow/deny（支持 `prefix*` 通配）
- `max_input_tokens`：限制输入 token（基于估算，配合 `gateway-tokenizer` 更准）
- `banned_phrases` / `banned_regexes`：按文本包含/正则拦截（case-insensitive）
- `block_pii`：内置 email/SSN 的粗粒度 PII 拦截
- `validate_schema`：对常见 OpenAI 请求做 schema 校验（JSON 与 multipart 都覆盖），不合规直接 400

此外，你还可以在 `router.rules[].guardrails` 做“按模型前缀覆盖”的策略（见「路由」）。

---

## 5) Passthrough 控制（仅 /v1/gateway demo）

`PassthroughConfig` 用于 `/v1/gateway` demo 端点（不是 `/v1/*` passthrough proxy）：

- `allow=false` 可以禁止 `passthrough=true` 的请求
- `bypass_cache=true` 可以在 passthrough 请求时绕过 control-plane cache

---

## 6) 资源滥用防护（DoS 基线）

建议至少做三件事：

1) `--proxy-max-in-flight` 限制全局并发
2) `backends[].max_in_flight` 限制单 backend 并发
3) `backends[].timeout_seconds` 设置合理超时

补充：

- 对大响应（files/audio download）谨慎开启 proxy cache
- 对不可信客户端启用 `validate_schema`，避免无意义的大 payload 打到 upstream

---

## 7) 企业级安全缺口（诚实说明）

当前 Ditto Gateway 还不内置：

- RBAC/SSO（多角色、多租户权限模型）
- 可配置的 IP allow/deny、mTLS 终止、WAF 集成
- 分布式限流（全局 rpm/tpm）

这些能力通常由外层 API gateway / service mesh 提供；Ditto 侧的补齐计划见 Roadmap。
