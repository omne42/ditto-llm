# Rust Native Provider Catalog

## 目标

Ditto-LLM 的 provider / model / auth / routing 元数据，逐步从“运行时依赖 TOML/JSON 配置和参考 catalog”收敛为“纯 Rust 的强类型内建 registry”。

最终目标有四个：

- 默认发行版只内置“通用 OpenAI 类 API 支持”，不默认内置特定厂商模型清单。
- 所有特定 provider 都以可插拔 feature/plugin 的方式接入，而不是散落在运行时代码里的硬编码分支。
- provider 的鉴权方式不再假定只有 API key；OpenAI / Anthropic / Google 等都要能表达 OAuth 等登录能力。
- 运行时解析请求时，优先查强类型 Rust registry，而不是拼字符串、猜接口、按模型前缀写条件分支。

## 当前问题

当前代码库里已经有这些能力，但它们分散在不同地方：

- `src/profile/config.rs`：ProviderConfig / ProviderAuth / ProviderApi
- `src/profile/*_model_catalog.rs`：部分 provider 的模型目录
- `catalog/provider_models/*.toml`：更细的模型、接口、证据数据
- `src/providers/*`：实际请求与响应编解码逻辑
- `src/profile/config_editor.rs`：交互式配置与 provider/model 操作

问题在于这些层次还没有被彻底解耦：

- “要做什么”和“打哪个接口”有时混在一起。
- “接口路径”和“报文协议”有时混在一起。
- provider 特例还经常要在运行时代码里写 `if model.starts_with(...)`。
- 外部 catalog 目前更像参考资料，不是编译期强约束的运行时真相。
- 默认 features 打开了多个 provider，和“核心最小内建 + 可选插件”的方向不一致。

## 核心设计

本设计采用一套统一的 Rust 强类型抽象，作为运行时唯一可信的 provider catalog。

外部抓取到的 TOML/JSON/CSV/网页数据只作为参考输入，不作为 Ditto 运行时的主数据源。

### 三个核心概念

#### `OperationKind`

表示“业务动作”，即用户到底要做什么。

示例：

- `Chat`
- `Response`
- `Embedding`
- `ImageGeneration`
- `ImageEdit`
- `VideoGeneration`
- `AudioSpeech`
- `AudioTranscription`
- `RealtimeSession`
- `Rerank`
- `ClassificationOrExtraction`

它不关心具体 URL，也不关心报文格式。

#### `ApiSurfaceId`

表示“上游接口面”，即实际调用的是哪一组 endpoint 语义。

示例：

- `OpenAiChatCompletions`
- `OpenAiResponses`
- `AnthropicMessages`
- `GoogleGenerateContent`
- `GooglePredict`
- `GooglePredictLongRunning`
- `DashscopeCompatibleChatCompletions`
- `DashscopeWsInference`
- `DashscopeWsRealtime`
- `DashscopeAudioTtsCustomization`
- `QianfanChatCompletionsV2`
- `QianfanEmbeddingsV2`

它回答“打哪个接口面”。

#### `WireProtocol`

表示“报文协议 / 编解码方言”，即请求和响应应该按哪种 schema 去序列化和解析。

示例：

- `OpenAiChatCompletions`
- `OpenAiResponses`
- `AnthropicMessages`
- `GoogleGenerateContent`
- `DashscopeRealtimeWs`
- `DashscopeInferenceWs`
- `DashscopeNative`
- `QianfanNative`

它回答“怎么说话”。

### 一个重要结论

`OperationKind`、`ApiSurfaceId`、`WireProtocol` 不能合并成一个字段。

原因：

- 同一个 `OperationKind` 可能对应多个 `ApiSurfaceId`。
- 同一个 `ApiSurfaceId` 可能与多个 provider node 结合出不同 base URL。
- 同一个 `WireProtocol` 可能被多个 provider surface 复用。

典型例子：

- Bailian 的 `qwen-tts-realtime` 和 `cosyvoice-v1` 都是 `AudioSpeech`，但一个走 `DashscopeWsRealtime`，一个走 `DashscopeWsInference`。
- OpenAI、OpenRouter、DashScope compatible chat 都可以共享 OpenAI Chat 的 wire protocol，但 surface 和 path 不是同一个。

