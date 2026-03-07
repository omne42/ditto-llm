#[derive(Debug, Clone)]
pub(super) struct OpenAiCompatibleProviderOptionsSchemaResult {
    pub selected_provider_options: Option<serde_json::Value>,
    pub warnings: Vec<crate::types::Warning>,
}

const OPENAI_COMPAT_KNOWN_CHAT_OPTION_KEYS: &[&str] = &[
    "model",
    "messages",
    "temperature",
    "max_tokens",
    "top_p",
    "seed",
    "presence_penalty",
    "frequency_penalty",
    "user",
    "logprobs",
    "top_logprobs",
    "stop",
    "stream",
    "stream_options",
    "tools",
    "tool_choice",
];

pub(super) fn apply_openai_compatible_provider_options_schema(
    family: crate::profile::OpenAiProviderFamily,
    selected_provider_options: Option<serde_json::Value>,
    reserved_keys: &[&str],
    provider_options_context: &'static str,
) -> OpenAiCompatibleProviderOptionsSchemaResult {
    let Some(selected_provider_options) = selected_provider_options else {
        return OpenAiCompatibleProviderOptionsSchemaResult {
            selected_provider_options: None,
            warnings: Vec::new(),
        };
    };

    let Some(obj) = selected_provider_options.as_object() else {
        // Keep old behavior for non-object values; downstream merge logic emits a warning.
        return OpenAiCompatibleProviderOptionsSchemaResult {
            selected_provider_options: Some(selected_provider_options),
            warnings: Vec::new(),
        };
    };

    let mut out = serde_json::Map::<String, serde_json::Value>::new();
    let mut warnings = Vec::<crate::types::Warning>::new();

    for (key, value) in obj {
        if reserved_keys.contains(&key.as_str()) {
            continue;
        }
        if OPENAI_COMPAT_KNOWN_CHAT_OPTION_KEYS.contains(&key.as_str()) {
            out.insert(key.clone(), value.clone());
            continue;
        }

        let mut handled_vendor_private_key = false;
        match family {
            crate::profile::OpenAiProviderFamily::OpenRouter => {
                if key == "provider" {
                    handled_vendor_private_key = true;
                    if value.is_object() {
                        out.insert("provider".to_string(), value.clone());
                    } else {
                        warnings.push(crate::types::Warning::Compatibility {
                            feature: provider_options_context.to_string(),
                            details: "provider_options key \"provider\" for openrouter expects a JSON object; dropping".to_string(),
                        });
                    }
                }
            }
            crate::profile::OpenAiProviderFamily::DeepSeek => {
                if key == "thinking" || key == "thinking_config" {
                    handled_vendor_private_key = true;
                    if let Some(thinking) = sanitize_deepseek_thinking(value) {
                        out.insert("thinking".to_string(), thinking);
                        if key != "thinking" {
                            warnings.push(crate::types::Warning::Compatibility {
                                feature: provider_options_context.to_string(),
                                details: format!(
                                    "provider_options key {key:?} mapped to \"thinking\" for deepseek"
                                ),
                            });
                        }
                    } else {
                        warnings.push(crate::types::Warning::Compatibility {
                            feature: provider_options_context.to_string(),
                            details: format!(
                                "provider_options key {key:?} for deepseek expects {{\"type\":\"enabled\"}} (or true); dropping"
                            ),
                        });
                    }
                }
            }
            crate::profile::OpenAiProviderFamily::MiniMax => {
                if key == "reasoning_split" || key == "reasoningSplit" {
                    handled_vendor_private_key = true;
                    if let Some(reasoning_split) = value.as_bool() {
                        out.insert(
                            "reasoning_split".to_string(),
                            serde_json::Value::Bool(reasoning_split),
                        );
                        if key != "reasoning_split" {
                            warnings.push(crate::types::Warning::Compatibility {
                                feature: provider_options_context.to_string(),
                                details: format!(
                                    "provider_options key {key:?} mapped to \"reasoning_split\" for minimax"
                                ),
                            });
                        }
                    } else {
                        warnings.push(crate::types::Warning::Compatibility {
                            feature: provider_options_context.to_string(),
                            details: format!(
                                "provider_options key {key:?} for minimax expects a boolean; dropping"
                            ),
                        });
                    }
                }
            }
            crate::profile::OpenAiProviderFamily::Doubao => {
                if key == "thinking" || key == "thinking_config" {
                    handled_vendor_private_key = true;
                    if let Some(thinking) = sanitize_doubao_thinking(value) {
                        out.insert("thinking".to_string(), thinking);
                        if key != "thinking" {
                            warnings.push(crate::types::Warning::Compatibility {
                                feature: provider_options_context.to_string(),
                                details: format!(
                                    "provider_options key {key:?} mapped to \"thinking\" for doubao"
                                ),
                            });
                        }
                    } else {
                        warnings.push(crate::types::Warning::Compatibility {
                            feature: provider_options_context.to_string(),
                            details: format!(
                                "provider_options key {key:?} for doubao expects {{\"type\":\"enabled|disabled|auto\"}}; dropping"
                            ),
                        });
                    }
                }
            }
            crate::profile::OpenAiProviderFamily::OpenAi
            | crate::profile::OpenAiProviderFamily::Kimi
            | crate::profile::OpenAiProviderFamily::Qwen
            | crate::profile::OpenAiProviderFamily::Glm
            | crate::profile::OpenAiProviderFamily::GenericOpenAiCompatible => {}
        }

        if handled_vendor_private_key {
            continue;
        }

        warnings.push(crate::types::Warning::Unsupported {
            feature: provider_options_context.to_string(),
            details: Some(format!(
                "provider_options key {key:?} is not in the openai-compatible schema for provider family {}; dropping",
                family.as_str()
            )),
        });
    }

    OpenAiCompatibleProviderOptionsSchemaResult {
        selected_provider_options: if out.is_empty() {
            None
        } else {
            Some(serde_json::Value::Object(out))
        },
        warnings,
    }
}

fn sanitize_deepseek_thinking(value: &serde_json::Value) -> Option<serde_json::Value> {
    if value.as_bool() == Some(true) {
        return Some(serde_json::json!({ "type": "enabled" }));
    }

    let obj = value.as_object()?;
    if obj
        .get("type")
        .and_then(serde_json::Value::as_str)
        != Some("enabled")
    {
        return None;
    }
    Some(serde_json::json!({ "type": "enabled" }))
}

fn sanitize_doubao_thinking(value: &serde_json::Value) -> Option<serde_json::Value> {
    let obj = value.as_object()?;
    let kind = obj.get("type").and_then(serde_json::Value::as_str)?;
    if !matches!(kind, "enabled" | "disabled" | "auto") {
        return None;
    }
    Some(serde_json::Value::Object(obj.clone()))
}
