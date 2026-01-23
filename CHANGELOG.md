# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Unified SDK: `LanguageModel` / `EmbeddingModel` + core request/response types.
- Multi-modal message parts: `ContentPart::Image` (images) and `ContentPart::File` (PDFs) with `FileSource` support.
- Providers: OpenAI (Responses + embeddings), Anthropic (Messages), Google (GenAI + embeddings).
- Provider: OpenAI-compatible Chat Completions (for LiteLLM / DeepSeek / Qwen / etc.).
- Streaming + tool calling support across providers (with compatibility warnings when unsupported).
- Stream utility: `collect_stream(StreamResult) -> CollectedStream` to aggregate `StreamChunk`s into a final `GenerateResponse`.
- Provider builders accept a custom `reqwest::Client` via `with_http_client` (proxy/headers/timeout customization).
- Examples: `basic`, `streaming`, `tool_calling`, `embeddings`, `openai_compatible`, `multimodal`.
- Roadmap: `TODO.md` with a scoped capability checklist (LiteLLM / AI SDK aligned).
- Optional integration smoke tests behind the `integration` feature (requires real API keys).
- Utilities: generic SSE parsing and JSON Schema → OpenAPI schema conversion (for tool schemas).
- Provider clients can be built from config: `*::from_config(&ProviderConfig, &Env)`.
- Auth helper: `resolve_auth_token_with_default_keys` (for provider-specific default env keys).
- Streaming emits request conversion warnings via `StreamChunk::Warnings`.
- Controlled request options via `ProviderOptions` (`reasoning_effort`, `response_format`).
- Streaming emits response ids (when available) via `StreamChunk::ResponseId`.
- OpenAI-only options via `ProviderOptions`: `parallel_tool_calls`.
- OpenAI Responses tool schemas default to `strict=true` when omitted.

### Changed

- Refactor crate layout into modules (`embedding`/`model`/`providers`/`types`/`utils`).
- Extend `DittoError` with `Api` and `Io` variants for richer provider and streaming errors.

### Fixed

- OpenAI Responses: map `finish_reason` consistently (generate + stream), including tool-call completion.
- OpenAI-compatible streaming: flush pending tool calls and always emit a final `FinishReason` even if the provider omits it.
- JSON Schema → OpenAPI conversion: support common constraints and `additionalProperties` for tool schemas.

## [0.1.1] - 2026-01-23

### Added

- Provider profile config (`base_url` / auth / model whitelist / capability flags)
- OpenAI-compatible `GET /models` discovery
- Model-level `thinking` config (mapped by consumers to `reasoning.effort`)
