#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn converts_pdf_file_part_to_document_block() {
        let tool_names = HashMap::new();
        let message = Message {
            role: Role::User,
            content: vec![ContentPart::File {
                filename: Some("doc.pdf".to_string()),
                media_type: "application/pdf".to_string(),
                source: FileSource::Base64 {
                    data: "AQIDBAU=".to_string(),
                },
            }],
        };

        let mut warnings = Vec::new();
        let out = Anthropic::message_to_anthropic_blocks(&message, &tool_names, &mut warnings)
            .expect("blocks");
        assert_eq!(out.0, "user");
        assert_eq!(out.1.len(), 1);
        assert_eq!(
            out.1[0].get("type").and_then(Value::as_str),
            Some("document")
        );
        assert_eq!(
            out.1[0].get("title").and_then(Value::as_str),
            Some("doc.pdf")
        );
        assert_eq!(
            out.1[0]
                .get("source")
                .and_then(Value::as_object)
                .and_then(|o| o.get("type"))
                .and_then(Value::as_str),
            Some("base64")
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn required_betas_includes_pdfs() {
        let messages = vec![Message {
            role: Role::User,
            content: vec![ContentPart::File {
                filename: None,
                media_type: "application/pdf".to_string(),
                source: FileSource::Url {
                    url: "https://example.com/doc.pdf".to_string(),
                },
            }],
        }];
        assert_eq!(
            Anthropic::required_betas(&messages),
            vec![BETA_PDFS_2024_09_25]
        );
    }

    #[test]
    fn converts_tool_result_to_tool_block() {
        let tool_names = HashMap::from([("c1".to_string(), "add".to_string())]);
        let message = Message {
            role: Role::Tool,
            content: vec![ContentPart::ToolResult {
                tool_call_id: "c1".to_string(),
                content: "{\"result\":3}".to_string(),
                is_error: Some(false),
            }],
        };

        let mut warnings = Vec::new();
        let out = Anthropic::message_to_anthropic_blocks(&message, &tool_names, &mut warnings)
            .expect("blocks");
        assert_eq!(out.0, "user");
        assert_eq!(out.1.len(), 1);
        assert_eq!(
            out.1[0].get("type").and_then(Value::as_str),
            Some("tool_result")
        );
        assert_eq!(
            out.1[0].get("tool_use_id").and_then(Value::as_str),
            Some("c1")
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn stop_reason_mapping() {
        assert_eq!(
            Anthropic::stop_reason_to_finish_reason(Some("tool_use")),
            FinishReason::ToolCalls
        );
        assert_eq!(
            Anthropic::stop_reason_to_finish_reason(Some("max_tokens")),
            FinishReason::Length
        );
    }

    #[test]
    fn tool_choice_none_is_unsupported() {
        assert_eq!(Anthropic::tool_choice_to_anthropic(&ToolChoice::None), None);
    }

    #[test]
    fn tool_schema_maps_input_schema() {
        let tool = Tool {
            name: "add".to_string(),
            description: Some("add".to_string()),
            parameters: json!({ "type": "object" }),
            strict: None,
        };
        let mut warnings = Vec::new();
        let mapped = Anthropic::tool_to_anthropic(&tool, &mut warnings);
        assert_eq!(mapped.get("name").and_then(Value::as_str), Some("add"));
        assert_eq!(
            mapped.get("input_schema"),
            Some(&json!({ "type": "object" }))
        );
    }
}
