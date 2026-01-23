# ditto-llm

Standalone Rust crate extracted from CodePM.

Ditto-LLM is a small Rust SDK that provides a unified interface for calling multiple LLM providers.

Current scope:

- Unified types + traits: `LanguageModel` / `EmbeddingModel`, `Message`/`ContentPart`, `Tool`, `StreamChunk`, `Warning`.
- Multi-modal inputs: images + PDF documents via `ContentPart::Image` / `ContentPart::File` (provider support varies; unsupported parts emit `Warning`).
- Providers:
  - OpenAI Responses API (generate + SSE streaming) and embeddings
  - OpenAI-compatible Chat Completions (LiteLLM / DeepSeek / Qwen / etc.)
  - Anthropic Messages API (generate + SSE streaming)
  - Google GenAI (generate + SSE streaming) and embeddings
- Provider profile config and model discovery (`ProviderConfig` / `GET /models`) for routing use-cases.

## Tool Schemas

For Google function calling, Ditto-LLM converts tool parameter JSON Schema into an OpenAPI-style schema.

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
cargo run --example multimodal -- ./image.png ./doc.pdf
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
