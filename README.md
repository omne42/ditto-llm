# ditto-llm

Standalone Rust crate extracted from CodePM.

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
  - Cohere embeddings and rerank (feature-gated)
- Batches: `BatchClient` for OpenAI/OpenAI-compatible `/batches` (feature `batches`).
- Provider profile config and model discovery (`ProviderConfig` / `GET /models`) for routing use-cases.

See `PROVIDERS.md` for a pragmatic provider/capability matrix (native adapters + OpenAI-compatible
gateway coverage).

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
cargo run --example batches --features batches -- ./requests.jsonl
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

let mut result = llm.stream_text(GenerateRequest::from(messages)).await?;
while let Some(delta) = result.text_stream.next().await {
    print!("{}", delta?);
}
let final_text = result.final_text()?.unwrap();
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

let mut result = llm.stream_object(GenerateRequest::from(messages), schema).await?;
while let Some(partial) = result.partial_object_stream.next().await {
    println!("{:?}", partial?);
}
let final_obj = result.final_json()?.unwrap();
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
`openai-compatible`, `anthropic`, `google`, `cohere`.

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
