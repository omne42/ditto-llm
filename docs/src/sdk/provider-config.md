# ProviderConfig 与 Profile

Ditto 里的 `ProviderConfig` 只描述一件事：**一个 provider node 的连接方式与默认行为**。

这里的“node”指一个具体可访问的上游入口，例如：

- 一个 OpenAI-compatible `/v1` 根地址
- 一个 Google GenAI `v1beta` 根地址
- 一个 Anthropic `/v1` 入口
- 一个带固定 headers / query params / auth 的企业代理节点

`ProviderConfig` 不是以下这些东西：

- 不是 provider 品牌目录
- 不是模型 catalog
- 不是“整个 OpenAI / Google / Anthropic 家族”的总配置
- 不是请求级的临时参数容器

如果把这些语义混在一起，最后就会把 `base_url`、鉴权、协议面、模型目录、请求参数全部塞进一个松散对象里，配置层和运行时都会开始漂。

在 Ditto 中，围绕 provider 的职责边界应当这样理解：

- `provider`：选择哪个运行时适配器或 provider pack，例如 `openai-compatible`、`openai`、`anthropic`、`google`
- `ProviderConfig`：给这个运行时提供一个具体 node 的连接参数与默认值
- catalog / registry：描述某个 provider pack 声明了哪些模型、能力、协议面
- `GenerateRequest.provider_options`：请求级覆盖，只影响当前一次调用

## ProviderConfig：字段边界

`ProviderConfig` 是一个可序列化/反序列化的结构（JSON/TOML/YAML 都可以），但它的语义必须保持为“单个 node 配置”。

- `provider: Option<String>`
  - 这个 node 绑定到哪个运行时 provider pack / adapter
  - 例如 `openai-compatible`、`openai`、`google`、`anthropic`
  - 它解决的是“由谁负责跑这个 node”，不是上游品牌展示名
- `enabled_capabilities: Vec<String>`
  - 这个 node 显式启用哪些一级 capability category，例如 `llm`、`embedding`、`image.generation`
  - 这些值会由 registry 校验，不能声明 provider pack 根本不支持的 capability
  - 它和 `capabilities` 不同：前者是一级 capability 开关，后者是 LLM 子能力表（如 `tools`、`vision`、`reasoning`）
- `base_url: Option<String>`
  - 这个 node 的根地址
  - 对 OpenAI-compatible 通常是 `/v1`
  - 对 Google 原生通常是 `https://generativelanguage.googleapis.com/v1beta`
  - 对官方 OpenAI 可留空，走默认 `https://api.openai.com/v1`
- `default_model: Option<String>`
  - 这个 node 的默认模型
  - 请求里的 `GenerateRequest.model` 会优先覆盖它
- `model_whitelist: Vec<String>`
  - 这个 node 在模型发现时允许暴露的模型白名单
  - 它是 node 级过滤器，不是 catalog 真相源
- `http_headers: BTreeMap<String, String>`
  - 每次请求都会附加到这个 node 的固定 headers
  - 适合企业代理、自定义 trace header、厂商要求的固定 header
- `http_query_params: BTreeMap<String, String>`
  - 每次请求都会附加到这个 node 的固定 query params
  - 典型场景是 Azure 风格 `api-version`
- `auth: Option<ProviderAuth>`
  - 这个 node 的鉴权方式
  - 支持 env、command、自定义 header/query、SigV4、OAuth client credentials 等
- `capabilities: Option<ProviderCapabilities>`
  - 这个 node 的能力声明
  - 它是 node 级提示与约束输入，不应被误用为“整个 provider 品牌的完整能力描述”
- `upstream_api: Option<ProviderApi>`
  - 这个 node 上游实际暴露的 API surface
  - 例如 `openai_chat_completions`、`openai_responses`、`gemini_generate_content`、`anthropic_messages`
  - 它描述“打哪个接口面”，不是 provider 名称
- `normalize_to: Option<ProviderApi>`
  - 这个 node 的响应在 Ditto 内部或向下游输出时，要被归一到哪个 API surface
  - 常用于 translation / proxy / 兼容输出路径
- `normalize_endpoint: Option<String>`
  - 需要强制指定归一后的目标 endpoint 时使用
  - 它是 endpoint/path 级覆盖，不是 provider 识别字段

> 在 Gateway 配置里，translation backend 的 `provider_config` 字段就是同一个 `ProviderConfig`。它仍然只描述 backend 连接到上游 node 的参数，不负责声明整个 provider catalog。

## 一个最小的 node 示例

OpenAI-compatible node：

```toml
provider = "openai-compatible"
enabled_capabilities = ["llm"]
base_url = "https://proxy.example.com/v1"
default_model = "gpt-4o-mini"
auth = { type = "api_key_env", keys = ["OPENAI_COMPAT_API_KEY"] }
http_headers = { "x-tenant" = "acme" }
```

Google 原生 node：

