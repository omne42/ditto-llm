# ProviderConfig 与 Profile

Ditto 通过 `ProviderConfig` + `Env` 把“连接 provider 所需的所有信息”集中管理，避免：

- base_url/token scattered 在代码里
- 不同模块各自拼 headers/query/auth，导致行为不一致
- Gateway 与 SDK 使用两套配置结构

在 Ditto 中，“Profile”是指围绕 provider 的配置、能力与模型发现的一组工具（位于 `src/profile`）。

## ProviderConfig：字段说明

`ProviderConfig` 是一个可序列化/反序列化的结构（可用于 JSON/TOML/YAML 等），核心字段：

- `base_url: Option<String>`
  - OpenAI-compatible upstream 的根地址（通常以 `/v1` 结尾）
  - 在 OpenAI 原生场景可以不填（使用默认 `https://api.openai.com/v1`）
- `default_model: Option<String>`
  - 默认模型 id；请求里 `GenerateRequest.model` 会优先覆盖它
- `model_whitelist: Vec<String>`
  - 用于模型发现（`/v1/models`）的 allowlist（为空表示不过滤）
- `http_headers: BTreeMap<String, String>`
  - 每个请求默认附加的 header（适合注入企业网关需要的自定义 header）
- `http_query_params: BTreeMap<String, String>`
  - 每个请求默认附加的 query params（例如 Azure 的 `api-version`）
- `auth: Option<ProviderAuth>`
  - provider 鉴权策略（env/command/header/query/sigv4/oauth 等）
- `capabilities: Option<ProviderCapabilities>`
  - provider 能力声明（用于路由/模型发现/策略判断；不等同于“硬保证”）

> 在 Gateway 配置里，backend 的 `provider_config` 字段就是同一份 `ProviderConfig` 结构。

## Env：统一 env 与 dotenv

`Env` 是一个很小的抽象：

- 先从 `Env.dotenv`（进程内注入的 dotenv map）查 key
- 再回退到 `std::env::var`

用途：

- 在 SDK 场景：你可以把 dotenv 内容作为字符串解析后注入，避免测试依赖真实环境变量
- 在 Gateway 场景：`--dotenv` 会把文件加载进 `Env`，同时也用于 `${ENV_VAR}` 占位符展开

## ProviderAuth：常见鉴权方式

Ditto 支持多种鉴权方式，覆盖企业网关的常见形态。

### 1) API key from env（最常见）

```json
{
  "auth": { "type": "api_key_env", "keys": ["OPENAI_API_KEY"] }
}
```

你可以把 env 的值设置为 `secret://...`，让 Ditto 在运行时解析（Vault/AWS/GCP/Azure/file/env）：

- `OPENAI_API_KEY=secret://env/REAL_OPENAI_API_KEY`
- `OPENAI_API_KEY=secret://file?path=/run/secrets/openai_api_key`
- `OPENAI_API_KEY=secret://vault/secret/openai?field=api_key`

### 2) 自定义 header（例如 `api-key`）

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

### 3) query param 鉴权（某些网关）

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

### 4) Command 鉴权（从外部命令取 token）

适合与 `aws-vault`、`gcloud auth print-access-token`、Vault CLI 等集成：

```json
{
  "auth": { "type": "command", "command": ["bash", "-lc", "security find-generic-password ..."] }
}
```

### 5) SigV4 / OAuth client credentials

用于 Bedrock（SigV4）与 Vertex（OAuth client credentials）等场景。字段较多，建议结合你组织的密钥管理方案统一下发。

#### SigV4（Bedrock / 自建 SigV4 网关）

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

> `access_keys/secret_keys/session_token_keys` 是 env key 列表（按顺序尝试）；如果你只用静态 AK/SK，可以只填前两项。

#### OAuth client credentials（Vertex / 企业 OAuth 网关）

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

> `client_id` / `client_secret` 字段也可以直接写死在配置里，但生产环境强烈不建议（用 env/secret 管理更安全）。

## ProviderOptions：请求级的 provider 特有字段

除了 `ProviderConfig` 的“连接/默认配置”外，Ditto 还提供 `GenerateRequest.provider_options` 用于请求级覆盖。

`ProviderOptions`（强类型）目前聚焦 OpenAI Responses 常见能力：

- `reasoning_effort`
- `response_format`（JSON schema）
- `parallel_tool_calls`

同时也允许传任意 JSON（弱类型）用于 provider 特有字段。

### Bucketed provider_options（按 provider 分桶）

你可以把 provider_options 写成一个对象，按 provider 名称分桶：

```json
{
  "*": { "parallel_tool_calls": true },
  "openai": { "reasoning_effort": "medium" },
  "openai-compatible": { "parallel_tool_calls": false }
}
```

Ditto 会在请求时按 `provider` 选择合适 bucket，并做合并（`*` + provider-specific）。

## 模型发现（/v1/models）

对于 OpenAI-compatible upstream，Ditto 提供：

- `list_available_models(&ProviderConfig, &Env)`：直接拉取并返回 `Vec<String>`
- `OpenAiModelsProvider`：实现 `Provider` trait，可作为“模型列表提供者”注入到路由系统

这部分常用于：

- 网关路由：按模型列表自动分配 backend
- 配置校验：启动时验证 default_model 是否存在

## 安全建议

- 不要把 token 写进配置文件，优先使用 env / `--dotenv` / command / KMS/Vault。
- 对 `auth.command` 要做 allowlist 与超时，避免被滥用。
- 将 `http_headers`/`http_query_params` 视为敏感面：不要允许非可信请求覆盖这些字段（Gateway 已默认避免把 client auth 头透传到 upstream）。
