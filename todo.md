# ditto-llm 重构推进 TODO

## 我们在做什么

我们正在把 `ditto-llm` 收敛到一个更清晰、可维护、可扩展的方向：

- 默认只提供 **OpenAI-compatible 的 LLM 能力**，作为最小核心。
- `openai`、`deepseek`、`gemini`、`claude` 等都作为 **可选 provider 包** 接入，而不是默认内建。
- 所有 provider 都必须按 **能力类别** 拆分，而不是继续把协议、provider、模型、能力和运行时实现混在一起。

## 我们要做成什么样

我们要把 `ditto-llm` 做成一个具有清晰分层的 Rust 项目：

- 默认构建只包含最小核心：`openai-compatible + llm`。
- provider 是可插拔的，capability 是可组合的。
- catalog 不是参考资料，而是运行时能力与模型支持的单一事实来源。
- 运行时根据 provider + capability 的强类型描述来构建，而不是依赖字符串判断、模型前缀猜测和 scattered `if/else`。

## 为什么要这样做

因为现在最容易失控的不是功能数量，而是抽象边界。我们必须用 **优秀且有好品味的抽象** 来控制复杂度：

- 保持整体复杂度合理。
- 避免重复代码和分支漂移。
- 避免 catalog、feature、runtime adapter 三套真相互相打架。
- 让新增 provider 和新增 capability 成为“增量接入”，而不是“继续堆硬编码”。

## 推进清单

### Phase 0：冻结方向与边界

- [x] 01. 写一份架构决议，明确默认核心是 `openai-compatible + llm`，其余 provider 全部为 opt-in。
- [x] 02. 在架构决议中明确区分 `protocol surface`、`provider`、`capability`、`catalog source` 四个概念。
- [x] 03. 明确废弃“默认启用官方 OpenAI provider”的当前产品语义。
- [x] 04. 明确默认交付目标不包含 GUI，不把 `apps/*` 视为核心运行时的一部分。
- [x] 05. 定义迁移不变量：已有配置如何兼容、哪些 feature 改名、哪些默认行为改变。
- [x] 06. 定义默认核心必须提供的最小体验：文本生成、流式输出、基础工具调用策略。
- [x] 07. 明确默认核心暂不内建 embeddings、image、audio、moderation、rerank、batch、realtime。
- [x] 08. 明确所有 provider 未来都必须按能力类别声明，不允许再以“一个 provider = 一坨模糊支持”存在。
- [x] 09. 明确 catalog 中的 provider 和运行时 provider 必须一一对应，禁止“文档有、实现无”的长期悬空状态。
- [x] 10. 把这份方向文档作为后续所有重构 PR 的判定基准。
- [x] 10.1 新增模块边界文档，明确 `core/capabilities/config/catalog/providers/runtime/gateway` 的目录收敛目标与单向依赖规则。
- [x] 10.2 引入 `crate::config` 与 `crate::runtime` facade，作为从 `profile` / `gateway` 迁移出来的稳定入口。
- [x] 10.3 将 `OpenAiProviderFamily` / `infer_openai_provider_quirks` 从 legacy `profile` 命名空间下沉到 `providers` 共享层，`profile` 仅保留兼容 re-export。
- [x] 10.4 把 `FileClient` 的 provider 具体实现从 `src/file.rs` 下沉回 provider 层，消除 capability facade 对 `OpenAI` / `OpenAICompatible` 的直接依赖。
- [x] 10.5 修复 `cargo check --features agent` 的真实失效路径，并补齐 OpenAI-family 共享 gate，恢复 advertised feature 的可编译性。

### Phase 1：重做 feature 体系

