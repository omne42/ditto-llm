use std::collections::BTreeMap;

use crate::{DittoError, Result};

use super::auth::{RequestAuth, resolve_request_auth_with_default_keys};
use super::*;

#[tokio::test]
async fn resolves_auth_token_with_custom_default_keys() -> Result<()> {
    let env = Env {
        dotenv: BTreeMap::from([("DITTO_TEST_KEY".to_string(), "sk-test".to_string())]),
    };
    let auth = ProviderAuth::ApiKeyEnv { keys: Vec::new() };
    let token = resolve_auth_token_with_default_keys(&auth, &env, &["DITTO_TEST_KEY"]).await?;
    assert_eq!(token, "sk-test");
    Ok(())
}

#[tokio::test]
async fn resolves_auth_token_from_secret_spec_in_env_value() -> Result<()> {
    let env = Env {
        dotenv: BTreeMap::from([
            (
                "DITTO_TEST_KEY".to_string(),
                "secret://env/REAL_TEST_KEY".to_string(),
            ),
            ("REAL_TEST_KEY".to_string(), "sk-test".to_string()),
        ]),
    };
    let auth = ProviderAuth::ApiKeyEnv {
        keys: vec!["DITTO_TEST_KEY".to_string()],
    };
    let token = resolve_auth_token_with_default_keys(&auth, &env, &["DITTO_TEST_KEY"]).await?;
    assert_eq!(token, "sk-test");
    Ok(())
}

#[tokio::test]
async fn resolves_http_header_env_auth() -> Result<()> {
    let env = Env {
        dotenv: BTreeMap::from([("DITTO_TEST_KEY".to_string(), "sk-test".to_string())]),
    };
    let auth = ProviderAuth::HttpHeaderEnv {
        header: "api-key".to_string(),
        keys: vec!["DITTO_TEST_KEY".to_string()],
        prefix: None,
    };
    let resolved = resolve_request_auth_with_default_keys(
        &auth,
        &env,
        &["DITTO_TEST_KEY"],
        "authorization",
        Some("Bearer "),
    )
    .await?;
    let RequestAuth::Http(resolved) = resolved else {
        panic!("expected http header auth");
    };
    assert_eq!(resolved.header.as_str(), "api-key");
    assert_eq!(resolved.value.to_str().unwrap_or_default(), "sk-test");
    Ok(())
}

#[tokio::test]
async fn resolves_query_param_env_auth() -> Result<()> {
    let env = Env {
        dotenv: BTreeMap::from([("DITTO_TEST_KEY".to_string(), "sk-test".to_string())]),
    };
    let auth = ProviderAuth::QueryParamEnv {
        param: "api_key".to_string(),
        keys: vec!["DITTO_TEST_KEY".to_string()],
        prefix: None,
    };
    let resolved = resolve_request_auth_with_default_keys(
        &auth,
        &env,
        &["DITTO_TEST_KEY"],
        "authorization",
        Some("Bearer "),
    )
    .await?;
    let RequestAuth::QueryParam(resolved) = resolved else {
        panic!("expected query param auth");
    };
    assert_eq!(resolved.param, "api_key");
    assert_eq!(resolved.value, "sk-test");
    Ok(())
}

#[test]
fn parses_dotenv_basic() {
    let parsed = parse_dotenv(
        r#"
# comment
export OPENAI_API_KEY="sk-test"
FOO=bar
EMPTY=
"#,
    );
    assert_eq!(
        parsed.get("OPENAI_API_KEY").map(String::as_str),
        Some("sk-test")
    );
    assert_eq!(parsed.get("FOO").map(String::as_str), Some("bar"));
    assert_eq!(parsed.get("EMPTY"), None);
}

#[test]
fn http_headers_accept_valid_pairs() -> Result<()> {
    let headers = BTreeMap::from([
        ("x-test".to_string(), "value".to_string()),
        ("x-other".to_string(), "123".to_string()),
    ]);
    let parsed = super::http::header_map_from_pairs(&headers)?;
    assert_eq!(
        parsed
            .get("x-test")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default(),
        "value"
    );
    Ok(())
}

