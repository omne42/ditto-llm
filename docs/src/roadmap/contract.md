# Superset Contract（兼容性口径）

本页把“成为 LiteLLM Proxy + Vercel AI SDK Core 的能力超集”这句话，落成一个**可执行、可验收**的契约：

- 哪些行为必须 **严格对齐**（否则就不是可替换/可迁移）
- 哪些行为是 **best-effort**（允许差异，但必须显式暴露）
- 哪些行为是 **非目标**（避免把项目拖成“全都要”的泥潭）

> 说明：Ditto 的实现策略是“分层 + feature gating”。默认构建保持小；需要 gateway/translation/metrics/otel 等能力时再显式开启 features。

---

## 1) 四种形态（长期不变）

Ditto 必须能同时以 4 种形态工作（见 `README.md` / `TODO.md`）：

1. **SDK**：Rust 里直接调用 providers（统一类型/Warnings/严格错误边界）
2. **Gateway**：提供 OpenAI-compatible HTTP surface + control-plane（virtual keys/limits/budget/cache/routing）
3. **Passthrough Proxy**：payload 不变形直通（对接 OpenAI-compatible upstream）
4. **Translation Proxy**：OpenAI in/out → native providers（并把差异显式化）

---

## 2) Must-have（严格对齐）

### 2.1 OpenAI Responses 的不变形路径

目标：对接 Codex CLI 等“依赖 items round-trip”场景时，不允许 silent downgrade。

契约：

- raw passthrough 时，**请求/响应 body 不做语义层面的改写**（除了必要的 hop-by-hop header 清理）。
- `/responses` 与 `/responses/compact` 需要支持 items round-trip（含 streaming）。
- 如果发生降级/兼容 shim，必须在响应头中显式标注（例如 `x-ditto-shim: ...`），并保持 OpenAI-style error shape。

### 2.2 Gateway 的 OpenAI-compatible 行为

契约（核心）：

- OpenAI-compatible 端点在行为上尽量对齐 OpenAI（含 SSE streaming）。
- 错误必须以 OpenAI-style error shape 返回（不要吞错/静默成功）。
- 差异必须可观测：响应头 `x-ditto-request-id` 贯穿；必要时添加 `x-ditto-*` headers 解释行为。

---

## 3) Best-effort（允许差异，但必须显式）

### 3.1 Provider 差异暴露

Ditto 的策略不是“把差异藏起来”，而是：

- SDK 层用 `Warning` 暴露兼容性缺口（例如 tools/schema/多模态不支持）
- Gateway 层用 OpenAI-style error + `x-ditto-*` headers 暴露降级/回退

### 3.2 结构化输出 / JSON Schema 转换

契约：

- 转换是 best-effort 且可能有损；不支持的关键字应被丢弃并以 `Warning` 暴露（避免静默数据损失）。

### 3.3 Token 计数与成本估算

契约：

- token 计数/成本估算允许 best-effort（失败可回退估算），但必须可观测、可配置、可限制（避免 OOM/超大缓冲）。

---

## 4) Non-goals（当前明确不做）

- 不 1:1 复刻 AI SDK UI/前端 hooks/RSC 生态（Ditto 只提供最小 JS/React client 以降低接入成本）。
- 不承诺一次性覆盖所有 LiteLLM 企业能力（RBAC/SSO/SCIM/审批流等按切片推进；外层 IAM/WAF/API gateway 可先承接）。

---

## 5) 验证（Stop Gate）

本仓库建议的最小验证集：

```bash
cargo fmt -- --check
cargo run --bin ditto-llms-txt -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features

cargo check --no-default-features
cargo clippy --no-default-features -- -D warnings
```

Node/前端（如改动涉及 `packages/*` 或 `apps/*`）：

```bash
pnpm -r run typecheck
pnpm -r run build
```

