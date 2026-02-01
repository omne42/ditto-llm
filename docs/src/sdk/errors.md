# 错误处理

Ditto 统一使用 `ditto_llm::Result<T>`（即 `Result<T, DittoError>`）。

## DittoError 结构

`DittoError` 主要覆盖：

- `Api { status, body }`
  - provider 返回非 2xx 时的错误（Ditto 会尽量把 body 读出来，便于排障）
- `Http(reqwest::Error)` / `Io(std::io::Error)` / `Json(serde_json::Error)`
  - 网络、IO、JSON 解析错误
- `InvalidResponse(String)`
  - 协议不符合预期、字段缺失、无法解析等“语义错误”
- `AuthCommand(String)`
  - `ProviderAuth::Command` 执行失败

## 生产建议

- **日志与脱敏**：`Api.body` 可能包含敏感信息；建议打日志前做脱敏或只保留摘要。
- **重试策略**：不要对所有 `DittoError` 盲目重试。一般建议只对：
  - 网络错误（`Http`）中的 transient 错误
  - 以及明确可重试的 `Api.status`（例如 429/502/503）
  做指数退避与上限控制。
- **错误聚合**：在 Gateway 场景里，推荐把请求 id（`x-request-id`/`x-ditto-request-id`）贯穿到日志与 trace，以便跨组件定位。

## 常见错误与定位思路

- `InvalidResponse("... model is not set ...")`
  - 没有设置默认 model，或请求里没填 `request.model`。请设置 `with_model(...)` 或在 request 中指定。
- `Api { status: 401/403, ... }`
  - token 无效、header 名不对、或网关需要 query param auth。检查 `ProviderConfig.auth` 与 base_url。
- `Api { status: 404/405, ... }`
  - base_url 不对（例如忘了 `/v1`）、或 upstream 不支持某个端点（例如 `/v1/responses`）。