#[test]
fn http_headers_reject_invalid_name() {
    let headers = BTreeMap::from([("bad header".to_string(), "value".to_string())]);
    let err = super::http::header_map_from_pairs(&headers)
        .expect_err("should reject invalid header name");
    match err {
        DittoError::InvalidResponse(_) => {}
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn http_headers_reject_invalid_value() {
    let headers = BTreeMap::from([("x-test".to_string(), "bad\nvalue".to_string())]);
    let err = super::http::header_map_from_pairs(&headers)
        .expect_err("should reject invalid header value");
    match err {
        DittoError::InvalidResponse(_) => {}
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn thinking_intensity_defaults_to_medium() {
    let parsed = toml::from_str::<ModelConfig>("").expect("parse toml");
    assert_eq!(parsed.thinking, ThinkingIntensity::Medium);
    assert_eq!(parsed.prompt_cache, None);
}

#[test]
fn model_config_accepts_best_and_max_context_aliases() {
    let parsed = toml::from_str::<ModelConfig>(
        r#"
max_context = 12345
best_context = 9000
"#,
    )
    .expect("parse toml");
    assert_eq!(parsed.context_window, Some(12345));
    assert_eq!(parsed.auto_compact_token_limit, Some(9000));
}

#[test]
fn model_config_parses_prompt_cache() {
    let parsed = toml::from_str::<ModelConfig>(
        r#"
prompt_cache = false
"#,
    )
    .expect("parse toml");
    assert_eq!(parsed.prompt_cache, Some(false));
}

#[test]
fn selects_exact_then_wildcard_model_config() {
    let models = BTreeMap::from([
        (
            "*".to_string(),
            ModelConfig {
                thinking: ThinkingIntensity::High,
                ..Default::default()
            },
        ),
        (
            "gpt-4.1".to_string(),
            ModelConfig {
                thinking: ThinkingIntensity::XHigh,
                ..Default::default()
            },
        ),
    ]);
    assert_eq!(
        select_model_config(&models, "gpt-4.1").map(|c| c.thinking),
        Some(ThinkingIntensity::XHigh)
    );
    assert_eq!(
        select_model_config(&models, "other").map(|c| c.thinking),
        Some(ThinkingIntensity::High)
    );
}

#[test]
fn parses_provider_capabilities_from_toml() {
    let parsed = toml::from_str::<ProviderConfig>(
        r#"
base_url = "https://example.com/v1"

[capabilities]
tools = true
vision = false
reasoning = true
json_schema = true
streaming = false
"#,
    )
    .expect("parse toml");
    assert_eq!(
        parsed.capabilities,
        Some(ProviderCapabilities {
            tools: true,
            vision: false,
            reasoning: true,
            json_schema: true,
            streaming: false,
            prompt_cache: true,
        })
    );
}

#[test]
fn parses_provider_protocol_fields_from_toml() {
    let parsed = toml::from_str::<ProviderConfig>(
        r#"
base_url = "https://example.com/v1"
upstream_api = "gemini_generate_content"
normalize_to = "openai_chat_completions"
normalize_endpoint = "/v1/chat/completions"
"#,
    )
    .expect("parse toml");
    assert_eq!(
        parsed.upstream_api,
        Some(ProviderApi::GeminiGenerateContent)
    );
    assert_eq!(
        parsed.normalize_to,
        Some(ProviderApi::OpenaiChatCompletions)
    );
    assert_eq!(
        parsed.normalize_endpoint.as_deref(),
        Some("/v1/chat/completions")
    );
}

#[test]
fn merge_openai_provider_config_merges_overrides() {
    let base = ProviderConfig {
        base_url: Some("https://upstream.example/v1".to_string()),
        default_model: Some("base-model".to_string()),
        model_whitelist: vec!["old".to_string()],
        http_headers: BTreeMap::from([("x-base".to_string(), "0".to_string())]),
        http_query_params: BTreeMap::new(),
        auth: None,
        capabilities: None,
        upstream_api: None,
        normalize_to: None,
        normalize_endpoint: None,
    };
    let overrides = ProviderConfig {
        base_url: Some("https://example.com/v1".to_string()),
        default_model: Some("my-model".to_string()),
        model_whitelist: vec!["m1".to_string(), "m1".to_string(), "m2".to_string()],
        http_headers: BTreeMap::from([("x-test".to_string(), "1".to_string())]),
        http_query_params: BTreeMap::new(),
        auth: None,
        capabilities: None,
        upstream_api: None,
        normalize_to: None,
        normalize_endpoint: None,
    };

    let resolved = merge_provider_config(base, &overrides);

    assert_eq!(resolved.base_url.as_deref(), Some("https://example.com/v1"));
    assert_eq!(resolved.default_model.as_deref(), Some("my-model"));
    assert_eq!(
        resolved.model_whitelist,
        vec!["m1".to_string(), "m2".to_string()]
    );
    assert_eq!(
        resolved.http_headers.get("x-base").map(String::as_str),
        Some("0")
    );
    assert_eq!(
        resolved.http_headers.get("x-test").map(String::as_str),
        Some("1")
    );
}

#[test]
fn infer_openai_provider_quirks_marks_qwen_as_best_effort_cache_usage() {
    let quirks = infer_openai_provider_quirks(
        "qwen-direct",
        "https://dashscope.aliyuncs.com/compatible-mode/v1",
    );
    assert_eq!(quirks.family, OpenAiProviderFamily::Qwen);
    assert!(quirks.prompt_cache_usage_may_be_missing());
}

#[test]
fn infer_openai_provider_quirks_marks_deepseek_as_reliable_cache_usage() {
    let quirks = infer_openai_provider_quirks("deepseek-direct", "https://api.deepseek.com/v1");
    assert_eq!(quirks.family, OpenAiProviderFamily::DeepSeek);
    assert!(!quirks.prompt_cache_usage_may_be_missing());
}

#[test]
fn infer_openai_provider_quirks_marks_openrouter_as_best_effort_cache_usage() {
    let quirks = infer_openai_provider_quirks("openrouter-direct", "https://openrouter.ai/api/v1");
    assert_eq!(quirks.family, OpenAiProviderFamily::OpenRouter);
    assert!(quirks.prompt_cache_usage_may_be_missing());
}

#[test]
fn openai_model_catalog_includes_gpt_4_1_metadata_from_docs() {
    let model = openai_model_catalog_entry("gpt-4.1").expect("gpt-4.1 must exist");
    assert_eq!(model.display_name, "GPT-4.1");
    assert_eq!(model.input.as_deref(), Some("Text, image"));
    assert_eq!(model.output.as_deref(), Some("Text"));
    assert_eq!(model.context_window, Some(1_047_576));
    assert_eq!(model.max_output_tokens, Some(32_768));
    assert_eq!(
        model.modalities.get("image"),
        Some(&OpenAiModalitySupport::InputOnly)
    );
    assert_eq!(model.features.get("streaming"), Some(&true));
    assert_eq!(model.features.get("function_calling"), Some(&true));
    assert_eq!(model.tools.get("mcp"), Some(&true));
}

#[test]
fn openai_model_catalog_includes_embedding_model_metadata_from_docs() {
    let model = openai_model_catalog_entry("text-embedding-3-large")
        .expect("text-embedding-3-large must exist");
    assert_eq!(model.display_name, "text-embedding-3-large");
    assert_eq!(model.input.as_deref(), Some("Text"));
    assert_eq!(model.output.as_deref(), Some("Text"));
    assert_eq!(
        model.modalities.get("text"),
        Some(&OpenAiModalitySupport::InputAndOutput)
    );
    assert!(model.features.is_empty());
}

#[test]
fn openai_model_catalog_includes_whisper_audio_metadata_from_docs() {
    let model = openai_model_catalog_entry("whisper-1").expect("whisper-1 must exist");
    assert_eq!(model.display_name, "Whisper");
    assert_eq!(model.input.as_deref(), Some("Audio"));
    assert_eq!(model.output.as_deref(), Some("Text"));
    assert_eq!(
        model.modalities.get("audio"),
        Some(&OpenAiModalitySupport::InputOnly)
    );
    assert_eq!(
        model.modalities.get("text"),
        Some(&OpenAiModalitySupport::OutputOnly)
    );
}

#[test]
fn google_model_catalog_includes_gemini_2_5_flash_metadata_from_docs() {
    let model =
        google_model_catalog_entry("gemini-2.5-flash").expect("gemini-2.5-flash page must exist");
    assert_eq!(model.display_name, "Gemini 2.5 Flash");
    assert_eq!(model.model_code, "gemini-2.5-flash");
    assert_eq!(
        model.supported_data_types.input,
        vec!["text", "image", "video", "audio", "pdf"]
    );
    assert_eq!(model.supported_data_types.output, vec!["text"]);
    assert_eq!(
        model.limits.get("input_token_limit").map(String::as_str),
        Some("1048576")
    );
    assert_eq!(
        model.capabilities.get("cached_content").map(String::as_str),
        Some("supported")
    );
    assert!(
        model
            .versions
            .iter()
            .any(|version| version.model == "gemini-2.5-flash-preview-05-20")
    );
}

#[test]
fn google_model_catalog_resolves_embedding_page_by_concrete_model_code() {
    let model = google_model_catalog_entry_by_model("gemini-embedding-001")
        .expect("gemini-embedding-001 must resolve through versions");
    assert_eq!(model.display_name, "Gemini Embedding");
    assert_eq!(model.supported_data_types.input, vec!["text"]);
    assert_eq!(model.supported_data_types.output, vec!["embeddings"]);
    assert_eq!(
        model
            .limits
            .get("output_dimension_size")
            .map(String::as_str),
        Some("3072")
    );
}

#[test]
fn google_model_catalog_maps_imagen_variants_back_to_the_same_doc_page() {
    let model = google_model_catalog_entry_by_model("imagen-4.0-ultra-generate-001")
        .expect("imagen ultra variant must resolve through versions");
    assert_eq!(model.display_name, "Imagen 4");
    assert_eq!(model.model_code, "imagen-4.0-generate-001");
    assert_eq!(model.supported_data_types.output, vec!["image"]);
    assert_eq!(
        model.limits.get("output_images").map(String::as_str),
        Some("up_to_4")
    );
}

#[test]
fn anthropic_model_catalog_includes_opus_4_6_metadata_from_docs() {
    let model =
        anthropic_model_catalog_entry("claude-opus-4-6").expect("claude-opus-4-6 must exist");
    assert_eq!(model.display_name, "Claude Opus 4.6");
    assert_eq!(model.api_model_id, "claude-opus-4-6");
    assert_eq!(model.api_alias.as_deref(), Some("claude-opus-4-6"));
    assert_eq!(
        model.bedrock_model_id.as_deref(),
        Some("anthropic.claude-opus-4-6-v1")
    );
    assert_eq!(model.vertex_model_id.as_deref(), Some("claude-opus-4-6"));
    assert_eq!(model.context_window_tokens, Some(200_000));
    assert_eq!(model.beta_context_window_tokens, Some(1_000_000));
    assert_eq!(model.max_output_tokens, Some(128_000));
    assert_eq!(model.status, AnthropicModelStatus::Active);
    assert_eq!(
        model.not_retired_before.as_deref(),
        Some("February 5, 2027")
    );
    assert_eq!(
        model.beta_headers.get("context_1m").map(String::as_str),
        Some("context-1m-2025-08-07")
    );
    assert_eq!(model.features.get("adaptive_thinking"), Some(&true));
    let pricing = model.pricing.as_ref().expect("pricing must exist");
    assert_eq!(pricing.input_usd_per_mtok, "5");
    assert_eq!(pricing.output_usd_per_mtok, "25");
}

#[test]
fn anthropic_model_catalog_resolves_haiku_alias_and_platform_ids() {
    let from_alias = anthropic_model_catalog_entry_by_model("claude-haiku-4-5")
        .expect("haiku alias must resolve");
    let from_vertex = anthropic_model_catalog_entry_by_model("claude-haiku-4-5@20251001")
        .expect("haiku vertex id must resolve");
    let from_bedrock =
        anthropic_model_catalog_entry_by_model("anthropic.claude-haiku-4-5-20251001-v1:0")
            .expect("haiku bedrock id must resolve");
    assert_eq!(from_alias.display_name, "Claude Haiku 4.5");
    assert_eq!(from_vertex.display_name, "Claude Haiku 4.5");
    assert_eq!(from_bedrock.display_name, "Claude Haiku 4.5");
    assert_eq!(from_alias.api_model_id, "claude-haiku-4-5-20251001");
    assert_eq!(from_alias.api_alias.as_deref(), Some("claude-haiku-4-5"));
    assert_eq!(from_alias.features.get("adaptive_thinking"), Some(&false));
}

#[test]
fn anthropic_model_catalog_includes_sonnet_4_6_thinking_and_cutoff_metadata() {
    let model = anthropic_model_catalog_entry_by_model("claude-sonnet-4-6")
        .expect("claude-sonnet-4-6 must resolve");
    assert_eq!(model.display_name, "Claude Sonnet 4.6");
    assert_eq!(model.comparative_latency.as_deref(), Some("fast"));
    assert_eq!(model.reliable_knowledge_cutoff.as_deref(), Some("Aug 2025"));
    assert_eq!(model.training_data_cutoff.as_deref(), Some("Jan 2026"));
    assert_eq!(model.features.get("extended_thinking"), Some(&true));
    assert_eq!(model.features.get("priority_tier"), Some(&true));
    assert_eq!(model.output_modalities, vec!["text"]);
    assert_eq!(model.status, AnthropicModelStatus::Active);
}

#[test]
fn anthropic_model_catalog_includes_retired_and_deprecated_models() {
    let sonnet_37 = anthropic_model_catalog_entry_by_model("claude-3-7-sonnet-latest")
        .expect("claude-3-7-sonnet-latest must resolve");
    assert_eq!(sonnet_37.api_model_id, "claude-3-7-sonnet-20250219");
    assert_eq!(sonnet_37.status, AnthropicModelStatus::Retired);
    assert_eq!(sonnet_37.deprecated_on.as_deref(), Some("October 28, 2025"));
    assert_eq!(
        sonnet_37.retirement_date.as_deref(),
        Some("February 19, 2026")
    );
    assert_eq!(
        sonnet_37.recommended_replacement.as_deref(),
        Some("claude-opus-4-6")
    );

    let haiku_3 = anthropic_model_catalog_entry("claude-3-haiku-20240307")
        .expect("claude-3-haiku-20240307 must exist");
    assert_eq!(haiku_3.status, AnthropicModelStatus::Deprecated);
    assert_eq!(haiku_3.retirement_date.as_deref(), Some("April 20, 2026"));
    assert_eq!(
        haiku_3.recommended_replacement.as_deref(),
        Some("claude-haiku-4-5-20251001")
    );
}

#[test]
fn anthropic_model_catalog_tracks_older_public_models_and_replacements() {
    let opus_41 = anthropic_model_catalog_entry_by_model("claude-opus-4-1")
        .expect("claude-opus-4-1 alias must resolve");
    assert_eq!(opus_41.api_model_id, "claude-opus-4-1-20250805");
    assert_eq!(opus_41.status, AnthropicModelStatus::Active);
    assert_eq!(
        opus_41.not_retired_before.as_deref(),
        Some("August 5, 2026")
    );

    let claude_1 = anthropic_model_catalog_entry("claude-1.0").expect("claude-1.0 must exist");
    assert_eq!(claude_1.status, AnthropicModelStatus::Retired);
    assert_eq!(
        claude_1.retirement_date.as_deref(),
        Some("November 6, 2024")
    );
    assert_eq!(
        claude_1.recommended_replacement.as_deref(),
        Some("claude-haiku-4-5-20251001")
    );
}
