# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Unified SDK: `LanguageModel` / `EmbeddingModel` + core request/response types.
- AI SDK-aligned helpers: `generate_text` / `stream_text`, `generate_object_json` / `stream_object` (structured outputs), and `embed_many` aliases.
- Structured output options: `ObjectOptions` (`output=Object|Array`, `strategy=Auto|NativeSchema|ToolCall|TextJson`) and streaming `element_stream` for array outputs.
- Multi-modal message parts: `ContentPart::Image` (images) and `ContentPart::File` (PDFs) with `FileSource` support.
- Providers: OpenAI (Responses + embeddings), Anthropic (Messages), Google (GenAI + embeddings).
- Provider: OpenAI-compatible Chat Completions (for LiteLLM / DeepSeek / Qwen / etc.) and embeddings.
- Streaming + tool calling support across providers (with compatibility warnings when unsupported).
- Stream utility: `collect_stream(StreamResult) -> CollectedStream` to aggregate `StreamChunk`s into a final `GenerateResponse`.
- Streaming: `abortable_stream(StreamResult) -> AbortableStream` with `StreamAbortHandle`.
- Provider builders accept a custom `reqwest::Client` via `with_http_client` (proxy/headers/timeout customization).
- Provider config: `ProviderConfig.http_headers` to apply default HTTP headers when building clients from config (also used for `/models` discovery).
- Provider config: `ProviderConfig.http_query_params` to apply default HTTP query params when building clients from config (also used for `/models` discovery).
- File upload helper for OpenAI and OpenAI-compatible providers: `upload_file` / `upload_file_with_purpose`.
- Examples: `basic`, `streaming`, `tool_calling`, `embeddings`, `openai_compatible`, `openai_compatible_embeddings`, `multimodal`.
- Roadmap: `TODO.md` with a scoped capability checklist (LiteLLM / AI SDK aligned).
- Optional integration smoke tests behind the `integration` feature (requires real API keys).
- Utilities: generic SSE parsing and JSON Schema → OpenAPI schema conversion (for tool schemas).
- Tool schemas: document the supported JSON Schema subset contract and add regression coverage.
- Provider clients can be built from config: `*::from_config(&ProviderConfig, &Env)`.
- Auth helper: `resolve_auth_token_with_default_keys` (for provider-specific default env keys).
- Provider auth: `ProviderAuth::HttpHeaderEnv` / `ProviderAuth::HttpHeaderCommand` for non-standard auth headers (e.g. `api-key` gateways).
- Provider auth: `ProviderAuth::QueryParamEnv` / `ProviderAuth::QueryParamCommand` for gateways that require auth in the query string.
- Streaming emits request conversion warnings via `StreamChunk::Warnings`.
- Controlled request options via `ProviderOptions` (`reasoning_effort`, `response_format`).
- Streaming emits response ids (when available) via `StreamChunk::ResponseId`.
- OpenAI-only options via `ProviderOptions`: `parallel_tool_calls`.
- OpenAI Responses tool schemas default to `strict=true` when omitted.
- Image generation: `ImageGenerationModel` + OpenAI/OpenAI-compatible `/images/generations`.
- Audio: `AudioTranscriptionModel` + `SpeechModel` for OpenAI/OpenAI-compatible `/audio/*`.
- Moderations: `ModerationModel` for OpenAI/OpenAI-compatible `/moderations`.
- Rerank: `RerankModel` + Cohere `/rerank`.

### Changed

- Refactor crate layout into modules (`embedding`/`model`/`providers`/`types`/`utils`).
- Extend `DittoError` with `Api` and `Io` variants for richer provider and streaming errors.
- `provider_options` supports per-provider buckets (`"*"` + provider ids) and passes through additional provider-specific keys where supported (conflicts are ignored with warnings).

### Fixed

- OpenAI Responses: map `finish_reason` consistently (generate + stream), including tool-call completion.
- OpenAI-compatible streaming: flush pending tool calls and always emit a final `FinishReason` even if the provider omits it.
- OpenAI-compatible: support legacy `function_call` (generate + stream) and map `finish_reason=\"function_call\"` to `ToolCalls`.
- OpenAI-compatible: map `ToolChoice::Required` to `tool_choice=\"required\"` (instead of silently degrading to `auto`).
- Parameter conversion: clamp out-of-range `temperature`/`top_p` and drop non-finite values with warnings (avoid silently sending `0`).
- Stop sequences: drop empty/duplicate entries; truncate to 4 for OpenAI-compatible + Anthropic with a warning (Google preserves count).
- Google tool schemas: resolve local JSON Schema `$ref`; emit `Warning::Compatibility(tool.parameters.$ref)` only for unresolvable refs.
- Google tool schemas: emit `Warning::Compatibility(tool.parameters.unsupported_keywords)` when tool parameter JSON Schema uses unsupported keywords (they are ignored).
- JSON Schema → OpenAPI conversion: support common constraints and `additionalProperties` for tool schemas.
- Tool call arguments: preserve raw JSON on parse failures, emit `Warning::Compatibility(tool_call.arguments)`, and avoid double-encoding when replaying assistant tool calls.
- `collect_stream`: preserve chunk ordering (text/reasoning/tool calls) and warn on invalid tool-call argument JSON.
- Audio transcriptions: fall back to text with a warning if JSON response parsing fails (avoid silently swallowing errors).

## [0.1.1] - 2026-01-23

### Added

- Provider profile config (`base_url` / auth / model whitelist / capability flags)
- OpenAI-compatible `GET /models` discovery
- Model-level `thinking` config (mapped by consumers to `reasoning.effort`)
