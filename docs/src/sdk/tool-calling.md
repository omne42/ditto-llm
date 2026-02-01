# 工具调用（Tool Calling）

Ditto 的工具调用是 “AI SDK Core” 常见用法的 Rust 化：把工具描述为 `Tool`（JSON Schema 参数），把模型输出的 tool calls 暴露为 `ContentPart::ToolCall`，由调用方决定是否、以及如何执行工具。

> 重要：Ditto 的默认 helper（`generate_text` / `generate_object_json`）都是单次请求，不会自动执行工具循环。  
> 如果你需要自动 loop（多步工具调用），请看「SDK → Agents（Tool Loop）」：它提供 `ToolLoopAgent`（max_steps / stop_when / approval hook），语义上对标 AI SDK 的 `maxSteps` / `stopWhen`。

## 定义 Tool

`Tool.parameters` 使用 JSON Schema（以 `serde_json::Value` 表示）：

```rust
use ditto_llm::Tool;
use serde_json::json;

let tools = vec![Tool {
    name: "add".to_string(),
    description: Some("Add two integers.".to_string()),
    parameters: json!({
        "type": "object",
        "properties": {
            "a": { "type": "integer" },
            "b": { "type": "integer" }
        },
        "required": ["a", "b"]
    }),
    strict: Some(true),
}];
```

## 发起带 tools 的请求

```rust
use ditto_llm::{ContentPart, GenerateRequest, LanguageModel, Message, ToolChoice};

let request = GenerateRequest {
    messages: vec![
        Message::system("You are a helpful assistant. Use tools when appropriate."),
        Message::user("Compute 40 + 2 using the add tool."),
    ],
    tools: Some(tools),
    tool_choice: Some(ToolChoice::Required),
    ..GenerateRequest::from(Vec::new())
};

let response = llm.generate(request).await?;

for part in &response.content {
    if let ContentPart::ToolCall { id, name, arguments } = part {
        eprintln!("tool_call id={id} name={name} args={arguments}");
    }
}
```

### ToolChoice 的含义

- `Auto`：模型自行决定是否调用工具
- `None`：禁止工具调用（即使你提供了 tools）
- `Required`：强制模型必须调用工具（常用于 “tool-call enforced JSON”）
- `Tool { name }`：强制调用某个工具

不同 provider 对 `tool_choice` 的支持程度可能不同；不支持时 Ditto 会尽量降级并给出 `Warning`。

## 手写一个最小 Tool Loop（两轮）

典型流程是：

1) 发送请求（带 tools）  
2) 从模型输出中提取 tool calls  
3) 执行工具，产出 `Message::tool_result(...)`  
4) 再发一次请求（把 tool call 与 tool result 作为上下文交给模型）  

参考 `examples/tool_calling.rs`，一个最小可跑的两轮 loop 形如：

```rust
use ditto_llm::{ContentPart, GenerateRequest, LanguageModel, Message};
use serde_json::json;

fn add(arguments: &serde_json::Value) -> serde_json::Value {
    let a = arguments.get("a").and_then(|v| v.as_i64()).unwrap_or(0);
    let b = arguments.get("b").and_then(|v| v.as_i64()).unwrap_or(0);
    json!({ "result": a + b })
}

let response = llm.generate(request).await?;

let mut followup = vec![
    Message::system("You are a helpful assistant."),
    Message::user("Compute 40 + 2 using the add tool."),
];

for part in &response.content {
    if let ContentPart::ToolCall { id, name, arguments } = part {
        let output = match name.as_str() {
            "add" => add(arguments),
            _ => json!({ "error": "unknown tool" }),
        };

        followup.push(Message {
            role: ditto_llm::Role::Assistant,
            content: vec![ContentPart::ToolCall {
                id: id.clone(),
                name: name.clone(),
                arguments: arguments.clone(),
            }],
        });
        followup.push(Message::tool_result(id.clone(), output.to_string()));
    }
}

let response = llm.generate(followup.into()).await?;
println!("{}", response.text());
```

## Streaming 场景的工具调用

在 streaming 时，工具调用可能以增量形式出现：

- `StreamChunk::ToolCallStart { id, name }`
- `StreamChunk::ToolCallDelta { id, arguments_delta }`

如果你需要在 streaming 中实时执行工具，需要自行维护一个 “tool call buffer” 做增量拼接。

（提示：`StreamCollector` 在收集模式下已经实现了拼接，但它会把所有内容累计到最终响应，适合“先收集、后执行”的场景。）

## 安全建议（强烈建议阅读）

工具调用是高风险边界：模型输出的参数本质是不可信输入。

最低限度建议：

- 对工具参数做 schema 校验与长度限制（尤其是字符串）
- 对危险工具做 allowlist（shell / filesystem / network）
- 在服务端做权限隔离（root 限制、只读模式、超时、并发上限）

Ditto 的 `agent` feature 提供了一套 “可控工具执行器” 参考实现（例如 `safe-fs-tools`），但默认不启用。
