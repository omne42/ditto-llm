# ditto-llm

Ditto-LLM is a small Rust SDK that provides a unified interface for calling multiple LLM providers.

Current scope:

- Unified types + traits: `LanguageModel` / `EmbeddingModel`, `Message`/`ContentPart`, `Tool`, `StreamChunk`, `Warning`.
- Text helpers: `generate_text` / `stream_text` (AI SDK-style `generateText` / `streamText`).
- Structured outputs: `generate_object_json` / `stream_object` (AI SDK-style `generateObject` / `streamObject`).
- Multi-modal inputs: images + PDF documents via `ContentPart::Image` / `ContentPart::File` (provider support varies; unsupported parts emit `Warning`).
- Parameter hygiene: `temperature`/`top_p` are clamped to provider ranges; non-finite values are dropped (with warnings).
- Providers:
  - OpenAI Responses API (generate + SSE streaming) and embeddings
  - OpenAI-compatible Chat Completions (LiteLLM / DeepSeek / Qwen / etc.) and embeddings
  - Anthropic Messages API (generate + SSE streaming)
  - Google GenAI (generate + SSE streaming) and embeddings
  - Cohere Chat API (generate + SSE streaming), embeddings, and rerank (feature-gated)
- Batches: `BatchClient` for OpenAI/OpenAI-compatible `/batches` (feature `batches`).
- Provider profile config and model discovery (`ProviderConfig` / `GET /models`) for routing use-cases.

Optional feature-gated modules:

- Agent tool loop: `ToolLoopAgent` + `ToolExecutor` (feature `agent`).
- Auth adapters: SigV4 signer + OAuth client-credentials flow (feature `auth`).
- Providers: Bedrock (SigV4) and Vertex (OAuth) adapters with generate + SSE streaming + tools (features `bedrock`, `vertex`).
- SDK utilities: stream protocol v1, HTTP adapters (SSE/NDJSON), telemetry sink, devtools JSONL logger, MCP tool adapter (feature `sdk`).
- Gateway control-plane: virtual keys, limits, cache, budget, routing, guardrails, passthrough, plus a `ditto-gateway` HTTP server (feature `gateway`).
- Gateway token counting: tiktoken-based input token estimation for proxy budgets/guardrails/costing (feature `gateway-tokenizer`).
- Gateway translation proxy: OpenAI-compatible `/v1/chat/completions`, `/v1/completions`, `/v1/responses`, `/v1/responses/compact`, `/v1/embeddings`, `/v1/moderations`, `/v1/images/generations`, `/v1/audio/transcriptions`, `/v1/audio/translations`, `/v1/audio/speech`, `/v1/rerank`, `/v1/batches`, and `/v1/models` backed by Ditto providers (feature `gateway-translation`).
- Gateway proxy caching: in-memory cache for non-streaming OpenAI-compatible responses (feature `gateway-proxy-cache`).
- Gateway OpenTelemetry: OTLP tracing exporter + structured logs for gateway HTTP requests (feature `gateway-otel`).

Non-goals (for now):

- The default build is not an API gateway/proxy; the `gateway` feature adds a lightweight control-plane + HTTP service. The `gateway-translation` feature adds translation for `/v1/chat/completions`, `/v1/completions`, `/v1/responses`, `/v1/responses/compact`, `/v1/embeddings`, `/v1/moderations`, `/v1/images/generations`, `/v1/audio/transcriptions`, `/v1/audio/translations`, `/v1/audio/speech`, `/v1/rerank`, `/v1/batches`, and `/v1/models`. Full OpenAI surface translation (etc) is tracked in `TODO.md`.
- Core helpers are single-step and return tool calls to the caller; the `agent` feature offers an opt-in tool loop, but it is not enabled by default.
- It is not a full UI SDK (no frontend hooks or middleware ecosystem); the `sdk` feature only provides protocol/telemetry/devtools/MCP utilities.
- Bedrock support targets Anthropic Messages-on-Bedrock; other Bedrock model families and Vertex service-account JWT flows are not covered yet.

See `PROVIDERS.md` for a pragmatic provider/capability matrix (native adapters + OpenAI-compatible
gateway coverage).

## Docs

This repo includes an `mdBook` under `docs/`.

```bash
cargo install mdbook
mdbook serve docs
```

If you don’t want to install mdBook, you can still read the Markdown directly in `docs/src`.

## Tool Schemas

For Google function calling, Ditto-LLM converts tool parameter JSON Schema into an OpenAPI-style
schema.

Contract:

