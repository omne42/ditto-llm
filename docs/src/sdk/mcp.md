# MCP（Tool Schema 互转）

Ditto 在 `sdk` feature 里提供了 MCP tool schema 的轻量互转（只做结构映射，不是 MCP client/server 实现）。

实现位置：`src/sdk/mcp.rs`。

如果你需要 **Gateway 级别** 的 MCP proxy（`/mcp*`）与 MCP tools 集成（`/v1/chat/completions` 的 `tools: [{"type":"mcp", ...}]`），请看「Gateway → MCP Gateway（/mcp + tools）」。

---

## 1) 为什么需要互转？

在工具调用生态里，常见的两种 schema 形状：

- Ditto：`Tool { name, description, parameters, strict }`
- MCP：`McpTool { name, description, inputSchema }`

如果你需要把 Ditto 的工具定义复用到 MCP（或反过来），可以用本页的转换函数。

---

## 2) 从 Ditto Tool 转 MCP Tool

```rust
use ditto_llm::sdk::mcp::to_mcp_tool;

let mcp = to_mcp_tool(&tool);
```

---

## 3) 从 MCP Tool 转 Ditto Tool

```rust
use ditto_llm::sdk::mcp::from_mcp_tool;

let tool = from_mcp_tool(&mcp);
```

注意：

- `McpTool` 没有 `strict` 字段，因此从 MCP 转回 Ditto 时 `strict` 会是 `None`。
