# Provider Coverage

Ditto-LLM 用 provider packs + capability packs 管理集成面，而不是默认把所有厂商与所有能力一起打包。

持续更新的 provider/capability/model completeness snapshot 见 `CATALOG_COMPLETENESS.md`。

## Default Core

默认构建只承诺一个稳定核心：

- provider pack：`provider-openai-compatible`
- capability pack：`cap-llm`

这条默认路径覆盖通用 OpenAI-compatible Chat Completions：文本生成、streaming、tool calling。

其它 provider 与能力都是显式 opt-in。

## Feature Model

### Provider packs

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

### Capability packs

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

## Provider x Capability x Feature x Status

This matrix tracks the stable public `provider-*` + `cap-*` surface.
Provider-specific runtime-only capabilities that do not yet have a public `cap-*` flag (for example `context.cache`, `video.generation`, `audio.voice_clone`, `ocr`) are tracked in `CATALOG_COMPLETENESS.md`.

| Provider pack | Capability | Required feature(s) | Runtime status | Notes |
| --- | --- | --- | --- | --- |
| `provider-openai-compatible` | LLM | `provider-openai-compatible + cap-llm` | default-core | Generic `/chat/completions`; used for LiteLLM and compatible gateways |
| `provider-openai-compatible` | Embedding | `provider-openai-compatible + cap-embedding` | implemented | OpenAI-compatible `/embeddings` |
| `provider-openai-compatible` | Image generation | `provider-openai-compatible + cap-image-generation` | implemented | OpenAI-compatible image generation |
| `provider-openai-compatible` | Image edit | `provider-openai-compatible + cap-image-edit` | implemented | OpenAI-compatible image edit |
| `provider-openai-compatible` | Audio transcription | `provider-openai-compatible + cap-audio-transcription` | implemented | OpenAI-compatible `/audio/transcriptions` |
| `provider-openai-compatible` | Audio speech | `provider-openai-compatible + cap-audio-speech` | implemented | OpenAI-compatible `/audio/speech` |
| `provider-openai-compatible` | Moderation | `provider-openai-compatible + cap-moderation` | implemented | OpenAI-compatible `/moderations` |
| `provider-openai-compatible` | Batch | `provider-openai-compatible + cap-batch` | implemented | OpenAI-compatible `/batches` |
| `provider-openai` | LLM | `provider-openai + cap-llm` | implemented | Native OpenAI `/responses` |
| `provider-openai` | Embedding | `provider-openai + cap-embedding` | implemented | Native OpenAI embeddings |
| `provider-openai` | Image generation | `provider-openai + cap-image-generation` | implemented | Native image generation |
| `provider-openai` | Image edit | `provider-openai + cap-image-edit` | implemented | Native image edit |
| `provider-openai` | Audio transcription | `provider-openai + cap-audio-transcription` | implemented | Native transcription |
| `provider-openai` | Audio speech | `provider-openai + cap-audio-speech` | implemented | Native speech |
| `provider-openai` | Moderation | `provider-openai + cap-moderation` | implemented | Native moderation |
| `provider-openai` | Batch | `provider-openai + cap-batch` | implemented | Native batches |
| `provider-openai` | Realtime | `provider-openai + cap-realtime` | implemented | Native realtime builder path |
| `provider-anthropic` | LLM | `provider-anthropic + cap-llm` | implemented | Native `/messages` |
| `provider-google` | LLM | `provider-google + cap-llm` | implemented | Native `generateContent` |
| `provider-google` | Embedding | `provider-google + cap-embedding` | implemented | Google embedding path |
| `provider-google` | Image generation | `provider-google + cap-image-generation` | implemented | Native `predict` builder path |
| `provider-google` | Realtime | `provider-google + cap-realtime` | implemented | Native live websocket session builder |
| `provider-cohere` | LLM | `provider-cohere + cap-llm` | implemented | Native chat |
| `provider-cohere` | Embedding | `provider-cohere + cap-embedding` | implemented | Native embeddings |
| `provider-cohere` | Rerank | `provider-cohere + cap-rerank` | implemented | Native rerank |
| `provider-bedrock` | LLM | `provider-bedrock + cap-llm` | implemented | Anthropic-on-Bedrock via SigV4 |
| `provider-vertex` | LLM | `provider-vertex + cap-llm` | implemented | Vertex generate/stream |
| `provider-deepseek` | LLM | `provider-deepseek + cap-llm` | implemented | Provider-specific pack over OpenAI-compatible runtime |
| `provider-kimi` | LLM | `provider-kimi + cap-llm` | implemented | Provider-specific pack over OpenAI-compatible runtime |
| `provider-openrouter` | LLM | `provider-openrouter + cap-llm` | implemented | Provider-specific pack over OpenAI-compatible runtime |
| `provider-xai` | LLM | `provider-xai + cap-llm` | implemented | Provider-specific pack over OpenAI-compatible runtime |
| `provider-xai` | Image generation | `provider-xai + cap-image-generation` | implemented | Provider-specific pack over OpenAI-compatible image runtime |
| `provider-bailian` | LLM | `provider-bailian + cap-llm` | implemented | Runtime registry matches reference catalog |
| `provider-bailian` | Embedding | `provider-bailian + cap-embedding` | implemented | Runtime registry matches reference catalog |
| `provider-bailian` | Image generation | `provider-bailian + cap-image-generation` | implemented | Runtime registry matches reference catalog |
| `provider-bailian` | Image edit | `provider-bailian + cap-image-edit` | implemented | Runtime registry matches reference catalog |
| `provider-bailian` | Audio transcription | `provider-bailian + cap-audio-transcription` | implemented | Runtime registry matches reference catalog |
| `provider-bailian` | Audio speech | `provider-bailian + cap-audio-speech` | implemented | Runtime registry matches reference catalog |
| `provider-bailian` | Rerank | `provider-bailian + cap-rerank` | implemented | Runtime registry matches reference catalog |
| `provider-doubao` | LLM | `provider-doubao + cap-llm` | implemented | Runtime registry matches reference catalog |
| `provider-doubao` | Embedding | `provider-doubao + cap-embedding` | implemented | Runtime registry matches reference catalog |
| `provider-doubao` | Image generation | `provider-doubao + cap-image-generation` | implemented | Runtime registry matches reference catalog |
| `provider-doubao` | Batch | `provider-doubao + cap-batch` | implemented | Runtime registry matches reference catalog |
| `provider-hunyuan` | LLM | `provider-hunyuan + cap-llm` | implemented | Runtime registry matches reference catalog |
| `provider-hunyuan` | Embedding | `provider-hunyuan + cap-embedding` | implemented | Runtime registry matches reference catalog |
| `provider-hunyuan` | Image generation | `provider-hunyuan + cap-image-generation` | implemented | Runtime registry matches reference catalog |
| `provider-minimax` | LLM | `provider-minimax + cap-llm` | implemented | Runtime registry matches reference catalog |
| `provider-minimax` | Image generation | `provider-minimax + cap-image-generation` | implemented | Runtime registry matches reference catalog |
| `provider-minimax` | Audio speech | `provider-minimax + cap-audio-speech` | implemented | Runtime registry matches reference catalog |
| `provider-qianfan` | LLM | `provider-qianfan + cap-llm` | implemented | Runtime registry matches reference catalog |
| `provider-qianfan` | Embedding | `provider-qianfan + cap-embedding` | implemented | Runtime registry matches reference catalog |
| `provider-qianfan` | Image generation | `provider-qianfan + cap-image-generation` | implemented | Runtime registry matches reference catalog |
| `provider-qianfan` | Image edit | `provider-qianfan + cap-image-edit` | implemented | Runtime registry matches reference catalog |
| `provider-qianfan` | Rerank | `provider-qianfan + cap-rerank` | implemented | Runtime registry matches reference catalog |
| `provider-zhipu` | LLM | `provider-zhipu + cap-llm` | implemented | Runtime registry matches reference catalog |
| `provider-zhipu` | Embedding | `provider-zhipu + cap-embedding` | implemented | Runtime registry matches reference catalog |
| `provider-zhipu` | Image generation | `provider-zhipu + cap-image-generation` | implemented | Runtime registry matches reference catalog |
| `provider-zhipu` | Audio transcription | `provider-zhipu + cap-audio-transcription` | implemented | Runtime registry matches reference catalog |
| `provider-zhipu` | Audio speech | `provider-zhipu + cap-audio-speech` | implemented | Runtime registry matches reference catalog |
| `provider-zhipu` | Rerank | `provider-zhipu + cap-rerank` | implemented | Runtime registry matches reference catalog |
| `provider-zhipu` | Realtime | `provider-zhipu + cap-realtime` | implemented | Runtime registry matches reference catalog |

## Integration Paths

This table is about runtime truth. Ditto still supports two integration styles:

1. Native adapters: direct provider APIs, highest fidelity.
2. OpenAI-compatible adapters: practical compatibility layer for unified upstreams.

Provider-specific packs such as `provider-deepseek`, `provider-kimi`, and `provider-openrouter` currently reuse the OpenAI-compatible runtime while keeping their own catalog/profile semantics explicit.

## Verification Pointers

- `cargo test --test openai_provider_capabilities --all-features -- --nocapture`
- `cargo test --test deepseek_provider_capabilities --all-features -- --nocapture`
- `cargo test --test anthropic_provider_capabilities --all-features -- --nocapture`
- `cargo test --test google_provider_capabilities --all-features -- --nocapture`
- `cargo test --test gateway_translation_custom_provider_resolution --all-features -- --nocapture`
