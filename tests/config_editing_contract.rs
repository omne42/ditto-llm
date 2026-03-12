#![cfg(feature = "config-editing")]

use ditto_llm::config::ProviderApi;
use ditto_llm::config_editing::{
    ConfigScope, ProviderAuthType, ProviderNamespace, ProviderUpsertRequest, upsert_provider_config,
};
use ditto_llm::foundation::error::DittoError;

#[tokio::test]
async fn provider_template_stays_minimal() -> ditto_llm::foundation::error::Result<()> {
    let temp = tempfile::tempdir()?;
    let config_path = temp.path().join("config_local.toml");
    tokio::fs::write(&config_path, "[project_config]\nenabled = true\n").await?;

    let report = upsert_provider_config(ProviderUpsertRequest {
        name: "openai-compatible".to_string(),
        config_path: Some(config_path.clone()),
        root: None,
        scope: ConfigScope::Workspace,
        namespace: ProviderNamespace::Openai,
        provider: None,
        enabled_capabilities: Vec::new(),
        base_url: Some("https://proxy.example/v1".to_string()),
        default_model: Some("chat-model".to_string()),
        upstream_api: Some(ProviderApi::OpenaiChatCompletions),
        normalize_to: None,
        normalize_endpoint: None,
        auth_type: ProviderAuthType::ApiKeyEnv,
        auth_keys: Vec::new(),
        auth_param: None,
        auth_header: None,
        auth_prefix: None,
        auth_command: Vec::new(),
        set_default: false,
        set_default_model: false,
        tools: None,
        vision: None,
        reasoning: None,
        json_schema: None,
        streaming: None,
        prompt_cache: None,
        discover_models: false,
        discovery_api_key: None,
        model_whitelist: Vec::new(),
        register_models: false,
        model_limit: None,
    })
    .await?;
    assert!(report.updated);

    let parsed = tokio::fs::read_to_string(&config_path).await?;
    let value = toml::from_str::<toml::Value>(&parsed)
        .map_err(|err| DittoError::Config(format!("parse test toml: {err}")))?;
    let provider = value
        .get("openai")
        .and_then(|v| v.get("providers"))
        .and_then(|v| v.get("openai-compatible"))
        .expect("provider table should exist");

    assert_eq!(
        provider.get("base_url").and_then(toml::Value::as_str),
        Some("https://proxy.example/v1")
    );
    assert_eq!(
        provider.get("default_model").and_then(toml::Value::as_str),
        Some("chat-model")
    );
    assert_eq!(
        provider.get("provider").and_then(toml::Value::as_str),
        Some("openai-compatible")
    );
    let enabled_capabilities = provider
        .get("enabled_capabilities")
        .and_then(toml::Value::as_array)
        .expect("enabled capabilities should exist")
        .iter()
        .filter_map(toml::Value::as_str)
        .collect::<Vec<_>>();
    assert_eq!(enabled_capabilities, vec!["llm"]);
    assert_eq!(
        provider.get("upstream_api").and_then(toml::Value::as_str),
        Some("openai_chat_completions")
    );
    assert!(provider.get("normalize_to").is_none());
    assert!(provider.get("normalize_endpoint").is_none());
    assert!(provider.get("capabilities").is_none());
    assert_eq!(
        provider
            .get("auth")
            .and_then(|v| v.get("type"))
            .and_then(toml::Value::as_str),
        Some("api_key_env")
    );
    let keys = provider
        .get("auth")
        .and_then(|v| v.get("keys"))
        .and_then(toml::Value::as_array)
        .expect("auth keys should exist")
        .iter()
        .filter_map(toml::Value::as_str)
        .collect::<Vec<_>>();
    assert_eq!(keys, vec!["OPENAI_COMPAT_API_KEY", "OPENAI_API_KEY"]);

    Ok(())
}

