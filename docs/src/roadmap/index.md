# Roadmap

Ditto-LLM 的路线是“先收敛，再扩展”，而不是“一次到位平台化”。近期目标不是追求能力面无限扩张，而是先把内核、服务层和企业边界拆清楚。

## 分层模型（L0 / L1 / L2）

- **L0：模型互转与直接调用层（本仓库）**
  - 统一请求/响应类型、provider adapters、warnings/错误边界
  - 近期优先支持 OpenAI chat/responses、Gemini native、CC native 的调用与互转
  - 以 OpenAI `chat.completions` 作为近期最重要的公共互操作表面
- **L1：Gateway/Proxy 平台层（本仓库）**
  - OpenAI-compatible API surface、路由、预算、缓存、观测、Admin API
  - 面向中小团队 / 中型团队可直接落地的 LiteLLM-like 服务层
  - 存储基线优先收敛为 SQLite / Postgres
- **L2：企业闭环平台层（独立仓库）**
  - Prompt 管理与评测闭环、Agent 评测、组织级治理与审批流
  - 更完整 RBAC/SSO/SCIM、配置中心、策略编排、合规闭环
  - 通过 L1 稳定契约接入，不反向耦合 L0/L1 内核

## 仓库边界（当前）

- 本仓库聚焦 **L0 + L1**，并保持 feature gating（默认构建小，按需开启能力）。
- **L2 不在本仓库内实现**；以独立 repo 迭代，依赖 L1 的 HTTP/Admin/事件契约。
- 这条边界用于保证：L1 可单独部署可运营；L2 可独立演进，不拖慢底层稳定性。

对接策略（rust-ui）：

- rust-ui 当前更偏“渲染契约层”（`stream mode` / `output status` / `data-* markers`）。
- Ditto 的主对接面是 **L0 stream protocol v1 + L1 网关 API**，而不是绑定某一前端 hook 协议。

## 本章内容

- [Rust Native Provider Catalog](./rust-native-provider-catalog.md)：catalog truth、runtime binding 与 typed route resolver
- [Provider Runtime Rollout](./provider-runtime-rollout.md)：基于 completeness dashboard 的剩余 runtime 落地顺序
- [架构总览：内核、服务、分层与企业仓库边界](./kernel-service-enterprise-boundaries.md)：近期支持面、分层方法、L0/L1/L2 边界、双仓结构
- [Architecture Decision](./architecture-decision.md)：默认核心、provider packs、capability 边界与迁移不变量
- [Superset Contract（兼容性口径）](./contract.md)：L0/L1 的能力边界与验证口径
- [Gap Analysis（对标 LiteLLM + AI SDK）](./gaps.md)：差距与优先级
- [Superset Roadmap（可执行切片）](./superset.md)：可直接实现的任务切片
- [企业与合规能力清单](./enterprise.md)：L2 目标能力与现实落地路径
