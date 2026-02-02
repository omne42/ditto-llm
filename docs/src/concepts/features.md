# Feature Flags

Ditto-LLM 通过 Cargo features 控制体积与依赖：默认构建偏 SDK，Gateway 相关能力需要显式开启。

## 常用组合

- 只当 SDK 用（默认）：`openai` / `anthropic` / `openai-compatible` / `streaming` / `tools` / `embeddings`
- 全 provider：`--features all-providers`
- 全能力：`--features all-capabilities`
- 全部：`--features all`

## Gateway 相关

- `gateway`：启用 `ditto-gateway` HTTP 服务与控制面
- `gateway-translation`：启用 OpenAI in/out → native providers 的 translation endpoints
- `gateway-proxy-cache`：启用非 streaming 的 proxy cache（内存；可选写入 Redis）
- `gateway-devtools`：启用 `--devtools` JSONL 日志（等价于 `gateway` + `sdk`）
- `gateway-store-redis` / `gateway-store-sqlite`：启用持久化（分布式部署推荐 Redis）
- `gateway-routing-advanced`：启用 retry / circuit breaker / active health checks
- `gateway-metrics-prometheus`：启用 Prometheus metrics endpoint
- `gateway-otel`：启用 OpenTelemetry OTLP export

## Agent/SDK 工具

- `sdk`：stream protocol v1（NDJSON/SSE）与 devtools/telemetry/MCP 等工具适配；以及 SDK cache middleware（含 streaming replay）
- `agent`：ToolLoopAgent + tool executors（含 `safe-fs-tools`）

> 小提示：生产环境建议按需开启 features，避免把不必要依赖带进最终镜像。
