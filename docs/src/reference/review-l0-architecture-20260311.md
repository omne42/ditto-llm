<!-- llms-txt:exclude -->

# 代码审查：L0 架构边界（2026-03-11）

2026-03-13 注：仓库随后已拆成 workspace，当前代码分别位于 `crates/ditto-core` 与
`crates/ditto-server`。本文保留 2026-03-11 当时的审查结论，但把可跳转文件链接更新到了
现有路径；没有直接现存对应物的 pre-split 文件（如 `src/lib.rs`、`src/compat/*`、
`src/types/llm.rs`）继续按历史名称记述。文中的 `ditto_llm::...` 也应按拆分前单 crate
路径理解；当前对应 namespace 为 `ditto_core::...` 或 `ditto_server::...`。

本次审查通过 `omne-agent` 发起，线程配置已显式落到 Yunwu：

- provider: `google.providers.yunwu`
- model: `gemini-3.1-pro-preview`
- base_url: `https://yunwu.ai/v1beta`
- thread: `c21d77bb-b4d9-4643-9c10-e06a7f719e9a`

审查目标：

- 对照 `docs/src/roadmap/ai-gateway-platform-deep-governance.dsl`
- 只看 L0 边界与其直接下层：`catalog`、`runtime_registry`、`runtime`、`provider_transport`、`provider_options`、`session_transport`、`providers`、`config`、`contracts`、`foundation`、`llm_core`
- 判断实现是否准确、抽象是否足够、是否无状态、性能开销是否可预测、结构是否足够干净

## Review 摘要

审查模型明确认可了这些部分应保持不变：

- `crates/ditto-core/src/catalog/` 的静态能力目录边界是准确的
- `crates/ditto-core/src/runtime/` 与 `runtime_registry` 作为动态装配层是成立的
- `crates/ditto-core/src/config/`、`crates/ditto-core/src/providers/`、`crates/ditto-core/src/foundation/`、`crates/ditto-core/src/llm_core/` 的职责分离总体正确

审查模型给出的主要问题有三条：

1. `provider_transport` 与 `session_transport` 没有作为显式根边界存在
2. `provider_options` 被埋在 `crates/ditto-core/src/types/` 里，边界不够显性
3. `contracts` 与 `types` 的职责分割仍不够贴合 DSL

## 采纳并落地的修复

本轮采纳了前两类问题，并按“最小结构修复”处理：

### 1. 把 provider transport 从泛 `config/utils` 提升为根边界

新增：

- `crates/ditto-core/src/provider_transport/mod.rs`
- `crates/ditto-core/src/provider_transport/config.rs`
- `crates/ditto-core/src/provider_transport/http.rs`

调整：

- 原 pre-split `src/config/http.rs` 已迁入 `crates/ditto-core/src/provider_transport/config.rs`
- 原 `src/utils/http.rs` 已迁入 `crates/ditto-core/src/provider_transport/http.rs`
- provider 适配器与相关调用点改为直接依赖 `crate::provider_transport::*`

这次修复后，`provider_transport` 明确负责：

- HTTP client 构造
- provider base URL / query 参数装配
- checked request 发送
- bounded response body 读取

### 2. 把 session transport 从泛 `utils` 提升为根边界

新增：

- `crates/ditto-core/src/session_transport/mod.rs`
- `crates/ditto-core/src/session_transport/sse.rs`
- `crates/ditto-core/src/session_transport/streaming.rs`

调整：

- 原 `src/utils/sse.rs` 已迁入 `crates/ditto-core/src/session_transport/sse.rs`
- 原 `src/utils/streaming.rs` 已迁入 `crates/ditto-core/src/session_transport/streaming.rs`
- websocket base URL 重写也收口到 `session_transport`
- provider streaming、gateway SSE bridge 与 realtime URL 组装改为直接依赖 `crate::session_transport::*`

这次修复后，`session_transport` 明确负责：

- SSE 数据帧解析
- 流式缓冲初始化
- websocket base URL 协商与 transport 级语义映射

### 3. 把 provider options 从 `types` 提升为根边界

新增：

- `crates/ditto-core/src/provider_options/mod.rs`
- `crates/ditto-core/src/provider_options/envelope.rs`
- `crates/ditto-core/src/provider_options/support.rs`

调整：

- 原 `src/types/provider_options_envelope.rs` 已迁入 `crates/ditto-core/src/provider_options/envelope.rs`
- 原 `src/types/provider_options_support.rs` 已迁入 `crates/ditto-core/src/provider_options/support.rs`
- `ProviderOptions`、`ReasoningEffort`、`ReasoningSummary`、`ResponseFormat`、`JsonSchemaFormat` 已从 `src/types/llm.rs` 提升到 `crates/ditto-core/src/provider_options/mod.rs`
- provider 侧对 provider options 的解析、merge、warning 逻辑改为直接依赖 `crate::provider_options::*`

这样做的结果是：

- `provider_options` 成为 DSL 对应的显式 L0 组件
- `types` 继续保留请求/响应载荷，但不再拥有 provider passthrough 机制本身

### 4. 后续推进：把 `runtime_registry` 从 `runtime` 内部提升为并列根边界

后续又继续做了一步结构收敛：

- 新增 `crates/ditto-core/src/runtime_registry/mod.rs`
- 原 pre-split `src/runtime/registry.rs` 已迁入 `crates/ditto-core/src/runtime_registry/snapshot.rs`
- 原 pre-split `src/runtime/provider_catalog.rs` 已迁入 `crates/ditto-core/src/runtime_registry/catalog.rs`
- `runtime` 现在只保留 route resolve / explain / transport planning / model building
- `runtime_registry` 单独拥有 registry snapshot 与 provider config semantics

这一步的目的不是换路径，而是收紧所有权：

- `runtime` 负责“动态解析与派发”
- `runtime_registry` 负责“基于 Catalog 衍生的动态装配状态”

这样代码结构和 DSL 中 `runtime`、`runtime_registry` 的并列关系就一致了

## 暂未在本轮处理的点

`contracts` 与 `types` 的进一步收敛这条意见本身是成立的，但它已经超出本轮“最小 L0 结构修复”的范围。

原因：

- 这不只是移动几个文件，而是会牵动几乎整个请求/响应载荷层
- 当前 `types` 中还承载大量 SDK / gateway / capabilities 的上层输入输出结构
- 如果这轮强拆，风险会明显高于 transport / options 这类边界提升

因此本轮只先把与 L0 DSL 直接对应的 `provider_transport`、`session_transport`、`provider_options` 三块收正；`contracts`/`types` 的进一步收敛，应该单独开一轮结构迁移来做

## 本轮结论

本轮审查后的判断是：

- `catalog`、`runtime_registry`、`runtime` 方向是对的，不需要返工
- L0 之前最不准确的地方，确实是 transport 与 provider options 边界没有显式站出来
- 现在这三块已经成为真实根模块，而不是继续留在 `utils` / `config` / `types` 的泛命名空间里
- 这次修复没有引入新的 facade 层，也没有为了兼容再加一层包装噪音

## 追加复核：Omne / Yunwu Gemini 3.1（2026-03-11）

本日又追加发起了一轮 `omne-agent` 复核，使用：

- provider: `google.providers.yunwu`
- model: `gemini-3.1-pro-preview`
- base_url: `https://yunwu.ai/v1beta`
- thread: `8239fa3f-c85d-4619-9332-eb5cfdaf6ee5`
- instructions: `/root/autodl-tmp/zjj/p/prompts/linus-rust.md`

这轮复核聚焦的是当前 L0 的入口层与所有权边界，而不是再看 transport 提升本身。

### 新增 review 结论

模型给出的主要意见有三条：

1. `src/core.rs` 是纯 facade 噪音，破坏了 DSL 中 `contracts` / `foundation` / `llm_core` 的并列关系
2. `ProviderOptions::from_value` 在 owned `Value` 上仍然无条件深拷贝，存在可避免的分配
3. `runtime/explain` 里的 `resolved_model: String` 可以进一步收紧成静态借用

### 采纳的修复

本轮采纳并落地了前两条。

#### 1. 移除 `core.rs` facade，恢复底层模块的直接所有权边界

调整：

- 删除 `src/core.rs`
- `src/lib.rs` 不再声明 `pub mod core;`
- `src/lib.rs` 改为直接公开 `contracts`、`foundation`、`llm_core`
- 根导出仍直接从 `foundation::error` 与 `llm_core` 提供，不再经过 `core` 中转
- 内部导入从 `crate::core::*` 收口到 `crate::contracts::*`、`crate::foundation::*`、`crate::llm_core::*`
- `tests/module_namespace_contract.rs` 改为验证直接模块路径，而不是 `ditto_llm::core::*`

这样处理后：

- DSL 的底层三块重新回到平面并列关系
- 内外部路径不再出现 `core -> foundation/llm_core/contracts` 这一层额外转发
- `core` 带来的第三套低层命名空间被移除

#### 2. 收紧 `ProviderOptions` 的解析所有权，消除 owned 路径上的多余 clone

调整：

- `ProviderOptions::from_value` 现在直接消费 `serde_json::Value`
- 新增 `ProviderOptions::from_value_ref`，只在必须保留借用时才 clone
- `ProviderOptionsEnvelope` 和 `types/llm.rs` 中拥有 `Value` 所有权的路径已切到 `from_value`
- provider 侧仍是借用 `&Value` 的解析点，统一改用 `from_value_ref`

这样处理后：

- owned `provider_options` 解析路径不再固定做一次深拷贝
- clone 只留在确实只有借用的调用点
- L0 这块的所有权流向更明确

#### 3. 把 `provider_options` 从 `types` 命名空间里彻底摘掉

调整：

- `src/types/mod.rs` 不再 re-export `ProviderOptions`、`ProviderOptionsEnvelope`、`JsonSchemaFormat`、`ResponseFormat`、`ReasoningEffort`、`ReasoningSummary`
- `types` 模块注释明确限定为“协议 payload DTO”
- `tests/module_namespace_contract.rs` 改为直接验证 `ditto_llm::provider_options::ProviderOptionsEnvelope`

这样处理后：

- `provider_options` 不再被 `types` 反向吞回去
- L0 的 `provider_options` 边界和 `types` 载荷层之间职责更干净
- 文档里“types 不拥有 provider passthrough 机制”这句话终于和代码一致了

### 明确拒绝的建议

`resolved_model: String` 这条没有采纳。

原因不是“懒得做”，而是这条意见在当前实现里并不严格成立：

- `catalog/resolver.rs` 的 `resolve_runtime_model()` 明确允许来自 request/config 的动态 model 字符串
- `RuntimeRouteExplain` / `RuntimeTransportPlan` 承载的是“最终生效值”，不是“仅限 catalog 静态字典项”
- 如果强行改成 `&'static str`，就会错误压缩来自用户输入和 provider config 的动态所有权语义

因此这里继续保留 `String`，是正确建模，不是保守。

### 复核后的最终判断

追加复核后的结论是：

- 这轮真正成立的结构问题，就是 `core.rs` 这层 facade 以及 `ProviderOptions` 的 owned 解析 clone
- 这两点已经落地修掉
- `runtime/explain` 的 `String` 保留是有明确所有权理由的，不应为了“看起来更 static”而误建模
- 当前 L0 比前一轮更接近 DSL：边界显式、层级更平、无额外兼容门面

## 追加复核：Omne / Yunwu Gemini 3.1（runtime 与 catalog 边界，2026-03-11）

本日又追加发起了一轮 `omne-agent` 复核，继续使用：

- provider: `google.providers.yunwu`
- model: `gemini-3.1-pro-preview`
- base_url: `https://yunwu.ai/v1beta`
- thread: `d1794a9d-e007-40b5-9e0f-b1dfb63fd21c`

这轮只看：

- `crates/ditto-core/src/runtime/mod.rs`
- `crates/ditto-core/src/runtime/resolver.rs`
- `crates/ditto-core/src/runtime/route.rs`
- `crates/ditto-core/src/runtime/explain.rs`
- `crates/ditto-core/src/runtime/transport.rs`
- `crates/ditto-core/src/catalog/mod.rs`
- `src/lib.rs`
- `crates/ditto-core/src/runtime_registry/mod.rs`

审查目标是确认两件事：

1. `runtime` 是否已经真正接管动态 route / explain / transport 装配
2. `catalog` 是否已经停止在公开表面上承担 `contracts` 的第二所有者角色

### 新增 review 结论

模型给出的结论很直接：

- **阻塞问题：无**
- 当前 `runtime` / `catalog` 的职责分离已经和 DSL 基本一致
- 当前实现满足“无状态、成本可预测、Machine-first”的方向

模型给出的两条非阻塞意见是：

1. `src/lib.rs` 仍保留了较多 `#[doc(hidden)] pub use` 根导出，公开面有些臃肿

## 追加落地：contract 加固收尾（2026-03-11）

围绕本轮后续 review 又继续落了五块结构修复，目标不是“继续整理目录”，而是把剩余 owner 泄漏真正收口：

### 1. `runtime_registry` 拆成 frontdoor / queries / semantics

调整：

- `crates/ditto-core/src/runtime_registry/catalog.rs` 现在只保留 frontdoor 类型与共享数据结构
- 新增 `crates/ditto-core/src/runtime_registry/queries.rs`
- 新增 `crates/ditto-core/src/runtime_registry/semantics.rs`

结果：

- northbound 的 provider preset / capability summary / model candidate 查询，不再和 southbound 的 builder provider 解析、capability support、context cache profile、behavior follow-up 混在一个文件里
- `runtime_registry` 现在是“派生 registry 视图 + 语义 helper”的清晰 owner，而不是继续膨胀的单文件装配点

### 2. `runtime/route.rs` 拆成 frontdoor / selection / endpoint

调整：

- 新增 `crates/ditto-core/src/runtime/route_selection.rs`
- 新增 `crates/ditto-core/src/runtime/route_endpoint.rs`
- `crates/ditto-core/src/runtime/route.rs` 保留 explain plan、前门入口和 join 过程

结果：

- provider / model / capability 选择逻辑，和 base_url / query / transport 适配逻辑分离了
- `route.rs` 不再继续充当 God Resolver

### 3. `OpenAITextModel` 不再是半成品对象

调整：

- `crates/ditto-core/src/providers/openai/text.rs` 的 `from_config()` 现在默认注入 builtin catalog surface resolver
- 公开构造后的对象已经自足，不再依赖 runtime 后续补注入才可用

结果：

