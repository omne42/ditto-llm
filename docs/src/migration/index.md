# 迁移

这部分面向两类读者：

- 已经在用 LiteLLM Proxy：希望迁移到 Ditto Gateway（或两者并行）。
- 已经在用 Vercel AI SDK：希望把“AI SDK Core 的调用方式”映射到 Rust/服务端。

迁移文档的目标不是“营销对比”，而是帮助你：

- 找到 Ditto 里对应的能力与配置
- 明确哪些能力已经对齐、哪些仍是 roadmap
- 识别行为差异（例如：Warnings、更严格的参数校验、stream 语义差异）
