# ditto-llm

Standalone Rust crate extracted from CodePM.

Ditto-LLM is a small Rust SDK that provides a unified interface for calling multiple LLM providers.

Current scope:

- Unified types + traits: `LanguageModel` / `EmbeddingModel`, `Message`/`ContentPart`, `Tool`, `StreamChunk`, `Warning`.
- Multi-modal inputs: images + PDF documents via `ContentPart::Image` / `ContentPart::File` (provider support varies; unsupported parts emit `Warning`).
- Providers:
  - OpenAI Responses API (generate + SSE streaming) and embeddings
  - OpenAI-compatible Chat Completions (LiteLLM / DeepSeek / Qwen / etc.) and embeddings
  - Anthropic Messages API (generate + SSE streaming)
  - Google GenAI (generate + SSE streaming) and embeddings
- Provider profile config and model discovery (`ProviderConfig` / `GET /models`) for routing use-cases.

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
cargo run --example multimodal -- ./image.png ./doc.pdf
```

## Stream Collection

If you want to consume a streaming response but still produce a final unified `GenerateResponse`,
use `collect_stream`:

```rust
use ditto_llm::{collect_stream, GenerateRequest, LanguageModel};

let stream = llm.stream(GenerateRequest::from(messages)).await?;
let collected = collect_stream(stream).await?;
println!("{}", collected.response.text());
```

## Streaming Cancellation

If you need an explicit abort handle (instead of relying on drop semantics), wrap the stream:

```rust
use ditto_llm::{abortable_stream, GenerateRequest, LanguageModel};

let stream = llm.stream(GenerateRequest::from(messages)).await?;
let abortable = abortable_stream(stream);
abortable.handle.abort();
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
