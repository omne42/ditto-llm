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
- Provider: Cohere Chat API (`/v2/chat`) with generate + SSE streaming + tool calls.
- Gateway translation: allow `provider=cohere` backends for OpenAI-compatible translation endpoints.
- Gateway translation: accept LiteLLM-style OpenAI-compatible provider aliases (e.g. `groq`, `mistral`, `deepseek`, `openrouter`).
- Gateway translation: support `POST /v1/responses/compact` via provider-backed compaction (best-effort).
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
- Usage: add `cache_input_tokens` (e.g., OpenAI `cached_tokens`) for prompt-cache accounting.
- Usage: add `cache_creation_input_tokens` (Anthropic / LiteLLM) for prompt-cache accounting.
- Gateway: pricing table supports LiteLLM prompt-cache costs (`cache_read_input_token_cost`, `cache_creation_input_token_cost`).
- Gateway: pricing table supports LiteLLM tiered costs (`*_above_*_tokens` keys).
- Gateway: pricing table supports LiteLLM service tier costs (`*_priority`, `*_flex`) and uses request `service_tier` for USD budget estimates.
- Gateway: cost budgeting accounts for per-backend `model_map` when pricing entries exist for the mapped model.
- Image generation: `ImageGenerationModel` + OpenAI/OpenAI-compatible `/images/generations`.
- Audio: `AudioTranscriptionModel` + `SpeechModel` for OpenAI/OpenAI-compatible `/audio/*`.
- Moderations: `ModerationModel` for OpenAI/OpenAI-compatible `/moderations`.
- Rerank: `RerankModel` + Cohere `/rerank`.
- Batches: `BatchClient` + OpenAI/OpenAI-compatible `/batches`.
- Document non-goals and optional future scope (gateway/control-plane features, agent loop, UI SDK surface, native auth adapters).
- Agent tool loop: `ToolLoopAgent` + `ToolExecutor`, stop hooks, approvals, and tool-result backfill (feature `agent`).
- Agent: built-in tool wrappers and executors (`http_fetch`, `fs_read_file`, `fs_write_file`, `fs_list_dir`, `fs_find`, `fs_grep`, `fs_stat`, `shell_exec`) for `ToolLoopAgent` (feature `agent`).
- Agent: add `fs_delete_file` tool + executor to delete files/directories within root (directories require `recursive=true`).
- Agent: `shell_exec` supports optional `stdin` (UTF-8) input.
- Agent: `http_fetch` supports `parse_json`, per-call `max_response_bytes`, emits `elapsed_ms`, and marks non-2xx responses as tool errors.
- Auth adapters: SigV4 signer + OAuth client-credentials flow (feature `auth`).
- Providers: Bedrock (SigV4) and Vertex (OAuth) minimal adapters (features `bedrock`, `vertex`).
- SDK utilities: stream protocol v1, telemetry sink, devtools JSONL logger, MCP tool adapter (feature `sdk`).
- SDK HTTP helpers: encode stream protocol v1 as NDJSON or SSE (feature `sdk`).
- Gateway control-plane: virtual keys, limits, cache, budget, routing, guardrails, passthrough, and `ditto-gateway` stub binary (feature `gateway`).
- Docs: clarify Bedrock/Vertex scope for minimal adapters.
- Bedrock: Anthropic Messages-on-Bedrock generate + streaming + tools support (feature `bedrock`).
- Vertex: GenAI generateContent + streamGenerateContent (SSE) + tools support (feature `vertex`).
- Gateway: `ditto-gateway` HTTP server with `/v1/gateway`, `/health`, `/metrics`, and `/admin/keys` (feature `gateway`).
- Gateway: OpenAI-compatible passthrough proxy for `ANY /v1/*` (incl. streaming) with per-backend header/query-param injection and optional devtools JSONL logging (feature `gateway-devtools`).
- Gateway: accept virtual keys via `x-api-key` (alias for `Authorization: Bearer ...` / `x-ditto-virtual-key`).
- Gateway: config supports `${ENV_VAR}` interpolation in proxy backend `base_url`/`headers`/`query_params`, backend `provider_config` fields, and `virtual_keys[].token` (resolved at startup).
- Gateway: `ditto-gateway` supports `--dotenv PATH` to load env vars for config interpolation and provider auth.
- Gateway: `ditto-gateway` supports `--admin-token-env` and `--redis-env` to load sensitive CLI options from env (works with `--dotenv`).
- Gateway: passthrough proxy supports per-backend model mapping via `model_map` (rewrites JSON `model` before forwarding).
- Gateway: add `gateway-translation` feature to serve `POST /v1/chat/completions` and `POST /v1/responses` via native Ditto providers (configured via backend `provider` + `provider_config`).
- Gateway: translation backends can also serve `POST /v1/embeddings` (best-effort OpenAI shape).
- Gateway: translation backends can also serve `POST /v1/moderations` and `POST /v1/images/generations` (best-effort OpenAI shapes).
- Gateway: translation backends can also serve `POST /v1/audio/transcriptions` and `POST /v1/audio/speech` (best-effort OpenAI behavior).
- Gateway: translation backends can also serve `POST /v1/rerank` (best-effort OpenAI behavior).
- Gateway: translation backends can also serve `GET|POST /v1/batches`, `GET /v1/batches/{id}`, and `POST /v1/batches/{id}/cancel` (best-effort OpenAI behavior).
- Gateway: guardrails support model allow/deny lists (exact or `prefix*`).
- Gateway: router rules support per-route guardrails overrides (by `model_prefix`).
- Gateway: when upstream does not implement `POST /v1/responses` (404/405/501), automatically fall back to `POST /v1/chat/completions` and return a best-effort Responses-like response/stream (`x-ditto-shim: responses_via_chat_completions`).
- Gateway: `--state PATH` persists admin virtual-key mutations to a JSON state file; proxy responses include `x-ditto-request-id`.
- Gateway: router supports weighted backends (`default_backends` / `rules[].backends`) and falls back on network errors when proxying.
- Gateway: optional sqlite persistence for admin virtual keys via `--sqlite PATH` (feature `gateway-store-sqlite`).
- Gateway: optional in-memory proxy cache for non-streaming OpenAI-compatible responses (feature `gateway-proxy-cache`).
- Gateway: proxy cache supports Redis-backed sharing when running with `--redis` (feature `gateway-store-redis`).
- Gateway: proxy cache supports admin purge and emits `x-ditto-cache-key` / `x-ditto-cache-source` headers (feature `gateway-proxy-cache`).
- Gateway: optional OpenTelemetry tracing exporter via OTLP (feature `gateway-otel`).
- Gateway: Prometheus metrics for per-backend in-flight gauge and request duration histogram (feature `gateway-metrics-prometheus`).
- Gateway: Prometheus metrics for per-path request counts and proxy request duration histogram (feature `gateway-metrics-prometheus`).
- Gateway: Prometheus metrics for proxy cache lookups/hits/misses (by path/source) and cache store/purge counters (feature `gateway-metrics-prometheus` + `gateway-proxy-cache`).
- Gateway: proxy backpressure via `--proxy-max-in-flight` (rejects when too many in-flight proxy requests).
- Gateway: per-backend proxy backpressure via `backends[].max_in_flight` (rejects with 429 + OpenAI-style error code `inflight_limit_backend`).
- Gateway: per-backend proxy request timeout via `backends[].timeout_seconds` (default: 300s).
- Gateway: optional active health checks for proxy backends (`--proxy-health-check*`, feature `gateway-routing-advanced`).
- Gateway: best-effort usage-based settling for proxy budgets (for non-streaming JSON responses, prefer `usage` tokens/cost over request estimates).
- Gateway: optional tiktoken-based input token counting for proxy budgets/guardrails/costing (feature `gateway-tokenizer`; falls back to request-size estimation).
- Gateway: virtual keys support optional `project_id` and `user_id` attribution; admin endpoints can aggregate `/admin/budgets` and `/admin/costs` by project/user.
- Gateway: virtual keys support shared budgets scoped by `project_id` / `user_id` via `project_budget` / `user_budget` (token + USD micros).
- Gateway: guardrails support regex patterns (`banned_regexes`) and optional PII blocking (`block_pii`).
- Gateway: guardrails support optional request schema validation via `guardrails.validate_schema`.
- Gateway: add Claude Code / Anthropic Messages API compatibility (`POST /v1/messages`, `POST /v1/messages/count_tokens`) and Gemini-compatible generateContent endpoints (`POST /v1beta/models/*:generateContent`, `POST /v1beta/models/*:streamGenerateContent`, and `POST /v1internal:*GenerateContent`).
- Gateway translation: support legacy `POST /v1/completions` (non-streaming + streaming).
- Gateway translation: serve `GET /v1/models` and `GET /v1/models/*` locally (no upstream OpenAI-compatible required).
- Gateway translation: support `POST /v1/audio/translations` (same parsing/response as transcriptions).
- Gateway: add `--json-logs`, `--proxy-cache*`, and `--otel*` CLI flags to `ditto-gateway`.
- Gateway admin key listing redacts tokens by default; `?include_tokens=true` returns full tokens.
- Multimodal example requires `--features base64` to enable base64 encoding dependency.

