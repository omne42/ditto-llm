#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FileSource, Role};
    use serde_json::json;

    #[test]
    fn converts_system_to_system_instruction() -> crate::Result<()> {
        let mut warnings = Vec::new();
        let tool_names = HashMap::new();
        let (contents, system) = Google::convert_messages(
            "gemini-pro",
            &[Message::system("sys"), Message::user("hi")],
            &tool_names,
            &mut warnings,
        )?;
        assert_eq!(warnings.len(), 0);
        assert_eq!(contents.len(), 1);
        assert!(system.is_some());
        Ok(())
    }

    #[test]
    fn tool_choice_maps_to_tool_config() {
        let config = Google::tool_config(Some(&ToolChoice::Tool {
            name: "add".to_string(),
        }))
        .expect("tool config");
        assert_eq!(
            config
                .get("functionCallingConfig")
                .and_then(|v| v.get("mode"))
                .and_then(Value::as_str),
            Some("ANY")
        );
    }

    #[test]
    fn tool_declaration_converts_schema() {
        let tool = Tool {
            name: "add".to_string(),
            description: Some("add".to_string()),
            parameters: json!({
                "type": "object",
                "properties": { "a": { "type": "integer" } }
            }),
            strict: None,
        };
        let mut warnings = Vec::new();
        let decl = Google::tool_to_google(tool, &mut warnings);
        assert!(warnings.is_empty());
        assert_eq!(decl.get("name").and_then(Value::as_str), Some("add"));
        assert!(decl.get("parameters").is_some());
    }

    #[test]
    fn tool_schema_ref_is_resolved_without_warning() {
        let tool = Tool {
            name: "add".to_string(),
            description: Some("add".to_string()),
            parameters: json!({
                "$ref": "#/$defs/Args",
                "$defs": {
                    "Args": { "type": "object", "properties": { "a": { "type": "integer" } } }
                }
            }),
            strict: None,
        };
        let mut warnings = Vec::new();
        let decl = Google::tool_to_google(tool, &mut warnings);
        assert_eq!(decl.get("name").and_then(Value::as_str), Some("add"));
        assert!(warnings.is_empty());
        assert_eq!(
            decl.get("parameters"),
            Some(&json!({
                "type": "object",
                "properties": { "a": { "type": "integer" } }
            }))
        );
    }

    #[test]
    fn tool_schema_unresolvable_ref_emits_warning() {
        let tool = Tool {
            name: "add".to_string(),
            description: Some("add".to_string()),
            parameters: json!({
                "$ref": "#/$defs/Missing",
                "$defs": {
                    "Args": { "type": "object", "properties": { "a": { "type": "integer" } } }
                }
            }),
            strict: None,
        };
        let mut warnings = Vec::new();
        let decl = Google::tool_to_google(tool, &mut warnings);
        assert_eq!(decl.get("name").and_then(Value::as_str), Some("add"));
        assert_eq!(decl.get("parameters"), Some(&json!({})));
        assert!(warnings.iter().any(|w| {
            matches!(w, Warning::Compatibility { feature, .. } if feature == "tool.parameters.$ref")
        }));
    }

    #[test]
    fn tool_schema_unsupported_keywords_emit_warning() {
        let tool = Tool {
            name: "add".to_string(),
            description: Some("add".to_string()),
            parameters: json!({
                "type": "object",
                "properties": { "a": { "type": "integer" } },
                "not": { "type": "object" }
            }),
            strict: None,
        };
        let mut warnings = Vec::new();
        let decl = Google::tool_to_google(tool, &mut warnings);
        assert_eq!(decl.get("name").and_then(Value::as_str), Some("add"));
        assert!(warnings.iter().any(|w| matches!(
            w,
            Warning::Compatibility { feature, details } if feature == "tool.parameters.unsupported_keywords" && details.contains("not")
        )));
    }

    #[test]
    fn converts_pdf_file_part_to_inline_data() -> crate::Result<()> {
        let mut warnings = Vec::new();
        let tool_names = HashMap::new();
        let (contents, _system) = Google::convert_messages(
            "gemini-pro",
            &[Message {
                role: Role::User,
                content: vec![ContentPart::File {
                    filename: Some("doc.pdf".to_string()),
                    media_type: "application/pdf".to_string(),
                    source: FileSource::Base64 {
                        data: "AQIDBAU=".to_string(),
                    },
                }],
            }],
            &tool_names,
            &mut warnings,
        )?;
        assert!(warnings.is_empty());
        assert_eq!(contents.len(), 1);
        let parts = contents[0]
            .get("parts")
            .and_then(Value::as_array)
            .expect("parts array");
        assert_eq!(parts.len(), 1);
        assert_eq!(
            parts[0]
                .get("inlineData")
                .and_then(Value::as_object)
                .and_then(|o| o.get("mimeType"))
                .and_then(Value::as_str),
            Some("application/pdf")
        );
        Ok(())
    }
}