- Conversion is best-effort and lossy: unsupported keywords are ignored (dropped), not errors.
- Unsupported keywords may emit `Warning::Compatibility(tool.parameters.unsupported_keywords)` to avoid silent data loss.
- `$ref` is best-effort: local refs (`#/...`) are resolved; unresolvable refs are ignored and a `Warning::Compatibility(tool.parameters.$ref)` is emitted.
- Root empty-object schemas (no properties + `additionalProperties` missing/false) are treated as
  "no parameters" and omitted.
- Boolean schemas (`true`/`false`) are treated as unconstrained schemas; at the root they are
  omitted.
- Nullable unions:
  - `type: ["string", "null"]` becomes `anyOf: [{ "type": "string" }]` + `nullable: true`
  - `anyOf: [{...}, {"type":"null"}]` becomes the same shape (single branch is flattened)
- `const` becomes `enum: [<const>]`.
- `additionalProperties` supports boolean and nested schemas.

Supported keywords (subset): `type`, `title`, `description`, `properties`, `required`, `items`,
`additionalProperties`, `enum`, `const`, `format`, `allOf`, `anyOf`, `oneOf`, `default`,
`minLength`/`maxLength`/`pattern`, `minItems`/`maxItems`/`uniqueItems`,
`minProperties`/`maxProperties`, `minimum`/`maximum`/`multipleOf`,
and `exclusiveMinimum`/`exclusiveMaximum` (number form → `minimum`/`maximum` + `exclusive* = true`).

## Examples

Examples expect provider API keys in environment variables.

```bash
cargo run --example basic
cargo run --example streaming
cargo run --example tool_calling
cargo run --example embeddings
cargo run --example openai_compatible
cargo run --example openai_compatible_embeddings
cargo run --example multimodal --features base64 -- ./image.png ./doc.pdf
cargo run --example batches --features batches -- ./requests.jsonl
```

## Gateway (optional)

Run the HTTP gateway (feature `gateway`):

```bash
cargo run --features gateway --bin ditto-gateway -- ./gateway.json --listen 0.0.0.0:8080
```

Backends are configured in `gateway.json` (OpenAI-compatible upstreams + injected headers/query params, e.g. `Authorization` and Azure-style `api-version`):

```json
{
  "backends": [
    {
      "name": "primary",
      "base_url": "https://api.openai.com/v1",
      "max_in_flight": 64,
      "timeout_seconds": 60,
      "headers": { "authorization": "Bearer ${OPENAI_API_KEY}" },
      "query_params": {}
    }
  ],
  "virtual_keys": [],
  "router": { "default_backend": "primary", "rules": [] }
}
```

`backends[].max_in_flight` optionally caps concurrent in-flight proxy requests per backend (rejects with HTTP 429 + OpenAI-style error code `inflight_limit_backend`).
`backends[].timeout_seconds` optionally overrides the backend request timeout in seconds (default: 300s).

Gateway config supports `${ENV_VAR}` interpolation in backend `base_url`/`headers`/`query_params`, backend `provider_config` fields (e.g. `base_url`/`http_headers`/`http_query_params`), and `virtual_keys[].token` (expanded at startup via the process env or `--dotenv`).

Translation backends (feature `gateway-translation`) can be configured with `provider` + `provider_config` (same shape as `ProviderConfig`):

```json
{
  "backends": [
    {
      "name": "anthropic",
      "provider": "anthropic",
      "provider_config": {
        "auth": { "type": "api_key_env", "keys": ["ANTHROPIC_API_KEY"] },
        "default_model": "claude-3-5-sonnet-20241022"
      }
    }
  ],
  "virtual_keys": [],
  "router": { "default_backend": "anthropic", "rules": [] }
}
```

For OpenAI-compatible upstreams, `provider` can be `openai-compatible`/`openai_compatible` or a LiteLLM-style alias (e.g. `groq`, `mistral`, `deepseek`, `qwen`, `together`, `fireworks`, `xai`, `perplexity`, `openrouter`, `ollama`, `azure`).

Routing (optional):

- `router.default_backends`: weighted primary selection (seeded by `x-request-id` when proxying)
- `router.rules[].backends`: per-model-prefix weighted backends (falls back to `router.default_backend` if empty)
- If multiple backends are selected, the OpenAI-compatible proxy will fall back to the next backend on network errors.
- With `--features gateway-routing-advanced`, proxying can also use retry/circuit breaker/active health checks (`--proxy-retry*` / `--proxy-circuit-breaker*` / `--proxy-health-check*`).

Endpoints:

