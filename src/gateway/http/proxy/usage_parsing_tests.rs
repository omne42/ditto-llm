#[cfg(test)]
mod usage_parsing_tests {
    use super::extract_openai_usage_from_bytes;
    use bytes::Bytes;
    use serde_json::json;

    #[test]
    fn parses_openai_usage_with_cache_and_reasoning_details() {
        let response = json!({
            "id": "chatcmpl_test",
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 3,
                "total_tokens": 13,
                "prompt_tokens_details": {
                    "cached_tokens": 8,
                    "cache_creation_tokens": 2
                },
                "completion_tokens_details": {
                    "reasoning_tokens": 1
                }
            }
        });

        let bytes = Bytes::from(response.to_string());
        let usage = extract_openai_usage_from_bytes(&bytes).expect("usage");
        assert_eq!(usage.input_tokens, Some(10));
        assert_eq!(usage.cache_input_tokens, Some(8));
        assert_eq!(usage.cache_creation_input_tokens, Some(2));
        assert_eq!(usage.output_tokens, Some(3));
        assert_eq!(usage.reasoning_tokens, Some(1));
        assert_eq!(usage.total_tokens, Some(13));
    }

    #[test]
    fn computes_total_tokens_when_missing() {
        let response = json!({
            "usage": {
                "input_tokens": 4,
                "output_tokens": 5
            }
        });

        let bytes = Bytes::from(response.to_string());
        let usage = extract_openai_usage_from_bytes(&bytes).expect("usage");
        assert_eq!(usage.input_tokens, Some(4));
        assert_eq!(usage.output_tokens, Some(5));
        assert_eq!(usage.total_tokens, Some(9));
    }
}