```toml
provider = "google"
enabled_capabilities = ["llm"]
base_url = "https://generativelanguage.googleapis.com/v1beta"
default_model = "gemini-2.5-pro"
upstream_api = "gemini_generate_content"
auth = { type = "query_param_env", param = "key", keys = ["GOOGLE_API_KEY"] }
```

上面两个例子表达的都是“一个具体 node”。

它们没有回答这些问题：

- Google 有哪些模型
- 这个品牌总体支持哪些能力
- 某个请求临时要不要加 `reasoning_effort`

这些问题分别属于 catalog / registry 和 request-level provider options。

## Env：统一 env 与 dotenv

`Env` 是一个很小的抽象：

- 先从 `Env.dotenv`（进程内注入的 dotenv map）查 key
- 再回退到 `std::env::var`

用途：

- SDK：你可以把 dotenv 内容作为字符串解析后注入，避免测试依赖真实环境变量
- Gateway：`--dotenv` 会把文件加载进 `Env`，同时也用于 `${ENV_VAR}` 占位符展开

## ProviderAuth：node 的鉴权策略

Ditto 支持多种 node 级鉴权方式。

### 1) API key from env

```json
{
  "auth": { "type": "api_key_env", "keys": ["OPENAI_API_KEY"] }
}
```

### 2) 自定义 header

```json
{
  "auth": {
    "type": "http_header_env",
    "header": "api-key",
    "keys": ["AZURE_OPENAI_API_KEY"],
    "prefix": null
  }
}
```

### 3) query param 鉴权

```json
{
  "auth": {
    "type": "query_param_env",
    "param": "api_key",
    "keys": ["GATEWAY_API_KEY"],
    "prefix": null
  }
}
```

### 4) command 鉴权

```json
{
  "auth": { "type": "command", "command": ["gcloud", "auth", "print-access-token"] }
}
```

`command` 的 stdout 支持：

- 纯文本 token
- JSON string：`"sk-..."`
- JSON object：`{"api_key":"..."}` / `{"token":"..."}` / `{"access_token":"..."}`

Ditto 会对 stdout 做 `trim()`，并施加默认安全边界：

- 超时：默认 15s，可通过 `DITTO_AUTH_COMMAND_TIMEOUT_MS/SECS` 调整
- 输出上限：stdout/stderr 各 64KiB

### 5) SigV4 / OAuth client credentials

用于 Bedrock（SigV4）与 Vertex / 企业 OAuth 网关等场景。

SigV4：

```json
{
  "auth": {
    "type": "sigv4",
    "access_keys": ["AWS_ACCESS_KEY_ID"],
    "secret_keys": ["AWS_SECRET_ACCESS_KEY"],
    "session_token_keys": ["AWS_SESSION_TOKEN"],
    "region": "us-east-1",
    "service": "bedrock"
  }
}
```

OAuth client credentials：

```json
{
  "auth": {
    "type": "oauth_client_credentials",
    "token_url": "https://example.com/oauth/token",
    "client_id_keys": ["OAUTH_CLIENT_ID"],
    "client_secret_keys": ["OAUTH_CLIENT_SECRET"],
    "scope": "https://www.googleapis.com/auth/cloud-platform",
    "audience": null,
    "extra_params": {}
  }
}
```

## ProviderOptions：请求级，不是 node 级

`GenerateRequest.provider_options` 用于请求级覆盖。

它和 `ProviderConfig` 的边界必须明确：

- `ProviderConfig`：这个 node 平时怎么连、默认用什么模型、附带什么鉴权/header/query
- `provider_options`：这一次请求要给 provider 传什么额外参数

`ProviderOptions`（强类型）目前聚焦常见生成参数：

- `reasoning_effort`
- `response_format`
- `parallel_tool_calls`

同时也允许传任意 JSON（弱类型）用于 provider 特有字段。

### Bucketed provider_options（按 provider 分桶）

```json
{
  "*": { "parallel_tool_calls": true },
  "openai": { "reasoning_effort": "medium" },
  "openai-compatible": { "parallel_tool_calls": false }
}
```

Ditto 会按当前 provider 选取 bucket，并做 `*` + provider-specific 合并。

## 模型发现

`list_available_models(&ProviderConfig, &Env)` 主要用于“拿这个 node 实际暴露了哪些模型”。

这和内建 catalog 的语义不同：

- node 发现：问当前上游入口此刻能列出什么
- catalog / registry：Ditto 自己对 provider / model / capability 的结构化认识

常见用途：

- 路由前探测某个 OpenAI-compatible node 当前可用模型
- 启动时校验 `default_model` 是否存在
- 对 node 结果再叠加 `model_whitelist` 做收敛

## 安全建议

- 不要把 token 明文写进配置文件，优先用 env / `--dotenv` / command / secret manager
- 对 `auth.command` 做 allowlist 与超时控制
- 把 `http_headers` / `http_query_params` 视为敏感面，不要让不可信输入覆盖它们
- 不要把“品牌目录”“模型目录”“请求临时参数”混进 `ProviderConfig`