- OpenAI-compatible proxy (passthrough): `ANY /v1/*` (e.g. `POST /v1/responses`, `POST /v1/chat/completions`, `GET /v1/models`).
  - If `virtual_keys` is non-empty, requests must include `Authorization: Bearer <virtual_key>` (or `x-ditto-virtual-key` / `x-api-key`).
  - If `virtual_keys` is non-empty, the client `Authorization` header is treated as a virtual key and is not forwarded upstream; the backend `headers` are applied instead.
  - If the upstream does **not** implement `POST /v1/responses` (returns 404/405/501), Ditto will fall back to `POST /v1/chat/completions` and return a best-effort Responses-like response/stream (adds `x-ditto-shim: responses_via_chat_completions`).
- OpenAI-compatible translation (feature `gateway-translation`): `GET /v1/models`, `GET /v1/models/*`, `POST /v1/chat/completions`, `POST /v1/completions`, `POST /v1/responses`, `POST /v1/responses/compact`, `POST /v1/embeddings`, `POST /v1/moderations`, `POST /v1/images/generations`, `POST /v1/audio/transcriptions`, `POST /v1/audio/translations`, `POST /v1/audio/speech`, `POST /v1/rerank`, and `/v1/batches` can be served by a backend with `provider` configured (adds `x-ditto-translation: <provider>`).
- Control-plane demo endpoint: `POST /v1/gateway` (JSON `GatewayRequest`; accepts `Authorization: Bearer <virtual_key>`).
- `GET /health`
- `GET /metrics`
- `GET /admin/keys` (admin token via `Authorization` or `x-admin-token` if configured). Redacts tokens unless `?include_tokens=true`.
- `POST /admin/keys` and `PUT|DELETE /admin/keys/:id` (requires the write admin token).
- `POST /admin/proxy_cache/purge` (requires the write admin token and `--proxy-cache`; body can be `{ \"cache_key\": \"...\" }` or `{ \"all\": true }`).
- `GET /admin/backends` and `POST /admin/backends/:name/reset` (reset requires the write admin token and `--features gateway-routing-advanced`).

CLI options:

- `--listen HOST:PORT` (or `--addr HOST:PORT`) sets the bind address (default: `127.0.0.1:8080`).
- `--dotenv PATH` loads a dotenv file (KEY=VALUE) for `${ENV_VAR}` interpolation and provider auth env lookups.
- `--admin-token TOKEN` enables `/admin/*` endpoints (write admin token).
- `--admin-token-env ENV` loads the write admin token from env (works with `--dotenv`).
- `--admin-read-token TOKEN` enables `/admin/*` read-only endpoints.
- `--admin-read-token-env ENV` loads the read-only admin token from env (works with `--dotenv`).
- `--backend name=url` adds/overrides a backend for `POST /v1/gateway` (the backend is a URL that accepts `GatewayRequest` JSON and returns `GatewayResponse` JSON).
- `--upstream name=base_url` adds/overrides an OpenAI-compatible upstream backend (in addition to `gateway.json`).
- `--state PATH` enables persistence for admin virtual-key mutations (writes a `GatewayStateFile` JSON with `virtual_keys`; if the file exists it is loaded on startup, otherwise it is created from `gateway.json`).
- `--sqlite PATH` enables persistence for admin virtual-key mutations in a sqlite file (requires `--features gateway-store-sqlite`; loaded on startup; cannot be combined with `--state`).
- `--redis URL` enables redis persistence (requires `--features gateway-store-redis`).
- `--redis-env ENV` loads the redis URL from env (works with `--dotenv`; requires `--features gateway-store-redis`).
- `--redis-prefix PREFIX` sets the redis key prefix (requires `--features gateway-store-redis` and `--redis`/`--redis-env`).
- `--json-logs` emits JSON log records to stderr.
- `--proxy-max-in-flight N` limits concurrent in-flight proxy requests (rejects with 429 when exceeded).
- `--proxy-cache` enables a best-effort cache for non-streaming OpenAI-compatible responses (requires `--features gateway-proxy-cache`). When combined with `--redis`, responses are also cached in Redis (shared across instances).
- `--proxy-cache-ttl SECS` sets the proxy cache TTL (implies `--proxy-cache`).
- `--proxy-cache-max-entries N` sets the in-memory proxy cache capacity (implies `--proxy-cache`).
- `--proxy-cache-max-body-bytes N` sets the maximum cached body size per entry (implies `--proxy-cache`).
- `--proxy-cache-max-total-body-bytes N` sets the in-memory total cached body budget (implies `--proxy-cache`).
- `--proxy-retry` enables retry on retryable statuses (requires `--features gateway-routing-advanced`).
- `--proxy-retry-status-codes CODES` overrides retry status codes (comma-separated; implies `--proxy-retry`).
- `--proxy-retry-max-attempts N` sets max retry attempts (implies `--proxy-retry`).
- `--proxy-circuit-breaker` enables a simple circuit breaker (requires `--features gateway-routing-advanced`).
- `--proxy-cb-failure-threshold N` sets circuit breaker failure threshold (implies `--proxy-circuit-breaker`).
- `--proxy-cb-cooldown-secs SECS` sets circuit breaker cooldown seconds (implies `--proxy-circuit-breaker`).
- `--proxy-health-checks` enables active health checks (requires `--features gateway-routing-advanced`).
- `--proxy-health-check-path PATH` overrides the health check request path (implies `--proxy-health-checks`; default: `/v1/models`).
- `--proxy-health-check-interval-secs SECS` sets health check interval seconds (implies `--proxy-health-checks`).
- `--proxy-health-check-timeout-secs SECS` sets health check timeout seconds (implies `--proxy-health-checks`).
- `--pricing-litellm PATH` loads LiteLLM-style pricing JSON for cost budgets (requires `--features gateway-costing`).
- `--prometheus-metrics` enables a Prometheus metrics endpoint (requires `--features gateway-metrics-prometheus`).
- `--prometheus-max-key-series N` limits per-key series cardinality (implies `--prometheus-metrics`).
- `--prometheus-max-model-series N` limits per-model series cardinality (implies `--prometheus-metrics`).
- `--prometheus-max-backend-series N` limits per-backend series cardinality (implies `--prometheus-metrics`).
- `--prometheus-max-path-series N` limits per-path series cardinality (implies `--prometheus-metrics`).
- `--devtools PATH` enables JSONL request/response logging (requires `--features gateway-devtools`).
- `--otel` enables OpenTelemetry tracing export via OTLP (requires `--features gateway-otel`).
- `--otel-endpoint URL` overrides the OTLP endpoint (implies `--otel`).
- `--otel-json` enables JSON formatted tracing logs (implies `--otel`).

