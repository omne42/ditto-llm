# 模板与示例

本页给你一组“可复制落地”的入口（对标 AI SDK docs 的 templates 思路）：你可以直接从仓库里的现成代码/配置开始，然后再回到 docs 深入理解原理与边界。

## Rust（SDK）示例

仓库根目录 `examples/` 下提供了覆盖主线能力的最小示例：

```bash
# 文本生成 / 流式 / tools / embeddings
cargo run --example basic
cargo run --example streaming
cargo run --example tool_calling
cargo run --example embeddings

# OpenAI-compatible（用于对接 LiteLLM / 其他兼容服务）
cargo run --example openai_compatible
cargo run --example openai_compatible_embeddings
```

多模态示例（需要本地文件输入）：

```bash
cargo run --example multimodal --features base64 -- ./image.png ./doc.pdf
```

下一步：

- 「SDK → 安装与最小用法」与「核心概念」。

## Gateway（部署/模板）

如果你想快速把 `ditto-gateway` 跑起来并具备“平台化控制面”：

- `deploy/docker-compose.yml`：本地可用的 docker-compose 模板（配套 `.env.example` / `gateway.example.json`）。
- `deploy/k8s/*`：Kubernetes 多副本模板。
- `deploy/helm/ditto-gateway`：Helm chart（包含 Grafana/Prometheus 辅助资源示例）。

对应 docs 页面：

- 「Gateway → Docker Compose（本地模板）」
- 「Gateway → Kubernetes（多副本模板）」
- 「Gateway → 部署：多副本与分布式」

## 多语言客户端示例（调用 Gateway 的 OpenAI-compatible `/v1/*`）

如果你只是想验证“对外兼容 OpenAI API”的调用方式：

- Node：`examples/clients/node/stream_chat_completions.mjs`（SSE streaming）
- Python：`examples/clients/python/chat_completions.py`
- Go：`examples/clients/go/chat_completions.go`

> 这些示例默认从环境变量读取 `DITTO_BASE_URL` 与（可选的）`DITTO_VK_TOKEN`，详见各目录的 `README.md`。

## Admin UI（可选）

仓库包含一个最小的 React 管理台：`apps/admin-ui`，用于：

- virtual keys 的增删改查
- 审计日志查看与导出（JSONL/CSV）

启动方式见：`apps/admin-ui/README.md`。