- `providers::OpenAITextModel` 的公开语义和 runtime builder 语义一致
- `runtime/builder_backends.rs` 不再需要为 OpenAI 文本模型注入闭包 resolver

### 4. OpenAI / OpenAI-compatible 兼容策略提升为显式 profile owner

调整：

- 删除 pre-split `src/providers/openai_compatible_family.rs`
- 新增 `crates/ditto-core/src/providers/openai_compat_profile.rs`
- `OpenAiCompatibleConfig` 新增 `family` override 字段
- `OpenAI` 与 `OpenAICompatible` 都改为消费 `OpenAiCompatibilityProfile`

结果：

- family 推断、显式 override、prompt cache reporting、thought-signature policy、catalog-backed model behavior 现在都集中到一个 machine-readable profile owner
- `runtime/builder_backends.rs` 不再为 `OpenAICompatible` 注入 request-model 行为闭包
- 兼容策略不再散落在 `openai/client.rs`、`openai_compatible/client.rs`、`provider_options_schema.rs` 三处各自猜测

### 5. `capabilities` 不再 re-export DTO，避免长成第二 owner

调整：

- `crates/ditto-core/src/capabilities/audio.rs`
- `crates/ditto-core/src/capabilities/batch.rs`
- `crates/ditto-core/src/capabilities/image.rs`
- `crates/ditto-core/src/capabilities/image_edit.rs`
- `crates/ditto-core/src/capabilities/moderation.rs`
- `crates/ditto-core/src/capabilities/rerank.rs`
- `crates/ditto-core/src/capabilities/video.rs`
- `crates/ditto-core/src/capabilities/mod.rs`

这些模块现在只保留 capability trait / frontdoor，不再 re-export `types` 或 `contracts` DTO。

结果：

- `capabilities` 回到 northbound facade 定位
- `types` / `contracts` 的 DTO 所有权不再被 capability 模块反向吞掉

### 本轮补充结论

经过这轮收尾，L0 现在更接近 review 里说的 “contract 加固阶段”：

- `runtime_registry` 不再是过满单点
- `runtime/route` 不再把所有动态解析塞进一个文件
- provider adapter 的兼容语义不再靠 runtime 闭包和 scattered heuristics 粘起来
- `capabilities` 便利门面没有继续长成第二个数据 owner

这轮之后，剩下更像“公开根导出继续瘦身”这类表面治理，而不是 L0 owner 还没站稳的问题。
2. `catalog` 中的部分静态描述结构（如 `ProviderPluginDescriptor`、`ModelBehaviorDescriptor`）未来仍有继续向更纯契约层下沉的空间

### 本轮采纳的修复

这轮在复核前已经完成、并被模型认可的结构修复是：

- 将 `catalog/resolver.rs` 物理迁入 `crates/ditto-core/src/runtime/route.rs`
- `runtime` 重新成为 route / explain / transport 的真实所有者
- `resolve_builtin_runtime_route()` 成为公开 runtime 入口，外部契约测试不再经由 `CatalogRegistry::resolve_runtime_route`
- `catalog/mod.rs` 不再公开 `contracts` 的镜像导出，只保留 crate 内部可见的内部别名面
- `src/lib.rs` 的根导出改为直接从 `contracts` 提供契约类型，而不是再从 `catalog` 中转

### 本轮结论

截至这一轮，L0 在关键边界上已经达到可接受状态：

- `catalog` 回到“静态能力字典”
- `runtime` 回到“动态装配与派发”
- `runtime_registry` 保持“衍生快照与语义解析”
- 当前剩余问题已经不是阻塞性的架构错误，而是后续可继续收口的 API 表面与契约下沉问题

## 追加复核：Omne / Yunwu Gemini 3.1（root owner 收口，2026-03-11）

本日继续通过 `omne-agent` 发起了两类复核：

- `exec` 线程 `29c76dd1-3d40-428e-9a2a-e84b350a1ba7` 与 `575915cb-5e98-4819-bf60-2843f7b0b854`
  都在 workspace read 后停住，没有产出最终 assistant 消息
- 为避免再次卡在工具读取，这一轮改为把当前 L0 边界摘要直接喂给模型复核
- 最终有效复核线程：`aceeba81-c9d9-4b0b-a33e-677f7ee0c5d6`

### 新增 review 结论

模型这轮给出的判断是：

- **阻塞问题：无**
- `catalog` / `runtime` / `runtime_registry` / `provider_options` 的 owner 已经单向明确
- 当前剩余问题主要集中在 `src/lib.rs` 根层 still-too-wide 的便利导出

模型给出的两条非阻塞意见是：

1. root 继续公开 provider 构造器，会让 `src/lib.rs` 持续膨胀
2. `contracts/config/types/capabilities` 仍然作为 root 便利入口存在，长期有继续膨胀成 God Facade 的风险

### 本轮采纳的修复

这轮采纳并落地了第一条意见，以及上一轮关于 root owner 收口的剩余部分：

- `src/lib.rs` 不再根层 re-export `catalog` / `runtime` / `runtime_registry` /
  `provider_options`
- `src/lib.rs` 继续收口，不再根层 re-export各 provider 构造器；真实 owner 统一为
  `ditto_llm::providers::*`
- 仓库内所有旧调用点已经改回 owner 路径：
  - `crate::catalog::builtin_registry()`
  - `crate::runtime::resolve_builtin_runtime_route(...)`
  - `crate::provider_options::*`
  - `crate::providers::*`
- `src/lib.rs` 新增了显式注释，说明 root 只保留前门 ergonomics，不再充当 L0 module 的第二 owner
- `src/bin/ditto-catalog-dashboard.rs`、相关 examples、相关 tests 一并改成真实 owner 模块路径
- provider_options 拆分后的残余测试导入也已补齐，避免 `types` 重新吞回 owner 语义

### 明确拒绝的建议

这一轮没有采纳“引入 `prelude` 再保留 root 一键导入”的做法。

原因很直接：

- 这会重新制造一层 facade
- 用户前面的架构要求已经明确偏向“不要兼容门面、不要包装噪音”
- 当前更合适的策略是直接使用真实 owner 路径，而不是再造一层新的便利聚合

### 本轮结论

截至这一轮，L0 的根表面已经进一步收平：

- `catalog` / `runtime` / `runtime_registry` / `provider_options` 不再有 root 第二 owner
- providers 也回到 `ditto_llm::providers::*` 的真实 owner
- root 现在主要保留错误类型、llm_core trait、以及少量使用侧便利入口

当前剩余的非阻塞问题，主要只剩下是否继续收缩 `contracts/config/types/capabilities`
在 root 的便利暴露；这已经是 API 风格选择问题，不再是 L0 结构边界错误。

## 追加复核：Omne / Yunwu Gemini 3.1（L0 owner 内部依赖与 Env 抽象，2026-03-11）

本日再次通过 `omne-agent` 发起复核，使用：

- provider: `google.providers.yunwu`
- model: `gemini-3.1-pro-preview`
- base_url: `https://yunwu.ai/v1beta`
- thread: `86400d62-7f1d-41a0-b59c-7a19fd211a98`

这轮只看：

- `docs/src/roadmap/ai-gateway-platform-deep-governance.dsl`
- `src/lib.rs`
- `crates/ditto-core/src/runtime/mod.rs`
- `crates/ditto-core/src/runtime/model_builders.rs`
- `crates/ditto-core/src/catalog/mod.rs`
- `crates/ditto-core/src/runtime_registry/mod.rs`
- `crates/ditto-core/src/provider_transport/mod.rs`
- `crates/ditto-core/src/provider_options/mod.rs`
- `crates/ditto-core/src/session_transport/mod.rs`
- `crates/ditto-server/src/gateway/application/translation/mod.rs`
- `crates/ditto-core/src/providers/openai_compatible/client.rs`

### 新增 review 结论

模型这轮给出的判断是：

1. `runtime` / `providers` / `gateway` 内部仍有部分实现回头依赖 root 便利导出，而不是 owner 模块
2. `openai_compatible/client.rs` 里直接读取进程环境变量，绕过了 `Env` 抽象
3. `translation/mod.rs` 仍然过大，但这是更大范围的物理拆分问题

### 本轮采纳并落地的修复

这轮采纳了前两类可局部、可验证的结构问题。

#### 1. 收掉 L0 owner 内部对 root front-door 的反向依赖

调整：

- `crates/ditto-core/src/runtime/resolver.rs` 不再从 root 导入 `OperationKind` / `RuntimeRoute*` / `Result`
- `crates/ditto-core/src/runtime/explain.rs`、`crates/ditto-core/src/runtime/transport.rs`、`crates/ditto-core/src/runtime/route.rs`、
  `crates/ditto-core/src/runtime/model_builders.rs` 改为直接依赖 `contracts` 与
  `foundation::error`
- `crates/ditto-core/src/provider_transport/config.rs`、`crates/ditto-core/src/provider_transport/http.rs`、
  `crates/ditto-core/src/session_transport/sse.rs`、`crates/ditto-core/src/session_transport/streaming.rs` 不再依赖
  root `DittoError` / `Result`
- `crates/ditto-core/src/provider_options/envelope.rs`、`crates/ditto-core/src/provider_options/support.rs` 改为
  `super::*`，不再反向引用 `crate::provider_options::*`
- `crates/ditto-core/src/providers/openai/chat_completions.rs` 改为直接依赖
  `crate::providers::openai_compatible::OpenAICompatible`

这样处理后：

- `runtime` / `provider_transport` / `session_transport` / `provider_options`
  这些 owner 内部不再回头吃 root 便利路径
- `providers` 子实现也不再经由 `providers/mod.rs` 的聚合 re-export 找同层 owner

#### 2. 把 `runtime/model_builders` 从 `providers` 聚合面切到具体 owner 模块

调整：

- `crates/ditto-core/src/runtime/model_builders.rs` 里的 provider 构造入口，已从
  `crate::providers::*` 聚合转发，收口到具体 owner 模块，例如：
  - `crate::providers::openai::*`
  - `crate::providers::openai_compatible::*`
  - `crate::providers::openai_compatible_audio::*`
  - `crate::providers::openai_compatible_batches::*`
  - `crate::providers::openai_compatible_images::*`
  - `crate::providers::openai_compatible_moderations::*`
  - `crate::providers::anthropic::*`
  - `crate::providers::google::*`
  - `crate::providers::cohere::*`
  - `crate::providers::bedrock::*`
  - `crate::providers::vertex::*`

这样处理后：

- `runtime` 看到的是 provider 真实 owner，而不是 `providers/mod.rs`
  这层聚合 facade
- `runtime -> providers` 的依赖方向更直接，也更符合 DSL 里 owner 单向关系

#### 3. 把 OpenAI-compatible 的环境开关收进 `Env`

调整：

- `crates/ditto-core/src/providers/openai_compatible/client.rs` 不再在请求构造逻辑里直接调用
  `std::env::var`
- `OMNE_OPENAI_COMPAT_SEND_PROMPT_CACHE_KEY` /
  `DITTO_OPENAI_COMPAT_SEND_PROMPT_CACHE_KEY` /
  `OMNE_OPENAI_COMPAT_SEND_TOOL_CALL_THOUGHT_SIGNATURE` /
  `DITTO_OPENAI_COMPAT_SEND_TOOL_CALL_THOUGHT_SIGNATURE`
  现在统一通过 `Env` 在 `from_config(...)` 阶段解析
- 这些开关在 provider 构造时一次性固化到 request quirks 中，后续请求发送阶段不再碰全局环境
- 同时新增显式 builder：
  - `with_prompt_cache_key_passthrough(...)`
  - `with_tool_call_thought_signature_passthrough(...)`

这样处理后：

- Data Plane 运行时不再在请求路径读进程环境
- `from_config` 路径继续支持显式环境开关，但经过 `Env` 抽象
- 非 `from_config` 调用者如果要开启兼容透传，也可以显式设置，而不是依赖隐藏全局状态

### 明确没有在本轮采纳的建议

`crates/ditto-server/src/gateway/application/translation/mod.rs` 的物理拆分这条意见这轮没有采纳。

原因：

- 这是一个跨千行文件的大手术，不是局部 owner 收敛
- 它会同时牵动协议映射、缓存、SSE 桥接和 gateway 行为回归
- 当前 L0 的主要阻塞点已经可以通过局部 owner 收敛解决，不需要在这轮引入高风险重构

### 本轮结论

截至这一轮，L0 的 owner 边界又进一步收紧：

- root front-door 与真实 owner 的反向依赖基本已从当前 L0 实现里清掉
- `runtime/model_builders` 不再依赖 `providers` 聚合 facade
- OpenAI-compatible 的兼容开关不再在请求路径直接读取全局环境

当前剩余的结构问题主要是 `translation/mod.rs` 过大，以及 `src/lib.rs`
根表面的长期收缩空间；它们已不属于这轮必须立即处理的 L0 owner 边界错误。

## 本轮推进：移除 `lib.rs` 对 `config/contracts/types/capabilities` 的第二 owner 角色（2026-03-11）

这轮没有再引入新 facade，而是把此前仍残留在 crate root 的便利导出继续收干净。

### 调整

- `src/lib.rs` 不再 re-export：
  - `capabilities::*`
  - `config::*`
  - `config_editing::*`
  - `contracts::*`
  - `types::*`
- root 不再保留公开的低层前门 re-export
- examples / tests / bins 已统一改回真实 owner 路径，例如：
  - `ditto_llm::types::*`
  - `ditto_llm::config::*`
  - `ditto_llm::config_editing::*`
  - `ditto_llm::contracts::*`
  - `ditto_llm::capabilities::*`

### 结果

- crate root 不再同时扮演 `config/contracts/types/capabilities` 的第二 owner
- L0 的显式 owner 模块和 DSL 对齐得更直接
- 仓库内调用面也不再依赖隐藏的 root convenience surface

### 校验

本轮完成后已通过：

- `cargo fmt --check`
- `cargo check --all-targets`
- `cargo test --test runtime_route_contract --test runtime_transport_contract --test runtime_explain_contract --test runtime_registry_contract --test catalog_resolver_contract --test gateway_translation_custom_provider_resolution --test gateway_translation_provider_aliases`

## 本轮推进：移除 `llm_core::error` 这层 second-owner facade（2026-03-11）

继续对照 DSL 的 L0 底层 owner 关系时，还剩一处很小但真实存在的重复入口：

