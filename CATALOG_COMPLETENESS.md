# Catalog Completeness Dashboard

Generated from the compiled builtin runtime registry plus `catalog/provider_models/*`.
For a full repo snapshot, regenerate with `cargo run -p ditto-core --all-features --bin ditto-catalog-dashboard`.

## Provider Summary

| Provider | Runtime | Reference | Models (match/ref/runtime) | Capabilities (done/planned/blocked/missing) | Validation |
| --- | --- | --- | --- | --- | --- |
| `anthropic` | yes | yes | 24/24/24 | 1/0/0/0 | ref:0 / exp:0 |
| `bailian` | yes | yes | 409/409/409 | 10/0/0/0 | ref:0 / exp:n/a |
| `deepseek` | yes | yes | 2/2/2 | 1/0/0/1 | ref:0 / exp:0 |
| `doubao` | yes | yes | 54/54/54 | 7/0/0/0 | ref:0 / exp:n/a |
| `google` | yes | yes | 21/21/21 | 5/0/0/0 | ref:0 / exp:0 |
| `hunyuan` | yes | yes | 27/27/27 | 4/0/0/0 | ref:0 / exp:n/a |
| `kimi` | yes | yes | 13/13/13 | 1/0/0/0 | ref:0 / exp:n/a |
| `minimax` | yes | yes | 20/20/20 | 7/0/0/1 | ref:0 / exp:n/a |
| `openai` | yes | yes | 79/79/79 | 9/0/0/0 | ref:0 / exp:0 |
| `openai-compatible` | yes | no | 0/0/0 | 0/0/0/0 | ref:0 / exp:n/a |
| `openrouter` | yes | yes | 300/300/300 | 1/0/0/0 | ref:0 / exp:n/a |
| `qianfan` | yes | yes | 211/211/211 | 7/0/0/0 | ref:0 / exp:n/a |
| `xai` | yes | yes | 12/12/12 | 3/0/0/0 | ref:0 / exp:n/a |
| `zhipu` | yes | yes | 53/53/53 | 10/0/0/0 | ref:0 / exp:n/a |

## Provider Details

### `anthropic`

- runtime plugin: present (Anthropic)
- reference catalog: present (Anthropic)
- models: matched 24 / reference 24 / runtime 24
- capability coverage (reference scope): done 1 / planned 0 / blocked 0 / missing 0
- reference validation issues: 0
- expectation issues: 0

| Capability bucket | Entries |
| --- | --- |
| Implemented | llm |
| Planned | - |
| Blocked | - |
| Missing runtime coverage | - |
| Runtime-only capability entries | - |

- missing reference models: -
- runtime-only models: -

### `bailian`

- runtime plugin: present (Alibaba Cloud Model Studio (Bailian))
- reference catalog: present (Alibaba Cloud Model Studio (Bailian))
- models: matched 409 / reference 409 / runtime 409
- capability coverage (reference scope): done 10 / planned 0 / blocked 0 / missing 0
- reference validation issues: 0
- expectation issues: n/a

| Capability bucket | Entries |
| --- | --- |
| Implemented | audio.speech, audio.transcription, classification_or_extraction, embedding, image.edit, image.generation, image.translation, llm, rerank, video.generation |
| Planned | - |
| Blocked | - |
| Missing runtime coverage | - |
| Runtime-only capability entries | - |

- missing reference models: -
- runtime-only models: -

### `deepseek`

- runtime plugin: present (DeepSeek API)
- reference catalog: present (DeepSeek API)
- models: matched 2 / reference 2 / runtime 2
- capability coverage (reference scope): done 1 / planned 0 / blocked 0 / missing 1
- reference validation issues: 0
- expectation issues: 0

| Capability bucket | Entries |
| --- | --- |
| Implemented | llm |
| Planned | - |
| Blocked | - |
| Missing runtime coverage | context.cache |
| Runtime-only capability entries | - |

- missing reference models: -
- runtime-only models: -

### `doubao`

- runtime plugin: present (Volcengine Ark / Doubao)
- reference catalog: present (Volcengine Ark / Doubao)
- models: matched 54 / reference 54 / runtime 54
- capability coverage (reference scope): done 7 / planned 0 / blocked 0 / missing 0
- reference validation issues: 0
- expectation issues: n/a

| Capability bucket | Entries |
| --- | --- |
| Implemented | 3d.generation, batch, context.cache, embedding, image.generation, llm, video.generation |
| Planned | - |
| Blocked | - |
| Missing runtime coverage | - |
| Runtime-only capability entries | - |

- missing reference models: -
- runtime-only models: -

### `google`

- runtime plugin: present (Google AI for Developers)
- reference catalog: present (Google AI for Developers)
- models: matched 21 / reference 21 / runtime 21
- capability coverage (reference scope): done 5 / planned 0 / blocked 0 / missing 0
- reference validation issues: 0
- expectation issues: 0

| Capability bucket | Entries |
| --- | --- |
| Implemented | embedding, image.generation, llm, realtime, video.generation |
| Planned | - |
| Blocked | - |
| Missing runtime coverage | - |
| Runtime-only capability entries | - |

- missing reference models: -
- runtime-only models: -

### `hunyuan`

- runtime plugin: present (Tencent Hunyuan)
- reference catalog: present (Tencent Hunyuan)
- models: matched 27 / reference 27 / runtime 27
- capability coverage (reference scope): done 4 / planned 0 / blocked 0 / missing 0
- reference validation issues: 0
- expectation issues: n/a

