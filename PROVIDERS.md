# Provider Coverage (Ditto-LLM vs AI SDK)

Ditto-LLM’s design mirrors the AI SDK approach: unify **semantics** (generate/stream/tools/etc.),
not HTTP endpoints.

This file tracks coverage using two paths:

1. **Native adapters**: direct provider APIs (best UX + full-fidelity behavior).
2. **OpenAI-compatible adapters**: any provider reachable via OpenAI-compatible gateways (e.g.
   LiteLLM) or native OpenAI-compatible APIs.

## Capability Matrix (high level)

| Capability | Unified Trait/Type | Native Providers | OpenAI-compatible Providers |
| --- | --- | --- | --- |
| Chat generation | `LanguageModel::{generate,stream}` | OpenAI `/responses`, Anthropic `/messages`, Google `generateContent` | `/chat/completions` |
| Tools | `Tool` / `ToolChoice` / `ContentPart::ToolCall` | OpenAI/Anthropic/Google | yes (depends on upstream) |
| JSON Schema output | `ProviderOptions.response_format` | OpenAI `/responses` | pass-through (depends on upstream) |
| Embeddings | `EmbeddingModel::embed` | OpenAI, Google, Cohere | `/embeddings` |
| Images | `ImageGenerationModel::generate` | OpenAI | `/images/generations` |
| Audio | `AudioTranscriptionModel` / `SpeechModel` | OpenAI | `/audio/*` |
| Moderations | `ModerationModel::moderate` | OpenAI | `/moderations` |
| Rerank | `RerankModel::rerank` | Cohere | (gateway-dependent) |
| Batches | (planned) | — | (planned) |

## Provider Coverage (pragmatic)

| Provider (AI SDK) | Ditto-LLM path | Notes |
| --- | --- | --- |
| OpenAI | Native | `/responses` + streaming/tools; also embeddings/images/audio/moderations (feature-gated) |
| OpenAI-compatible | Native adapter | Use for LiteLLM / DeepSeek / Qwen / Groq / Mistral / Together / Fireworks / xAI / Perplexity, etc. |
| Anthropic | Native | `/messages` + streaming/tools |
| Google | Native | `generateContent` + streaming/tools; embeddings (feature-gated) |
| Cohere | Native (partial) | embeddings + rerank; chat adapter TBD |
| Azure OpenAI | OpenAI-compatible | Needs `http_query_params = { \"api-version\" = \"...\" }` + `api-key` header |
| Amazon Bedrock | Gateway-first | Native SigV4 adapter TBD; use a gateway for now |
| Google Vertex | Gateway-first | Native auth adapter TBD; use a gateway for now |

## Feature Bundles

- `--features all`: all providers + all optional endpoint traits
- `--features all-providers`: all provider adapters
- `--features all-capabilities`: all optional endpoint traits