- `foundation::error` 已经是错误类型的真实 owner
- 但 `crates/ditto-core/src/llm_core/mod.rs` 仍然公开：
  - `pub mod error { pub use crate::foundation::error::{...}; }`
  - 以及 `pub use error::{DittoError, ProviderResolutionError, Result};`

这会让 `llm_core` 在公开表面上再次扮演 `foundation::error` 的第二 owner。

### 调整

- [crates/ditto-core/src/llm_core/mod.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/llm_core/mod.rs)
  删除了 `pub mod error`
- 同文件删除了对 `DittoError` / `ProviderResolutionError` / `Result` 的公开 re-export
- 同时新增显式标识注释：
  `LLM-CORE-NO-ERROR-FACADE`

### 结果

- `llm_core` 重新只拥有 provider-agnostic model / layer / stream 前门
- 共享错误类型继续只由 `foundation::error` 持有
- `llm_core::error` 这层 facade 噪音被去掉，L0 底层 owner 关系更平

### 校验

本轮完成后已通过：

- `cargo fmt --check`
- `cargo check --all-targets`
- `cargo test --test module_namespace_contract --test default_core_contract`

## 追加复核：Omne / Yunwu Gemini 3.1（gateway translation 与 builder 边界，2026-03-11）

本日又通过 `omne-agent` 发起了一轮针对当前 L0 收尾质量的复核：

- thread: `49557314-615d-45b8-8d79-f99b7422d171`
- effective provider: `google.providers.yunwu`
- effective model: `gemini-3.1-pro-preview`
- effective base_url: `https://yunwu.ai/v1beta`
- final review turn: `62a33791-ba51-4c74-acd1-7cf961769956`

这轮复核要求模型只基于已经读入线程的关键 L0 文件给最终判断，避免再次做大范围 repo 扫描。

### 新增 review 结论

模型这轮给出的结论是：

- `Blocking findings: none`
- 当前 L0 结构已经基本准确，也足够干净
- 还剩 3 个值得继续收口的非阻塞点

### 本轮采纳并落地的修复

#### 1. gateway translation 不再直连 `runtime_registry` 单例

调整：

- `crates/ditto-server/src/gateway/application/translation/mod.rs` 不再直接导入
  `builtin_runtime_registry_catalog`
- `crates/ditto-core/src/runtime/model_builders.rs` 新增：
  - `builtin_runtime_supports_capability(...)`
  - `builtin_runtime_supports_file_builder(...)`
- `crates/ditto-core/src/runtime/mod.rs` 将上述两个入口收进 runtime 前门

这样处理后：

- gateway application 对 capability/file-builder 支持判断只依赖 `runtime`
- `runtime_registry` 继续留在 runtime 背后，不再泄漏为 gateway application 的隐藏全局依赖

#### 2. translation 内嵌 LRU 状态移入 gateway cache adapters

调整：

- 新增 `crates/ditto-server/src/gateway/adapters/cache/local_lru.rs`
- 原先写在 `crates/ditto-server/src/gateway/application/translation/mod.rs` 里的 `ModelCache<V>`
  已移除
- translation runtime 现在改用 `gateway::adapters::cache::LocalLruCache`

这样处理后：

- translation application 不再自己拥有缓存机制实现
- bounded in-process cache 逻辑被收口到 gateway adapters/cache 下
- 原有 recency 行为测试也一并迁到新模块，避免回归保护丢失

#### 3. builder 的 capability -> invocation probe 顺序收口到 contracts

调整：

- `crates/ditto-core/src/contracts/ids.rs` 新增
  `invocation_operations_for_capability(...)`
- `crates/ditto-core/src/contracts/mod.rs` 公开该契约 helper
- `crates/ditto-core/src/runtime/model_builders.rs` 删除本地
  `builder_operations_for_capability(...)` 与对应静态数组，改为直接依赖
  contracts 中的 ordered probe 映射

这里明确采用的是“generic runtime builder probe operations”语义，而不是简单把
`capability_for_operation()` 强行反转；原因是 builder 探测顺序本身也是契约的一部分，
但它描述的是“通用 builder 会探测哪些调用面”，不是 capability 全量操作宇宙。

### 本轮结论

截至这一轮：

- L0 没有新的阻塞性架构问题
- `gateway` 对 `runtime_registry` 的隐藏单例依赖已被去掉
- translation 的本地缓存实现已退出 application 模块本体
- builder probe 顺序不再在 runtime 内单独维护第二份规则表

这意味着当前 L0 不只是“方向对”，而是在 owner 收口和边界表达上也更接近 DSL 了。

## 本轮推进：移除 `compat/profile` 这层 test-only shadow facade（2026-03-11）

本轮继续使用 `omne-agent/.omne_data/.env` 调起 Yunwu 的
`gemini-3.1-pro-preview` 做 L0 结构评审。

- 一个较大范围的评审线程再次停在只读文件阶段，没有产出 `assistant_message`
  或 `turn_completed`
- 随后改用更窄范围再次评审 `src/compat/profile/mod.rs`
- 线程 `229a6cd5-c47a-4fd0-add2-81f3b181c6f5`
  / turn `3cb2e913-042c-4bb8-b7a8-270494071508`
  正常完成，结论是：`compat/profile` 确实违反 L0 owner 边界

Omne 给出的核心判断是：

- `src/compat/profile/mod.rs` 把 `config`、`runtime_registry`、
  `provider_transport`、`config_editing` 等独立 owner 重导出到一个聚合层
- `src/compat/profile/catalog_bridge.rs` 只是
  `runtime_registry` 的影子通道
- 这种聚合面模糊了 DSL 里已经明确分开的 L0 owner 与控制面/数据面边界

### 实际修复

- `pre-split src/lib.rs`
  移除了 `#[cfg(test)] mod compat;`，不再把 `compat` 放进 crate 编译图
- 直接删除了整个
  `pre-split src/compat/`
  目录下的 shadow facade 文件，包括：
  - `src/compat/profile/mod.rs`
  - `src/compat/profile/catalog_bridge.rs`
  - 以及只被这层 facade 持有的 test-only catalog / client / models 包装

### 为什么直接删除，而不是继续收缩

- 这层代码在正常构建里没有任何生产入边，只靠 `#[cfg(test)] mod compat;`
  被测试编译临时拉起
- 既然它不是正式兼容面，也不是正式 owner，继续保留只会制造“第二命名空间”
- 对这类死 facade，最小且最干净的修复就是删除，而不是再做一层瘦身

### 测试迁移

`compat/profile/tests.rs` 里仍然覆盖真实 owner 行为的部分，已经迁回 owner 本身：

- [crates/ditto-core/src/config/auth.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/config/auth.rs)
- [crates/ditto-core/src/config/env.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/config/env.rs)
- [crates/ditto-core/src/config/provider_config.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/config/provider_config.rs)
- [crates/ditto-core/src/provider_transport/config.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/provider_transport/config.rs)
- [crates/ditto-core/src/providers/openai_compat_profile.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/providers/openai_compat_profile.rs)

没有 runtime 调用面的那些 test-only catalog 包装和其测试，随 facade 一并删除。

## 本轮推进：把 OpenAI text surface 判定收回 `catalog` owner（2026-03-11）

继续对照 DSL 的 L0 owner 边界时，发现还有一处跨层读取：

- [crates/ditto-core/src/providers/openai/text.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/providers/openai/text.rs)
  之前通过 `runtime_registry::builtin_runtime_registry_catalog()` 判断
  `gpt-4.1` / `davinci-002` 这类模型支持哪个 text surface
- 但这类判断本质上是“静态支持矩阵查询”
- 按 DSL，静态矩阵归 `catalog`，`runtime_registry` 则应只拥有基于 catalog
  派生出来的 runtime-facing registry 视图

### 实际修复

- [crates/ditto-core/src/catalog/mod.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/catalog/mod.rs)
  新增了 `CatalogRegistry::supports_operation(...)`
- [crates/ditto-core/src/providers/openai/text.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/providers/openai/text.rs)
  改为通过 `builtin_registry().supports_operation(...)`
  做 surface 判定
- 同时补了一条显式标记注释：
  `STATIC-SURFACE-RESOLUTION`
  方便后续快速定位这条 owner 边界

### 结果

- `providers` 适配层不再依赖 `runtime_registry` 这个派生视图 owner
- OpenAI text surface 选择重新回到 `catalog` 这个静态字典 owner
- L0 依赖方向更接近 DSL：`providers -> catalog`，而不是
  `providers -> runtime_registry`

## 本轮推进：收紧 `runtime` 门面对底层 owner 的泄漏（2026-03-11）

这轮再次调用 Omne/Yunwu 做 L0 总体复评：

- 线程 `75ab9be6-dbf6-4bb1-8bc0-c0a6eda4f4c7`
  / turn `cfab4d90-5849-4711-b17f-5d6a81db7b16`
  在限定文件全部读取后停在 `2026-03-11T09:19:19Z`
- 随后再起一个只问“当前最值得处理的一条剩余结构问题”的窄线程
  `c82a12ac-c509-4e3e-bbb6-c4c6bd5a71ef`
  / turn `21d24c27-0f87-4d83-967a-3b35095c79a1`
  也停在文件读取阶段，没有产出最终 `assistant_message`

虽然 Yunwu 本轮没有返回最终结论，但结合它的读取轨迹和本地依赖检查，
当前最值得继续收口的一点是：

- [crates/ditto-core/src/runtime/resolver.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/runtime/resolver.rs)
  的 `RuntimeCatalogResolver` 之前同时暴露了：
  - `registry()`，把 `catalog::CatalogRegistry` 原样漏出
  - `registry_snapshot()`，把 `runtime_registry::RuntimeRegistrySnapshot` 原样漏出

这会让 `runtime` 从“装配 owner”又退回成“其他 owner 的转发门面”，
与 DSL 里 `runtime` / `catalog` / `runtime_registry` 的单向边界不一致。

### 实际修复

- 删除了 `RuntimeCatalogResolver::registry()`
- 删除了 `RuntimeCatalogResolver::registry_snapshot()`
- 在同一位置补了显式边界注释和标记：
  `RUNTIME-ASSEMBLY-ONLY`

### 结果

- `runtime` 继续只暴露 route / explain / transport assembly 能力
- 原始静态字典继续留在 `catalog`
- snapshot 视图继续留在 `runtime_registry`
- `runtime` 不再回流成底层 owner 的第二入口

## 本轮推进：把 `runtime` 公开 API 收到 builtin frontdoor（2026-03-11）

继续推进 L0 边界时，发现上一轮只删掉了
`RuntimeCatalogResolver` 对原始 owner 的一部分泄漏，但 `runtime` 公开 API
本身仍然把 `CatalogRegistry` 暴露给外部：

- `runtime::explain_runtime_route(registry: CatalogRegistry, ...)`
- `runtime::plan_runtime_transport(registry: CatalogRegistry, ...)`
- `runtime::RuntimeCatalogResolver`

这说明 `runtime` 仍然允许外部通过自己的公开表面直接操作 `catalog` owner，
与 DSL 里“`runtime` 是装配层、`catalog` 是静态字典 owner”的单向边界不一致。

### 实际修复

- [crates/ditto-core/src/runtime/mod.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/runtime/mod.rs)
  不再公开：
  - `RuntimeCatalogResolver`
  - `explain_runtime_route`
  - `plan_runtime_transport`
- 同时补了显式标记注释：
  `RUNTIME-BUILTIN-FRONTDOOR`
- [crates/ditto-core/src/runtime/explain.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/runtime/explain.rs)
  的 `explain_runtime_route(...)` 改为 `pub(crate)`
- [crates/ditto-core/src/runtime/transport.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/runtime/transport.rs)
  的 `plan_runtime_transport(...)` 改为 `pub(crate)`
- [crates/ditto-core/src/runtime/resolver.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/runtime/resolver.rs)
  删掉了整个 `RuntimeCatalogResolver` facade，只保留
  `resolve_builtin_runtime_route(...)`

### 结果

- `runtime` 对外继续保留 builtin assembly frontdoor：
  - `resolve_builtin_runtime_route`
  - `explain_builtin_runtime_route`
  - `plan_builtin_runtime_transport`
- `CatalogRegistry` 不再通过 `runtime` 公开 API 反向泄漏
- `runtime` 的抽象层次更接近 DSL：只做装配，不再充当 `catalog` 的转发表面

## 本轮推进：按 Omne 评审把模型级策略收回 runtime 注入（2026-03-11）

前面挂住的 Omne 线程在中断后实际返回了完整意见。
主线程 `75ab9be6-dbf6-4bb1-8bc0-c0a6eda4f4c7`
/ turn `cfab4d90-5849-4711-b17f-5d6a81db7b16`
给出的两条当前最有价值的结构意见是：

- `providers/openai/text.rs` 不应直接查全局静态字典决定 surface
- `providers/openai_compatible/client.rs` 不应在 client 内部硬编码
  `"deepseek-reasoner"` 这类模型名策略

这两条本质上都指向同一个 owner 原则：

- `catalog` / `runtime_registry` 是静态事实来源
- `runtime` 负责把这些事实解析后注入 provider adapter
- `providers` 不应越级回读全局字典，也不应自己私藏模型级特判

### 实际修复

- [crates/ditto-core/src/providers/openai/text.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/providers/openai/text.rs)
  增加了 `with_operation_support_resolver(...)`
- 同文件不再直接依赖 `catalog::builtin_registry`
- surface 判定改为依赖 runtime 注入的 operation support resolver
- [crates/ditto-core/src/runtime/model_builders.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/runtime/model_builders.rs)
  在构建 OpenAI text model 时注入静态支持矩阵查询
- 同处补了显式标记注释：
  `MODEL-SURFACE-RESOLUTION`
- [crates/ditto-core/src/providers/openai_compatible/client.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/providers/openai_compatible/client.rs)
  删除了 `inferred_model_behavior(...)`
  里对 `"deepseek-reasoner"` 的字符串硬编码
- `request_quirks_for_model(...)` 现在只合并：
  - family/base-url 级默认 quirks
  - runtime 注入的 model behavior
  - 显式 builder override

### 结果

- OpenAI text surface 选择重新变成 `runtime -> provider` 的单向注入
- OpenAI-compatible 的模型级奇葩行为重新只接受 runtime/catalog 注入
- provider adapter 本身不再反向依赖全局静态事实来源，也不再私藏模型名特判

### 暂未处理

Omne 还指出 `config_editing.rs` 依然带有文件写入和交互式 I/O，
这在长期上仍然更像外层 CLI / control plane 逻辑。
这条需要更大的仓库边界调整，这轮没有继续上提。

