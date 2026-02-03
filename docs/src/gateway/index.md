# Gateway（LiteLLM-like）

Ditto-LLM 的 Gateway 是一个可选启用的 HTTP 服务（feature `gateway`），对外提供 OpenAI-compatible 的 `/v1/*` surface，并在内部提供控制面能力：

- virtual keys（鉴权 / 归因）
- 限流（rpm/tpm）
- 预算（tokens / USD）
- 路由（weighted / fallback）
- 缓存（control-plane cache + optional proxy cache）
- 审计（可选持久化）
- 观测（request id / logs / Prometheus / OTel）
- MCP gateway（`/mcp*` + MCP tools 集成）
- A2A agents（`/a2a/*` JSON-RPC 代理）

## Passthrough vs Translation

Gateway 同时支持两类路径：

- **Passthrough proxy**：`ANY /v1/*` 原样转发到 OpenAI-compatible upstream（不变形）。
- **Translation proxy**（feature `gateway-translation`）：把 OpenAI in/out 翻译到 native providers（Anthropic/Google/...）。

建议先从「运行网关」开始，跑通最小配置，再逐步加上 virtual keys / redis / routing / observability。

如果你更偏好“按任务走”的教程（复制配置即可跑），请看「Gateway Recipes（可复制落地）」。
