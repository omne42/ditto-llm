# 观测：logs / Prometheus / OTel

Ditto Gateway 的观测分三层：

1) **响应头 + request id**：每个请求都能被定位与串联。  
2) **内置 metrics**：`GET /metrics`（JSON counters）与可选 Prometheus。  
3) **Tracing（可选）**：OpenTelemetry OTLP exporter（feature `gateway-otel`）。

实现位置：

- request id / 响应头：`src/gateway/http/proxy/core.rs`
- JSON metrics：`src/gateway/observability.rs` + `GET /metrics`
- Prometheus：`src/gateway/metrics_prometheus.rs` + `GET /metrics/prometheus`
- OTel：`src/gateway/otel.rs`

---

## 1) 请求链路：x-request-id 与 x-ditto-request-id

Ditto 会尽量保证每个响应都有可追踪的 request id：

- 若客户端提供 `x-request-id`，Ditto 会复用它
- 否则 Ditto 会生成一个 `ditto-<ts_ms>-<seq>` 风格的 id

响应里你会看到：

- `x-ditto-request-id: <id>`
- `x-request-id: <id>`（为了兼容下游链路）

并且 proxy 会附带：

- `x-ditto-backend: <backend_name>`

> 在排障时，请优先用 `x-ditto-request-id` 做全链路 grep。

---

## 2) JSON metrics：`GET /metrics`

返回 `ObservabilitySnapshot`（简单计数器）：

```json
{
  "requests": 123,
  "cache_hits": 10,
  "rate_limited": 2,
  "guardrail_blocked": 1,
  "budget_exceeded": 0,
  "backend_calls": 120
}
```

适用：

- 本地调试
- 简单 health/观测面板

限制：

- 指标粒度较粗
- 进程内计数（重启清零）

---

## 3) Prometheus metrics（可选）

前置：

- 编译启用 feature `gateway-metrics-prometheus`
- 运行时传 `--prometheus-metrics`

访问：

- `GET /metrics/prometheus`

### 控制指标基数（很重要）

Prometheus 的最大坑是“label 基数爆炸”。Ditto 提供一组上限参数：

- `--prometheus-max-key-series N`
- `--prometheus-max-model-series N`
- `--prometheus-max-backend-series N`
- `--prometheus-max-path-series N`

建议：

- 先用较小的上限跑在生产影子环境，观察 cardinality 再逐步放宽。

---

## 4) 结构化 JSON 日志（轻量）

运行时传 `--json-logs` 后，Ditto 会把关键事件以 JSON 行写到 stderr（见 `emit_json_log`）。

事件示例（概念）：

- `proxy.request` / `proxy.response` / `proxy.error`
- `proxy.blocked`（预算/存储错误导致的拦截）
- `gateway.request` / `gateway.response` / `gateway.error`（/v1/gateway demo）

适用：

- 直接接入日志系统（ELK / Loki / CloudWatch）
- 用 request id 做关联分析

---

## 5) OpenTelemetry（Tracing，可选）

前置：

- 编译启用 feature `gateway-otel`
- 运行时传 `--otel`（可选 `--otel-endpoint` 指定 OTLP HTTP endpoint）

示例：

```bash
RUST_LOG=info \
cargo run --features "gateway gateway-otel" --bin ditto-gateway -- ./gateway.json \
  --otel --otel-endpoint http://127.0.0.1:4318/v1/traces
```

说明：

- OTel 使用 `tracing_subscriber::EnvFilter`，可以通过 `RUST_LOG` 控制级别。
- `--otel-json` 会把 tracing logs 也输出为 JSON（便于收集）。

---

## 6) Devtools JSONL（可选）

如果你需要把请求/响应记录为 JSONL 以便重放或离线分析：

- 编译启用 `gateway-devtools`（它隐含 `gateway` + `sdk`）
- 运行时传 `--devtools <path>`

> Devtools 日志包含敏感信息的风险更高；生产环境务必配合脱敏/权限控制。

更多格式与用法见「SDK → Devtools（JSONL 日志）」。
