# Node client (OpenAI-compatible)

These examples call `ditto-gateway` via the OpenAI-compatible `/v1/*` surface.

## Streaming chat completions (SSE)

```bash
export DITTO_BASE_URL=http://127.0.0.1:8080
export DITTO_VK_TOKEN=ditto-vk-...

node examples/clients/node/stream_chat_completions.mjs
```