## 纯 Rust Catalog 的边界

Ditto 运行时最终只认 Rust 里的 builtin registry。

外部文件的角色：

- 抓取官方 docs / SDK / demo / 社区仓库的参考资料
- 帮助人工核对 edge cases
- 帮助生成候选数据
- 不直接参与运行时查表

这意味着：

- 参考 catalog 可以继续存在于仓库里，但属于“证据和数据来源层”。
- 真正参与运行时的，是 `src/catalog/` 下的 Rust 静态定义。
- 如果未来需要自动化同步，应该是“离线脚本 -> 生成 Rust 源文件”，而不是“运行时读 TOML/JSON”。

当前已经补上一层 Rust reference schema/validator：`src/catalog/reference_schema.rs`。

它的职责是：

- 以统一的最小公共结构解析 `catalog/provider_models/*.json` 与 `catalog/provider_models/*.toml`
- 对 provider id、model `api_surfaces`、以及 records 中显式声明的 API surface 做离线一致性校验
- 通过目录级测试保证同名 JSON/TOML 在 canonical 语义上等价

这层只服务离线校验与生成前处理，不参与 runtime route resolution。

另外，`core_provider_reference_catalog_expectations()` 已把四份核心直连 provider reference catalog 固定成显式语义约束：

- `openai.*`：官方 OpenAI provider 的完整 reference 目录
- `deepseek.*`：官方 DeepSeek provider 的完整 reference 目录
- `google.*`：Gemini / Google provider 的完整 reference 目录
- `anthropic.*`：Claude / Anthropic provider 的完整 reference 目录

这些约束会校验 provider 元信息、官方 source URL 前缀、最小模型规模，以及该 provider 目录必须覆盖的关键 API surface。

在这之上，reference schema 还会派生两层稳定能力描述：

- `ReferenceModelCapabilityProfile`：把每个 model 的 `api_surfaces` 归一成稳定 capability set，并显式暴露 `unmapped_api_surfaces`
- `ReferenceProviderCapabilityProfile`：把 provider 目录下所有 model 的 capability 做并集，形成 provider 级完整能力集合

这样 Phase 3 里 model/provider 的 capability 分类已经在 Rust reference 层收敛，不再只是散落在脚本或 `api_surfaces` 字符串里。

现在又补上了两层离线生成结果：

- `scripts/generate_rust_provider_catalog.py` 会从 `catalog/provider_models/*.json` 生成按 provider 拆分的 `src/catalog/generated/providers/*.rs` 与聚合入口 `src/catalog/generated/providers/mod.rs`
- 生成结果里的 `ProviderModelDescriptor` / `ProviderPluginDescriptor` 已带 `capability_statuses`，并把 `implemented` / `planned` / `blocked` 区分清楚
- runtime 侧只把 `implemented` 纳入 `capability_set()` / `capability_bindings()`，其余状态只作为显式元数据保留
- `scripts/generate_rust_profile_catalogs.py` 会把 OpenAI / Google / Anthropic 的公开 profile model catalog 生成到 `src/profile/generated_catalogs.rs`
- `src/profile/*_model_catalog.rs` 不再 `include_str!()` 或运行时解析 TOML，而是只从 Rust 生成结果初始化 `OnceLock`

这意味着 Phase 3 的单一事实来源已经落地到运行时边界：参考 TOML/JSON 只参与离线校验与生成，不再参与 runtime capability judgment。

## 默认内建能力

默认构建只内置“通用 OpenAI 类 API 支持”。

这里的“通用 OpenAI 类”指：

- 标准 OpenAI 的 chat / responses / embeddings / streaming / tools 等接口语义
- 不绑定某一个特定 provider
- 不内置特定厂商模型清单
- 不默认携带 OpenRouter / DeepSeek / Qwen / Gemini / Claude 等特定平台的 registry 数据

这样做的原因：

- 最小默认构建更干净。
- feature 边界更清楚。
- provider 扩展不会反向污染核心抽象。
- Omne 和其他上层调用方可以按需装配具体 provider 插件。

