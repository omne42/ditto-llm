# 结构化输出：generate_object_json

Ditto 的结构化输出对标 AI SDK 的 `generateObject` / `streamObject`，但把“不同 provider 的能力差异”显式暴露出来。

核心 API 来自 `LanguageModelObjectExt`：

- `generate_object_json(request, schema) -> GenerateObjectResponse<Value>`
- `generate_object<T>(request, schema) -> GenerateObjectResponse<T>`
- `stream_object(request, schema) -> StreamObjectResult`
- `stream_object_with(request, schema, options) -> StreamObjectResult`

## 最小示例：生成 JSON object

```rust
use ditto_llm::{GenerateRequest, JsonSchemaFormat, LanguageModelObjectExt, Message};
use serde_json::json;

let schema = JsonSchemaFormat {
    name: "recipe".to_string(),
    schema: json!({
        "type": "object",
        "properties": {
            "title": { "type": "string" },
            "steps": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["title", "steps"]
    }),
    strict: None,
};

let out = llm
    .generate_object_json(GenerateRequest::from(vec![Message::user("Give me a recipe.")]), schema)
    .await?;

println!("{}", out.object);
println!("warnings={:?}", out.response.warnings);
```

## Strategy：Ditto 如何让不同 provider“尽量”产出结构化结果

Ditto 通过 `ObjectOptions.strategy` 控制策略（默认 `Auto`）：

- `Auto`（默认）
  - `provider == "openai"` → `NativeSchema`
  - 其他 provider：
    - 若启用了 feature `tools` → `ToolCall`（用工具调用强约束输出）
    - 否则 → `TextJson`（从文本里解析 JSON）
- `NativeSchema`
  - 通过 `ResponseFormat::JsonSchema` 注入到 `provider_options`（native 支持时最稳）
- `ToolCall`
  - 注入一个“内部工具”，让模型通过 tool call 返回 JSON（更可控，但需要 tools 支持）
- `TextJson`
  - 不强约束，只在最后从输出文本里 parse JSON（best-effort，风险最大）

无论哪种策略，Ditto 都可能在必要时回退到文本 JSON 解析，并用 `Warning` 标记发生了降级。

## Object vs Array（AI SDK elementStream 对齐）

`ObjectOptions.output` 决定顶层形状：

- `ObjectOutput::Object`（默认）
- `ObjectOutput::Array`

当你选择 `Array` 时，Ditto 会把 schema 包装为：

```json
{ "type": "array", "items": <your_schema> }
```

并在 streaming 场景提供 `element_stream`（对齐 AI SDK 的 `elementStream`）。

## Streaming：partial objects 与 element stream

```rust
use futures_util::StreamExt;
use ditto_llm::{GenerateRequest, LanguageModelObjectExt, Message};

let (handle, mut partial_object_stream) = llm
    .stream_object(GenerateRequest::from(vec![Message::user("Generate JSON.")]), schema)
    .await?
    .into_partial_stream();

while let Some(partial) = partial_object_stream.next().await {
    println!("partial={}", partial?);
}

let final_json = handle.final_json()?.unwrap();
println!("final={final_json}");
```

Streaming arrays：

```rust
use futures_util::StreamExt;
use ditto_llm::{GenerateRequest, LanguageModelObjectExt, Message, ObjectOptions, ObjectOutput};

let (handle, mut element_stream) = llm
    .stream_object_with(
        GenerateRequest::from(vec![Message::user("List items as JSON array.")]),
        schema, // schema for a single element
        ObjectOptions {
            output: ObjectOutput::Array,
            ..ObjectOptions::default()
        },
    )
    .await?
    .into_element_stream();

while let Some(element) = element_stream.next().await {
    println!("element={}", element?);
}

let _final = handle.final_summary()?;
```

## 建议与坑

- **Schema 越明确越好**：尤其是 `required` 与 `type`，可以显著降低解析失败率。
- **对错误要敏感**：`final_json()` / `final_object()` 只有在 stream 完成后才会返回结果；若解析失败会返回 `DittoError::InvalidResponse(...)`。
- **把 warnings 当成信号**：比如 “fallback to text json parsing” 的 Warning，通常意味着 provider 不支持你期望的能力或模型没遵守约束。
