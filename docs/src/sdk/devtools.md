# Devtools（JSONL 日志）

Ditto 的 `DevtoolsLogger` 是一个非常轻量的 JSONL 记录器（feature `sdk`），用于：

- 在开发/集成测试阶段记录请求/响应/事件，方便重放与离线分析
- 在 Gateway 里记录管理面与部分关键事件（通过 `--devtools` 启用）

实现位置：`src/sdk/devtools.rs`，Gateway 集成点：`src/bin/ditto-gateway.rs`（`--devtools`）。

---

## 1) 记录格式

每行一个 JSON：

```json
{ "ts_ms": 1738368000000, "kind": "proxy.request", "payload": { "...": "..." } }
```

字段：

- `ts_ms`：毫秒时间戳
- `kind`：事件名（你自定义或由网关写入）
- `payload`：任意 JSON（建议结构化，避免塞大文本/敏感信息）

---

## 2) 在你自己的服务里使用

```rust
use ditto_llm::sdk::devtools::DevtoolsLogger;
use serde_json::json;

fn main() -> ditto_llm::Result<()> {
    let logger = DevtoolsLogger::new("./logs/devtools.jsonl");
    logger.log_event("app.start", json!({ "ok": true }))?;
    Ok(())
}
```

---

## 3) 在 Gateway 里启用（推荐用于调试）

前置：

- 编译启用 `gateway-devtools`（它包含 `gateway` + `sdk`）

启动：

```bash
cargo run --features "gateway-devtools" --bin ditto-gateway -- ./gateway.json \
  --devtools ./logs/ditto-gateway.jsonl
```

注意事项：

- devtools 日志可能包含敏感信息（尤其是管理面 payload）；生产环境务必做好权限与脱敏。
- JSONL 文件会持续 append；建议配合日志轮转或定期清理。
