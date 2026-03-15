#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::{FileSource, Role};
    use serde_json::json;

    #[test]
    fn converts_system_to_system_instruction() -> crate::error::Result<()> {
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
    fn late_system_message_is_downgraded_to_user_content() -> crate::error::Result<()> {
        let mut warnings = Vec::new();
        let tool_names = HashMap::new();
        let (contents, system) = Google::convert_messages(
            "gemini-pro",
            &[
                Message::system("sys-start"),
                Message::user("hi"),
                Message::system("late-system"),
            ],
            &tool_names,
            &mut warnings,
        )?;
        assert!(system.is_some());
        assert_eq!(contents.len(), 2);
        assert_eq!(
            contents[1].get("role").and_then(Value::as_str),
            Some("user")
        );
        assert_eq!(
            contents[1]
                .get("parts")
                .and_then(Value::as_array)
                .and_then(|parts| parts.first())
                .and_then(|part| part.get("text"))
                .and_then(Value::as_str),
            Some("[SYSTEM MESSAGE]\nlate-system")
        );
        assert!(warnings.iter().any(|w| matches!(
            w,
            Warning::Compatibility { feature, .. } if feature == "system_message.mid_conversation"
        )));
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
    fn tool_schema_additional_properties_are_removed_for_google() {
        let tool = Tool {
            name: "add".to_string(),
            description: Some("add".to_string()),
            parameters: json!({
                "type": "object",
                "properties": {
                    "a": { "type": "integer" },
                    "meta": {
                        "type": "object",
                        "properties": {
                            "tag": { "type": "string" }
                        },
                        "additionalProperties": { "type": "string" }
                    }
                },
                "required": ["a"],
                "additionalProperties": false
            }),
            strict: None,
        };
        let mut warnings = Vec::new();
        let decl = Google::tool_to_google(tool, &mut warnings);
        assert_eq!(
            decl.get("parameters"),
            Some(&json!({
                "type": "object",
                "properties": {
                    "a": { "type": "integer" },
                    "meta": {
                        "type": "object",
                        "properties": {
                            "tag": { "type": "string" }
                        }
                    }
                },
                "required": ["a"]
            }))
        );
        assert!(warnings.iter().any(|w| matches!(
            w,
            Warning::Compatibility { feature, .. } if feature == "tool.parameters.google_unsupported_keywords"
        )));
    }

    #[test]
    fn assistant_tool_call_replays_google_thought_signature() -> crate::error::Result<()> {
        let mut warnings = Vec::new();
        let tool_names = HashMap::new();
        let (contents, _system) = Google::convert_messages(
            "gemini-pro",
            &[Message {
                role: Role::Assistant,
                content: vec![ContentPart::ToolCall {
                    id: "call_1__gts_6869".to_string(),
                    name: "add".to_string(),
                    arguments: json!({ "a": 1 }),
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
        let function_call = parts[0]
            .get("functionCall")
            .and_then(Value::as_object)
            .expect("function call");
        assert_eq!(
            function_call.get("name").and_then(Value::as_str),
            Some("add")
        );
        assert_eq!(
            parts[0].get("thoughtSignature").and_then(Value::as_str),
            Some("hi")
        );
        assert!(function_call.get("thoughtSignature").is_none());
        Ok(())
    }

    #[test]
    fn parse_google_candidate_reads_part_level_thought_signature() {
        let candidate = json!({
            "content": {
                "parts": [{
                    "functionCall": {
                        "name": "add",
                        "args": { "a": 1, "b": 2 }
                    },
                    "thoughtSignature": "hi"
                }]
            }
        });
        let mut seq = 0u64;
        let mut has_tool_calls = false;
        let parts = parse_google_candidate(&candidate, &mut seq, &mut has_tool_calls);
        assert!(has_tool_calls);
        assert_eq!(seq, 1);
        assert_eq!(parts.len(), 1);
        match &parts[0] {
            ContentPart::ToolCall {
                id,
                name,
                arguments,
            } => {
                assert_eq!(id, "call_0__gts_6869");
                assert_eq!(name, "add");
                assert_eq!(arguments, &json!({ "a": 1, "b": 2 }));
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn parse_usage_metadata_maps_cached_content_tokens() {
        let usage = Google::parse_usage_metadata(&json!({
            "promptTokenCount": 120,
            "cachedContentTokenCount": 48,
            "candidatesTokenCount": 16,
            "totalTokenCount": 136
        }));
        assert_eq!(usage.input_tokens, Some(120));
        assert_eq!(usage.cache_input_tokens, Some(48));
        assert_eq!(usage.output_tokens, Some(16));
        assert_eq!(usage.total_tokens, Some(136));
    }

    #[test]
    fn converts_pdf_file_part_to_inline_data() -> crate::error::Result<()> {
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
