#![cfg(feature = "provider-google")]

use std::collections::BTreeSet;

use ditto_core::capabilities::RealtimeSessionRequest;
use ditto_core::catalog::builtin_registry;
use ditto_core::config::{Env, ProviderConfig};
use ditto_core::contracts::{
    CapabilityKind, OperationKind, ProviderProtocolFamily, RuntimeRouteRequest,
};
use ditto_core::runtime::resolve_builtin_runtime_route;

fn google_env() -> Env {
    Env::parse_dotenv("GOOGLE_API_KEY=test-google-key\n")
}

fn google_config(default_model: &str) -> ProviderConfig {
    ProviderConfig {
        base_url: Some("https://generativelanguage.googleapis.com/v1beta".to_string()),
        default_model: Some(default_model.to_string()),
        ..ProviderConfig::default()
    }
}

#[test]
fn google_catalog_runtime_spec_matches_enabled_capabilities() {
    let plugin = builtin_registry()
        .plugin("google")
        .expect("google plugin should be available");
    let runtime_spec = plugin.runtime_spec();
    let actual = runtime_spec
        .capabilities
        .iter()
        .map(|capability| capability.as_str())
        .collect::<BTreeSet<_>>();

    let mut expected = BTreeSet::from([CapabilityKind::LLM.as_str()]);
    #[cfg(feature = "embeddings")]
    expected.insert(CapabilityKind::EMBEDDING.as_str());
    #[cfg(feature = "images")]
    expected.insert(CapabilityKind::IMAGE_GENERATION.as_str());
    #[cfg(feature = "realtime")]
    expected.insert(CapabilityKind::REALTIME.as_str());
    #[cfg(feature = "videos")]
    expected.insert(CapabilityKind::VIDEO_GENERATION.as_str());

    assert_eq!(runtime_spec.protocol_family, ProviderProtocolFamily::Google);
    assert_eq!(actual, expected);
    assert!(
        plugin
            .capability_resolution(Some("gemini-3.1-pro"))
            .effective_supports(CapabilityKind::LLM)
    );

    #[cfg(feature = "embeddings")]
    {
        assert!(
            plugin
                .capability_resolution(Some("gemini-embedding"))
                .effective_supports(CapabilityKind::EMBEDDING)
        );
    }

    #[cfg(feature = "images")]
    {
        assert!(
            plugin
                .capability_resolution(Some("imagen-4"))
                .effective_supports(CapabilityKind::IMAGE_GENERATION)
        );
        let route = resolve_builtin_runtime_route(RuntimeRouteRequest::new(
            "google",
            Some("imagen-4"),
            OperationKind::IMAGE_GENERATION,
        ))
        .expect("google image generation route should resolve");
        assert_eq!(
            route.url,
            "https://generativelanguage.googleapis.com/v1beta/models/imagen-4:predict"
        );
        assert_eq!(route.invocation.surface.as_str(), "image.generation");
    }

    #[cfg(feature = "realtime")]
    {
        assert!(
            plugin
                .capability_resolution(Some("gemini-2.5-flash-live"))
                .effective_supports(CapabilityKind::REALTIME)
        );
        let route = resolve_builtin_runtime_route(RuntimeRouteRequest::new(
            "google",
            Some("gemini-2.5-flash-live"),
            OperationKind::REALTIME_SESSION,
        ))
        .expect("google realtime route should resolve");
        assert_eq!(
            route.url,
            "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent"
        );
        assert_eq!(route.invocation.surface.as_str(), "realtime.websocket");
    }

    #[cfg(feature = "videos")]
    {
        assert!(
            plugin
                .capability_resolution(Some("veo-2.0-generate-001"))
                .effective_supports(CapabilityKind::VIDEO_GENERATION)
        );
        let route = resolve_builtin_runtime_route(RuntimeRouteRequest::new(
            "google",
            Some("veo-2.0-generate-001"),
            OperationKind::VIDEO_GENERATION,
        ))
        .expect("google video generation route should resolve");
        assert_eq!(
            route.url,
            "https://generativelanguage.googleapis.com/v1beta/models/veo-2.0-generate-001:predictLongRunning"
        );
        assert_eq!(route.invocation.surface.as_str(), "video.generation");
    }

    assert!(
        builtin_registry()
            .resolve("google", "gemini-3.1-pro", OperationKind::CHAT_COMPLETION)
            .is_some()
    );
}

#[cfg(all(
    feature = "gateway-translation",
    feature = "provider-google",
    feature = "cap-llm"
))]
#[tokio::test]
async fn gateway_builder_constructs_google_llm() -> ditto_core::foundation::error::Result<()> {
    let model = ditto_core::runtime::build_language_model(
        "google",
        &google_config("gemini-3.1-pro"),
        &google_env(),
    )
    .await?;

    assert_eq!(model.provider(), "google");
    assert_eq!(model.model_id(), "gemini-3.1-pro");
    Ok(())
}

#[cfg(all(
    feature = "gateway-translation",
    feature = "provider-google",
    feature = "cap-embedding"
))]
#[tokio::test]
async fn gateway_builder_constructs_google_embeddings() -> ditto_core::foundation::error::Result<()>
{
    let model = ditto_core::runtime::build_embedding_model(
        "google",
        &google_config("gemini-embedding"),
        &google_env(),
    )
    .await?
    .expect("google embedding builder should return a model");

    assert_eq!(model.provider(), "google");
    assert_eq!(model.model_id(), "gemini-embedding");
    Ok(())
}

#[cfg(all(
    feature = "gateway-translation",
    feature = "provider-google",
    feature = "cap-image-generation"
))]
#[tokio::test]
async fn gateway_builder_constructs_google_image_generation()
-> ditto_core::foundation::error::Result<()> {
    let model = ditto_core::runtime::build_image_generation_model(
        "google",
        &google_config("imagen-4"),
        &google_env(),
    )
    .await?
    .expect("google image generation builder should return a model");

    assert_eq!(model.provider(), "google");
    assert_eq!(model.model_id(), "imagen-4");
    Ok(())
}

#[cfg(all(
    feature = "gateway-translation",
    feature = "provider-google",
    feature = "videos"
))]
#[tokio::test]
async fn gateway_builder_constructs_google_video_generation()
-> ditto_core::foundation::error::Result<()> {
    let model = ditto_core::runtime::build_video_generation_model(
        "google",
        &google_config("veo-2.0-generate-001"),
        &google_env(),
    )
    .await?
    .expect("google video generation builder should return a model");

    assert_eq!(model.provider(), "google");
    assert_eq!(model.model_id(), "veo-2.0-generate-001");
    Ok(())
}

#[cfg(all(
    feature = "gateway-translation",
    feature = "provider-google",
    feature = "cap-realtime"
))]
#[tokio::test]
async fn gateway_builder_constructs_google_realtime() -> ditto_core::foundation::error::Result<()> {
    let model = ditto_core::runtime::build_realtime_session_model(
        "google",
        &google_config("gemini-2.5-flash-live"),
        &google_env(),
    )
    .await?
    .expect("google realtime builder should return a model");

    assert_eq!(model.provider(), "google");
    assert_eq!(model.model_id(), "gemini-2.5-flash-live");

    let session = model
        .prepare_session(RealtimeSessionRequest::default())
        .await?;
    assert_eq!(
        session.url,
        "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent"
    );
    assert_eq!(
        session.headers.get("x-goog-api-key").map(String::as_str),
        Some("test-google-key")
    );
    assert_eq!(
        session.setup_payload,
        Some(serde_json::json!({
            "setup": {
                "model": "models/gemini-2.5-flash-live"
            }
        }))
    );
    Ok(())
}
