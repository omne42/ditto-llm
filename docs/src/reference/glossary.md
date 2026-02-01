# 术语表

## Provider

模型提供方（OpenAI / Anthropic / Google / Cohere / Bedrock / Vertex / OpenAI-compatible upstream 等）。

## Native adapter

Ditto 直接对接 provider 的原生 API（语义更完整、warnings 更准确）。

## OpenAI-compatible adapter

Ditto 通过 OpenAI-compatible API（通常是 `/v1/chat/completions` 等）对接 upstream。

## Gateway

Ditto 的 HTTP 服务（feature `gateway`），对外暴露 OpenAI-compatible 的 `/v1/*` surface，并提供控制面能力。

## Passthrough proxy

`ANY /v1/*` 原样转发到 OpenAI-compatible upstream（不变形）。

## Translation proxy

把 OpenAI in/out 翻译为 native provider 请求/响应（feature `gateway-translation`）。

## Backend

Gateway 内部用于处理请求的后端条目（`GatewayConfig.backends[]`），可以是：

- passthrough upstream（`base_url`）
- translation backend（`provider` + `provider_config`）

## Router / RouteRule

按 `model` 选择 backend 的规则系统（`RouterConfig` / `RouteRule`）。

## Virtual Key

对外发放给调用方的 API key，用来做鉴权、归因、预算、限流、路由与审计。

## Admin Token

管理面 `/admin/*` 的鉴权 token（只在启用时挂载）。

## Scope

预算/缓存等的“隔离维度”。

例如 budgets：

- key scope：`<virtual_key_id>`
- project scope：`project:<project_id>`
- user scope：`user:<user_id>`

## Budget（token budget）

按 token 计的总额度（`BudgetConfig.total_tokens`）。

## Cost budget

按美元计的总额度（`BudgetConfig.total_usd_micros`），需要 pricing table。

## Proxy cache

对 `/v1/*` passthrough 的非 streaming 成功响应做缓存（可选）。

## Control-plane cache

对 `POST /v1/gateway` demo 端点做缓存（可选）。

## Warnings

Ditto 明确暴露“能力差异/降级/参数处理”的机制（SDK 侧为 `Warning`；Gateway 可将其用于策略或观测）。
