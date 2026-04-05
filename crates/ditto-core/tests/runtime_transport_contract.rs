use std::collections::BTreeMap;

#[cfg(feature = "provider-google")]
use ditto_core::config::ProviderApi;
use ditto_core::config::{ProviderAuth, ProviderConfig};
use ditto_core::contracts::{CapabilityKind, OperationKind};
#[cfg(all(feature = "cap-realtime", feature = "provider-openai"))]
use ditto_core::runtime::RuntimeTransportBaseUrlRewrite;
use ditto_core::runtime::{
    RuntimeTransportAuthPlan, RuntimeTransportAuthSelectionSource,
    RuntimeTransportCredentialSource, RuntimeTransportRequest,
};

#[test]
fn runtime_transport_plan_exposes_openai_compatible_http_shape() {
    let provider_config = ProviderConfig {
        base_url: Some("https://proxy.example/v1".to_string()),
        default_model: Some("gpt-4o-mini".to_string()),
        http_headers: BTreeMap::from([("x-tenant".to_string(), "acme".to_string())]),
        http_query_params: BTreeMap::from([("api-version".to_string(), "2024-10-21".to_string())]),
        auth: Some(ProviderAuth::ApiKeyEnv {
            keys: vec!["OPENAI_PROXY_KEY".to_string()],
        }),
        ..ProviderConfig::default()
    };

    let plan = ditto_core::runtime::plan_builtin_runtime_transport(
        RuntimeTransportRequest::new("openai-compatible", None, OperationKind::CHAT_COMPLETION)
            .with_provider_config(&provider_config)
            .with_required_capability(CapabilityKind::LLM),
    )
    .expect("runtime transport plan should resolve");

    assert_eq!(plan.resolved_provider, "openai-compatible");
    assert_eq!(plan.transport, "http");
    assert_eq!(plan.origin_base_url, "https://proxy.example/v1");
    assert_eq!(plan.base_url, "https://proxy.example/v1");
    assert_eq!(plan.base_url_rewrite, None);
    assert_eq!(plan.path, "/v1/chat/completions");
    assert_eq!(
        plan.configured_query_params,
        vec![("api-version".to_string(), "2024-10-21".to_string())]
    );
    assert_eq!(
        plan.query_params,
        vec![("api-version".to_string(), "2024-10-21".to_string())]
    );
    assert_eq!(plan.configured_http_headers, vec!["x-tenant".to_string()]);

    match &plan.auth {
        RuntimeTransportAuthPlan::HttpHeader {
            source,
            header_name,
            prefix,
            credential,
        } => {
            assert_eq!(*source, RuntimeTransportAuthSelectionSource::ProviderConfig);
            assert_eq!(header_name, "authorization");
            assert_eq!(prefix.as_deref(), Some("Bearer "));
            assert_eq!(
                credential,
                &RuntimeTransportCredentialSource::Env {
                    keys: vec!["OPENAI_PROXY_KEY".to_string()],
                }
            );
        }
        other => panic!("unexpected auth plan: {other:?}"),
    }

    let json = serde_json::to_value(&plan).expect("runtime transport plan should serialize");
    assert!(json.get("auth").is_some());
}

#[cfg(feature = "provider-google")]
#[test]
fn runtime_transport_plan_uses_explicit_bearer_auth_for_yunwu() {
    let provider_config = ProviderConfig {
        base_url: Some("https://yunwu.ai/v1beta".to_string()),
        default_model: Some("gemini-3.1-pro".to_string()),
        auth: Some(ProviderAuth::HttpHeaderEnv {
            header: "Authorization".to_string(),
            keys: vec!["YUNWU_API_KEY".to_string()],
            prefix: Some("Bearer ".to_string()),
        }),
        upstream_api: Some(ProviderApi::GeminiGenerateContent),
        ..ProviderConfig::default()
    };

    let plan = ditto_core::runtime::plan_builtin_runtime_transport(
        RuntimeTransportRequest::new("yunwu", None, OperationKind::CHAT_COMPLETION)
            .with_provider_config(&provider_config)
            .with_required_capability(CapabilityKind::LLM),
    )
    .expect("yunwu gemini transport plan should resolve");

    assert_eq!(plan.resolved_provider, "google");
    assert_eq!(plan.origin_base_url, "https://yunwu.ai/v1beta");
    assert_eq!(plan.base_url, "https://yunwu.ai/v1beta");

    match &plan.auth {
        RuntimeTransportAuthPlan::HttpHeader {
            source,
            header_name,
            prefix,
            credential,
        } => {
            assert_eq!(*source, RuntimeTransportAuthSelectionSource::ProviderConfig);
            assert_eq!(header_name, "Authorization");
            assert_eq!(prefix.as_deref(), Some("Bearer "));
            assert_eq!(
                credential,
                &RuntimeTransportCredentialSource::Env {
                    keys: vec!["YUNWU_API_KEY".to_string()],
                }
            );
        }
        other => panic!("unexpected auth plan: {other:?}"),
    }
}

#[cfg(all(feature = "cap-realtime", feature = "provider-openai"))]
#[test]
fn runtime_transport_plan_reports_websocket_base_url_rewrite() {
    let provider_config = ProviderConfig {
        default_model: Some("gpt-realtime".to_string()),
        ..ProviderConfig::default()
    };

    let plan = ditto_core::runtime::plan_builtin_runtime_transport(
        RuntimeTransportRequest::new("openai", None, OperationKind::REALTIME_SESSION)
            .with_provider_config(&provider_config),
    )
    .expect("openai realtime transport plan should resolve");

    assert_eq!(plan.transport, "websocket");
    assert_eq!(plan.origin_base_url, "https://api.openai.com/v1");
    assert_eq!(plan.base_url, "wss://api.openai.com/v1");
    assert_eq!(
        plan.base_url_rewrite,
        Some(RuntimeTransportBaseUrlRewrite::HttpsToSecureWebsocket)
    );
    assert_eq!(plan.path, "/v1/realtime");
    assert!(
        plan.url
            .starts_with("wss://api.openai.com/v1/realtime?model=")
    );
}