## Provider 插件化

特定 provider 不应继续只体现为“某个 HTTP client 模块”，而应同时体现为一个可插拔的 catalog/plugin 单元。

每个 provider 插件至少要声明：

- provider id
- 支持的 auth 方式
- 支持的 operation
- 该 provider 下的 binding rules
- 支持的 model / family / alias / selector
- 证据等级与验证状态

建议的 feature 粒度：

- `openai`: 通用 OpenAI API 支持
- `anthropic`: Anthropic provider plugin
- `google`: Google provider plugin
- `openai-compatible`: OpenAI-compatible provider plugin 框架
- 其他 provider 未来继续挂在对应 feature 或子 feature 下

可预期的后续细分：

- `provider-openrouter`
- `provider-bailian`
- `provider-qianfan`
- `provider-minimax`
- `provider-kimi`
- `provider-zhipu`
- `provider-doubao`

是否立即拆到这么细，要视维护成本决定；但抽象层必须允许这样拆。

## Auth 插件化

当前 `ProviderAuth` 已支持 API key / header / query / command / SigV4 / OAuth client credentials。

后续 catalog 层要表达的不是“具体秘钥值”，而是“一个 provider plugin 支持哪些登录方式”。

需要支持的 auth 能力包括：

- `ApiKeyHeader`
- `ApiKeyQuery`
- `CommandToken`
- `SigV4`
- `OAuthClientCredentials`
- `OAuthDeviceCode`
- `OAuthBrowserPkce`
- `StaticBearer`

其中：

- OpenAI / Anthropic / Google 未来都可能存在 OAuth 登录封装。
- catalog 层要能声明“支持哪种 auth flow”。
- runtime 配置层再决定“实际启用哪种 auth flow”。

## 运行时结构

运行时至少需要下面这些类型：

已落地的第一批核心强类型：

- `ProviderId`
- `CapabilityKind`
- `ProviderCapabilitySet`
- `ProviderCapabilityBinding`
- `ProviderRuntimeSpec`
- `ModelCapabilityDescriptor`
- `ProviderCapabilityResolution`

这些类型已经接入 `src/catalog/` 与 runtime route resolver，不再只是文档草图。

补上的第二批落地点：

- `CatalogRegistry::plugin_by_hint` / `provider_runtime_spec_by_hint` / `provider_capability_resolution(_by_hint)` 已下沉到 catalog 层，provider 名称匹配不再在 profile 层自造一套。
- 默认构建现在内置了 generic `openai-compatible` builtin plugin，用来承接默认 feature 组合 `provider-openai-compatible + cap-llm`。
- `ProviderConfig.capabilities` 已通过 catalog scope 做约束解析：只有落在 LLM scope 的 provider/model 才允许声明这些 LLM 专属能力字段。
- provider/capability 主链现在使用命名错误类型 `ProviderResolutionError`，不再继续把 catalog/provider 解析失败全部塞进 `InvalidResponse`。
- `ProviderPluginDescriptor::capability_resolution(...)` 内部的能力域归并已由命名结构承接，不再靠裸元组在 provider/model/effective scope 之间传值。
- `RuntimeRouteRequest` 已显式携带 `required_capability`，`resolve_runtime_route()` 会在 route resolution 前先校验 provider/model 的有效 capability 支持，而不是只靠 operation 猜测。
- `CatalogRegistry::plugin_for_runtime_request(...)` 已成为运行时 provider hint -> catalog plugin 的统一入口；它会先按 id/hint 匹配，再按 `ProviderConfig.upstream_api` 回落到 openai-compatible / google / anthropic 主插件。
- `profile::routing` 现在在生成 stage targets 时会经过 catalog runtime resolver，已从“测试里能跑”接成“主运行时默认经过”的入口。
- routing phase 校验现在按 capability-first 处理；LLM 路由会依次尝试 `chat.completion -> response -> text.completion`，因此像 `computer-use-preview` 这样的 response-only 模型不再在 completion 路由阶段被误判为不可用。
- `resolve_http_provider_config` 与 `resolve_provider_request_auth_required/optional` 已收敛为 provider runtime 的公共辅助层；Google / Anthropic / Cohere / OpenAI-like 以及 `/models` 发现路径不再各自重复解析 `auth`、`base_url`、`default_model` 与默认 HTTP client。
- gateway translation 的 `build_*_model()` 现在在存在默认模型时会先经过 `RuntimeRoute` 解析，再把解析出的 canonical runtime provider 与 `base_url/default_model` 回灌到 adapter 构建；没有默认模型时则回落到 strict plugin/runtime-spec 选择，不再只靠 builder 本地猜测。
- 这条 builder 主链仍保留“exact provider id -> upstream_api -> 明确 legacy alias -> generic openai-compatible” 的严格顺序，因此像 `yunwu-openai` 这样的自定义 provider 不会因为 fuzzy hint 命中 `openai` 而被错误重写到官方 OpenAI runtime。
- `BuiltinProviderCapabilitySummary` 已暴露给 `catalog_bridge` / `config_editor`，CLI、配置编辑器和文档现在可以读取同一份 provider 最小 capability 摘要，而不是各写一套 provider 特例。
- 对未编译进来的已知 OpenAI-like provider hint，runtime resolver 现在至少能推断官方 base URL；像 `deepseek` 这类 path 形态不同的 provider，还会在 generic openai-compatible fallback 时做最小路径归一，避免 `v1` 前缀漂移。