## 本轮推进：移除 crate root 对 L0 类型的公开代持（2026-03-11）

继续对照 DSL 的 L0 owner 列表时，crate root 还残留了一小组公开前门：

- `Env` / `ProviderAuth` / `ProviderConfig` 从 `config` 被顶层重新导出
- `ReasoningActivationKind` / `ReasoningOutputMode` 从 `catalog` 被顶层重新导出
- `ContextCacheMode` / `ContextCacheProfile` 从 `capabilities::context_cache` 被顶层重新导出

这组 root alias 虽然量不大，但语义上仍然让 `lib.rs` 继续扮演
`config` / `catalog` / `capabilities::context_cache` 的第二 owner，不符合前面已经收紧的
“显式 owner 模块、不要隐藏前门”的方向。

### 实际修复

#### 1. `lib.rs` 不再公开这些 L0 类型

调整：

- `src/lib.rs` 删除了：
  - `pub use config::{Env, ProviderAuth, ProviderConfig};`
  - `pub use catalog::{ReasoningActivationKind, ReasoningOutputMode};`
  - `pub use capabilities::context_cache::{ContextCacheMode, ContextCacheProfile};`
- `lib.rs` 顶部注释同步改为：crate root 只作为模块索引，不再承担低层 owner 的公开代持

结果：

- `config` / `catalog` / `capabilities::context_cache` 重新成为各自类型的唯一显式入口
- crate root 不再对这些 L0 类型形成“顺手可拿”的第二路径

#### 2. 调用面切回显式 owner 路径

调整：

- `tests/deepseek_provider_capabilities.rs`
- `tests/minimax_provider_capabilities.rs`
- `tests/gateway_translation/tests.rs`

这些测试里原先的 root 路径：

- `ditto_llm::ProviderConfig`
- `ditto_llm::ProviderAuth`
- `ditto_llm::Env`
- `ditto_llm::ContextCacheMode`
- `use ditto_llm::{ReasoningActivationKind, ReasoningOutputMode}`

都改回了显式 owner 模块：

- `ditto_llm::config::*`
- `ditto_llm::catalog::*`
- `ditto_llm::capabilities::context_cache::*`

结果：

- 当前代码不再依赖这些 root alias
- 这条 owner 规则也在 tests 里得到直接体现，后续如果有人想把这些类型重新挂回 root，
  很容易被 review 看出来

### 校验

本轮完成后已通过：

- `cargo fmt --check --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml`
- `cargo check --all-targets --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml`
- `cargo test --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml --test deepseek_provider_capabilities --test minimax_provider_capabilities`
- `cargo test --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml --features 'gateway-translation gateway-routing-advanced gateway-costing gateway-store-sqlite gateway-proxy-cache provider-openai-compatible provider-anthropic provider-google cap-llm cap-embedding' --test gateway_translation_custom_provider_resolution`
- `cargo test --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml --features 'gateway-translation gateway-routing-advanced gateway-costing gateway-store-sqlite gateway-proxy-cache provider-openai-compatible provider-anthropic provider-google cap-llm cap-embedding cap-image-generation cap-moderation cap-audio-transcription cap-audio-speech cap-batch' --test gateway_translation_provider_aliases`

## 本轮推进：把 `runtime_registry` 的 builtin 派生查询收成显式 query view（2026-03-11）

这轮再次尝试通过 `omne-agent/.omne_data/.env` 调起 Yunwu 的
`gemini-3.1-pro-preview` 做 L0 当前态复核。

实际结果有两次：

- thread `29d431a7-1af5-42a9-b0b4-671387780ba0`
  在读取指定文件后以 `llm stream failed after 3 attempts: api error (502 Bad Gateway)`
  失败
- thread `7e0b9fcb-5924-4194-8ebe-1fe2ea9d45db`
  成功开始按限定范围读取 DSL 与 L0 owner 文件，但没有产出最终
  `assistant_message`；事件停在文件读取之后

因此，这轮没有拿到新的可引用 Omne 结论文本，只能基于前面已记录的 Omne 剩余意见
和当前代码状态继续推进。

### 继续推进的剩余结构点

前面几轮 review 已经点过一个尚未收口的非阻塞问题：

- `crates/ditto-core/src/runtime_registry/catalog.rs` 仍然暴露一组直接绑死
  `builtin_registry()` 的全局裸函数

这会让 `runtime_registry` 看起来仍像“helper bag”，而不是 DSL 里那个明确拥有
“基于 catalog 派生出来、供 runtime / config / gateway 消费的查询语义”的 owner。

### 实际修复

#### 1. 新增显式 query view：`BuiltinRuntimeRegistryCatalog`

调整：

- `crates/ditto-core/src/runtime_registry/catalog.rs` 新增
  `BuiltinRuntimeRegistryCatalog`
- 新增公开构造入口：
  `runtime_registry::builtin_runtime_registry_catalog()`
- 之前散落的 public helper：
  - provider presets
  - capability summaries
  - model candidates
  - provider config semantics
  - openai-compatible capability profile
  现在都收成这个 view 上的方法

结果：

- `runtime_registry` 对外不再以“全局裸函数集合”暴露 builtin 派生查询
- 这组查询现在有了明确 owner 对象，和 `builtin_runtime_registry()` 快照形成并列但不同职责的两条面：
  - snapshot：稳定机读快照
  - catalog view：运行时派生查询语义

#### 2. 内部调用面也改成通过 query view 进入

调整：

- `crates/ditto-core/src/runtime/model_builders.rs`
- `crates/ditto-core/src/providers/openai/text.rs`
- `crates/ditto-server/src/gateway/application/translation/mod.rs`
- `crates/ditto-server/src/config_editing.rs`
- `src/compat/profile/openai_models.rs`
  这些调用面不再直连旧 helper 名称，而是统一通过
  `builtin_runtime_registry_catalog()` 调方法

结果：

- `runtime` / `providers` / `gateway` / `config_editing` 对
  `runtime_registry` 的依赖都重新落在同一个显式入口上
- 代码里不再保留“表面上是 owner 模块，实际还是一串散 helper”的旧形态

#### 3. 合同测试同步到新 owner 入口

调整：

- `tests/default_core_contract.rs`
- `tests/catalog_resolver_contract.rs`
- `tests/gateway_translation_provider_aliases.rs`
  都改为走新的 `runtime_registry` / `runtime` owner 路径

结果：

- contract tests 不再依赖已收掉的旧 helper / gateway builder 代持入口
- `gateway_translation_provider_aliases` 也继续遵守“builder 归 `runtime` owning”

### 校验

本轮完成后已通过：

- `cargo fmt --check --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml`
- `cargo check --all-targets --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml`
- `cargo test --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml --test default_core_contract --test catalog_resolver_contract`
- `cargo test --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml runtime_registry::catalog::tests --lib`
- `cargo test --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml --features 'gateway-translation gateway-routing-advanced gateway-costing gateway-store-sqlite gateway-proxy-cache provider-openai-compatible provider-anthropic provider-google cap-llm cap-embedding' --test gateway_translation_custom_provider_resolution`
- `cargo test --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml --features 'gateway-translation gateway-routing-advanced gateway-costing gateway-store-sqlite gateway-proxy-cache provider-openai-compatible provider-anthropic provider-google cap-llm cap-embedding cap-image-generation cap-moderation cap-audio-transcription cap-audio-speech cap-batch' --test gateway_translation_provider_aliases`

## 追加复核：Omne / Yunwu Gemini 3.1（live 线程命中 owner 泄漏，先落地高置信修复，2026-03-11）

本轮再次使用 `omne-agent/.omne_data/.env` 调起 Yunwu 的
`gemini-3.1-pro-preview` 做 L0 结构复核。

实际执行信息：

- thread: `861c591c-1a94-4223-8586-1a548458ec38`
- thread: `d4156ea0-bd76-4d81-a276-15f2647db9be`
- model_routed: `gemini-3.1-pro-preview`
- provider routing source: `google.providers.yunwu`

这两条 live thread 在本轮结束前都没有产生最终 `assistant_message`，但事件日志里能确认
模型已经实际读到了这批当前边界文件：

- `docs/src/roadmap/ai-gateway-platform-deep-governance.dsl`
- `src/lib.rs`
- `crates/ditto-core/src/foundation/mod.rs`
- `crates/ditto-core/src/runtime/mod.rs`
- `crates/ditto-core/src/runtime/model_builders.rs`
- `crates/ditto-core/src/runtime_registry/mod.rs`
- `crates/ditto-core/src/config/mod.rs`
- `crates/ditto-core/src/provider_transport/mod.rs`
- `crates/ditto-core/src/provider_options/mod.rs`
- `crates/ditto-core/src/session_transport/mod.rs`
- `crates/ditto-core/src/contracts/mod.rs`
- `crates/ditto-core/src/llm_core/mod.rs`
- `crates/ditto-server/src/gateway/application/translation/mod.rs`

结合这轮 live review 已命中的检查面和本地源码核对，当前最确定的两处 owner 泄漏是：

1. `src/lib.rs` 又把 `foundation::error` / `llm_core` 暴露成 public root frontdoor
2. `crates/ditto-core/src/foundation/mod.rs` 仍然通过 `foundation::env` / `foundation::utils`
   二次代持 `config` / `utils`

### 实际修复

#### 1. 重新收回 `lib.rs` 的低层 public frontdoor

调整：

- `src/lib.rs` 中
  `foundation::error::{DittoError, ProviderResolutionError, Result}` 改回 `pub(crate) use`
- `src/lib.rs` 中 `llm_core::*` 改回 `pub(crate) use`
- 同时保留 root 上那几项较窄的 convenience alias：
  `ContextCacheMode` / `ContextCacheProfile` /
  `ReasoningActivationKind` / `ReasoningOutputMode` /
  `Env` / `ProviderAuth` / `ProviderConfig`

结果：

- crate 对外不再把 `foundation::error` 与 `llm_core` 公开成第二 owner
- crate 内部仍可继续使用紧凑路径，避免这一轮无意义的大面积重写
- 受影响测试已切到显式 owner 路径：
  - `tests/agent_loop.rs`
  - `tests/deepseek_provider_capabilities.rs`
  - `tests/integration_smoke.rs`

#### 2. 去掉 `foundation` 对 `config` / `utils` 的 facade 代持

调整：

- `crates/ditto-core/src/foundation/mod.rs` 删除了：
  - `foundation::env -> crate::config::{Env, parse_dotenv}`
  - `foundation::utils -> crate::utils::*`
- 模块注释改为显式说明：
  - config parsing 留在 `crate::config`
  - generic helpers 留在各自 owner 模块

结果：

- `foundation` 重新只拥有真正的底层支撑职责
- `config` 与 `utils` 不再被 `foundation` 反向吞成自己的子入口
- 这一步没有引入仓库内迁移成本，因为当前代码里没有任何现存调用面依赖
  `foundation::env` 或 `foundation::utils`

### 这轮没有继续扩大的部分

`crates/ditto-server/src/gateway/application/translation/mod.rs` 里仍然存在一些 crate-root 紧凑导入，
例如 `crate::DittoError` / `crate::Result`。

这类内部路径确实还不够“owner path first”，但它们已经不再构成 public API 泄漏；
本轮先优先收掉 public owner 边界和显式 facade，避免把一次局部 owner 修复扩展成
全仓内部导入风格迁移。

### 校验

本轮完成后已通过：

- `cargo fmt --check`
- `cargo check --all-targets`
- `cargo test --features agent --test agent_loop`
- `cargo test --features 'provider-deepseek cap-llm' --test deepseek_provider_capabilities`
- `cargo test --features integration --test integration_smoke`

## 本轮推进：让 `runtime` 成为 builder 的唯一公开 owner（2026-03-11）

继续对照 DSL 的 L0 `runtime` 定义时，当前实现里还剩一处结构不够收口的点：

- builder 逻辑实际在 `crates/ditto-core/src/runtime/model_builders.rs`
- 但 gateway 和若干 capability / provider 测试仍把这套入口当成
  `gateway::translation::build_*`
- 这样等于让 L1/gateway 在表面上继续代持了 L0 runtime builder

### 调整

- `crates/ditto-core/src/runtime/mod.rs`
  - 不再公开子模块 `model_builders`
  - 改为由 `runtime` 自身直接 `pub use build_*`
- `crates/ditto-server/src/gateway/application/translation/mod.rs`
  - 从 `use crate::runtime::model_builders;`
    改为直接依赖 `crate::runtime::{build_*}`
- `crates/ditto-server/src/bin/ditto-gateway.rs`
  - backend 组装切到 `ditto_llm::runtime::build_language_model`
- 下列测试全部切回 `ditto_llm::runtime::build_*`
  - `tests/gateway_translation_custom_provider_resolution.rs`
  - `tests/minimax_provider_capabilities.rs`
  - `tests/anthropic_provider_capabilities.rs`
  - `tests/google_provider_capabilities.rs`
  - `tests/deepseek_provider_capabilities.rs`
  - `tests/gateway_translation/tests.rs`
  - `tests/openai_provider_capabilities.rs`
- `tests/module_namespace_contract.rs`
  - 新增对 `ditto_llm::runtime::build_language_model`
    和 `ditto_llm::runtime::build_context_cache_model` 的 contract 锁定

### 结果

- `runtime` 成为 builder 的唯一公开 owner
- `model_builders.rs` 退回 runtime 内部实现文件，而不是外部依赖的路径契约
- `gateway::translation` 不再在公开表面上继续代持 runtime builder
- bin / tests 的调用路径也和 L0 owner 结构重新对齐

### 顺手修复

在补这轮 gateway 校验时，还顺手修了两处当前分支里的小问题：

- `crates/ditto-server/src/gateway/application/translation/mod.rs`
  - 旧的 `RuntimeRouteRequest::with_provider_config(...)`
    已改为当前正确的 `with_runtime_hints(...)`
- `crates/ditto-server/src/bin/ditto_gateway/config_cli.rs`
  - 缺少 `gateway-cli-interactive` feature 时，
    改为返回结构化 `DittoError::Config(...)`
    而不是错误地把 `&str` 直接 `.into()`

### 校验

本轮完成后已通过：

- `cargo fmt --check`
- `cargo check --all-targets`
- `cargo test --test module_namespace_contract`
- `cargo test --features 'gateway-translation gateway-routing-advanced gateway-costing gateway-store-sqlite gateway-proxy-cache provider-openai-compatible provider-anthropic provider-google cap-llm cap-embedding' --test gateway_translation_custom_provider_resolution`

