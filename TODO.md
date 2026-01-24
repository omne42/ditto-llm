# Ditto-LLM TODO（LiteLLM / AI SDK：我们“需要”的能力清单）

本文是 `ditto-llm` 的**能力口径 + 全量待办清单**。目标不是 1:1 复刻 LiteLLM / Vercel AI SDK，而是把 CodePM/agent 需要的那部分能力做成稳定、可测试的 Rust SDK。

在 CodePM monorepo 内的相关背景文档（从本文件位置出发的相对路径）：

- `../../docs/ditto_llm.md`
- `../../docs/special_directives.md`
- `../../docs/model_routing.md`

---

## 0) 需求口径（先钉死范围，避免“全都要”）

根据现有文档与仓库实现，我把“我们需要”的最小集定义为：

- 统一 **语言模型** 语义层：`LanguageModel::{generate, stream}`
- 统一 **Embeddings**：`EmbeddingModel::{embed, embed_single}`
- **Streaming**：text delta + tool_call delta + finish_reason + usage + response_id
- **Tools (function calling)**：`tools`/`tool_choice`，并且差异用 `Warning` 显式暴露
- **多模态输入（输入侧）**：image + PDF（url/base64；有能力差异就发 `Warning`）
- **多 provider**（满足 CodePM 路由/兼容需求）：
  - OpenAI Responses（`/responses`）
  - OpenAI-compatible Chat Completions（`/chat/completions`，用于对接 LiteLLM/DeepSeek/Qwen 等）
  - Anthropic Messages（`/messages`）（可选但已实现）
  - Google GenAI（`generateContent`）（可选但已实现）

如果你希望把 LiteLLM 的这些端点也纳入“必须做”，请在本文 §3.4 勾选并确认优先级：
`/audio/*`、`/images/*`、`/batches`、`/rerank`、`/moderations`。

---

## 1) Done（当前仓库已具备的能力）

以下条目在 `bitto-llm/ditto-llm` 里已落地（以代码与 examples 为准）：

- [x] 统一 types：`Message`/`ContentPart`/`Tool`/`ToolChoice`/`Usage`/`FinishReason`/`Warning`（`src/types/mod.rs`）
- [x] 统一 traits：`LanguageModel`、`EmbeddingModel`（`src/model.rs`、`src/embedding.rs`）
- [x] Providers：
  - [x] OpenAI Responses + embeddings（`src/providers/openai.rs`）
  - [x] OpenAI-compatible Chat Completions + embeddings（`src/providers/openai_compatible.rs`）
  - [x] Anthropic Messages（`src/providers/anthropic.rs`）
  - [x] Google GenAI + embeddings（`src/providers/google.rs`，feature `google`）
- [x] Streaming：通用 SSE 解析 + providers 的 event 兼容处理（`src/utils/sse.rs` + 各 provider）
- [x] Tools：generate + stream tool deltas（含多 tool_calls 拼接与 warnings）（providers）
- [x] 多模态输入：`ContentPart::Image` / `ContentPart::File(PDF)`（providers + `examples/multimodal.rs`）
- [x] 受控扩展点：`ProviderOptions`（`reasoning_effort` / `response_format(json_schema)` / `parallel_tool_calls`）（`src/types/mod.rs`）
- [x] 路由/配置层：`ProviderConfig` / `ProviderAuth` / `.env` 解析 / `GET /models` 发现（`src/profile.rs`）
- [x] Examples：`basic`/`streaming`/`tool_calling`/`embeddings`/`openai_compatible`/`openai_compatible_embeddings`/`multimodal`（`examples/`）
- [x] 集成 smoke tests（feature `integration`）（`tests/integration_smoke.rs`）
- [x] Repo 级 githooks（Conventional Commits + changelog 绑定）（`githooks/`）

---

## 2) 能力清单（对齐 LiteLLM / AI SDK 的“我们需要”部分）

> 这里的 checkbox 口径：**我们是否需要 + 是否已实现**。如果不需要，就写进“不做”，别留成“以后再说”。

### 2.1 生成与流式（LanguageModel）

- [x] 非流式 `generate`（统一 response：content/finish_reason/usage/warnings）
- [x] 流式 `stream`（统一 chunks：text/tool_call/usage/finish_reason/response_id/warnings）
- [x] 取消/中断语义：显式 `StreamAbortHandle`（`abortable_stream` / `LanguageModelExt::stream_abortable`）
- [x] （可选）流式聚合器：`collect_stream(StreamResult) -> CollectedStream`（保序；仅合并相邻 text/reasoning；见 `src/stream.rs`）

### 2.2 Tools（function calling）

- [x] `tools`/`tool_choice` 映射（含 `strict` 的兼容性处理与 warnings）
- [x] tool call streaming：增量 args 拼接、多 tool_calls 处理
- [x] OpenAI-compatible：兼容 legacy `function_call`（非流式 + streaming），并将 `finish_reason="function_call"` 视为 `ToolCalls`
- [x] tool_call arguments 的 JSON roundtrip：解析失败保留 raw string，并发出 `Warning::Compatibility(tool_call.arguments)`；回放到 OpenAI/OpenAI-compatible 时不会二次转义
- [x] JSON Schema → OpenAPI schema（Google tool schema 子集转换）
- [x] （可选）工具 schema 的“严格子集”文档化：把支持/不支持关键字写成稳定契约，并用测试矩阵固化

