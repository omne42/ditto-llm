# Providers 能力矩阵

本页是“查表型”入口，帮助你快速回答：

- 默认构建到底承诺什么
- 某个 provider pack 要配哪个 capability pack
- 当前哪些 provider/capability 已实现，哪些还只是规划或 catalog 元数据
- Gateway 的 passthrough 与 translation 分别覆盖到哪里

仓库根目录的 `PROVIDERS.md` 是更完整的矩阵与状态表；`CATALOG_COMPLETENESS.md` 则给出持续更新的 provider/capability/model completeness dashboard。本页给出最重要的阅读顺序。

---

## 1) 默认核心是什么

Ditto 的默认构建不是“全家桶”。

默认只打开：

- `provider-openai-compatible`
- `cap-llm`

这意味着默认只承诺：

- 通用 OpenAI-compatible 文本生成
- SSE streaming
- tool calling

其它 provider 与能力都必须显式开启。

---

## 2) 两类 feature：provider packs 与 capability packs

### Provider packs

- `provider-openai-compatible`
- `provider-openai`
- `provider-anthropic`
- `provider-google`
- `provider-cohere`
- `provider-bedrock`
- `provider-vertex`
- 以及 `provider-deepseek`、`provider-kimi`、`provider-openrouter` 等 provider-specific packs

### Capability packs

- `cap-llm`
- `cap-embedding`
- `cap-image-generation`
- `cap-image-edit`
- `cap-audio-transcription`
- `cap-audio-speech`
- `cap-moderation`
- `cap-rerank`
- `cap-batch`
- `cap-realtime`

一个功能真正可用，通常需要同时满足：

- provider pack 已启用
- capability pack 已启用
- runtime 已经实现这条 provider × capability 绑定

---

## 3) Ditto 的两条 provider 路径

### A) Native adapters

直接调用 provider 原生 API，语义最完整：

- OpenAI：`/responses`
- Anthropic：`/messages`
- Google：`generateContent`
- Cohere：`/v2/chat`
- Bedrock / Vertex：各自原生认证与入口

### B) OpenAI-compatible adapters

通过 OpenAI-compatible upstream 调用：

- `/chat/completions`
- `/embeddings`
- `/batches`
- 以及上游自己支持的其它 OpenAI 兼容端点

这条路径适合 LiteLLM、OpenRouter、DeepSeek、Qwen、Kimi、本地代理等统一入口。

---

## 4) 怎么看实现状态

优先看仓库根目录的 `PROVIDERS.md`，因为它直接按下面这个维度列状态：

- provider pack
- capability
- Cargo feature
- runtime status
- 备注（native / openai-compatible 复用 / planned 等）

阅读建议：

1. 先确认你要的 provider pack 是否是默认核心，还是可选 pack。
2. 再确认对应 capability pack 是否已经实现，不要只看 catalog 里“声明过”。
3. 如果你走 Gateway translation，再额外确认 translation surface 是否已挂载该 capability。

---

## 5) 下一步读哪里

- `SDK → ProviderConfig 与 Profile`：看 node 配置边界
- `Gateway → 配置文件`：看 gateway backend / translation backend 的配置方式
- 仓库根目录 `PROVIDERS.md`：看完整实现状态表