- [x] 11. 设计统一的 feature 命名规范：`provider-*` 表示 provider，`cap-*` 表示能力。
- [x] 12. 重新定义默认 feature，仅保留 `provider-openai-compatible` 与 `cap-llm` 所需依赖。
- [x] 13. 设计 `cap-streaming` 是否并入 `cap-llm` 的规则，并固定下来，不允许反复摇摆。
- [x] 14. 把现有 `openai-compatible` feature 迁移为 `provider-openai-compatible`。
- [x] 15. 把现有 `openai` feature 迁移为 `provider-openai`，明确它表示官方 OpenAI provider。
- [x] 16. 把现有 `anthropic` feature 迁移为 `provider-anthropic`，明确它表示 Claude / Anthropic provider。
- [x] 17. 把现有 `google` feature 迁移为 `provider-google`，明确它表示 Gemini / Google provider。
- [x] 18. 把 `provider-deepseek` 明确定义为依赖 `provider-openai-compatible` 的 provider 包，而不是默认核心的一部分。
- [x] 19. 定义所有能力 feature：`cap-llm`、`cap-embedding`、`cap-image-generation`、`cap-image-edit`、`cap-audio-transcription`、`cap-audio-speech`、`cap-moderation`、`cap-rerank`、`cap-batch`、`cap-realtime`。
- [x] 20. 为 feature 体系写清楚非法组合与推荐组合，避免用户继续靠猜。

### Phase 2：重做核心数据结构与抽象边界

- [x] 21. 新建 `ProviderId` 强类型，替换运行时到处传裸字符串的做法。
- [x] 22. 新建 `CapabilityKind` 强类型枚举，作为能力分类的唯一真相。
- [x] 23. 新建 `ProviderCapabilitySet`，明确一个 provider 具备哪些能力。
- [x] 24. 新建 `ProviderRuntimeSpec`，包含 provider id、协议族、默认 base_url、auth 方式、能力集合。
- [x] 25. 新建 `ProviderCapabilityBinding`，描述“某 provider 的某能力由哪个 adapter 构建器负责实现”。
- [x] 26. 新建 `ModelCapabilityDescriptor`，描述具体模型支持的能力子集，而不是只保留模糊的 surface 信息。
- [x] 27. 明确 `provider` 级能力和 `model` 级能力的关系，避免 runtime 把两者混用。
- [x] 28. 把 `ProviderConfig.capabilities` 从“宽松提示”提升为“受 catalog 约束的显式声明”。
- [x] 29. 把所有 provider / capability 相关错误收敛为命名错误类型，而不是继续泛化成 `InvalidResponse`。
- [x] 30. 清理所有与 provider/capability 相关的裸元组返回值，统一改为命名结构体。

### Phase 3：让 catalog 成为运行时单一事实来源

- [x] 31. 统一 `catalog/provider_models/*.json` 与 `catalog/provider_models/*.toml` 的 schema，保证 provider 与 capability 描述一致。
- [x] 32. 明确 `catalog/provider_models/openai.*` 表示官方 OpenAI provider 的完整能力与模型目录。
- [x] 33. 明确 `catalog/provider_models/deepseek.*` 表示 DeepSeek provider 的完整能力与模型目录。
- [x] 34. 明确 `catalog/provider_models/google.*` 表示 Gemini / Google provider 的完整能力与模型目录。
- [x] 35. 明确 `catalog/provider_models/anthropic.*` 表示 Claude / Anthropic provider 的完整能力与模型目录。
- [x] 36. 为所有 model entry 增加稳定的 capability 分类字段，而不只依赖 `api_surfaces`。
- [x] 37. 为所有 provider entry 增加完整能力集合字段，保证 provider 级声明不是散落在别处。
- [x] 38. 为所有能力增加实现状态字段，区分 `implemented`、`planned`、`blocked`，但运行时只接受 `implemented`。
- [x] 39. 建立离线生成流程：从 `catalog/provider_models/*` 生成 `src/catalog/generated/*` 的 Rust 静态定义。
  - [x] 生成结果按 provider 拆分到 `src/catalog/generated/providers/*.rs` 与 `src/catalog/generated/providers/mod.rs`，不再把所有 provider 压进单一 `providers.rs`。
- [x] 40. 禁止运行时直接依赖 JSON/TOML 做能力判断，只允许读取 Rust 生成结果。

### Phase 4：重做 catalog registry 与 resolver