## 本轮修复：按 review 收口 OpenAI-Compatible 请求模型行为与 root front-door 回归（2026-03-11）

这轮不是再做新一层架构改造，而是处理三条已经能复现的正确性 / API 回归问题：

1. `OpenAICompatible` 把模型行为错误地固化在构建时的 `default_model`
2. `lib.rs` 把 root front-door 直接收成 `pub(crate)`，造成真实 source break
3. 两个 Google 回归测试因为 `ProviderConfig` 新字段没有补齐而无法编译

### 1. `OpenAICompatible` 改回按实际请求模型决策

调整：

- [crates/ditto-core/src/providers/openai_compatible/client.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/providers/openai_compatible/client.rs)
  新增了 request-time 的 `request_quirks_for_model(...)`
- provider 现在先基于真实 `request.model` 解析请求行为，再进入
  `messages_to_chat_messages(...)` / `build_chat_completions_body(...)`
- runtime builder 不再按 `ProviderConfig.default_model` 预先塞死布尔开关；
  改为在 [crates/ditto-core/src/runtime/model_builders.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/runtime/model_builders.rs)
  注入一个按模型查询的 resolver
- direct client 场景下，`DeepSeek + deepseek-reasoner` 这种已知严格模型，
  也会走 family/model 级安全推断，不再因为没有 `default_model` 而漏掉
  `reasoning_content` / `tool_choice=required` 约束

结果：

- `request.model` 覆盖构建时默认模型时，行为跟着真实请求走
- 没有 `default_model` 的 DeepSeek reasoner 直连 client，也不会再把严格约束漏掉
- provider adapter 仍然只消费 runtime 注入的行为决议，没有回退成“直接依赖 runtime_registry”

### 2. 恢复窄 root front-door，避免 source break

调整：

- `pre-split src/lib.rs`
  恢复了一组有限的 public aliases：
  - `DittoError` / `ProviderResolutionError` / `Result`
  - `LanguageModel` / `StreamResult` 等常用 llm_core 前门
  - `Env` / `ProviderConfig` / `ProviderAuth`
  - `ContextCacheMode` / `ContextCacheProfile`
  - `ReasoningActivationKind` / `ReasoningOutputMode`
- 同时补了显式注释，说明这是稳定 SDK front-door，不是重新把 root 变成
  全量 L0 module 的第二 owner

结果：

- `cargo test --features agent --test agent_loop` 这类真实用户入口重新可编译
- root 只恢复“高频、用户向”的窄前门，而不是回退到整层 `pub use *`

### 3. Google 回归测试改成稳态 struct update

调整：

- [tests/google_stream_http_header_auth.rs](/root/autodl-tmp/zjj/p/ditto-llm/tests/google_stream_http_header_auth.rs)
- [tests/google_stream_empty_response_fallback.rs](/root/autodl-tmp/zjj/p/ditto-llm/tests/google_stream_empty_response_fallback.rs)

这两个测试里的 `ProviderConfig` struct literal 改成了：

- 显式字段 + `..ProviderConfig::default()`

结果：

- `openai_compatible` 这类新增配置字段不会再把测试直接打编译挂
- 这类 provider config 测试后续更不容易因为字段扩展再次脆断

### 校验

本轮完成后已通过：

- `cargo fmt --check`
- `cargo check --all-targets`
- `cargo test --features agent --test agent_loop`
- `cargo test --features 'google streaming openai-compatible' --test google_stream_http_header_auth --test google_stream_empty_response_fallback`
- `cargo test --features 'provider-openai-compatible cap-llm' request_quirks_for_model --lib`
- `cargo test --features 'provider-openai-compatible cap-llm' deepseek_reasoner --lib`
- `cargo test --features 'provider-deepseek cap-llm' --test deepseek_provider_capabilities`
- `cargo test --test integration_smoke`

## 追加收口：provider 不再直连 `catalog`（2026-03-11）

在前面几轮调整之后，L0 里还残留了一类边界泄漏：

- `crates/ditto-core/src/providers/openai/text.rs`
- `crates/ditto-core/src/providers/openai_compatible/client.rs`

这两个 provider 文件虽然已经属于 L0 实现层，但它们仍然直接调用
`catalog::builtin_registry()` 读取 builtin operation / behavior 细节。

这会带来两个问题：

- `providers` 重新知道了 `catalog` 的静态目录形状，而不是只依赖 L0 暴露出来的运行时视图
- `runtime_registry` 作为 “catalog 衍生的运行时查询入口” 的 owner 关系被绕开了

### 本轮修复

本轮继续按最小结构修复处理，没有扩展职责，只把 owner 路径收正。

调整：

- `crates/ditto-core/src/runtime_registry/catalog.rs` 新增 crate-private helper：
  - `builtin_provider_supports_operation`
  - `builtin_provider_requires_reasoning_content_followup`
  - `builtin_provider_required_tool_choice_support`
- `crates/ditto-core/src/runtime_registry/mod.rs` 统一 re-export 这些 helper 给 crate 内部使用
- `crates/ditto-core/src/providers/openai/text.rs` 不再直连 `catalog`，改为通过
  `runtime_registry::builtin_provider_supports_operation(...)` 判断 text surface
- `crates/ditto-core/src/providers/openai_compatible/client.rs` 不再读取 provider behavior descriptor，
  改为通过 `runtime_registry` helper 查询：
  - assistant tool followup 是否要求 reasoning content
  - `tool_choice = required` 是否受支持

### 结果

这次收口后的 owner 关系更清楚了：

- `catalog` 继续只拥有静态 provider/model/behavior 目录
- `runtime_registry` 继续拥有“从 catalog 派生出来、给运行时消费”的查询接口
- `providers` 只依赖运行时查询 helper，不再自己回读静态目录实现

这一步没有新增 facade，也没有引入兼容层；只是把 provider 侧最后两处
`catalog` 直连改回了正确的 L0 owner 路径。

## 本轮推进：按 Omne/Yunwu 对 runtime owner 边界继续收口（2026-03-11）

本轮再次通过 `omne-agent` 调起 Yunwu 的 `gemini-3.1-pro-preview` 做 L0 结构评审。
这次 Omne 的核心判断是：

- 阻塞问题：
  - gateway translation 仍然直接越过 `runtime` facade，调用 `catalog::builtin_registry()` 做 capability / file-builder 判断，属于 L1 吞掉 L0 边界
  - `runtime/model_builders.rs` 一边声明自己是 runtime assembly，一边继续散落直接查询全局 `builtin_registry()`，形成“runtime_registry 显式 owner”和“catalog 全局单例隐式 owner”并存
  - `builtin_registry()` 继续充当隐形 root owner，不利于 machine-first / 无状态 / 成本可预测的边界表达
- 非阻塞问题：
  - `resolve_builder_runtime_for_capability()` 在已经拿到 `route` 后，又回头做一次 registry 查询
  - `runtime_registry::snapshot` 仍直接从 builtin catalog 构造快照，后续如果要支持非内置来源，扩展面还不够理想
- 建议修复：
  - gateway 不再直接依赖 `catalog`
  - builder / capability 查询统一收敛到 `runtime` / `runtime_registry` owner
  - 提供一次性返回 capability / builder 语义的统一查询入口，避免各处自己摸 registry

### 实际修复

#### 1. 把 builder / capability 查询收回 `runtime_registry`

调整：

- [crates/ditto-core/src/runtime_registry/catalog.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/runtime_registry/catalog.rs)
  新增了 crate 内部 builder/capability helper：
  - `resolve_builtin_builder_provider(...)`
  - `builtin_provider_supports_capability(...)`
  - `builtin_provider_supports_file_builder(...)`
- 这些 helper 现在成为 `catalog` -> `runtime_registry` 的唯一桥接点，把
  provider 解析、builder family 推导、capability support 判断集中在同一个 owner

结果：

- 不再需要由 gateway 或 runtime 子模块各自直连 `builtin_registry()`
- `catalog` 继续拥有静态真值，但“如何把真值变成 runtime-facing capability / builder 语义”
  已经回到 `runtime_registry`

#### 2. `runtime/model_builders.rs` 不再散落直连全局 registry

调整：

- [crates/ditto-core/src/runtime/model_builders.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/runtime/model_builders.rs)
  去掉了对 `ProviderPluginDescriptor` / `builtin_registry()` 的直接依赖
- 默认 builder 解析、capability 支持判断、context-cache provider 判断
  全部改为走 `crate::runtime_registry::*`
- 在 `resolve_builder_runtime_for_capability()` 里，
  已经拿到 `route` 后不再回查 registry，只根据 resolved route 的 provider hint
  做 builder family 归一

结果：

- `runtime` 仍然负责 assembly，但不再自己在多个分支上摸全局 catalog 单例
- builder owner 路径从“runtime 子文件各自查 catalog”收敛成
  “runtime 子文件 -> runtime_registry helper -> catalog truth”

#### 3. `gateway/application/translation` 不再越级读 catalog

调整：

- [crates/ditto-server/src/gateway/application/translation/mod.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-server/src/gateway/application/translation/mod.rs)
  已移除对 `catalog::builtin_registry()` 的直接调用
- `supports_runtime_capability()` 改为走
  `runtime_registry::builtin_provider_supports_capability(...)`
- `supports_file_builder()` 改为走
  `runtime_registry::builtin_provider_supports_file_builder(...)`

结果：

- gateway/L1 不再自己承担 provider capability 解析逻辑
- translation 对 L0 的依赖重新回到显式 owner 模块，而不是越过 facade 去摸底层真值

### 校验

本轮完成后已通过：

- `cargo test --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml runtime_registry::catalog::tests --lib`
- `cargo test -p omne-app-server llm_stream_tests --manifest-path /root/autodl-tmp/zjj/p/omne-agent/Cargo.toml`

## 追加复核：Omne / Yunwu Gemini 3.1（L0 隐式 owner 与 provider 反向依赖，2026-03-11）

本轮再次通过 `omne-agent` 发起复核，使用：

- provider: `google.providers.yunwu`
- model: `gemini-3.1-pro-preview`
- base_url: `https://yunwu.ai/v1beta`
- thread: `5ea700b9-55bd-4f89-8414-ced00867d615`
- omne_root env: `/root/autodl-tmp/zjj/p/omne-agent/.omne_data/.env`

这轮 review 明确要求模型只看当前 L0 与直接 owner 边界，不讨论产品规划或兼容 facade。

### 新增 review 结论

模型这轮给出的四条意见里，前两条是成立的结构问题，第三条不宜直接照搬，第四条有一半成立：

1. `runtime/route.rs` 里的 provider-hint base URL / path 推断把静态 provider 真值偷偷重建了一遍
2. `providers/openai_compatible/client.rs` 反向依赖 `runtime_registry`，让 provider adapter 回头查询上层 owner
3. `OpenAiProviderFamily` 应直接被 `ProviderProtocolFamily` 取代
4. `runtime/model_builders.rs` 里的 `canonical_builder_provider_from_hint` 维护了一份额外 provider taxonomy

### 采纳并落地的修复

#### 1. 去掉 `runtime/route` 的 provider-hint 魔法推断

调整：

- `crates/ditto-core/src/runtime/route.rs` 删除：
  - `INFERRED_RUNTIME_PROVIDER_HINTS`
  - `OpenAiPathStyle`
  - `normalize_runtime_invocation(...)`
  - `inferred_runtime_provider_hint(...)`
  - `runtime_provider_inference_hint(...)`
- `resolve_runtime_base_url(...)` 现在只接受三类 base URL 来源：
  - endpoint override
  - provider config
  - plugin default
  - 以及基于 `upstream_api` 的协议级 fallback
- `RuntimeBaseUrlSelectionSource` / explain 输出也同步去掉了
  `inferred_from_provider_hint`

结果：

- runtime 不再基于 `deepseek/openrouter/kimi/xai/...` 这种品牌字符串偷偷重建 provider truth
- 如果没有 dedicated catalog provider，也没有显式 `base_url`，generic alias 现在会失败而不是“猜一个官方地址”
- `upstream_api` 级 fallback 仍然保留，因为它来自显式声明的协议，而不是 provider 品牌猜测

#### 2. 去掉 `OpenAICompatible` 对 `runtime_registry` 的反向依赖

调整：

- `crates/ditto-core/src/providers/openai_compatible/client.rs` 不再 import `runtime_registry`
- provider 侧不再在请求路径按 `model` 动态查询：
  - `builtin_provider_requires_reasoning_content_followup(...)`
  - `builtin_provider_required_tool_choice_support(...)`
- `OpenAICompatibleRequestQuirks` 新增显式字段，承接 runtime 注入的 chat-completions 行为决议
- `OpenAICompatible` 新增显式 builder：
  - `with_assistant_tool_call_requires_reasoning_content(...)`
  - `with_tool_choice_required_support(...)`
- `crates/ditto-core/src/runtime/model_builders.rs` 在构建 openai-compatible LLM 时，通过
  `runtime_registry` 解析行为后显式注入这些 quirk

结果：

- provider adapter 回到“消费装配结果”的位置
- `runtime` 重新成为 catalog 行为解析的 owner
- provider 请求路径不再回头查询上层 registry

#### 3. 去掉 `model_builders` 里的额外 provider alias 字典

调整：

- `crates/ditto-core/src/runtime/model_builders.rs` 删除 `canonical_builder_provider_from_hint(...)`
- builder 选择现在统一通过
  `runtime_registry::resolve_builtin_builder_provider(...)`
  从 catalog/plugin class 推导
- route 成功后的 builder family 归一，也不再靠手写 alias 列表

结果：

- `model_builders` 不再维护第二份 provider taxonomy
- builder 归一规则重新回到 catalog/plugin class 派生结果
- Omne 提到的 “shadow registry” 在这块已经收掉

#### 4. 顺手把 `RuntimeRouteRequest` 的 config helper 从 `config` 里拿掉

调整：

- 删除 `crates/ditto-core/src/config/provider_config.rs` 里对 `RuntimeRouteRequest` 的 inherent impl
  `with_provider_config(...)`
- 调用点统一改成显式：
  `with_runtime_hints(provider_config.runtime_hints())`

结果：

- `RuntimeRouteRequest` 不再在 `config` 命名空间里挂额外 method
- route hint 的拼装位置重新变成显式 owner 调用

### 明确没有采纳的意见

这轮没有直接把 `OpenAiProviderFamily` 替换成 `ProviderProtocolFamily`。

