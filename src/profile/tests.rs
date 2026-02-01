use std::collections::BTreeMap;

use crate::{DittoError, Result};

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