- [x] 41. 扩展 `ProviderPluginDescriptor`，让它显式暴露 provider 级 capability set。
- [x] 42. 扩展 `ProviderModelDescriptor`，让它显式暴露 model 级 capability set。
- [x] 43. 让 `CatalogRegistry` 能回答“provider 是否支持 capability”这个一等问题。
- [x] 44. 让 `CatalogRegistry` 能回答“provider + model 是否支持 capability”这个一等问题。
- [x] 45. 把当前只关注 route resolution 的 registry 扩展成 capability-aware registry。
- [x] 46. 让 `resolve_runtime_route()` 的输入包含 capability 约束，而不是只看 operation。
- [x] 47. 把 resolver 从“测试里能跑”提升为“主运行时必须经过的入口”。
- [x] 48. 为 resolver 建立 catalog/runtime 一致性测试，确保 route、protocol、capability 三者不漂移。
- [x] 49. 删除或废弃运行时里绕过 registry 的 provider 特例分支。
- [x] 50. 为所有 provider 生成最小 capability 摘要，供 CLI、配置编辑器与文档统一读取。

### Phase 5：先落默认核心 `openai-compatible + llm`

- [x] 51. 默认构建只保留 openai-compatible 的 LLM 主路径。
- [x] 52. 默认公开 API 只承诺 LLM 能力，不再对外暗示完整 provider 能力集合。
- [x] 53. 默认示例代码全部改为 openai-compatible llm，去掉默认 OpenAI 官方示例的中心位置。
- [x] 54. 默认环境变量命名和 README 指南都改成 generic openai-compatible 心智模型。
- [x] 55. 默认 `ProviderConfig` 模板只生成 openai-compatible llm 所需字段。
- [x] 56. 默认 `GET /models` / routing 示例优先展示 openai-compatible llm 路径。
- [x] 57. 默认 feature 的测试矩阵只围绕 `provider-openai-compatible + cap-llm` 展开，先做到稳如石头。
- [x] 58. 默认核心的所有类型、文档、错误信息中去掉“官方 OpenAI 即默认”的暗示。
- [x] 59. 默认核心严格禁止自动暴露 embeddings、image、audio、moderation、rerank、batch、realtime 入口。
- [x] 60. 为默认核心写一组端到端 contract tests，保证它是一个稳定而最小的基座。

### Phase 6：把官方 OpenAI provider 做成完整可选包

- [x] 61. 设计 `provider-openai + cap-llm` 的官方 OpenAI LLM 构建路径，并与默认核心彻底分离。
- [x] 62. 实现 `provider-openai + cap-embedding` 的官方 OpenAI embedding 构建路径。
- [x] 63. 实现 `provider-openai + cap-image-generation` 的官方 OpenAI image generation 构建路径。
- [x] 64. 补出 `cap-image-edit` 的独立抽象，并实现 `provider-openai + cap-image-edit`。
- [x] 65. 实现 `provider-openai + cap-audio-transcription` 的官方 OpenAI audio transcription 构建路径。
- [x] 66. 实现 `provider-openai + cap-audio-speech` 的官方 OpenAI speech 构建路径。
- [x] 67. 实现 `provider-openai + cap-moderation` 的官方 OpenAI moderation 构建路径。
- [x] 68. 实现 `provider-openai + cap-batch` 的官方 OpenAI batch 构建路径。
- [x] 69. 设计并实现 `provider-openai + cap-realtime` 的官方 OpenAI realtime 构建路径。
- [x] 70. 为官方 OpenAI provider 建立 capability completeness 测试，逐项对齐 `catalog/provider_models/openai.*`。

### Phase 7：把 DeepSeek、Claude、Gemini 做成能力分层的可选 provider

- [x] 71. 实现 `provider-deepseek + cap-llm`，明确它复用 openai-compatible protocol family。
- [x] 72. 为 DeepSeek 建立独立 capability spec，不再把 quirks 散落在字符串判断里。
- [x] 73. 实现 `provider-anthropic + cap-llm`，作为 Claude 的主路径。
- [x] 74. 为 Anthropic provider 补齐其 catalog 中已声明且优先级最高的非 LLM 能力接入计划。
- [x] 75. 实现 `provider-google + cap-llm`，作为 Gemini 的主路径。
- [x] 76. 实现 `provider-google + cap-embedding`，把 Gemini / Google embedding 能力纳入同一体系。
- [x] 77. 为 Google provider 设计 image / realtime 等能力的独立 capability 抽象接入口。
- [x] 78. 对 DeepSeek、Anthropic、Google 分别建立“provider 声明能力 = runtime 可查询能力”的一致性检查。
- [x] 79. 当某 provider 的某 capability 尚未实现时，运行时必须显式拒绝，不允许假装支持。
- [x] 80. 给这三个 provider 分别补一组 capability-gated 示例与文档，避免用户误配。