### 2.3 Structured Output（对齐 AI SDK 的 generateObject 思路）

- [x] OpenAI Responses：`response_format(json_schema)`（`ProviderOptions`）
- [x] OpenAI-compatible：`response_format` 透传 + 不支持时 warnings（取决于上游实现）
- [x] Anthropic/Google：当前不做 SDK 侧 fallback；不支持则发 `Warning::Unsupported(response_format)`（保持简单可审计）

### 2.4 多模态输入（image / PDF）

- [x] Image：url/base64
- [x] PDF：url/base64/file_id（provider 视情况支持；不支持时 warnings）
- [x] （可选）文件上传 helper：把 bytes 上传为 `file_id`（OpenAI `/files`；`OpenAI::{upload_file, upload_file_with_purpose}` / `OpenAICompatible::{...}`）

### 2.5 Config / Routing（对齐 LiteLLM“代理接入”需求）

- [x] `ProviderConfig`/`ProviderAuth`/`.env` 解析
- [x] OpenAI-compatible `GET /models` 发现（用于模型列表与 allowlist）
- [x] 默认 HTTP headers：`ProviderConfig.http_headers`（from_config + `/models` 发现会应用）
- [x] 更通用的 auth header：支持非 Bearer 头（如 `api-key`），见 `ProviderAuth::HttpHeaderEnv` / `ProviderAuth::HttpHeaderCommand`
- [x] auth query param（企业网关要求 token 在 query string）

---

## 3) Backlog（按优先级推进）

### P0（必须先做：范围决策，不然全是幻觉）

- [x] **确认 Ditto-LLM 的端点范围**：只做 CodePM/agent “需要”的核心端点；其它端点进入 P2（按真实需求再启用）
  - DoD：在本文 §3.4 里把“必须/可选/不做”勾死，并给出每项 1 句使用场景（别写空话）

- [x] **确认 structured output 的跨 provider 口径**（§2.3）
  - DoD：选 A 或 B，并写清楚失败模式（warnings vs hard error vs retry）

### P1（强烈建议：减少接入成本/踩坑）

- [x] **ProviderAuth 扩展**（直连 Azure/企业网关/非 Bearer 兼容实现）
  - DoD：新增受控的 auth/header 表达（`http_header_env`/`http_header_command`），并补齐单测覆盖

- [x] **HTTP client 可配置化（调用方注入）**：各 provider 提供 `with_http_client(reqwest::Client)`
  - DoD：保持默认简单，但允许调用方覆盖（timeout/custom headers/proxy 等由 reqwest 负责）

- [x] **Config 默认 headers**：`ProviderConfig.http_headers`（无需写代码即可为网关/代理附加 header）

- [x] **Stream 聚合器（可选）**：`StreamChunk` → `GenerateResponse`
  - DoD：支持 text + tool_calls + usage + finish_reason + warnings；行为有单测（见 `src/stream.rs`）

### P2（扩面：只有在“我们真需要”时才做）

- [ ] **新增 endpoints traits（如果 P0 决策为需要）**
  - `ImageGenerationModel`（/images）
  - `AudioTranscriptionModel` / `SpeechModel`（/audio）
  - `RerankModel`（/rerank）
  - `ModerationModel`（/moderations）
  - `BatchClient`（/batches）
  - DoD：每个 trait 至少一个 provider 先跑通（OpenAI 或 OpenAI-compatible），并给出 examples + 单测（mock）

### 3.4 LiteLLM 端点范围勾选（P0 输出）

- [x] 必须：`/chat/completions` + streaming + tools + multimodal（输入侧） + usage/finish_reason（对接 LiteLLM/DeepSeek/Qwen 等）
- [x] 必须：`/responses`（直连 OpenAI Responses；LiteLLM 作为 Responses 网关是可选路径）
- [x] 必须：`/embeddings`（用于记忆/检索等 agent 基础能力）
- [ ] 可选：`/audio/transcriptions`、`/audio/speech`（只有当 agent 需要语音 I/O 时才加）
- [ ] 可选：`/image/generations`（属于生成类媒体端点；当前不在 CodePM 核心）
- [ ] 可选：`/batches`（批处理是平台能力；先保持 SDK 简单）
- [ ] 可选：`/rerank`（检索质量提升项；等真实需求再做）
- [x] 不做：`/a2a`（这属于 agent gateway，不是 LLM SDK 核心）

---

## 4) 验证（本仓库内可复制）

```bash
cd bitto-llm/ditto-llm

cargo fmt -- --check
cargo test --all-features
cargo clippy --all-targets --all-features -- -D warnings
```

跑 examples（需要相应环境变量）：

```bash
cargo run --example openai_compatible
cargo run --example multimodal -- <image_path> <pdf_path>
```