#[tokio::test]
async fn editing_rejects_unsupported_enabled_capability() -> ditto_llm::foundation::error::Result<()>
{
    let temp = tempfile::tempdir()?;
    let config_path = temp.path().join("config_local.toml");
    tokio::fs::write(&config_path, "[project_config]\nenabled = true\n").await?;

    let result = upsert_provider_config(ProviderUpsertRequest {
        name: "openai-compatible".to_string(),
        config_path: Some(config_path),
        root: None,
        scope: ConfigScope::Workspace,
        namespace: ProviderNamespace::Openai,
        provider: Some("openai-compatible".to_string()),
        enabled_capabilities: vec!["embedding".to_string()],
        base_url: Some("https://proxy.example/v1".to_string()),
        default_model: Some("text-embedding-3-small".to_string()),
        upstream_api: Some(ProviderApi::OpenaiChatCompletions),
        normalize_to: None,
        normalize_endpoint: None,
        auth_type: ProviderAuthType::ApiKeyEnv,
        auth_keys: Vec::new(),
        auth_param: None,
        auth_header: None,
        auth_prefix: None,
        auth_command: Vec::new(),
        set_default: false,
        set_default_model: false,
        tools: None,
        vision: None,
        reasoning: None,
        json_schema: None,
        streaming: None,
        prompt_cache: None,
        discover_models: false,
        discovery_api_key: None,
        model_whitelist: Vec::new(),
        register_models: false,
        model_limit: None,
    })
    .await;

    if cfg!(feature = "embeddings") {
        assert!(
            result.is_ok(),
            "embedding-enabled builds should accept embedding capability"
        );
    } else {
        let err = result.expect_err("editing should reject unsupported capabilities");
        assert!(matches!(
            err,
            DittoError::ProviderResolution(
                ditto_llm::foundation::error::ProviderResolutionError::ConfiguredCapabilityUnsupported {
                    ref provider,
                    ref capability
                }
            ) if provider == "openai-compatible" && capability == "embedding"
        ));
    }

    Ok(())
}

#[tokio::test]
async fn provider_upsert_accepts_caller_resolved_model_whitelist()
-> ditto_llm::foundation::error::Result<()> {
    let temp = tempfile::tempdir()?;
    let config_path = temp.path().join("config_local.toml");
    tokio::fs::write(&config_path, "[project_config]\nenabled = true\n").await?;

    let report = upsert_provider_config(ProviderUpsertRequest {
        name: "openai-compatible".to_string(),
        config_path: Some(config_path.clone()),
        root: None,
        scope: ConfigScope::Workspace,
        namespace: ProviderNamespace::Openai,
        provider: Some("openai-compatible".to_string()),
        enabled_capabilities: Vec::new(),
        base_url: Some("https://proxy.example/v1".to_string()),
        default_model: Some("gpt-4o-mini".to_string()),
        upstream_api: Some(ProviderApi::OpenaiChatCompletions),
        normalize_to: None,
        normalize_endpoint: None,
        auth_type: ProviderAuthType::ApiKeyEnv,
        auth_keys: vec!["OPENAI_COMPAT_API_KEY".to_string()],
        auth_param: None,
        auth_header: None,
        auth_prefix: None,
        auth_command: Vec::new(),
        set_default: false,
        set_default_model: false,
        tools: None,
        vision: None,
        reasoning: None,
        json_schema: None,
        streaming: None,
        prompt_cache: None,
        discover_models: true,
        discovery_api_key: None,
        model_whitelist: vec!["gpt-4o-mini".to_string(), "gpt-4.1-mini".to_string()],
        register_models: true,
        model_limit: None,
    })
    .await?;

    assert_eq!(report.discovered_models, 2);
    let parsed = tokio::fs::read_to_string(&config_path).await?;
    let value = toml::from_str::<toml::Value>(&parsed)
        .map_err(|err| DittoError::Config(format!("parse test toml: {err}")))?;
    let provider = value
        .get("openai")
        .and_then(|v| v.get("providers"))
        .and_then(|v| v.get("openai-compatible"))
        .expect("provider table should exist");
    let whitelist = provider
        .get("model_whitelist")
        .and_then(toml::Value::as_array)
        .expect("model_whitelist should exist")
        .iter()
        .filter_map(toml::Value::as_str)
        .collect::<Vec<_>>();
    assert_eq!(whitelist, vec!["gpt-4o-mini", "gpt-4.1-mini"]);

    let registered = value
        .get("openai")
        .and_then(|v| v.get("models"))
        .expect("openai.models should exist");
    assert!(registered.get("gpt-4o-mini").is_some());
    assert!(registered.get("gpt-4.1-mini").is_some());

    Ok(())
}
