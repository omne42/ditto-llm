use serde_json::Value;
use tiktoken_rs::{CoreBPE, tokenizer};

pub fn estimate_input_tokens(path_and_query: &str, model: &str, request: &Value) -> Option<u32> {
    if model.is_empty() {
        return None;
    }

    let path = strip_query(path_and_query);
    let path = path.strip_suffix('/').unwrap_or(path);

    let bpe = bpe_for_model(model);

    let tokens = match path {
        "/v1/chat/completions" => count_chat_completions_input_tokens(model, bpe, request)?,
        "/v1/responses" => count_responses_input_tokens(bpe, request)?,
        "/v1/embeddings" => count_string_or_array_tokens(bpe, request.get("input")?)?,
        "/v1/moderations" => count_string_or_array_tokens(bpe, request.get("input")?)?,
        _ => return None,
    };

    Some(clamp_usize_to_u32(tokens))
}

fn strip_query(path_and_query: &str) -> &str {
    path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query)
}

fn bpe_for_model(model: &str) -> &'static CoreBPE {
    let tokenizer = tokenizer::get_tokenizer(model).unwrap_or(tokenizer::Tokenizer::Cl100kBase);
    match tokenizer {
        tokenizer::Tokenizer::O200kHarmony => tiktoken_rs::o200k_harmony_singleton(),
        tokenizer::Tokenizer::O200kBase => tiktoken_rs::o200k_base_singleton(),
        tokenizer::Tokenizer::Cl100kBase => tiktoken_rs::cl100k_base_singleton(),
        tokenizer::Tokenizer::R50kBase => tiktoken_rs::r50k_base_singleton(),
        tokenizer::Tokenizer::P50kBase => tiktoken_rs::p50k_base_singleton(),
        tokenizer::Tokenizer::P50kEdit => tiktoken_rs::p50k_edit_singleton(),
        tokenizer::Tokenizer::Gpt2 => tiktoken_rs::r50k_base_singleton(),
    }
}

fn count_chat_completions_input_tokens(
    model: &str,
    bpe: &CoreBPE,
    request: &Value,
) -> Option<usize> {
    let messages = request.get("messages")?.as_array()?;
    let (tokens_per_message, tokens_per_name) = if model.starts_with("gpt-3.5") {
        (4i64, -1i64)
    } else {
        (3i64, 1i64)
    };

    let mut num_tokens: i64 = 0;
    for message in messages {
        let obj = message.as_object()?;
        let role = obj
            .get("role")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let name = obj.get("name").and_then(|value| value.as_str());

        num_tokens = num_tokens.saturating_add(tokens_per_message);
        num_tokens = num_tokens.saturating_add(clamp_usize_to_i64(
            bpe.encode_with_special_tokens(role).len(),
        ));

        if let Some(content) = obj.get("content") {
            num_tokens = num_tokens.saturating_add(clamp_usize_to_i64(
                count_chat_message_content_tokens(bpe, content),
            ));
        }

        if let Some(name) = name {
            num_tokens = num_tokens.saturating_add(clamp_usize_to_i64(
                bpe.encode_with_special_tokens(name).len(),
            ));
            num_tokens = num_tokens.saturating_add(tokens_per_name);
        }
    }
    num_tokens = num_tokens.saturating_add(3);

    let extra_fields = [
        "tools",
        "functions",
        "tool_choice",
        "response_format",
        "stop",
    ];
    for field in extra_fields {
        if let Some(value) = request.get(field) {
            num_tokens =
                num_tokens.saturating_add(clamp_usize_to_i64(count_json_tokens(bpe, value)));
        }
    }

    Some(std::cmp::max(num_tokens, 0) as usize)
}

fn count_chat_message_content_tokens(bpe: &CoreBPE, content: &Value) -> usize {
    match content {
        Value::String(text) => bpe.encode_with_special_tokens(text).len(),
        Value::Array(parts) => parts
            .iter()
            .map(|part| match part {
                Value::String(text) => bpe.encode_with_special_tokens(text).len(),
                Value::Object(obj) => {
                    let part_type = obj
                        .get("type")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default();
                    if part_type != "text" && part_type != "input_text" {
                        return 0;
                    }
                    let Some(text) = obj.get("text").and_then(|value| value.as_str()) else {
                        return 0;
                    };
                    bpe.encode_with_special_tokens(text).len()
                }
                _ => 0,
            })
            .sum(),
        _ => 0,
    }
}

fn count_responses_input_tokens(bpe: &CoreBPE, request: &Value) -> Option<usize> {
    let mut tokens: usize = 0;
    if let Some(instructions) = request.get("instructions").and_then(|value| value.as_str()) {
        tokens = tokens.saturating_add(bpe.encode_with_special_tokens(instructions).len());
    }
    tokens = tokens.saturating_add(count_responses_input_value_tokens(
        bpe,
        request.get("input")?,
    ));

    let extra_fields = ["tools", "tool_choice", "response_format", "stop"];
    for field in extra_fields {
        if let Some(value) = request.get(field) {
            tokens = tokens.saturating_add(count_json_tokens(bpe, value));
        }
    }

    Some(tokens)
}