原因很直接：

- 当前 `ProviderProtocolFamily` 的粒度只到 `OpenAi/Dashscope/Ark/Zhipu/...`
- 但 `openai_compatible` 这层现有 quirk 需要区分 `OpenRouter/DeepSeek/Kimi/MiniMax/Doubao`
  这些更细的 payload / provider_options 差异
- 如果现在硬替，会把 provider-specific 行为重新压平成更粗的 contract family，
  反而丢信息

所以这条建议这轮明确拒绝，不是忽略，而是当前 contract 抽象还不够承载它。

### 校验

本轮完成后已通过：

- `cargo fmt --check`
- `cargo check --all-targets`
- `cargo check --lib --features provider-openai,cap-llm`
- `cargo test --test runtime_route_contract --test runtime_explain_contract --test runtime_transport_contract --test catalog_resolver_contract`
- `cargo test --lib model_builder_tests`
- `cargo test --lib runtime_route_requires_explicit_base_url_for_generic_openai_like_aliases`
- `cargo test --lib messages_to_chat_messages_adds_empty_reasoning_for_deepseek_reasoner_tool_calls`
- `cargo test --lib build_body_rejects_deepseek_reasoner_required_tool_choice`
- `cargo test --lib auto_selects_surface_from_catalog --features provider-openai,cap-llm`

本轮尝试了：

- `cargo check --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml --features gateway --lib`

该检查仍然失败，但失败点是仓库里已经存在的 `gateway/transport/http/*` feature-gating
问题（`proxy_budget_reservations` / `admin` / `router` 相关条件编译不一致），
不是这次 runtime owner 收口引入的回归。

## 追加复核：Omne / Yunwu Gemini 3.1（L0 当前状态，2026-03-11）

本日又追加发起了一轮 `omne-agent` 复核，使用：

- provider: `google.providers.yunwu`
- model: `gemini-3.1-pro-preview`
- thread: `470057e6-29d3-4daf-a7f0-b25fae735c67`

这轮只看当前 L0 相关文件的结构边界，不看命名和格式。

### Omne 结论

Omne 这轮给出的阻塞意见有三条：

1. `crates/ditto-server/src/config_editing.rs` 仍然在配置编辑层直接发 HTTP 做模型发现，破坏了
   `config_editing` 作为确定性文档编辑层的边界
2. `crates/ditto-core/src/runtime/model_builders.rs` 仍然在 builder 内部自行查 `builtin_registry()` 并重复推导装配策略，
   没有完全收敛到上游显式 runtime 计划
3. `crates/ditto-core/src/providers/openai_compatible/client.rs` 仍然在 provider 侧直接读 `Env`
   控制请求开关，破坏了显式契约传导

非阻塞意见里，Omne 还点到了：

- `crates/ditto-core/src/capabilities/text.rs` 还在通过 `pub use crate::types::*` 形成第二 owner
- `gateway/application/translation` 仍然维持有界缓存和按需实例化
- `runtime_registry/catalog.rs` 仍然保留全局裸函数入口

### 采纳并落地的修复

这轮采纳了两条阻塞问题和一条非阻塞问题，保持“局部收敛、不过度重构”：

#### 1. 把模型发现副作用移出 `config_editing`

调整：

- `crates/ditto-server/src/config_editing.rs` 删除了基于 `reqwest` 的 `discover_models_for_provider()` 及相关 HTTP 解析逻辑
- `ProviderUpsertRequest` 新增显式 `model_whitelist: Vec<String>`
- `upsert_provider_config()` 现在只消费调用方传入的 whitelist；如果 `discover_models = true`
  但没有注入 whitelist，会直接报错
- `crates/ditto-server/src/bin/ditto_gateway/config_cli.rs` 现在负责 `--discover-models` 的 HTTP 发现，
  发现完成后再把结果注入 `ProviderUpsertRequest.model_whitelist`

结果：

- `config_editing` 重新回到“确定性文档树编辑”职责
- 网络发现仍然保留在外层 CLI 控制面，而不是继续留在 L0 编辑层里
- 这次修复没有把 discovery 逻辑塞回别的 L0 模块

#### 2. 把 OpenAI-Compatible 的 passthrough 开关改成显式 provider config

调整：

- `crates/ditto-core/src/config/provider_config.rs` 新增 `OpenAiCompatibleConfig`
- `ProviderConfig` 新增 `openai_compatible: Option<OpenAiCompatibleConfig>`
- `crates/ditto-core/src/providers/openai_compatible/client.rs` 不再扫描
  `OMNE_OPENAI_COMPAT_SEND_PROMPT_CACHE_KEY` /
  `OMNE_OPENAI_COMPAT_SEND_TOOL_CALL_THOUGHT_SIGNATURE`
  这类环境变量
- `OpenAICompatible::from_config()` 现在只读取显式 provider config 中的：
  - `send_prompt_cache_key`
  - `send_tool_call_thought_signature`

结果：

- provider 侧不再直接读取全局环境开关
- 这类请求行为开关变成了显式配置契约，而不是隐式环境探测
- 相关单测已同步改成验证显式 config 路径

#### 3. 移除 `capabilities::text` 对 `types` 的公开代持

调整：

- `crates/ditto-core/src/capabilities/text.rs` 把 `pub use crate::types::{...}` 改成了私有 `use`

结果：

- `capabilities::text` 不再作为 `GenerateRequest` / `GenerateResponse` /
  `StreamChunk` 等协议载荷的第二 owner
- `types` 继续保持这些 DTO 的唯一公开 owner

### 本轮明确没有继续硬拆的部分

这轮没有直接按 Omne 建议去重写 `runtime/model_builders.rs`，原因是：

- 那不是局部修复，而是一次真正的 runtime builder 契约重构
- 会牵动 `runtime`、`gateway`、provider builders 和测试矩阵
- 当前先把“副作用越界”和“全局环境探测”两条更明确的结构错误修掉，收益更直接

`gateway/application/translation` 的缓存 / snapshot 化问题也没有在这轮继续硬拆；
它仍然是后续要处理的结构议题，但不适合和这轮 L0 owner 收口混在一次提交里做。

### 校验

本轮修复后已通过：

- `cargo fmt --check`
- `cargo check --all-targets`
- `cargo test --features config-editing --test config_editing_contract`
- `cargo test --features config-interactive --test config_interactive`
- `cargo test from_config_reads_explicit_passthrough_flags --lib`

## 本轮推进：去掉 `capabilities::text` 对 `provider_options` 的第二 owner 路径（2026-03-11）

继续对照 DSL 的 L0 owner 列表时，发现还有一处公开边界泄漏：

- `provider_options` 已经是独立 L0 owner
- 但 `crates/ditto-core/src/capabilities/text.rs` 仍然 `pub use crate::provider_options::{...}`
  把 `JsonSchemaFormat` / `ProviderOptions` / `ResponseFormat` / `Reasoning*`
  再暴露到 `capabilities::text::*`

这和前面处理 root / gateway 的问题是同一类：能力 facade 不应再成为
`provider_options` 的第二 owner 入口。

### 调整

- [crates/ditto-core/src/capabilities/text.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/capabilities/text.rs)
  移除了对 `crate::provider_options::{...}` 的 `pub use`
- 同时补了一句显式注释，说明：
  - `capabilities::text` 只负责 text-oriented request/response convenience surface
  - provider passthrough 继续由 `crate::provider_options` 单独拥有

### 结果

- `provider_options` 的 owner 边界进一步收紧
- `capabilities::text` 不再把 provider passthrough 语义和 text capability facade 混在一起
- 仓库内也没有任何现有调用面依赖 `capabilities::text::ProviderOptions` 这条路径，因此这次收口没有引入额外迁移成本

### 校验

本轮完成后已通过：

- `cargo fmt --check`
- `cargo check --all-targets`
- `cargo test --test module_namespace_contract`

## 本轮推进：按 Omne/Yunwu 的 L0 结构评审继续收口 owner 边界（2026-03-11）

本轮使用 `omne-agent/.omne_data/.env` 调起 Yunwu 的
`gemini-3.1-pro-preview`，针对当前 L0 相关文件做了一次仅关注结构问题的评审。

Omne 这轮给出的核心意见有三条：

- `src/lib.rs` 仍然把 `foundation::error` 和 `llm_core` 暴露成 root 前门，
  root 还在扮演低层 owner 的第二入口
- `crates/ditto-server/src/config_editing.rs` 把文件变更、交互式编辑、模型探测这类副作用流程挂在
  `config` 纯配置层命名空间下面，破坏了 L0 `config` 作为无状态配置语义 owner 的边界
- `crates/ditto-server/src/gateway/application/translation/mod.rs` 通过
  `pub use crate::runtime::model_builders::{...}` 吞掉了 runtime builder 的 owner 路径，
  让 gateway/L1 在表面上成了 L0 builder 的第二入口

### 实际修复

#### 1. `lib.rs` 不再公开低层前门

调整：

- `src/lib.rs` 新增 `pub mod config_editing;`
- `foundation::error::{DittoError, ProviderResolutionError, Result}` 与
  `llm_core::*` 的 root alias 从 `pub use` 改成了 `pub(crate) use`

结果：

- crate 内部还能保留紧凑路径，避免无意义的大面积内部改名
- crate 对外不再把 root 暴露成 `foundation::error` / `llm_core` 的第二 owner
- tests / examples 已同步切到显式 owner 路径，例如：
  - `ditto_llm::foundation::error::*`
  - `ditto_llm::llm_core::model::LanguageModel`

#### 2. 把副作用型 config 编辑流程从 `config` 命名空间中剥离

调整：

- pre-split `src/config/editing.rs` 已移动为 `crates/ditto-server/src/config_editing.rs`
- `crates/ditto-core/src/config/mod.rs` 不再声明 `pub mod editing;`
- `crates/ditto-server/src/bin/ditto_gateway/config_cli.rs`
- `src/compat/profile/mod.rs`
- `tests/config_editing_contract.rs`
- `tests/config_interactive.rs`
  这些调用面全部改到 `ditto_llm::config_editing::*`

结果：

- `config` 重新收敛为无状态配置 schema / defaults / resolution owner
- 文件 I/O、交互式编辑、模型发现这类控制面副作用，不再挂在 `config::*`
  这个 L0 配置层入口下面
- 这一步先完成的是命名空间 owner 收口；还没有进一步把这些流程拆到独立 crate
  或独立仓库

#### 3. 去掉 gateway 对 runtime builders 的公开代持

调整：

- `crates/ditto-server/src/gateway/application/translation/mod.rs` 移除了
  `pub use crate::runtime::model_builders::{...}`
- 文件内部改为显式通过 `model_builders::build_*` 调用 runtime builder

结果：

- gateway 不再把 runtime builder 暴露成自己的公开表面
- `runtime::model_builders` 继续保持 L0 装配入口 owner
- `translation` 对 L0 builder 的依赖关系在源码里也重新变成了显式跨边界调用

### 本轮没有继续上升的部分

Omne 的更强版本建议，是把 `config_editing` 这类副作用流程彻底搬出当前 L0 crate。
这轮没有继续做到那一步，原因是：

- 这已经不是 owner 收口，而是仓库/产品分层调整
- 会同时牵动 CLI、gateway control surface、feature 组合和测试装配
- 当前先把公开命名空间收干净，已经足以消除最直接的 L0 边界泄漏

### 校验

本轮完成后已通过：

- `cargo fmt --check`
- `cargo check --all-targets`
- `cargo test --test runtime_route_contract --test runtime_transport_contract --test runtime_explain_contract --test runtime_registry_contract --test catalog_resolver_contract --test gateway_translation_custom_provider_resolution --test gateway_translation_provider_aliases`

## 追加复核：Omne / Yunwu Gemini 3.1（root internal facade，2026-03-11）

本轮继续使用 `omne-agent` 对当前 L0 实现做结构复核，显式确认线程配置为：

- provider: `google.providers.yunwu`
- model: `gemini-3.1-pro-preview`
- base_url: `https://yunwu.ai/v1beta`
- thread: `50053c84-a13e-473d-8ed8-095bce0eee9f`
- review turn: `841625d4-fc55-4b6c-bec1-d5c981909d64`
- final verdict turn: `4306a3f8-376f-454f-a5c2-61a8b7fff486`
- instructions: `/root/autodl-tmp/zjj/p/prompts/linus-rust.md`

### 新增 review 结论

模型给出的结论是：

- `Blocking findings: none.`

非阻塞意见有三条：

1. `contracts` 与 `types` 之间仍有 payload debt
2. `src/lib.rs` 还残留 `foundation::error` 与 `llm_core` 的 root facade 痕迹
3. `crates/ditto-core/src/runtime/model_builders.rs` 里仍有 builtin singleton lookup，后续可考虑 injection

### 本轮采纳的修复

这轮只采纳第 2 条，因为它是当前唯一局部、低风险、直接改善 L0 owner
边界的一项。

调整：

- `pre-split src/lib.rs` 移除了
  `foundation::error` 与 `llm_core` 的 `pub(crate)` alias
- 在 `pre-split src/lib.rs` 增加了显式标识
  `ROOT-NO-LOWLEVEL-ALIASES`
- crate 内部所有原先依赖 `crate::Result`、`crate::DittoError`、
  `crate::StreamResult`、`crate::LayeredLanguageModel`、`crate::collect_stream`
  的调用点，全部收口到真实 owner 路径：
  - `crate::foundation::error::*`
  - `crate::llm_core::*`

结果：

- crate root 不再对内保留低层 owner 的第二入口
- 内部依赖图与公开命名空间重新一致，不再依赖隐藏 facade
- 这轮修复没有新增任何兼容层、prelude 或别名包装

### 本轮暂不处理的意见

`contracts` / `types` 这条意见是成立的，但它已经是单独的 payload 分层迁移议题；
这轮不适合为了去掉 root alias 顺手把整层 DTO 所有权一起重排。

`runtime/model_builders.rs` 的 singleton lookup 这条也没有在本轮继续硬拆。
当前 `runtime` 仍然就是 builtin runtime 的装配 frontdoor，这条更像下一轮
builder injection / registry ownership 的设计议题，而不是本轮 owner-path 收口。

### 校验

本轮完成后已通过：

- `cargo fmt --check --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml`
- `cargo check --all-targets --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml`
- `cargo test --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml --test module_namespace_contract --test default_core_contract`

## 本轮推进：统一 runtime 的 builtin 装配视图（2026-03-11）

