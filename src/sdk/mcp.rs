use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::Tool;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "inputSchema", default)]
    pub input_schema: Value,
}

impl McpTool {
    pub fn new(name: impl Into<String>, input_schema: Value) -> Self {
        Self {
            name: name.into(),
            description: None,
            input_schema,
        }
    }
}

pub fn to_mcp_tool(tool: &Tool) -> McpTool {
    McpTool {
        name: tool.name.clone(),
        description: tool.description.clone(),
        input_schema: tool.parameters.clone(),
    }
}

pub fn from_mcp_tool(tool: &McpTool) -> Tool {
    Tool {
        name: tool.name.clone(),
        description: tool.description.clone(),
        parameters: tool.input_schema.clone(),
        strict: None,
    }
}

pub fn to_mcp_tools(tools: &[Tool]) -> Vec<McpTool> {
    tools.iter().map(to_mcp_tool).collect()
}

pub fn from_mcp_tools(tools: &[McpTool]) -> Vec<Tool> {
    tools.iter().map(from_mcp_tool).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn mcp_tool_roundtrip() {
        let tool = Tool {
            name: "sum".to_string(),
            description: Some("add two numbers".to_string()),
            parameters: json!({
                "type": "object",
                "properties": {
                    "a": { "type": "number" },
                    "b": { "type": "number" }
                },
                "required": ["a", "b"]
            }),
            strict: None,
        };

        let mcp = to_mcp_tool(&tool);
        let back = from_mcp_tool(&mcp);
        assert_eq!(back.name, tool.name);
        assert_eq!(back.description, tool.description);
        assert_eq!(back.parameters, tool.parameters);
        assert_eq!(back.strict, None);
    }
}
