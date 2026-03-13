# Superset Contract（兼容性口径）

本页把“成为 LiteLLM Proxy + Vercel AI SDK Core 的能力超集”这句话，落成一个**可执行、可验收**的契约：

- 哪些行为必须 **严格对齐**（否则就不是可替换/可迁移）
- 哪些行为是 **best-effort**（允许差异，但必须显式暴露）
- 哪些行为是 **非目标**（避免把项目拖成“全都要”的泥潭）

> 说明：Ditto 的实现策略是“分层 + feature gating”。默认构建保持小；需要 gateway/translation/metrics/otel 等能力时再显式开启 features。

---

## 0) 分层与仓库边界（长期不变）

Ditto 的产品分层：

- **L0（本仓库）**：模型互转与直接调用（SDK/adapter/protocol）
- **L1（本仓库）**：Gateway/Proxy 平台能力（API + routing + control-plane）
- **L2（独立仓库）**：企业闭环平台（评测/治理/审批/组织体系）

契约：

- L2 通过 L1 的稳定接口接入（而不是直连 L0 内部实现细节）。
- L1 必须可独立部署和运营，不依赖 L2 才能工作。
- L0/L1 的 feature gating 口径保持稳定，避免“默认全家桶”。

---

## 1) 四种形态（L0/L1 范围内长期不变）

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

下面这些不是“建议做一下”的检查，而是新的结构演进门槛。涉及核心结构、feature、provider、catalog、gateway 主路径的改动，默认都应把这些 gate 维持为绿色：

```bash
cargo fmt --all -- --check
cargo run -p ditto-core --bin ditto-llms-txt -- --check
cargo check
cargo test --all-targets                # default core: provider-openai-compatible + cap-llm
cargo check --examples                  # default examples must stay generic openai-compatible
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features

cargo check --no-default-features
cargo clippy --no-default-features -- -D warnings
```

feature matrix（至少覆盖 CI 当前维护的 provider-only 组合）：

```bash
cargo clippy -p ditto-core --no-default-features --features openai --all-targets -- -D warnings
cargo clippy -p ditto-core --no-default-features --features openai-compatible --all-targets -- -D warnings
cargo clippy -p ditto-core --no-default-features --features anthropic --all-targets -- -D warnings
cargo clippy -p ditto-core --no-default-features --features google --all-targets -- -D warnings
cargo clippy -p ditto-core --no-default-features --features cohere --all-targets -- -D warnings
cargo clippy -p ditto-core --no-default-features --features bedrock --all-targets -- -D warnings
cargo clippy -p ditto-core --no-default-features --features vertex --all-targets -- -D warnings
```

Node/前端：默认 gate 只覆盖 `packages/*`；`apps/admin-ui` 作为可选资产单独验证。

```bash
pnpm run typecheck
pnpm run build
# optional:
pnpm run typecheck:admin-ui
pnpm run build:admin-ui
```

---

## 6) Gateway Contract v0.1（冻结产物）

为了给 rust-ui 复用并避免手写模型漂移，L1 网关契约在仓库内提供两份冻结产物：

- OpenAPI：`contracts/gateway-contract-v0.1.openapi.yaml`
- Rust 类型包：`crates/ditto-gateway-contract-types`

该类型包内置：

- `GATEWAY_CONTRACT_VERSION = \"0.1.0\"`
- `GATEWAY_OPENAPI_V0_1_YAML`（直接嵌入上面的 OpenAPI 文件）

并在测试里校验：

- 版本号一致
- 关键路径存在（`/v1/chat/completions`、`/admin/audit`、`/admin/budgets`、`/admin/costs`、`/admin/reservations/reap`）
- 查询参数默认值一致（`audit limit=100`、`ledger limit=1000`、`offset=0`）
- 网关真实响应可被 contract 类型反序列化（`tests/gateway_contract_v0_1.rs`）

此外，CI 有 contract guard 闸门（`.github/workflows/ci.yml` 的 `gateway-contract-guard` job）：

- OpenAPI diff：识别 breaking / feature / patch 级别改动
- semver gate：改动级别与 `info.version` 升级级别必须匹配
- 冻结产物一致性：`info.version`、`GATEWAY_CONTRACT_VERSION`、`package.version` 必须一致；contract id 也必须一致