### Changed

- Refactor crate layout into modules (`embedding`/`model`/`providers`/`types`/`utils`).
- Extend `DittoError` with `Api` and `Io` variants for richer provider and streaming errors.
- `provider_options` supports per-provider buckets (`"*"` + provider ids) and passes through additional provider-specific keys where supported (conflicts are ignored with warnings).
- `provider_options`: accept `openai_compatible` as an alias bucket for `openai-compatible`, and add `bedrock`/`vertex` buckets.
- Format: rustfmt cleanup (no behavior changes).
- Format: rustfmt cleanup (imports order).
- Dev: pre-commit rejects oversized staged Rust files (default 1000 lines; configurable via `DITTO_MAX_RS_LINES`).
- Refactor: split `gateway::translation` module into sub-files (no behavior changes).
- Refactor: split `gateway::http` module into sub-files (no behavior changes).
- Refactor: split `gateway::interop` module into sub-files (no behavior changes).
- Refactor: split `gateway::redis_store` module into sub-files (no behavior changes).
- Refactor: split `providers::openai` module into sub-files (no behavior changes).
- Refactor: split `providers::openai_compatible` module into sub-files (no behavior changes).
- Refactor: split `providers::anthropic` module into sub-files (no behavior changes).
- Refactor: split `providers::bedrock` module into sub-files (no behavior changes).
- Refactor: split `providers::cohere` module into sub-files (no behavior changes).
- Refactor: split `providers::google` module into sub-files (no behavior changes).
- Refactor: split `agent::toolbox` module into sub-files (no behavior changes).
- Refactor: split `object` module into sub-files (no behavior changes).
- Refactor: split `profile` module into sub-files (no behavior changes).
- Dev: fix clippy warnings (`cargo clippy --all-targets --all-features -- -D warnings`).