Response headers:

- `x-ditto-backend`: which backend handled the request
- `x-ditto-request-id`: request id (uses incoming `x-request-id` or generates one)
- `x-ditto-cache`: `hit` when served from the optional proxy cache
- `x-ditto-cache-key`: cache key for the optional proxy cache (when enabled and cacheable)
- `x-ditto-cache-source`: `memory` or `redis` when `x-ditto-cache=hit`
- `x-ditto-shim`: present when `POST /v1/responses` is shimmed via `POST /v1/chat/completions`
- `x-ditto-translation`: present when a translation backend handled the request

## Stream Collection

If you want to consume a streaming response but still produce a final unified `GenerateResponse`,
use `collect_stream`:

```rust
use ditto_llm::{collect_stream, GenerateRequest, LanguageModel};

let stream = llm.stream(GenerateRequest::from(messages)).await?;
let collected = collect_stream(stream).await?;
println!("{}", collected.response.text());
```

## Text (generateText / streamText)

Single-step text helpers (no tool execution loop):

```rust
use ditto_llm::{GenerateRequest, LanguageModelTextExt};

let out = llm.generate_text(GenerateRequest::from(messages)).await?;
println!("{}", out.text);
```

Streaming:

```rust
use futures_util::StreamExt;
use ditto_llm::{GenerateRequest, LanguageModelTextExt};

let (handle, mut text_stream) = llm
    .stream_text(GenerateRequest::from(messages))
    .await?
    .into_text_stream();
while let Some(delta) = text_stream.next().await {
    print!("{}", delta?);
}
let final_text = handle.final_text()?.unwrap();
println!("\nfinal={final_text}");
```

## Structured Output (generateObject / streamObject)

Use `LanguageModelObjectExt` to request structured output (AI SDK-style `generateObject` / `streamObject`).

Defaults (`ObjectOptions::default()`):

- `strategy = Auto`:
  - `openai` → JSON Schema via `response_format` (native)
  - other providers (incl. `openai-compatible`) → tool-call enforced JSON (wraps output under `{"value": ...}`)
  - always falls back to extracting JSON from text if needed
- `output = Object` (top-level object)

```rust
use ditto_llm::{GenerateRequest, JsonSchemaFormat, LanguageModelObjectExt, Message};
use serde_json::json;

let schema = JsonSchemaFormat {
    name: "recipe".to_string(),
    schema: json!({ "type": "object" }),
    strict: None,
};

let out = llm
    .generate_object_json(GenerateRequest::from(vec![Message::user("hi")]), schema)
    .await?;

println!("{}", out.object);
```

Streaming (partial objects):