继续对照 DSL 的 L0 目标时，`runtime` 还有一处结构噪音：

- `resolver`
- `explain`
- `transport`
- `model_builders`

这些 runtime 入口虽然职责已经对，但内部仍然分散地直接调用：

- `crate::catalog::builtin_registry()`
- `builtin_runtime_registry_catalog()`

这会让 builtin 装配状态的进入点在 `runtime` 内部继续散开，等于把
“builtin global lookup” 当成隐式依赖，而不是让 `runtime` 自己显式拥有
builtin 装配视图。

### 调整

- 新增 [crates/ditto-core/src/runtime/builtin.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/runtime/builtin.rs)
- 增加显式标识 `RUNTIME-BUILTIN-ASSEMBLY`
- `BuiltinRuntimeAssembly` 统一持有：
  - `CatalogRegistry`
  - `BuiltinRuntimeRegistryCatalog`
- [crates/ditto-core/src/runtime/resolver.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/runtime/resolver.rs)
- [crates/ditto-core/src/runtime/explain.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/runtime/explain.rs)
- [crates/ditto-core/src/runtime/transport.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/runtime/transport.rs)
- [crates/ditto-core/src/runtime/model_builders.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/runtime/model_builders.rs)
  这几个 builtin 入口全部改为先进入这层 assembly 视图，再取 catalog / registry
  做解析与装配
- [crates/ditto-core/src/runtime_registry/catalog.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/runtime_registry/catalog.rs)
  新增 `BuiltinRuntimeRegistryCatalog::from_registry(...)`
  让 `runtime_registry` 的 builtin 视图可以从显式 catalog 派生，而不是只能自己回头再摸一次全局 builtin catalog

### 结果

- `runtime` 对 builtin 状态的依赖收敛成单一内部入口
- `runtime_registry` 继续只是 runtime 依赖的 registry 视图，不再表现成四处散落的隐式全局 helper
- `model_builders` 里 OpenAI surface resolver 和 OpenAI-compatible behavior resolver
  也都改成从同一个 builtin assembly 视图取数据
- 公开 API 没有变化，这轮只处理 L0 内部装配路径

### 为什么这样做，而不是继续做更重的 injection

这轮没有把 `runtime` 彻底改造成外部传入 registry 实例的对象系统。

原因：

- DSL 当前要求的是 L0 owner 清晰、装配入口收口、成本可预测
- 这里的核心问题是“builtin lookup 分散”，不是“必须面向接口做可替换 runtime 实例”
- 先收成统一 assembly 视图，已经能把 runtime 的 builtin 依赖从隐式散点收成显式单点
- 如果未来真的需要多 registry / plugin runtime，再在这层视图之上扩成实例化 runtime，会比现在直接硬上注入更干净

### 校验

本轮完成后已通过：

- `cargo fmt --check --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml`
- `cargo check --all-targets --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml`
- `cargo test --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml --test runtime_route_contract --test runtime_transport_contract --test runtime_explain_contract --test runtime_registry_contract --test catalog_resolver_contract --test module_namespace_contract --test default_core_contract`

## 本轮推进：把 outcome contract 从 `types` 收口到 `contracts`（2026-03-11）

继续对照 DSL 里的 L0 owner 定义时，`contracts` 和 `types` 之间还有一块非常明确的
边界漂移：

- `Usage`
- `FinishReason`
- `Warning`

这些类型虽然长期放在 `src/types/llm.rs`，但它们的语义不是某个 wire payload 的 owner；
它们是 provider、gateway、sdk、stream collector 共同消费的统一结果契约，更符合
DSL 里 `contracts` 的定位。

### 调整

- 新增 [crates/ditto-core/src/contracts/outcome.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/contracts/outcome.rs)
- 新增 [crates/ditto-core/src/contracts/tool_call.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/contracts/tool_call.rs)
- 增加显式标识 `CONTRACTS-OUTCOME-OWNER`
- [crates/ditto-core/src/contracts/mod.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/contracts/mod.rs)
  现在公开：
  - `FinishReason`
  - `Usage`
  - `Warning`
  - `parse_tool_call_arguments_json_or_string`（crate 内部）
- [src/types/mod.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/types/mod.rs)
  增加显式标识 `TYPES-PAYLOAD-ONLY`
- `pre-split src/types/llm.rs`
  不再定义这三类 outcome contract，只引用 `crate::contracts::*`
- 删除 `pre-split src/types/tool_call.rs`
- provider / gateway / sdk / llm_core / tests / examples 内部依赖全部改成显式 owner 路径：
  - `crate::contracts::FinishReason`
  - `crate::contracts::Usage`
  - `crate::contracts::Warning`

### 结果

- `types` 更接近纯 payload DTO owner
- `contracts` 真正开始拥有跨 surface 的统一结果语义，而不是只拥有 route / endpoint / provider metadata
- tool call 参数解析 helper 也不再挂在 `types` 下，避免 DTO 层继续承载行为逻辑
- 这轮没有搬动 `ContentPart` / `Tool` / `ToolChoice`，因为它们仍然更接近 request/response payload 本身

### 校验

本轮完成后已通过：

- `cargo fmt --check --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml`
- `cargo check --all-targets --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml`
- `cargo test --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml --test runtime_route_contract --test runtime_transport_contract --test runtime_explain_contract --test runtime_registry_contract --test catalog_resolver_contract --test module_namespace_contract --test default_core_contract`

## 本轮推进：按 Omne reviewer 收紧 LLM call contract / provider_options owner（2026-03-11）

这轮先用 Omne reviewer 重新审了一次当前 L0 实现，再按可接受意见做结构修正。

评审线程：

- thread: `a50f5a51-3966-41f2-92bd-8ed4adc7b718`
- model: `gemini-3.1-pro-preview`
- provider: `google.providers.yunwu`
- mode: `reviewer`
- sandbox: `read_only`

Omne 返回：

- `Blocking findings: 2.`
- High:
  - `llm_core` 直接依赖 `types::{GenerateRequest, GenerateResponse, StreamChunk}`
    ，绕过了 `contracts`
- Medium:
  - `GenerateRequest` 自身承载 `provider_options` merge / parse / bucket 选择逻辑
- Low:
  - `capabilities` root namespace 容易被误读成 L0 owner
- Low:
  - `provider_transport` / `session_transport` 目前仍偏薄，缺少更强的 transport policy 契约

### 接受并落地的部分

#### 1. 把 LLM 原子调用契约提升到 `contracts`

- 新增 [crates/ditto-core/src/contracts/llm.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/contracts/llm.rs)
- 增加显式标识 `LLM-CONTRACT-OWNER`
- `Role` / `Message` / `ContentPart` / `Tool` / `ToolChoice`
- `GenerateRequest` / `GenerateResponse` / `StreamChunk`
  现在都以 `crate::contracts::*` 为 canonical owner
- [crates/ditto-core/src/contracts/mod.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/contracts/mod.rs)
  明确公开上述 LLM call contract
- `pre-split src/types/llm.rs`
  改成非 owning 的聚合入口，并增加显式标识 `TYPES-LLM-AGGREGATE`
- [crates/ditto-core/src/llm_core/model.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/llm_core/model.rs)
- [crates/ditto-core/src/llm_core/layer.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/llm_core/layer.rs)
- [crates/ditto-core/src/llm_core/stream.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/llm_core/stream.rs)
- [crates/ditto-core/src/capabilities/text.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/capabilities/text.rs)
- [crates/ditto-core/src/session_transport/streaming.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/session_transport/streaming.rs)
- [crates/ditto-core/src/object/core.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/object/core.rs)
  这几处核心调用路径已改成显式依赖 `crate::contracts::*`

#### 2. 把 request-scoped provider passthrough 行为收回 `provider_options`

- 新增 [crates/ditto-core/src/provider_options/request.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/provider_options/request.rs)
- 增加显式标识 `PROVIDER-OPTIONS-REQUEST-OWNER`
- 新公开函数：
  - `request_with_provider_options`
  - `request_with_provider_response_format`
  - `request_provider_options_for`
  - `request_provider_options_value_for`
  - `request_parsed_provider_options_for`
  - `request_parsed_provider_options`
- [crates/ditto-core/src/provider_options/mod.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/provider_options/mod.rs)
  现在由 `provider_options` 显式对外暴露这些 helper
- `GenerateRequest` 本体不再携带 provider passthrough merge / parse 逻辑，只保留数据字段
- [crates/ditto-core/src/providers/anthropic/messages_api.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/providers/anthropic/messages_api.rs)
- [crates/ditto-core/src/providers/bedrock/messages_api.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/providers/bedrock/messages_api.rs)
- [crates/ditto-core/src/providers/bedrock/client.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/providers/bedrock/client.rs)
- [crates/ditto-core/src/providers/cohere/chat_api.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/providers/cohere/chat_api.rs)
- [crates/ditto-core/src/providers/google/generate_api.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/providers/google/generate_api.rs)
- [crates/ditto-core/src/providers/openai/chat_completions.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/providers/openai/chat_completions.rs)
- [crates/ditto-core/src/providers/openai/completions_legacy.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/providers/openai/completions_legacy.rs)
- [crates/ditto-core/src/providers/openai/responses.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/providers/openai/responses.rs)
- [crates/ditto-core/src/providers/openai_compatible/chat_completions.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/providers/openai_compatible/chat_completions.rs)
- [crates/ditto-core/src/providers/vertex.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/providers/vertex.rs)
- [crates/ditto-server/src/gateway/application/translation/mod.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-server/src/gateway/application/translation/mod.rs)
- [examples/cache_bench_deepseek_direct.rs](/root/autodl-tmp/zjj/p/ditto-llm/examples/cache_bench_deepseek_direct.rs)
  这些调用点都已改成显式使用 `crate::provider_options::*`

#### 3. 只对 `capabilities` 做角色澄清，不把它误报成 L0 owner

这条 low finding 没有按“直接删除 `capabilities`”处理。

原因：

- 现在 `capabilities` 仍然承担 northbound convenience surface
- 直接移除会把本轮从 L0 owner 收口，扩大成上层公开 facade 的大规模删改

这轮做的是把语义说清楚，避免它继续被看成 L0 owner：

- [crates/ditto-core/src/capabilities/mod.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/capabilities/mod.rs)
  增加显式标识 `CAPABILITIES-FACADE-NOT-L0-OWNER`
- [tests/module_namespace_contract.rs](/root/autodl-tmp/zjj/p/ditto-llm/tests/module_namespace_contract.rs)
  拆成两类断言：
  - `direct_l0_namespaces_expose_low_level_owners`
  - `northbound_capability_facades_remain_available`

### 本轮明确延后的部分

Omne 关于 `provider_transport` / `session_transport` “仍偏薄、缺少更强 transport policy 契约”
这条 low finding，这轮没有硬改。

原因：

- 这是 transport policy 设计议题，不是简单 owner-path 或 facade-path 问题
- 如果这轮强行处理，很容易把范围扩大到连接池策略、幂等键策略、retry policy
  契约设计，而不是继续推进当前 L0 owner 收口
- 当前更适合先把 owner 归位，再单独评估是否需要新增 machine-readable transport policy contract

### 校验

本轮完成后已通过：

- `cargo fmt --check --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml`
- `cargo check --all-targets --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml`
- `cargo test --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml --test runtime_route_contract --test runtime_transport_contract --test runtime_explain_contract --test runtime_registry_contract --test catalog_resolver_contract --test module_namespace_contract --test default_core_contract`

额外观察到一条当前库内单测失败：

- `cargo test --lib --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml`
  - failing test:
    `config::routing_policy::tests::weighted_selection_returns_primary_then_fallbacks`
  - 失败原因为 `openai-primary` 缺少 `base_url` 导致 runtime route resolution 报错

这条失败和本轮 `contracts/provider_options/capabilities` owner 收口没有直接耦合，因此未在这轮顺手扩大处理。

## 本轮推进：把 `provider_transport` / `session_transport` 补成显式 policy owner（2026-03-11）

前面 Omne 已经指出一条仍然成立但当时被延后的 low finding：

- `provider_transport` / `session_transport` 已经作为根边界存在
- 但内部语义仍主要停留在函数和常量上，缺少 machine-readable transport policy contract

这轮继续推进 L0 时，就把这块补成显式 policy owner，但不扩大到连接池/重试实现重写。

### 调整

#### 1. `provider_transport` 增加显式 HTTP policy owner

- 新增 [crates/ditto-core/src/provider_transport/policy.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/provider_transport/policy.rs)
- 增加显式标识 `PROVIDER-TRANSPORT-POLICY-OWNER`
- 新增公开类型：
  - `HttpClientPolicy`
  - `HttpResponseBodyPolicy`
  - `HttpTransportPolicy`
- 默认语义现在被固定为可序列化 policy，而不是散在 helper 内部的 magic constant：
  - timeout: `300_000ms`
  - max_error_body_bytes: `64KiB`
  - max_response_body_bytes: `16MiB`
- [crates/ditto-core/src/provider_transport/config.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/provider_transport/config.rs)
  新增 `*_with_policy(...)` 入口，让 client/materialization 逻辑可显式落在 policy 上
- [crates/ditto-core/src/provider_transport/http.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/provider_transport/http.rs)
  新增 `send_checked_with_policy` / `send_checked_json_with_policy` /
  `send_checked_bytes_with_policy`

结果：

- `provider_transport` 不再只是“有几个 helper”
- 它开始真正拥有 provider-facing HTTP transport 的默认策略语义

#### 2. `session_transport` 增加显式 session policy / websocket rewrite owner

- 新增 [crates/ditto-core/src/session_transport/policy.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/session_transport/policy.rs)
- 增加显式标识 `SESSION-TRANSPORT-POLICY-OWNER`
- 新增公开类型：
  - `SessionTransportPolicy`
  - `WebsocketBaseUrlResolution`
  - `WebsocketBaseUrlRewrite`
- [crates/ditto-core/src/session_transport/sse.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/session_transport/sse.rs)
  的 `SseLimits` 现在也成为可序列化的显式 policy 组成部分
- websocket base URL 重写不再只是字符串函数，改成
  `resolve_websocket_base_url(...) -> WebsocketBaseUrlResolution`

结果：

- `session_transport` 现在显式拥有：
  - SSE frame/line/event 限制
  - websocket base URL rewrite 结果语义
- session/frame 语义可以被外部测试和上层逻辑直接检查，不必再靠阅读内部 helper 实现

