use ditto_llm::{
    ConfigScope, DittoError, OperationKind, ProviderApi, ProviderAuthType, ProviderNamespace,
    ProviderUpsertRequest, RuntimeRouteRequest, builtin_provider_capability_summaries,
    builtin_provider_presets, upsert_provider_config,
};

fn is_default_core_build() -> bool {
    !(cfg!(feature = "provider-openai")
        || cfg!(feature = "provider-anthropic")
        || cfg!(feature = "provider-google")
        || cfg!(feature = "provider-cohere")
        || cfg!(feature = "provider-bedrock")
        || cfg!(feature = "provider-vertex")
        || cfg!(feature = "provider-bailian")
        || cfg!(feature = "provider-deepseek")
        || cfg!(feature = "provider-doubao")
        || cfg!(feature = "provider-hunyuan")
        || cfg!(feature = "provider-kimi")
        || cfg!(feature = "provider-minimax")
        || cfg!(feature = "provider-openrouter")
        || cfg!(feature = "provider-qianfan")
        || cfg!(feature = "provider-xai")
        || cfg!(feature = "provider-zhipu")
        || cfg!(feature = "embeddings")
        || cfg!(feature = "images")
        || cfg!(feature = "audio")
        || cfg!(feature = "moderations")
        || cfg!(feature = "rerank")
        || cfg!(feature = "batches")
        || cfg!(feature = "realtime"))
}

#[test]
fn default_core_exposes_only_generic_openai_compatible_llm_surface() {
    let presets = builtin_provider_presets();
    let preset = presets
        .iter()
        .find(|preset| preset.provider == "openai-compatible")
        .expect("openai-compatible preset should exist");
    assert_eq!(preset.provider, "openai-compatible");

    let summaries = builtin_provider_capability_summaries();
    let summary = summaries
        .iter()
        .find(|summary| summary.provider == "openai-compatible")
        .expect("openai-compatible summary should exist");
    assert!(
        summary
            .capabilities
            .iter()
            .any(|capability| capability.as_str() == "llm")
    );

    if is_default_core_build() {
        assert_eq!(presets.len(), 1);
        assert_eq!(summaries.len(), 1);
        assert_eq!(summary.capabilities.len(), 1);
    }
}

#[test]
fn default_core_rejects_non_llm_runtime_routes() {
    let result = ditto_llm::builtin_registry().resolve_runtime_route(
        RuntimeRouteRequest::new(
            "openai-compatible",
            Some("text-embedding-3-small"),
            OperationKind::EMBEDDING,
        )
        .with_provider_config(&ditto_llm::ProviderConfig {
            base_url: Some("https://proxy.example/v1".to_string()),
            default_model: Some("text-embedding-3-small".to_string()),
            ..ditto_llm::ProviderConfig::default()
        })
        .with_required_capability(ditto_llm::CapabilityKind::EMBEDDING),
    );

    if cfg!(feature = "embeddings") {
        assert!(
            result.is_ok(),
            "embedding-enabled builds should expose embedding routes"
        );
    } else {
        let err = result.expect_err("default core should not expose embedding routes");
        assert!(matches!(
            err,
            DittoError::ProviderResolution(
                ditto_llm::ProviderResolutionError::RuntimeRouteCapabilityUnsupported { .. }
            )
        ));
    }
}

#[tokio::test]
async fn default_core_provider_template_stays_minimal() -> ditto_llm::Result<()> {
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
        register_models: false,
        model_limit: None,
    })
    .await?;
    assert!(report.updated);

    let parsed = tokio::fs::read_to_string(&config_path).await?;
    let value = toml::from_str::<toml::Value>(&parsed)
        .map_err(|err| ditto_llm::DittoError::Config(format!("parse test toml: {err}")))?;
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
async fn default_core_provider_template_rejects_unsupported_enabled_capability()
-> ditto_llm::Result<()> {
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
        let err = result.expect_err("default core should reject unsupported capabilities");
        assert!(matches!(
            err,
            DittoError::ProviderResolution(
                ditto_llm::ProviderResolutionError::ConfiguredCapabilityUnsupported { ref provider, ref capability }
            ) if provider == "openai-compatible" && capability == "embedding"
        ));
    }

    Ok(())
}