- `ProviderId`
- `OperationKind`
- `ApiSurfaceId`
- `WireProtocol`
- `TransportKind`
- `EndpointTemplate`
- `ModelSelector`
- `AuthMethodKind`
- `VerificationStatus`
- `EvidenceLevel`
- `ProviderPluginDescriptor`
- `ModelBinding`
- `ResolvedInvocation`

其中关键思想是：

- URL 不应作为 catalog 的唯一真相。
- 真正可复用的是 `transport + path_template + wire_protocol + auth capability`。
- base URL、workspace、region、node override 属于运行时 node config，而不是 catalog 的事实层。

## 为什么要这样做

### 1. 让复杂 provider 的规则可维护

像 Bailian / Qianfan / MiniMax 这种 provider，不能靠运行时代码里的字符串判断长期维护。

Rust registry 的好处是：

- 规则集中
- 类型明确
- 增删 provider 不用修改一堆分支
- 每条绑定都可以挂证据和验证状态

### 2. 让 Omne 和 Ditto 共享同一套底层能力

Omne 不应该重复实现 provider/model 配置逻辑，也不应该调用一个单独二进制来“侧向控制 Ditto”。

更合理的是：

- Ditto 提供 Rust library API
- Omne 调 Ditto 的高层封装
- 两边共享同一个 catalog resolver

### 3. 让 feature flags 真正反映产品边界

默认只内置 generic OpenAI，其他 provider 全靠 feature 打开，能更符合“核心 + 插件”的架构。

### 4. 让外部参考数据真正进入系统

现在抓到的大量官方 endpoint、GitHub SDK/demo 证据，如果只是留在 TOML/JSON 里，运行时并不会直接受益。

把它们收敛进 Rust registry 后，Ditto 才能直接利用这些信息：

- 自动补全 provider/model
- 做 route resolution
- 做 capability gate
- 做 auth 推荐
- 做证据级别告警

## 怎么做

采用分阶段推进，而不是一次性替换现有系统。

### 阶段 1：建立纯 Rust catalog 核心类型

目标：

- 新增 `src/catalog/` 模块
- 定义核心 enums / structs
- 不改现有 HTTP provider 行为
- 不强行替换 `profile/config`

验收标准：

- 可以在 Rust 里表达一个 generic OpenAI provider class
- 可以表达至少一条 model binding
- 有最小 resolver 能查表得到 route 结果

### 阶段 2：内置 generic OpenAI registry

目标：

- 默认构建只保证 generic OpenAI 支持
- 不默认携带特定 provider 模型目录
- 调整 default features 与文档

验收标准：

- 默认 feature 编译通过
- `cargo check` 默认构建通过
- 文档反映新的默认 feature 语义