| Capability bucket | Entries |
| --- | --- |
| Implemented | embedding, image.generation, image.question, llm |
| Planned | - |
| Blocked | - |
| Missing runtime coverage | - |
| Runtime-only capability entries | - |

- missing reference models: -
- runtime-only models: -

### `kimi`

- runtime plugin: present (Kimi by Moonshot AI)
- reference catalog: present (Kimi by Moonshot AI)
- models: matched 13 / reference 13 / runtime 13
- capability coverage (reference scope): done 1 / planned 0 / blocked 0 / missing 0
- reference validation issues: 0
- expectation issues: n/a

| Capability bucket | Entries |
| --- | --- |
| Implemented | llm |
| Planned | - |
| Blocked | - |
| Missing runtime coverage | - |
| Runtime-only capability entries | - |

- missing reference models: -
- runtime-only models: -

### `minimax`

- runtime plugin: present (MiniMax)
- reference catalog: present (MiniMax)
- models: matched 20 / reference 20 / runtime 20
- capability coverage (reference scope): done 7 / planned 0 / blocked 0 / missing 1
- reference validation issues: 0
- expectation issues: n/a

| Capability bucket | Entries |
| --- | --- |
| Implemented | audio.speech, audio.voice_clone, audio.voice_design, image.generation, llm, music.generation, video.generation |
| Planned | - |
| Blocked | - |
| Missing runtime coverage | context.cache |
| Runtime-only capability entries | - |

- missing reference models: -
- runtime-only models: -

### `openai`

- runtime plugin: present (Generic OpenAI API)
- reference catalog: present (OpenAI)
- models: matched 79 / reference 79 / runtime 79
- capability coverage (reference scope): done 9 / planned 0 / blocked 0 / missing 0
- reference validation issues: 0
- expectation issues: 0

| Capability bucket | Entries |
| --- | --- |
| Implemented | audio.speech, audio.transcription, embedding, image.edit, image.generation, llm, moderation, realtime, video.generation |
| Planned | - |
| Blocked | - |
| Missing runtime coverage | - |
| Runtime-only capability entries | batch (implemented) |

- missing reference models: -
- runtime-only models: -

### `openai-compatible`

- runtime plugin: present (Generic OpenAI-Compatible API)
- reference catalog: missing
- models: matched 0 / reference 0 / runtime 0
- capability coverage (reference scope): done 0 / planned 0 / blocked 0 / missing 0
- reference validation issues: 0
- expectation issues: n/a

| Capability bucket | Entries |
| --- | --- |
| Implemented | - |
| Planned | - |
| Blocked | - |
| Missing runtime coverage | - |
| Runtime-only capability entries | audio.speech (implemented), audio.transcription (implemented), batch (implemented), embedding (implemented), image.edit (implemented), image.generation (implemented), llm (implemented), moderation (implemented) |

- missing reference models: -
- runtime-only models: -

### `openrouter`

- runtime plugin: present (OpenRouter)
- reference catalog: present (OpenRouter)
- models: matched 300 / reference 300 / runtime 300
- capability coverage (reference scope): done 1 / planned 0 / blocked 0 / missing 0
- reference validation issues: 0
- expectation issues: n/a

| Capability bucket | Entries |
| --- | --- |
| Implemented | llm |
| Planned | - |
| Blocked | - |
| Missing runtime coverage | - |
| Runtime-only capability entries | - |

- missing reference models: -
- runtime-only models: -

### `qianfan`

- runtime plugin: present (Baidu Qianfan / Wenxin)
- reference catalog: present (Baidu Qianfan / Wenxin)
- models: matched 211 / reference 211 / runtime 211
- capability coverage (reference scope): done 7 / planned 0 / blocked 0 / missing 0
- reference validation issues: 0
- expectation issues: n/a

| Capability bucket | Entries |
| --- | --- |
| Implemented | embedding, image.edit, image.generation, llm, ocr, rerank, video.generation |
| Planned | - |
| Blocked | - |
| Missing runtime coverage | - |
| Runtime-only capability entries | - |

- missing reference models: -
- runtime-only models: -

### `xai`

- runtime plugin: present (xAI / Grok)
- reference catalog: present (xAI / Grok)
- models: matched 12 / reference 12 / runtime 12
- capability coverage (reference scope): done 3 / planned 0 / blocked 0 / missing 0
- reference validation issues: 0
- expectation issues: n/a

| Capability bucket | Entries |
| --- | --- |
| Implemented | image.generation, llm, video.generation |
| Planned | - |
| Blocked | - |
| Missing runtime coverage | - |
| Runtime-only capability entries | - |

- missing reference models: -
- runtime-only models: -

### `zhipu`

- runtime plugin: present (Zhipu AI / GLM)
- reference catalog: present (Zhipu AI / GLM)
- models: matched 53 / reference 53 / runtime 53
- capability coverage (reference scope): done 10 / planned 0 / blocked 0 / missing 0
- reference validation issues: 0
- expectation issues: n/a

| Capability bucket | Entries |
| --- | --- |
| Implemented | audio.speech, audio.transcription, audio.voice_clone, embedding, image.generation, llm, ocr, realtime, rerank, video.generation |
| Planned | - |
| Blocked | - |
| Missing runtime coverage | - |
| Runtime-only capability entries | - |

- missing reference models: -
- runtime-only models: -