```rust
use futures_util::StreamExt;

let (handle, mut partial_object_stream) = llm
    .stream_object(GenerateRequest::from(messages), schema)
    .await?
    .into_partial_stream();
while let Some(partial) = partial_object_stream.next().await {
    println!("{:?}", partial?);
}
let final_obj = handle.final_json()?.unwrap();
println!("{final_obj}");
```

Streaming arrays (AI SDK `elementStream`):

```rust
use ditto_llm::{ObjectOptions, ObjectOutput};
use futures_util::StreamExt;

let mut result = llm
    .stream_object_with(
        GenerateRequest::from(messages),
        schema, // schema for a single element; ditto wraps it as {type:"array", items: ...}
        ObjectOptions {
            output: ObjectOutput::Array,
            ..ObjectOptions::default()
        },
    )
    .await?;

while let Some(element) = result.element_stream.next().await {
    println!("element = {}", element?);
}
```

## Streaming Cancellation

If you need an explicit abort handle (instead of relying on drop semantics), wrap the stream:

```rust
use ditto_llm::{abortable_stream, GenerateRequest, LanguageModel};

let stream = llm.stream(GenerateRequest::from(messages)).await?;
let abortable = abortable_stream(stream);
abortable.handle.abort();
```

## Embeddings

`EmbeddingModelExt` provides AI SDK-style aliases:

```rust
use ditto_llm::EmbeddingModelExt;

let vectors = embeddings.embed_many(vec!["hello".to_string(), "world".to_string()]).await?;
let one = embeddings.embed_one("hi".to_string()).await?;
```

## Custom HTTP Client

Providers accept a custom `reqwest::Client` so you can configure timeouts, proxies, and default
headers (e.g. enterprise gateways):

```rust
let http = reqwest::Client::builder().build()?;
let llm = ditto_llm::OpenAI::new(api_key).with_http_client(http);
```

When building providers from config, you can also set default headers via
`ProviderConfig.http_headers`.

## Provider Auth (Custom Headers / Query Params)

Providers apply their standard auth headers by default (OpenAI/OpenAI-compatible: bearer token;
Anthropic: `x-api-key`; Google: `x-goog-api-key`).

If you need a non-standard auth header (e.g. Azure / enterprise gateways), use:

```toml
auth = { type = "http_header_env", header = "api-key", keys = ["AZURE_OPENAI_API_KEY"] }
```

If your gateway expects auth in a query param (e.g. `...?api_key=...`), use:

```toml
auth = { type = "query_param_env", param = "api_key", keys = ["GATEWAY_API_KEY"] }
```

## Provider Query Params (Optional)

If your provider requires additional fixed query params on every request (e.g. Azure OpenAI
`api-version`), set `ProviderConfig.http_query_params`:

```toml
base_url = "https://{resource}.openai.azure.com/openai/deployments/{deployment}"
http_query_params = { "api-version" = "2024-02-01" }
auth = { type = "http_header_env", header = "api-key", keys = ["AZURE_OPENAI_API_KEY"] }
```

## Provider Options (Per Provider)

Requests that support `provider_options` accept either:

- **Legacy (flat)**: a single JSON object applied to the current provider.
- **Bucketed**: a JSON object keyed by provider id (optionally with a `"*"` default bucket).

Bucketed example:

```json
{
  "provider_options": {
    "*": { "parallel_tool_calls": false },
    "openai": { "reasoning_effort": "high" },
    "openai-compatible": { "response_format": { "type": "json_schema", "json_schema": { "name": "answer", "schema": { "type": "object" } } } }
  }
}
```

Precedence is `"*"` (base) → provider bucket (override). Provider ids are: `openai`,
`openai-compatible` (also accepts `openai_compatible` as an alias key), `anthropic`, `google`,
`cohere`, `bedrock`, `vertex`.

## File Upload (Optional)

If you want to send PDFs via `file_id` (instead of inlining base64), OpenAI and OpenAI-compatible
providers expose a small upload helper:

```rust
let file_id = llm.upload_file("doc.pdf", pdf_bytes).await?;
```

## Development

Enable repo-local git hooks:

```bash
git config core.hooksPath githooks
```

This enforces Conventional Commits and requires each commit to include `CHANGELOG.md`.

### Integration Tests (Optional)

Enable the `integration` feature and set real credentials:

- OpenAI Responses: `OPENAI_API_KEY` + `OPENAI_MODEL`
- OpenAI-compatible: `OPENAI_COMPAT_BASE_URL` + `OPENAI_COMPAT_MODEL` (+ `OPENAI_COMPAT_API_KEY` optional)

Then run:

```bash
cargo test --all-features
```
