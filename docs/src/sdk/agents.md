# Agents（Tool Loop）

本页对标 AI SDK 的 “Agents / tool calling loop” 思路：当你需要 **多步工具调用**（而不仅是一次 tool call），Ditto 提供一个可选的、可控的工具循环实现：`ToolLoopAgent`（feature `agent`）。

> 默认的 `generate_text` / `generate_object_json` / `generate` 都是“单次请求”：不会自动执行工具、不会自动循环。

实现位置：`src/agent/tool_loop.rs`、`src/agent/types.rs`、`src/agent/toolbox/*`。

---

## 1) 何时应该用 Tool Loop？

适合：

- 你希望模型可以“调用工具 → 看到结果 → 再决定下一步”（最多 N 步）
- 你希望有明确的 `max_steps` 与 `stop_when`（对标 AI SDK 的 `maxSteps` / `stopWhen`）
- 你希望在执行危险工具前做审批/拦截

不适合：

- 你只想要一次工具调用（直接用「工具调用（Tool Calling）」页里手写两轮示例即可）
- 你把工具暴露给不可信输入但又没有隔离/审批（这是安全事故高发区）

---

## 2) 最小示例：max_steps + stop_when

启用 features：

- `agent`（会带上一个参考工具箱执行器：基于 `safe-fs-tools` 的文件系统工具、受控 shell 执行等；`safe-fs-tools` 会由 Cargo 自动拉取）

示例（伪代码风格，展示结构）：

```rust
use ditto_llm::{
    agent::{ToolLoopAgent, ToolboxExecutor},
    Message, OpenAI,
};

#[tokio::main]
async fn main() -> ditto_llm::Result<()> {
    let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
        ditto_llm::DittoError::InvalidResponse("missing OPENAI_API_KEY".into())
    })?;
    let llm = OpenAI::new(api_key).with_model("gpt-4o-mini");

    // 只允许在某个 root 下读写文件（建议给临时目录/工作区，而不是整个磁盘）
    let executor = ToolboxExecutor::new("./sandbox")?;

    let agent = ToolLoopAgent::new(llm, executor)
        .with_max_steps(8)
        .with_stop_when(|state| {
            // 例：如果模型已经给出最终文本（没有 tool calls），就停止
            state.last_tool_calls.is_empty()
        });

    let req = vec![
        Message::system("You are a helpful assistant. Use tools when needed."),
        Message::user("Read ./sandbox/input.txt and summarize it."),
    ]
    .into();

    let outcome = agent.run(req).await?;
    println!("stop_reason={:?}", outcome.stop_reason);
    println!("final_text={}", outcome.final_response.text());
    Ok(())
}
```

---

## 3) 审批/拦截：with_approval

Tool loop 默认会“批准所有工具调用”。生产中强烈建议加审批钩子：

- allowlist 工具名
- 对参数做长度/路径约束
- 对高风险工具（shell / http_fetch / 写文件）加人工或策略审批

`with_approval` 允许你针对每个 `ToolCall` 做决策：

- `Approve`
- `Deny { reason }`
- `Result(result)`（直接注入工具结果，不执行实际工具）

---

## 4) 安全边界（强烈建议）

如果你要把 tool loop 用在服务端：

- **隔离 root**：文件系统工具必须限定在工作目录（不要给 `/`）。
- **禁用或收紧 shell**：只允许少量白名单 program，限制 cwd/timeout。
- **限制网络**：`http_fetch` 需要域名 allowlist + 响应大小上限。
- **审计与脱敏**：日志里不要记录敏感文件内容与 token。

> “工具调用 = 执行不可信输入” 这件事本质上比 LLM 本身更危险；Ditto 只是提供可控的骨架，不替你做安全兜底。

下一步：

- 「工具调用（Tool Calling）」：工具 schema 与单次调用的基础形状
- 「SDK → 错误处理」：如何在 loop 中处理失败/超时/降级
