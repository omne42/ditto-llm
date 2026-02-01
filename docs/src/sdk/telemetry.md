# Telemetry（可插拔事件钩子）

Ditto 的 `Telemetry` 是一个非常小的抽象（feature `sdk`）：

- 你可以向 `Telemetry` 发事件（`TelemetryEvent`）
- 由你实现的 `TelemetrySink` 决定如何处理（写日志、发指标、写 tracing span、上报到你的平台）

实现位置：`src/sdk/telemetry.rs`。

> 现阶段 Telemetry 是“通用积木”，不会自动帮你埋点；它的价值在于让你用统一接口把 Ditto 集成到你现有的观测体系里。

---

## 1) 数据结构

- `TelemetryEvent { name: String, data: Option<Value> }`
- `TelemetrySink::emit(event)`
- `Telemetry::emit(event)`：对外统一入口

---

## 2) 最小示例：接入 tracing

```rust
use ditto_llm::sdk::telemetry::{Telemetry, TelemetryEvent, TelemetrySink};
use serde_json::Value;

struct TracingSink;

impl TelemetrySink for TracingSink {
    fn emit(&self, event: TelemetryEvent) {
        eprintln!("telemetry name={} data={:?}", event.name, event.data);
    }
}

fn main() {
    let telemetry = Telemetry::new(TracingSink);
    telemetry.emit(TelemetryEvent::with_data("llm.request", Value::Null));
}
```

---

## 3) 设计建议（好品味）

- 事件名用“点分层级”：`llm.request` / `llm.response` / `gateway.proxy.error` 等
- `data` 里只放结构化、可聚合字段（不要塞 prompt 全文与 token）
- 所有敏感字段（token、用户输入）默认脱敏或不记录
