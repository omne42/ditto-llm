#![cfg(feature = "config-interactive")]

use ditto_core::error::DittoError;
use ditto_server::config_editing::{
    ConfigScope, ModelUpsertRequest, ProviderAuthType, ProviderNamespace, ProviderUpsertRequest,
    complete_model_upsert_request_interactive, complete_provider_upsert_request_interactive,
};

#[test]
fn provider_interactive_requires_tty() {
    let err = complete_provider_upsert_request_interactive(ProviderUpsertRequest {
        name: "openai-compatible".to_string(),
        config_path: None,
        root: None,
        scope: ConfigScope::Workspace,
        namespace: ProviderNamespace::Openai,
        provider: None,
        enabled_capabilities: Vec::new(),
        base_url: None,
        default_model: None,
        upstream_api: None,
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
    .expect_err("non-tty tests should reject interactive mode");

    assert!(matches!(err, DittoError::Config(message) if message.to_string().contains("TTY")));
}

#[test]
fn model_interactive_requires_tty() {
    let err = complete_model_upsert_request_interactive(ModelUpsertRequest {
        name: "gpt-4o-mini".to_string(),
        config_path: None,
        root: None,
        scope: ConfigScope::Workspace,
        provider: None,
        fallback_providers: Vec::new(),
        set_default: false,
        thinking: None,
        context_window: None,
        auto_compact_token_limit: None,
        prompt_cache: None,
    })
    .expect_err("non-tty tests should reject interactive mode");

    assert!(matches!(err, DittoError::Config(message) if message.to_string().contains("TTY")));
}
