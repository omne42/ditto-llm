# Source Layout

## Core Runtime

- `crates/ditto-core/src/config/`
  - `ProviderConfig`、routing policy 和相关配置装配。
  - 严格格式识别与 typed load 复用 `config-kit`，不在本仓重复造通用加载轮子。
- `crates/ditto-core/src/contracts/`
  - provider / model / endpoint / guard / tool call 等核心契约。
- `crates/ditto-core/src/providers/`
  - 各 provider 适配器与协议归一化。
- `crates/ditto-core/src/provider_transport/`
  - HTTP transport、header / query 注入、timeout policy 等 provider 传输层细节。
- `crates/ditto-core/src/runtime/` 与 `runtime_registry/`
  - route resolution、runtime assembly、explain/debug 相关逻辑。
- `crates/ditto-core/src/capabilities/`、`types/`、`object/`、`sdk/`、`session_transport/`、`agent/`
  - 面向使用者的 SDK 能力层。
- `crates/ditto-core/src/catalog/`
  - 内建与生成 catalog 的读取和桥接逻辑。

## Gateway And Service

- `crates/ditto-gateway-contract-types/src/`
  - 冻结的 gateway Rust 契约类型包；与 `contracts/gateway-contract-v0.1.openapi.yaml` 一起构成对外 L1 artifact。
- `crates/ditto-server/src/gateway/`
  - HTTP gateway 的 domain / application / transport 与控制面模块。
- `crates/ditto-server/src/bin/`
  - `ditto-gateway`、审计导出/校验、store bench 等入口。
- `crates/ditto-server/src/config_editing.rs`
  - gateway config 编辑辅助逻辑。

## Checked-In Artifacts And Packages

- `catalog/`
  - provider 与 provider model 的检查入库产物。
- `contracts/`
  - 对外契约产物。
- `packages/`
  - `ditto-client` 与 `ditto-react`。
- `apps/admin-ui/`
  - 可选 admin UI 资产，不属于稳定核心契约。
- `deploy/`
  - Docker Compose、Kubernetes、Helm 与观测模板。

## Documentation

- `docs/src/`
  - `mdBook` 源文档，是事实来源。
- `docs/book/`
  - 生成站点，不是事实来源。
- `docs/tmp/`
  - 临时工作记录，不是事实来源。

## Layout Constraint

- provider transport quirks 应留在 `provider_transport/`、`providers/` 与 `runtime/` 邻近位置，不要塞进共享基础库。
- checked-in contracts 和 catalog 产物必须与 Rust 实现源码分开维护。
- `docs/book/` 只能由 `docs/src/` 生成，不应手工改动。
