# Yunwu Gemini Native（Bearer + Streaming）

## 结论

Yunwu 的 Gemini native 入口不要照搬官方 Google GenAI 的 `query_param_env` 写法。

- 官方 Google GenAI 常见鉴权：`?key=...`
- Yunwu Gemini native 实测可工作的鉴权：`Authorization: Bearer <token>`
- `:streamGenerateContent?alt=sse` 返回的是标准 `text/event-stream`

如果把 Yunwu 误配成 `query_param_env`：

- `generateContent` 可能长时间无首字节返回或直接超时
- `streamGenerateContent` 也可能无首字节返回，表现为“Gemini 不能流式”

这不是 Ditto 的 SSE 解析格式问题。Yunwu 的流式回包实测是两帧：

1. 第一帧带 `candidates[0].content.parts[0].text`
2. 第二帧带 `thoughtSignature`、`finishReason: "STOP"`、`usageMetadata`

## 推荐 ProviderConfig

```toml
[openai]
provider = "google.providers.yunwu"
model = "gemini-3.1-pro-preview"

[google.providers.yunwu]
base_url = "https://yunwu.ai/v1beta"
default_model = "gemini-3.1-pro-preview"
upstream_api = "gemini_generate_content"
normalize_to = "openai_chat_completions"
normalize_endpoint = "/v1/chat/completions"

[google.providers.yunwu.auth]
type = "http_header_env"
header = "Authorization"
prefix = "Bearer "
keys = ["YUNWU_API_KEY"]
```

## 回归要求

- Omne / Ditto 的 Yunwu Gemini 样例不要再使用 `query_param_env`
- 至少保留一个回归测试，覆盖：
  - `HttpHeaderEnv { header = "Authorization", prefix = "Bearer " }`
  - `streamGenerateContent`
  - “第一帧文本 + 第二帧 thoughtSignature/usage/STOP” 的 SSE 序列