### Fixed

- Docs: update README gateway translation endpoints list (`/v1/models`, `/v1/completions`, `/v1/audio/translations`).
- Docs: fix README `gateway.json` example indentation.
- Gateway: do not mount `/admin/*` routes unless an admin token is configured.
- Gateway: apply per-route guardrails overrides to OpenAI proxy requests.
- Gateway: extend request schema validation coverage (`/v1/completions`, `/v1/moderations`, `/v1/images/generations`, `/v1/audio/speech`, `/v1/rerank`, `/v1/batches`).
- Gateway tokenizer: estimate input tokens for additional OpenAI endpoints (`/v1/completions`, `/v1/images/generations`, `/v1/audio/speech`, `/v1/rerank`).
- Gateway translation: use `--dotenv` env values when lazily building provider clients (embeddings/moderations/images/audio/rerank/batches).
- Gateway proxy cache: include `x-api-key` in the cache scope when virtual keys are disabled.
- Gateway: keep proxy backpressure permits until the response body is drained (including non-streaming responses).
- Streaming: abort background stream tasks when the consumer streams are dropped.
- Providers: avoid panicking if the default `reqwest::Client` build fails (fall back to `reqwest::Client::new()`).
- Security: redact sensitive fields in `Debug` for gateway key config and auth-related types.
- Tests: make Vertex `generateContent` mock matching robust to float serialization differences.
- OpenAI Responses: map `finish_reason` consistently (generate + stream), including tool-call completion.
- OpenAI Responses: include `instructions` (from system messages) to satisfy providers that require it.
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
- Bedrock eventstream header parsing validates header value lengths for all types.

## [0.1.1] - 2026-01-23

### Added

- Provider profile config (`base_url` / auth / model whitelist / capability flags)
- OpenAI-compatible `GET /models` discovery
- Model-level `thinking` config (mapped by consumers to `reasoning.effort`)