#### 3. `runtime` 改成消费 `session_transport` 的 rewrite owner

- [crates/ditto-core/src/runtime/route.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/runtime/route.rs)
  不再自己私有定义 websocket base URL rewrite 判定
- 现在直接消费 `session_transport::resolve_websocket_base_url(...)`
- [crates/ditto-core/src/runtime/transport.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/runtime/transport.rs)
  继续把这个 owner 语义映射成 `RuntimeTransportBaseUrlRewrite`

结果：

- websocket rewrite 语义的真正 owner 回到 `session_transport`
- `runtime` 只做 transport explain / plan 映射，不再重复保留一份判定逻辑

### 合同测试

- 新增 [tests/transport_policy_contract.rs](/root/autodl-tmp/zjj/p/ditto-llm/tests/transport_policy_contract.rs)
  固定：
  - `HttpTransportPolicy::default()` 的默认值
  - `SessionTransportPolicy::default()`
  - `resolve_websocket_base_url(...)` 的 rewrite 结果

### 校验

本轮完成后已通过：

- `cargo fmt --check --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml`
- `cargo check --all-targets --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml`
- `cargo test --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml --test runtime_transport_contract --test transport_policy_contract --test module_namespace_contract`

## 追加复核：Omne / Yunwu Gemini 3.1（thread `81cf32e6-893a-41ef-ab1f-fa209f00d9ca`）

这轮继续通过 `omne-agent` 发起复核，模型仍为：

- provider: `google.providers.yunwu`
- model: `gemini-3.1-pro-preview`
- base_url: `https://yunwu.ai/v1beta`

Omne 返回 `Blocking findings: 2`，其中当前最应该立刻收口的一条是：

1. `src/types/llm.rs` 只是对 `src/contracts` 里 canonical LLM contracts 的纯转发，导致 `contracts::*` 和 `types::*` 同时公开同一组 L0 对象。

同一轮 review 里，模型还补充了另外三条观察：

- `capabilities` 仍然是 northbound facade，和 L0 core crate 结构混在一起
- `foundation` 还没有 DSL 里承诺的 low-allocation buffer / memory-pool 层
- `catalog::builtin_registry()` 与 `runtime_registry::builtin_runtime_registry()` 的 builtin 入口仍然双轨并存

### 本轮采纳的修复

只采纳并落地了第一条阻塞项，因为它是明确的结构重复，而且不会引入兼容层。

#### 1. 移除 `types` 对 canonical LLM contracts 的重复公开路径

调整：

- 删除 `pre-split src/types/llm.rs`
- [src/types/mod.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/types/mod.rs) 不再 `mod llm;`，也不再 re-export：
  - `GenerateRequest`
  - `GenerateResponse`
  - `Message`
  - `ContentPart`
  - `Role`
  - `Tool`
  - `ToolChoice`
  - `StreamChunk`
  - `ImageSource`
  - `FileSource`
- `types` 模块注释改成显式声明：
  - 它只拥有 modality-specific payload DTO
  - canonical LLM call contracts 一律归 `contracts`

结果：

- `ditto_llm::types::{GenerateRequest, ...}` 这条重复公开路径被彻底移除
- `ditto_llm::contracts::{GenerateRequest, ...}` 成为唯一 northbound canonical owner
- 仓库内部也同步迁移到 `crate::contracts::*`，没有留下“只对外删除、对内继续 alias”的兼容面

#### 2. 同步迁移内部与测试/示例调用点

这轮顺带把仍然依赖旧路径的调用点一起收口：

- agent 层改为直接依赖 `crate::contracts::*`
- openai/openai-compatible/google 相关 provider 入口改为直接依赖 `crate::contracts::*`
- 示例、集成测试、gateway translation 支撑测试改为直接依赖 `ditto_llm::contracts::*`
- `GenerateRequestSupport` 保持留在 `types` 的隐藏辅助层，不错误提升到 `contracts`

这样处理后，`types` 只继续承载 image/audio/batch/moderation/rerank/video 这些 payload DTO；LLM request/response/stream/message/tool 语义完全回到 `contracts`。

### 本轮未继续处理的点

这轮没有继续动：

- `capabilities` 作为 L1 facade 的进一步拆分
- `foundation` 的 buffer / pool owner
- `catalog` 与 `runtime_registry` 的 builtin 入口收敛

原因很简单：这三条都属于新的结构设计轮次，不适合和“去掉重复公开路径”混在一个补丁里一起改。

### 本轮校验

本轮修复后已通过：

- `cargo fmt --check --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml`
- `cargo check --all-targets --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml`
- `cargo test --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml --test module_namespace_contract --test default_core_contract --test runtime_transport_contract --test transport_policy_contract`
- `cargo test --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml --features agent --test agent_loop`
- `cargo test --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml --features 'google streaming openai-compatible' --test google_stream_http_header_auth --test google_stream_empty_response_fallback`

## 追加修补：收口 OpenAI provider 的隐式环境开关

后续复核又指出一条值得立即收口的实现风险：

- `crates/ditto-core/src/providers/openai/client.rs` 曾直接读取进程环境变量
  `DITTO_OPENAI_RESPONSES_SEND_TOOL_CALL_THOUGHT_SIGNATURE` /
  `OMNE_OPENAI_RESPONSES_SEND_TOOL_CALL_THOUGHT_SIGNATURE`
  来决定是否向上游透传 tool-call thought signature

这会绕过：

- `ProviderConfig`
- `Env`
- runtime-derived 显式 plan

也就是说，同一份配置在不同进程环境下可能得出不同 provider 行为，这不符合 L0 想要的 machine-first / predictable-cost 方向。

### 本轮采纳的修复

调整：

- [crates/ditto-core/src/providers/openai/client.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/providers/openai/client.rs)
  新增显式 override 字段：
  - `tool_call_thought_signature_passthrough: Option<bool>`
- 新增 builder：
  - `OpenAI::with_tool_call_thought_signature_passthrough(bool)`
- `OpenAI::from_config(...)` 现在从
  `ProviderConfig.openai_compatible.send_tool_call_thought_signature`
  读取显式 override
- 删除基于 `std::env::var(...)` 的 provider-side env flag 读取
- `should_send_function_call_thought_signature(...)` 的语义改成：
  - 先看显式 override
  - 未显式配置时，再退回到由显式 `base_url/model` 推导出的兼容 heuristic

这次处理后：

- provider 层不再直接摸 ambient process env
- 同一份 `ProviderConfig + Env` 会稳定导出同一份 OpenAI request quirk 语义
- 兼容 LitellM + Gemini 的必要 heuristic 仍然保留，但它现在只依赖显式输入

### 顺手修补的测试残留

在带 `openai` feature 跑单测时，还暴露了另一处与前面 L0 收敛一致的残留：

- [crates/ditto-core/src/providers/openai/raw_responses.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/providers/openai/raw_responses.rs)
  的测试模块还在依赖已经移除的 `crate::types::*` LLM contract 路径

本轮已一起改为直接依赖 `crate::contracts::*`。

### 本轮校验

本轮补丁后已通过：

- `cargo fmt --check --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml`
- `cargo check --all-targets --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml`
- `cargo test --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml --features openai --lib thought_signature_passthrough`
- `cargo test --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml --features openai --lib passthrough_override`

## 继续推进：把 runtime 的 builder 装配协议抽成单独 owner

后续架构复核里最重要的一条中长期风险，是 `runtime` 正在逐渐变成唯一的大装配中心。

这次没有直接去重写 `runtime/model_builders.rs` 里的 provider/capability match，而是先做更稳的一步：

- 把“provider + config + capability -> builder backend/config” 这条装配协议从
  [crates/ditto-core/src/runtime/model_builders.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/runtime/model_builders.rs)
  里拆出来，单独收口成新的 owner

### 本轮采纳的修复

新增：

- [crates/ditto-core/src/runtime/builder_protocol.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/runtime/builder_protocol.rs)

这个模块现在显式拥有：

- `configured_default_model(...)`
- `BuilderAssemblyRequest`
- `BuilderAssemblyPlan`
- `default_builder_assembly(...)`
- `resolve_builder_assembly(...)`

并加了显式标识：

- `RUNTIME-BUILDER-ASSEMBLY-PROTOCOL`

这意味着：

- provider/config/capability 到 builder backend/runtime config 的解析语义，有了单独 owner
- `model_builders` 只消费装配计划，再做 adapter 实例化
- route 解析、catalog/runtime_registry 回退、builder provider 选择，不再继续散在一个超大文件内部 helper 里

### 这次结构收敛的效果

调整后：

- [crates/ditto-core/src/runtime/model_builders.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/runtime/model_builders.rs)
  从 `1039` 行降到 `850` 行
- 协议相关测试也一起迁到
  [crates/ditto-core/src/runtime/builder_protocol.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/runtime/builder_protocol.rs)
  的 owner 位置

这还不是最终形态，但已经把 `runtime` 内部的两类职责分开了：

- `builder_protocol` 负责装配计划解析
- `model_builders` 负责把装配计划映射成具体 adapter

### 本轮校验

本轮补丁后已通过：

- `cargo fmt --check --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml`
- `cargo check --all-targets --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml`
- `cargo test --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml --lib builder_assembly`

## 继续推进：context cache 不再在 runtime frontdoor/backends 硬编码 provider 名字

本轮又用 `omne-agent` 补做了一次 L0 结构复核。最终跑通的配置是：

- `omne-root`: `/root/autodl-tmp/zjj/p/omne-agent/.omne_data`
- provider: `google.providers.yunwu`
- model: `gemini-3.1-pro-preview`
- base_url: `https://yunwu.ai/v1beta`
- thread: `edacceb9-fd04-4368-aa6c-48285f845623`
- turn: `c9c3cd88-d425-42be-91f5-58e9a8eebb62`

这轮 review 给出的 3 条意见里，真正应该立即采纳的是第一条：

1. `runtime/builder_protocol.rs` 和 `runtime/builder_backends.rs` 不应该继续靠
   `"minimax"` / `"deepseek"` 这类 provider 名字硬编码来维持 context cache 路径
2. `build_language_model()` 里捕获 `builtin_runtime_assembly()` 的动态行为 resolver
   有 owner leakage 风险
3. `BuilderAssemblyPlan` 里通过 clone 整个 `ProviderConfig` 再覆写字段，所有权语义偏粗

### 本轮采纳的修复

这轮只采纳第 1 条。

原因很直接：

- 第 1 条是成立的结构性泄漏，而且修复范围清晰
- 第 2 条如果照单全收，会把前面刚修好的“按 request.model 动态解析
  `tool_choice_required` / `reasoning_content`”重新退化回 build-time 冻结
- 第 3 条有道理，但属于下一轮把 `BuilderAssemblyPlan` 细化成
  `base config + route overrides` 的重构，不适合在这一轮硬插进去

### 这次实际落地的变化

#### 1. `context cache` assembly 不再做 provider-name fallback

调整：

- [crates/ditto-core/src/runtime/builder_protocol.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/runtime/builder_protocol.rs)
  的 `resolve_context_cache_assembly(...)`
  不再包含 `"minimax" -> "openai-compatible"` 这类硬编码 fallback
- 现在这条路径只保留：
  - 解析 catalog owner
  - 补 `default_base_url`
  - 校验 `CapabilityKind::CONTEXT_CACHE`

这意味着：

- `builder_protocol` 不再代持 provider-specific context cache backend 语义
- context cache frontdoor 回到“只做装配，不藏 provider 特判”

#### 2. `runtime_registry` 现在拥有 context cache profile 的静态解析

新增：

- [crates/ditto-core/src/runtime_registry/catalog.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/runtime_registry/catalog.rs)
  里的 `resolve_context_cache_profile(...)`

并加了显式标识：

- `RUNTIME-CONTEXT-CACHE-PROFILE-RESOLUTION`

这个 owner 现在负责：

- 从 catalog `behavior.context_cache_modes` 提取静态 cache mode
- 从 `OperationKind::CONTEXT_CACHE` 的 binding 里补充 transport-derived mode
- 对带 `anthropic.messages` binding 的 provider 自动推导
  `ContextCacheMode::AnthropicCompatible`
- 对存在 context cache binding 的 provider 补出 `ContextCacheMode::Passive`

结果是：

- DeepSeek 的 `Passive` profile 来自 catalog behavior
- MiniMax 的 `Passive + AnthropicCompatible` 来自 catalog binding 事实
- runtime frontdoor/backends 不再按 provider 名字解释这些模式

#### 3. `builder_backends` 改成包装 registry-derived profile，而不是构造 provider client

调整：

- [crates/ditto-core/src/runtime/builder_backends.rs](/root/autodl-tmp/zjj/p/ditto-llm/crates/ditto-core/src/runtime/builder_backends.rs)
  的 `build_context_cache_model(...)`
  不再实例化 `OpenAICompatible`
- 原先的 provider-name match 被删掉，改成：
  - 从 `plan.config.default_model` 取最终 model id
  - 调 `runtime_registry.resolve_context_cache_profile(...)`
  - 包装成通用的 `CatalogContextCacheAdapter`

这样处理后：

- `context cache` 这条构建路径不再依赖 openai-compatible provider family heuristics
- builder backend 只消费 registry 导出的静态 profile，不再自己解释 provider 行为
- `provider()` / `model_id()` 仍保持最终外部语义：例如 `deepseek`、`minimax`

### 本轮校验

本轮补丁后已通过：

- `cargo fmt --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml`
- `cargo test --manifest-path /root/autodl-tmp/zjj/p/ditto-llm/Cargo.toml --features 'gateway-translation provider-deepseek provider-minimax openai-compatible' --test deepseek_provider_capabilities --test minimax_provider_capabilities`

本轮未继续用 `--lib` / `--all-targets` 做全量 gate 的原因也记录一下：

- 当前工作树里存在与本轮无关的未收口改动，`lib test` 会撞上已有的
  `config_editing.rs` / `gateway/config.rs` 编译错误
- 这些错误不在本轮修改文件里，也不是本轮引入

### 这轮之后的判断

这次收口后，context cache 这条路径终于比较像 L0 该有的样子了：

- `runtime/builder_protocol` 只做装配协议
- `runtime_registry` 拥有静态 profile 解释权
- `builder_backends` 只把静态 profile 包装成能力对象

也就是说，之前那个“战术性可用但带 provider-name 特判”的补丁，现在已经进一步收成了
真正的 owner-local 实现，而不是继续让 runtime 长字符串分支。
