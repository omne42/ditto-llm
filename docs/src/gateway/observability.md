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

### 指标契约（Prometheus）

> 口径：以下是 Ditto Gateway **目前实现**的 Prometheus 指标族（`ditto_gateway_proxy_*`）。指标名与 label 我们会尽量保持稳定；如果需要破坏性调整，会在 `CHANGELOG.md` 里明确标注。

#### Labels 与基数控制

- `path`：不是原始 URL，而是**归一化后的 OpenAI 路径**（实现：`src/gateway/metrics_prometheus.rs` 的 `normalize_proxy_path_label`）。
  - 例如：`/v1/models/<id>` 会归一化为 `/v1/models/*`，未知路径会归一化为 `/v1/*`。
- `virtual_key_id`：未启用 virtual key 或未命中时为 `public`。
- `model`：仅在请求体里可解析到 `model` 时才打点。
- `backend` / `source` / `target` / `scope`：均做了基数上限控制。
- 当某个 label 维度超过 `--prometheus-max-*-series` 上限时，会把后续新值聚合到 `__overflow__`（避免把 Prometheus 打爆）。

#### Histogram buckets（duration）

`*_duration_seconds` 直方图使用固定 buckets（秒）：`0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1, 2.5, 5, 10`。

#### Metrics（概览）

| Metric | Type | Labels | Meaning |
| --- | --- | --- | --- |
| `ditto_gateway_proxy_requests_total` | counter | - | 代理请求总数 |
| `ditto_gateway_proxy_requests_by_key_total` | counter | `virtual_key_id` | 按 virtual key id 分组的请求计数 |
| `ditto_gateway_proxy_requests_by_model_total` | counter | `model` | 按 model 分组的请求计数 |
| `ditto_gateway_proxy_requests_by_path_total` | counter | `path` | 按归一化 OpenAI 路径分组的请求计数 |
| `ditto_gateway_proxy_request_duration_seconds` | histogram | `path` | 端到端代理请求耗时（按 path） |
| `ditto_gateway_proxy_responses_total` | counter | `status` | 按 HTTP status 分组的响应计数 |
| `ditto_gateway_proxy_responses_by_path_status_total` | counter | `path,status` | 按 path+status 分组的响应计数 |
| `ditto_gateway_proxy_backend_attempts_total` | counter | `backend` | 后端尝试次数（含 fallback） |
| `ditto_gateway_proxy_backend_success_total` | counter | `backend` | 后端成功次数 |
| `ditto_gateway_proxy_backend_failures_total` | counter | `backend` | 后端失败次数（网络错误/可重试 status 等） |
| `ditto_gateway_proxy_backend_in_flight` | gauge | `backend` | 后端 in-flight 请求数（用于背压观测） |
| `ditto_gateway_proxy_backend_request_duration_seconds` | histogram | `backend` | 后端请求耗时（按 backend） |
| `ditto_gateway_proxy_cache_lookups_total` | counter | - | proxy cache 查找次数 |
| `ditto_gateway_proxy_cache_lookups_by_path_total` | counter | `path` | 按 path 分组的 proxy cache 查找次数 |
| `ditto_gateway_proxy_cache_hits_total` | counter | - | proxy cache 命中次数 |
| `ditto_gateway_proxy_cache_hits_by_source_total` | counter | `source` | 按来源分组的命中次数（例如 memory/redis） |
| `ditto_gateway_proxy_cache_hits_by_path_total` | counter | `path` | 按 path 分组的命中次数 |
| `ditto_gateway_proxy_cache_misses_total` | counter | - | proxy cache 未命中次数 |
| `ditto_gateway_proxy_cache_misses_by_path_total` | counter | `path` | 按 path 分组的未命中次数 |
| `ditto_gateway_proxy_cache_stores_total` | counter | `target` | cache 写入次数（例如 memory/redis） |
| `ditto_gateway_proxy_cache_store_errors_total` | counter | `target` | cache 写入错误次数 |
| `ditto_gateway_proxy_cache_purges_total` | counter | `scope` | admin purge 次数（按 scope） |

### Dashboard / 告警模板

仓库内提供一套“开箱即用但不强绑定”的模板资产：

- Grafana dashboard：`deploy/grafana/ditto-gateway.dashboard.json`
- PrometheusRule（Prometheus Operator）：`deploy/prometheus/ditto-gateway-prometheusrule.yaml`

它们默认基于 Ditto 的 `ditto_gateway_proxy_*` 指标族（见 `GET /metrics/prometheus` 输出），你可以按自己平台的 label/job 约定做小幅调整。

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
