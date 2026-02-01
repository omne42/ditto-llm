# 测试与集成

Ditto 的测试分三类：

1) **纯逻辑/纯解析单测**：不依赖网络  
2) **HTTP mock 测试**：通过本地 `httpmock` 起一个 server（需要能 bind localhost）  
3) **集成 smoke tests（可选）**：需要真实 API key（feature `integration`）  

## 运行单测

默认：

```bash
cargo test
```

带某些 feature 才会编译的测试（例如 gateway）：

```bash
cargo test --features gateway
```

一次性覆盖更多能力（按需启用，避免无谓编译）：

```bash
cargo test --features all
```

## httpmock 测试说明

仓库里有大量基于 `httpmock` 的测试，用于验证：

- 请求/响应协议是否符合预期
- warnings 生成逻辑是否正确
- gateway 路由/缓存/预算等行为

如果运行环境禁止 bind `127.0.0.1`，这些测试会自动跳过（见 `src/utils/test_support.rs`）。

## integration smoke tests（真实调用，可选）

仓库包含 `tests/integration_smoke.rs`（`#![cfg(feature = "integration")]`）。

启用方式：

```bash
cargo test --features integration
```

这些测试会在缺少环境变量时自动跳过，不会 hard-fail。你需要按 provider 设置对应 key，例如：

- `OPENAI_API_KEY` + `OPENAI_MODEL`
- 以及可选的 embeddings keys（取决于你启用的 provider/features）

## 示例（examples/）

examples 是最直接的“可运行文档”：

```bash
cargo run --example basic
cargo run --example streaming
cargo run --example tool_calling
```

某些 examples 需要额外 features（例如 batches、audio、images），请按报错提示开启。
