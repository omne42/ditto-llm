# Feature Flags

Ditto-LLM now exposes two public feature namespaces:

- `provider-*`: upstream provider packs
- `cap-*`: user-visible capability packs

Legacy feature names such as `openai`, `openai-compatible`, `streaming`, and `embeddings` still exist for compatibility, but they are no longer the documented primary interface.

## Default Build

The default build is intentionally small:

- `provider-openai-compatible`
- `cap-llm`

That means the default crate targets the minimal OpenAI-compatible LLM path:

- text generation,
- streaming text output,
- basic tool-calling strategy.

It does not imply official OpenAI, gateway/server mode, or non-LLM endpoints.

## Public Provider Packs

Current public provider packs:

- `provider-openai-compatible`
- `provider-openai`
- `provider-anthropic`
- `provider-google`
- `provider-cohere`
- `provider-bedrock`
- `provider-vertex`
- `provider-bailian`
- `provider-deepseek`
- `provider-doubao`
- `provider-hunyuan`
- `provider-kimi`
- `provider-minimax`
- `provider-openrouter`
- `provider-qianfan`
- `provider-xai`
- `provider-zhipu`

Notes:

- Some provider packs currently reuse a shared protocol-family implementation.
- A provider pack being present does not mean every advertised capability is implemented yet; runtime capability checks still matter.

## Public Capability Packs

Current public capability packs:

- `cap-llm`
- `cap-embedding`
- `cap-image-generation`
- `cap-image-edit`
- `cap-audio-transcription`
- `cap-audio-speech`
- `cap-moderation`
- `cap-rerank`
- `cap-batch`
- `cap-realtime`

Rule:

- `cap-llm` includes the legacy `streaming` and `tools` toggles.
- There is no separate documented `cap-streaming`; streaming is part of the LLM baseline.

## Recommended Combinations

- Default SDK core: `cargo add ditto-llm` or `--features provider-openai-compatible,cap-llm`
- Official OpenAI LLM: `--no-default-features --features provider-openai,cap-llm`
- Anthropic LLM: `--no-default-features --features provider-anthropic,cap-llm`
- Google LLM: `--no-default-features --features provider-google,cap-llm`
- OpenAI-compatible embeddings: `--no-default-features --features provider-openai-compatible,cap-embedding`
- Official OpenAI multimodal/image/audio surface: add the matching capability packs on top of `provider-openai`
- Gateway core: `--no-default-features --features provider-openai-compatible,cap-llm,gateway`

## Capability-Gated Provider Packs

- DeepSeek direct LLM: `--no-default-features --features provider-deepseek,cap-llm`
- Anthropic direct LLM: `--no-default-features --features provider-anthropic,cap-llm`
- Google direct LLM: `--no-default-features --features provider-google,cap-llm`
- Google direct embeddings: `--no-default-features --features provider-google,cap-embedding`

Runtime notes:

- `provider-deepseek` has a dedicated catalog/runtime spec, but today it intentionally reuses the OpenAI-compatible LLM runtime path. Non-LLM builders are rejected explicitly.
- `provider-anthropic` is currently an LLM-only runtime pack. Non-LLM builders are rejected explicitly instead of silently pretending support.
- `provider-google` exposes runtime-implemented `llm`, `embedding`, `image.generation`, and `realtime` when the corresponding capability packs are enabled.
- Google `video.generation` is also runtime-implemented today behind the internal `videos` capability toggle; its public `cap-*` promotion is a separate API/feature-pack decision.

## Invalid Or Misleading Combinations

Cargo cannot enforce all semantic constraints, but these combinations are intentionally discouraged:

- capability pack without a provider pack: compiles poorly or is not useful
- provider pack without the capability pack you intend to call: may compile, but the endpoint is not part of the supported surface
- assuming `cap-realtime` means ready-to-use runtime support everywhere: it is only a capability namespace, not a blanket implementation guarantee
- assuming legacy feature names describe the preferred public API: they do not

## Gateway Features

Gateway remains opt-in and orthogonal to provider/capability packs:

- `gateway`
- `gateway-translation`
- `gateway-proxy-cache`
- `gateway-devtools`
- `gateway-store-redis`
- `gateway-store-sqlite`
- `gateway-store-postgres`
- `gateway-store-mysql`
- `gateway-routing-advanced`
- `gateway-metrics-prometheus`
- `gateway-costing`
- `gateway-tokenizer`
- `gateway-otel`

## Compatibility Notes

Legacy/internal feature names are retained because large parts of the codebase still use them in `cfg(feature = ...)` checks. The migration plan is:

1. public docs and examples move to `provider-*` / `cap-*`
2. runtime and registry work moves to capability-aware/provider-aware construction
3. old names remain only as compatibility aliases until downstream callers are updated