### 阶段 3：引入 provider plugin registry

目标：

- 让 `google` / `anthropic` / `openai-compatible` 等 feature 都能注册自己的 builtin plugin
- 每个 plugin 暴露支持的 auth kinds、binding rules、可选 model metadata

验收标准：

- plugin 注册表可以枚举当前编译进来的 provider
- resolver 能按 feature 决定候选 provider 集合

### 阶段 4：把配置编辑器和 Omne 集成到同一 resolver

目标：

- `provider add/list/delete`
- `model add/list/delete`
- provider/model 候选推荐
- Omne 直接复用 Ditto library API

验收标准：

- Omne 不重复实现 provider/model 策略逻辑
- CLI / 交互式流程直接使用 Ditto catalog API

### 阶段 5：逐步迁移 provider-specific 运行时判断

目标：

- 把现有散落在运行时里的 provider/model 特例逐步迁移到 binding rules
- 让 runtime 更偏“查表 + serializer dispatch”

验收标准：

- 新增 provider 不需要继续复制大量 `if model.starts_with(...)`
- 已知复杂 provider 的主要特殊路径可由 registry 表达

## 非目标

当前阶段不做这些事：

- 不立即删除现有 `profile/config` 结构
- 不立即删除参考型 TOML/JSON 文件
- 不立即重构所有 provider HTTP client
- 不立即实现全部 OAuth 登录流程
- 不在一个 PR 里完成所有 provider 的完整迁移

## 任务清单

### 文档与设计

- 新建本设计文档
- 明确三个核心概念的边界
- 明确“外部参考数据”与“运行时 Rust registry”的职责分离

### 基础代码

- 新增 `src/catalog/` 模块
- 增加核心 enums / structs
- 增加最小 resolver trait / function
- 增加 builtin generic OpenAI registry

### 构建与 feature

- 收敛默认 features 为最小核心
- 让 provider plugins 与 features 对齐
- 调整文档和示例中的 feature 说明

### 集成

- 让 config editor 后续可以读取 catalog resolver
- 为 Omne 暴露可复用的高层库 API

## 当前执行顺序

本轮先做：

1. 文档落地
2. `src/catalog/` 第一版核心类型
3. generic OpenAI builtin registry
4. 收敛默认 features 与文档

provider-specific registry 的完整迁移，在这之后逐步推进。
## Runtime Truth Overrides

Reference catalogs describe the full upstream surface area. Builtin runtime plugins are allowed to narrow that surface until the corresponding runtime traits/builders exist.

Current builder resolution order for gateway translation is now intentionally strict:

- exact builtin provider id first
- then explicit `ProviderConfig.upstream_api` fallback
- then explicit legacy alias mapping
- and only then the generic OpenAI-compatible default

This avoids fuzzy provider-name matching from silently rewriting custom provider names to the wrong runtime.

Gateway translation now also resolves each OpenAI-facing endpoint through an explicit descriptor layer:

- external endpoint kind (for example `chat.completions`, `responses`, `audio.translations`)
- runtime operation used for catalog resolution and builder gating
- runtime capability requirement used before the request is dispatched

That keeps endpoint exposure and runtime capability checks aligned. One explicit temporary bridge still exists today: `POST /v1/audio/translations` is routed through the current `audio.transcription` runtime operation until a dedicated audio-translation builder/runtime surface lands.

Current explicit overrides:

- DeepSeek: builtin runtime treats the provider as `llm`-only and uses the dedicated provider capability spec to gate non-LLM builders, even though the underlying transport still reuses the OpenAI-compatible adapter.
- Anthropic: the current generated catalog only declares `CHAT_COMPLETION`, so the non-LLM runtime plan is intentionally empty today. Additional non-LLM capabilities should not be advertised until the catalog grows beyond `llm` and the matching runtime trait/builder exists.
- Google: builtin runtime now advertises `llm`, `embedding`, `image.generation`, `realtime`, and `video.generation` when the corresponding capability toggles are compiled in. The remaining work for Google is no longer provider runtime coverage, but whether `video.generation` should graduate to a public `cap-*` feature pack.

