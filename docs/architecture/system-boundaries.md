# 系统边界

## 目标

`ditto-llm` 负责把多 provider LLM 调用、协议兼容和可部署网关收敛成一个可复用产品层。它不是所有跨仓基础设施的承载仓。

## 本仓负责什么

- `ditto-core` 中的 provider adapters、capability types、warnings、stream semantics、runtime routing
- native provider 协议与 OpenAI-compatible surface 之间的 shape conversion / translation
- `ditto-server` gateway 的 auth、routing、budgets、cache、audit、admin、translation / passthrough HTTP surfaces
- `ditto-server::gateway::*` 根级 facade 与必要兼容 shim；`gateway` 内部 `domain / application / transport / adapters` 目录不作为对外稳定边界
- provider catalog、gateway contract、客户端包和部署模板

## 已复用的共享基建

- `omne_foundation/config-kit`
  - 用于严格配置文档加载，以及 JSON/TOML/YAML 感知的解析入口。
- `omne_foundation/http-auth-kit`
  - 用于 header/query auth、OAuth client-credentials 和 AWS SigV4 这类通用 HTTP 鉴权协议构件。
- `omne_foundation/redaction-kit`
  - 用于 JSON / query / regex / Prometheus label 脱敏和稳定采样原语。
- `omne_foundation/speech-transcription-kit`
  - 用于 provider-neutral 转写请求 / 响应 DTO 和 provider provenance 语义；Ditto 只依赖默认轻量 DTO feature，不依赖 foundation provider catalog 或转写 job feature。
- `omne_foundation/policy-meta`
  - 用于可复用的 write-scope 等策略元语义。
- `omne-runtime/omne-integrity-primitives`
  - 用于 SHA-256、审计链等完整性原语。
- 其他足够通用的 foundation kits
  - 例如 i18n、secret、text assets 等非产品专属语义。

## 继续留在本仓的能力

- provider-specific HTTP / streaming quirks 与协议归一化
- provider catalog 形状和 runtime resolution 逻辑
- provider auth schema、环境变量 key 选择、secret 解析和 provider 默认鉴权策略
- OpenAI / OpenAI-compatible audio multipart transport、转写 provider routing 与 model builder
- gateway translation 语义、控制面数据模型和产品级 feature slicing
- gateway OpenAPI/types、tenant/project/user scope、budgets、rate-limits、cache、idempotency 和 guardrails
- L0 / L1 边界与 Ditto 自己的产品路线

## 不负责什么

- 通用 config 领域的完整抽象
- 任意产品都能直接复用的通用 gateway runtime
- L2 企业闭环平台
- agent orchestration / governance runtime

## 复用原则

- 只有当一个能力跨仓复用且不携带 provider/gateway 假设时，才应上提。
- 应用侧通用 kit 进入 `omne_foundation`；更底层的完整性或运行时原语进入 `omne-runtime`。
- provider quirks、translation 细节和 Ditto 自己的控制面语义继续留在本仓。