fn count_responses_input_value_tokens(bpe: &CoreBPE, value: &Value) -> usize {
    match value {
        Value::String(text) => bpe.encode_with_special_tokens(text).len(),
        Value::Array(items) => items
            .iter()
            .map(|item| count_responses_input_item_tokens(bpe, item))
            .sum(),
        Value::Object(obj) => {
            if let Some(content) = obj.get("content") {
                return count_responses_content_tokens(bpe, content);
            }
            let part_type = obj
                .get("type")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            if part_type == "input_text" || part_type == "text" {
                let Some(text) = obj.get("text").and_then(|value| value.as_str()) else {
                    return 0;
                };
                return bpe.encode_with_special_tokens(text).len();
            }
            0
        }
        _ => 0,
    }
}

fn count_responses_input_item_tokens(bpe: &CoreBPE, item: &Value) -> usize {
    if let Value::Object(obj) = item {
        let item_type = obj.get("type").and_then(|value| value.as_str());
        if matches!(item_type, Some("input_text") | Some("text")) {
            if let Some(text) = obj.get("text").and_then(|value| value.as_str()) {
                return bpe.encode_with_special_tokens(text).len();
            }
        }
        if let Some(content) = obj.get("content") {
            return count_responses_content_tokens(bpe, content);
        }
    }
    count_responses_input_value_tokens(bpe, item)
}

fn count_responses_content_tokens(bpe: &CoreBPE, content: &Value) -> usize {
    match content {
        Value::String(text) => bpe.encode_with_special_tokens(text).len(),
        Value::Array(parts) => parts
            .iter()
            .map(|part| {
                let Value::Object(obj) = part else {
                    return 0;
                };
                let part_type = obj
                    .get("type")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();
                if part_type != "input_text" && part_type != "text" {
                    return 0;
                }
                let Some(text) = obj.get("text").and_then(|value| value.as_str()) else {
                    return 0;
                };
                bpe.encode_with_special_tokens(text).len()
            })
            .sum(),
        _ => 0,
    }
}

fn count_string_or_array_tokens(bpe: &CoreBPE, value: &Value) -> Option<usize> {
    match value {
        Value::String(text) => Some(bpe.encode_with_special_tokens(text).len()),
        Value::Array(items) => Some(
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(|text| bpe.encode_with_special_tokens(text).len())
                .sum(),
        ),
        _ => None,
    }
}

fn count_json_tokens(bpe: &CoreBPE, value: &Value) -> usize {
    let json = serde_json::to_string(value).unwrap_or_default();
    bpe.encode_with_special_tokens(&json).len()
}

fn clamp_usize_to_u32(value: usize) -> u32 {
    if value > usize::try_from(u32::MAX).unwrap_or(usize::MAX) {
        u32::MAX
    } else {
        value as u32
    }
}

fn clamp_usize_to_i64(value: usize) -> i64 {
    if value > usize::try_from(i64::MAX).unwrap_or(usize::MAX) {
        i64::MAX
    } else {
        value as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_chat_completions_like_tiktoken() {
        let request = serde_json::json!({
            "model": "gpt-4o-mini",
            "messages": [{"role":"user","content":"hello"}],
        });
        let tokens =
            estimate_input_tokens("/v1/chat/completions", "gpt-4o-mini", &request).expect("tokens");

        let expected = tiktoken_rs::num_tokens_from_messages(
            "gpt-4o-mini",
            &[tiktoken_rs::ChatCompletionRequestMessage {
                role: "user".to_string(),
                content: Some("hello".to_string()),
                name: None,
                function_call: None,
            }],
        )
        .expect("num_tokens_from_messages") as u32;

        assert_eq!(tokens, expected);
    }

    #[test]
    fn counts_text_parts_and_ignores_image_parts() {
        let request = serde_json::json!({
            "model": "gpt-4o-mini",
            "messages": [{
                "role":"user",
                "content": [
                    {"type":"text","text":"hello"},
                    {"type":"image_url","image_url":{"url":"data:image/png;base64,AAAA"}},
                    {"type":"text","text":"world"}
                ]
            }],
        });

        let bpe = bpe_for_model("gpt-4o-mini");
        let tokens_per_message = 3i64;
        let mut expected: i64 = 0;
        expected += tokens_per_message;
        expected += clamp_usize_to_i64(bpe.encode_with_special_tokens("user").len());
        expected += clamp_usize_to_i64(bpe.encode_with_special_tokens("hello").len());
        expected += clamp_usize_to_i64(bpe.encode_with_special_tokens("world").len());
        expected += 3;

        let tokens =
            estimate_input_tokens("/v1/chat/completions", "gpt-4o-mini", &request).expect("tokens");
        assert_eq!(tokens, expected as u32);
    }

    #[test]
    fn counts_embeddings_input_strings() {
        let request = serde_json::json!({
            "model": "gpt-4o-mini",
            "input": ["hello", "world"],
        });

        let bpe = bpe_for_model("gpt-4o-mini");
        let expected = bpe.encode_with_special_tokens("hello").len()
            + bpe.encode_with_special_tokens("world").len();

        let tokens =
            estimate_input_tokens("/v1/embeddings", "gpt-4o-mini", &request).expect("tokens");
        assert_eq!(tokens, expected as u32);
    }
}