### Phase 8：统一 runtime 构建链与 translation/gateway 路径

- [x] 81. 统一所有 `build_*_model()` 入口，让它们先查 registry 再决定 adapter 构建。
- [x] 82. 让 `TranslationBackend` 按 capability binding 构建，而不是继续手写猜 provider 支持什么。
- [x] 83. 拆分 `TranslationBackend` 的职责，避免它继续吸纳所有 provider 与能力的初始化逻辑。
- [x] 84. 让 gateway translation 路径基于 capability set 决定挂载哪些 endpoint。
- [x] 85. 让 routing 先匹配 capability，再匹配 provider/model，而不是继续靠 operation 和特例混推。
- [x] 86. 在 gateway 层新增 capability gating，禁止把未实现能力暴露为可调用入口。
- [x] 87. 清理 provider runtime 中重复的 auth/base_url/model 解析逻辑，收敛到统一构建辅助层。
- [x] 88. 清理 provider runtime 中重复的 HTTP client 初始化逻辑，避免继续散落在各 adapter 中。
- [x] 89. 对 translation/gateway 主路径建立 capability-aware integration tests。
- [x] 90. 把现有 catalog resolver 真正接入主运行时，而不是继续只在测试里自娱自乐。

### Phase 9：配置、CLI、文档与非核心内容清理

- [x] 91. 重写 `ProviderConfig` 文档，明确它描述的是 provider node，而不是任意 base_url 片段。
- [x] 92. 在配置层引入显式 `provider` 与 `enabled_capabilities` 语义，并由 registry 校验。
- [x] 93. 让配置编辑器从 registry 读取 provider/capability 信息，不再手写 provider 特例。
- [x] 94. 重写 README 的 provider 章节，改成“默认核心 + 可选 provider packs + 可选 capability packs”。
- [x] 95. 新增一张 feature 对照表，覆盖 provider × capability × feature 名称 × 实现状态。
- [x] 96. 从默认 workspace/CI 流程中移除 `apps/*`，把 GUI 从核心交付面中剥离出去。
- [x] 97. 清理 docs、CHANGELOG、README 中把 `apps/admin-ui` 当核心能力的表述。
- [x] 98. 让 `cargo check`、`cargo clippy -D warnings`、feature matrix 成为新的结构演进门槛。
- [x] 99. 建立 catalog/runtime completeness dashboard，持续显示哪些 provider/capability/model 已完成、哪些未完成。
- [x] 100. 为 `ditto-llm/catalog` 中剩余所有 provider 制定逐项落地计划，并持续推进直到它们的全部模型与能力都在 runtime 中实现（已完成 `deepseek`/`minimax` 的 `context.cache` 切片、OpenAI native 的 `video.generation` 切片，以及 Google native 的 `image.generation`、`realtime`、`video.generation` 切片；provider-level runtime gap 已清零，后续见 `docs/src/roadmap/provider-runtime-rollout.md` 的 capability 公共化阶段）。

## 最终目标

我们的最终目标是：**`ditto-llm/catalog` 中定义的所有 provider，其所有能力与所有模型都在 `ditto-llm` 的运行时中得到实现**。

为了达到这个目标，整体设计必须始终保持 **优秀且有好品味的抽象**：

- 用清晰分层控制复杂度。
- 用强类型约束替代字符串猜测与散落分支。
- 用统一 registry 消灭重复代码。
- 用 capability-first 的方式组织 provider，实现真正可维护、可扩展、无重复的架构。


T0: 调用所有provider的能力
T1: 相似产品，能够互相转化。来提供proxy，优先对齐到openai实现。无网关。
T2：网关feature，server，类litellm
T3：参考ditto-llm/TODO.md
