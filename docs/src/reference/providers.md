# Providers 能力矩阵

本页是“查表型”入口，帮助你快速回答：

- 我该启用哪些 Cargo features？
- 某个 provider 支持哪些能力（stream/tools/embeddings/…）？
- Gateway 的 passthrough 与 translation 分别覆盖到什么程度？

更完整的矩阵请直接看仓库根目录的 `PROVIDERS.md`（它会随代码演进而更新）。

---

## 1) Ditto 的两条集成路径

### A) Native adapters（推荐）

直接调用 provider 的原生 API，语义最完整、兼容性最清晰：

- OpenAI：Responses API（`/responses`）
- Anthropic：Messages API（`/messages`）
- Google：GenAI（`generateContent` / `streamGenerateContent`）
- Cohere：Chat v2（`/v2/chat`）
- Bedrock / Vertex：各自的认证与 API（feature-gated）

适用：

- 你在 Rust 服务端直接集成模型
- 你希望 Warnings 明确暴露能力差异

### B) OpenAI-compatible adapters（务实）

通过 OpenAI-compatible upstream（例如 LiteLLM Proxy、各类兼容网关或厂商兼容层）：

- Chat Completions（`/chat/completions`）
- Embeddings（`/embeddings`）
- 以及 upstream 自己支持的其它 OpenAI 端点（视具体实现）

适用：

- 你已经有一个“兼容 OpenAI API 的统一入口”
- 你需要把 Ditto 放在现有网关/平台上逐步替换

---

## 2) Provider features（Cargo）

常见 provider feature：

- `openai`
- `openai-compatible`
- `anthropic`
- `google`
- `cohere`
- `bedrock`（依赖 `auth` + `base64`）
- `vertex`（依赖 `auth`）

你也可以直接用 bundles：

- `all-providers`
- `all`

---

## 3) Capability features（Cargo）

Ditto 把能力拆成独立 feature，避免默认拉入所有依赖：

- `streaming`
- `tools`
- `embeddings`
- `images`
- `audio`
- `moderations`
- `rerank`
- `batches`

以及 bundles：

- `all-capabilities`
- `all`

> 注意：某个能力 feature 打开，只代表 Ditto **提供了相应 trait 与实现入口**；具体 provider 是否支持，还要看 provider 自身能力与请求映射（不支持会产出 `Warning`）。

---

## 4) Gateway 相关 features（Cargo）

Gateway 的能力也拆成多个 feature：

- `gateway`：HTTP server + passthrough proxy（`ANY /v1/*`）+ 基础控制面
- `gateway-translation`：OpenAI in/out → native providers（translation backends）
- `gateway-proxy-cache`：非 streaming proxy cache
- `gateway-routing-advanced`：retry/circuit-breaker/health-checks
- `gateway-store-sqlite` / `gateway-store-redis`：持久化 store
- `gateway-costing`：美元预算（需要 pricing table）
- `gateway-tokenizer`：更准确的 token 估算（tiktoken）
- `gateway-metrics-prometheus`：Prometheus 指标
- `gateway-otel`：OpenTelemetry tracing

推荐组合（多副本生产常见）：

- `gateway`
- `gateway-store-redis`
- `gateway-routing-advanced`（可选）
- `gateway-proxy-cache`（可选）
- `gateway-metrics-prometheus` 或 `gateway-otel`（可选）
- `gateway-tokenizer`（可选：预算更准）

---

## 5) 快速决策：我该用哪个 provider 路径？

- 你只需要“一个 HTTP 入口”并且 upstream 已经统一：**OpenAI-compatible**（SDK 或 Gateway passthrough）。
- 你要拿到最完整语义（tool calling / structured outputs / provider options）：**Native adapters**。
- 你需要“OpenAI API surface → Anthropic/Google/Cohere 等原生 API”的转换：启用 **Gateway translation**（`gateway-translation`）。

下一步：

- 继续阅读「SDK → ProviderConfig 与 Profile」
- 或阅读「Gateway → 配置文件」「Gateway → HTTP Endpoints」
