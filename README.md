# ditto-llm

Standalone Rust crate extracted from CodePM.

Ditto-LLM is a small Rust SDK that provides a unified interface for calling multiple LLM providers.

Current scope:

- Unified types + traits: `LanguageModel` / `EmbeddingModel`, `Message`/`ContentPart`, `Tool`, `StreamChunk`, `Warning`.
- Providers:
  - OpenAI Responses API (generate + SSE streaming) and embeddings
  - OpenAI-compatible Chat Completions (LiteLLM / DeepSeek / Qwen / etc.)
  - Anthropic Messages API (generate + SSE streaming)
  - Google GenAI (generate + SSE streaming) and embeddings
- Provider profile config and model discovery (`ProviderConfig` / `GET /models`) for routing use-cases.

## Examples

Examples expect provider API keys in environment variables.

```bash
cargo run --example basic
cargo run --example streaming
cargo run --example tool_calling
cargo run --example embeddings
cargo run --example openai_compatible
```

## Development

Enable repo-local git hooks:

```bash
git config core.hooksPath githooks
```

This enforces Conventional Commits and requires each commit to include `CHANGELOG.md`.
