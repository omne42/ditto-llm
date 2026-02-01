# Provider / Model / Request

## Provider

Ditto 里“provider”是一个字符串标识，用于选择适配器与默认行为。

常见值：

- `openai`
- `anthropic`
- `google`
- `openai-compatible`（以及 LiteLLM-style aliases：`azure` / `deepseek` / `qwen` / `groq` / `mistral` / `openrouter` / ...）

## Model

`model` 通常是 provider 内部的模型 id。Ditto 会尽量把它原样传递，但在 Gateway 场景可能会被 `model_map` 重写。

## Request / Response

SDK 的核心请求/响应类型：

- `GenerateRequest` / `GenerateResponse`
- `StreamResult` / `StreamChunk`
- `Tool` / `ToolChoice` / `ContentPart`
- `Usage` / `Warning`

建议结合「SDK」章节的示例阅读。
