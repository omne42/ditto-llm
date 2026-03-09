#![cfg(any(feature = "provider-openai", feature = "openai"))]

use std::collections::BTreeSet;

use ditto_llm::{
    BehaviorSupport, CacheUsageReportingKind, CapabilityKind, ContextCacheModeId, Env,
    OperationKind, ProviderConfig, RealtimeSessionRequest, ReasoningActivationKind,
    ReasoningOutputMode, builtin_registry,
};

fn openai_env() -> Env {
    Env::parse_dotenv("OPENAI_API_KEY=sk-test\n")
}

fn openai_config(default_model: &str) -> ProviderConfig {
    ProviderConfig {
        base_url: Some("https://api.openai.com/v1".to_string()),
        default_model: Some(default_model.to_string()),
        ..ProviderConfig::default()
    }
}

#[test]
fn openai_catalog_runtime_spec_matches_enabled_capabilities() {
    let plugin = builtin_registry()
        .plugin("openai")
        .expect("official openai plugin should be available");
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
    {
        expected.insert(CapabilityKind::IMAGE_GENERATION.as_str());
        expected.insert(CapabilityKind::IMAGE_EDIT.as_str());
    }
    #[cfg(feature = "audio")]
    {
        expected.insert(CapabilityKind::AUDIO_SPEECH.as_str());
        expected.insert(CapabilityKind::AUDIO_TRANSCRIPTION.as_str());
    }
    #[cfg(feature = "moderations")]
    expected.insert(CapabilityKind::MODERATION.as_str());
    #[cfg(feature = "batches")]
    expected.insert(CapabilityKind::BATCH.as_str());
    #[cfg(feature = "realtime")]
    expected.insert(CapabilityKind::REALTIME.as_str());
    #[cfg(feature = "videos")]
    expected.insert(CapabilityKind::VIDEO_GENERATION.as_str());

    assert_eq!(actual, expected);
    assert!(
        plugin
            .capability_resolution(Some("gpt-4.1"))
            .effective_supports(CapabilityKind::LLM)
    );

    #[cfg(feature = "embeddings")]
    assert!(
        plugin
            .capability_resolution(Some("text-embedding-3-small"))
            .effective_supports(CapabilityKind::EMBEDDING)
    );

    #[cfg(feature = "images")]
    {
        let image_caps = plugin.capability_resolution(Some("gpt-image-1"));
        assert!(image_caps.effective_supports(CapabilityKind::IMAGE_GENERATION));
        assert!(image_caps.effective_supports(CapabilityKind::IMAGE_EDIT));
        assert!(
            builtin_registry()
                .resolve("openai", "gpt-image-1", OperationKind::IMAGE_EDIT)
                .is_some()
        );
    }

    #[cfg(feature = "audio")]
    {
        assert!(
            plugin
                .capability_resolution(Some("tts-1"))
                .effective_supports(CapabilityKind::AUDIO_SPEECH)
        );
        assert!(
            plugin
                .capability_resolution(Some("whisper-1"))
                .effective_supports(CapabilityKind::AUDIO_TRANSCRIPTION)
        );
    }

    #[cfg(feature = "moderations")]
    assert!(
        plugin
            .capability_resolution(Some("omni-moderation-latest"))
            .effective_supports(CapabilityKind::MODERATION)
    );

    #[cfg(feature = "batches")]
    assert!(
        plugin
            .capability_resolution(None)
            .effective_supports(CapabilityKind::BATCH)
    );

    #[cfg(feature = "realtime")]
    {
        assert!(
            plugin
                .capability_resolution(Some("gpt-realtime"))
                .effective_supports(CapabilityKind::REALTIME)
        );
        let route = builtin_registry()
            .resolve_runtime_route(ditto_llm::RuntimeRouteRequest::new(
                "openai",
                Some("gpt-realtime"),
                OperationKind::REALTIME_SESSION,
            ))
            .expect("official realtime route should resolve");
        assert_eq!(
            route.url,
            "wss://api.openai.com/v1/realtime?model=gpt-realtime"
        );
    }

    #[cfg(feature = "videos")]
    {
        assert!(
            plugin
                .capability_resolution(Some("sora-2"))
                .effective_supports(CapabilityKind::VIDEO_GENERATION)
        );
        let route = builtin_registry()
            .resolve_runtime_route(ditto_llm::RuntimeRouteRequest::new(
                "openai",
                Some("sora-2"),
                OperationKind::VIDEO_GENERATION,
            ))
            .expect("official video route should resolve");
        assert_eq!(route.url, "https://api.openai.com/v1/videos");
        assert_eq!(route.invocation.surface.as_str(), "video.generation.async");
        assert_eq!(route.invocation.wire_protocol.as_str(), "openai.videos");
        assert_eq!(route.invocation.model, "sora-2");
        assert_eq!(route.invocation.async_job, Some(true));
    }
}

#[test]
fn openai_text_model_routes_and_behaviors_match_official_catalog() {
    let registry = builtin_registry();

    assert!(
        registry
            .resolve("openai", "davinci-002", OperationKind::TEXT_COMPLETION)
            .is_some()
    );
    assert!(
        registry
            .resolve("openai", "gpt-4", OperationKind::CHAT_COMPLETION)
            .is_some()
    );
    assert!(
        registry
            .resolve("openai", "gpt-4", OperationKind::RESPONSE)
            .is_none()
    );
    assert!(
        registry
            .resolve("openai", "gpt-5-pro", OperationKind::RESPONSE)
            .is_some()
    );
    assert!(
        registry
            .resolve("openai", "gpt-5-pro", OperationKind::CHAT_COMPLETION)
            .is_none()
    );
    assert!(
        registry
            .resolve("openai", "gpt-5.1-codex-max", OperationKind::RESPONSE)
            .is_some()
    );
    assert!(
        registry
            .resolve(
                "openai",
                "gpt-5.1-codex-max",
                OperationKind::CHAT_COMPLETION
            )
            .is_none()
    );

    let legacy = registry
        .behavior("openai", "davinci-002", OperationKind::TEXT_COMPLETION)
        .expect("legacy completions behavior should exist");
    assert_eq!(legacy.tool_calls, BehaviorSupport::Unsupported);
    assert_eq!(legacy.reasoning_output, ReasoningOutputMode::Unsupported);
    assert_eq!(
        legacy.reasoning_activation,
        ReasoningActivationKind::Unavailable
    );
    assert!(legacy.context_cache_modes.is_empty());
    assert_eq!(
        legacy.cache_usage_reporting,
        CacheUsageReportingKind::Unknown
    );

    let gpt4o = registry
        .behavior("openai", "gpt-4o", OperationKind::CHAT_COMPLETION)
        .expect("gpt-4o chat behavior should exist");
    assert_eq!(gpt4o.tool_calls, BehaviorSupport::Supported);
    assert_eq!(gpt4o.tool_choice_required, BehaviorSupport::Supported);
    assert_eq!(gpt4o.reasoning_output, ReasoningOutputMode::Unsupported);
    assert_eq!(
        gpt4o.reasoning_activation,
        ReasoningActivationKind::Unavailable
    );
    assert!(
        gpt4o
            .context_cache_modes
            .contains(&ContextCacheModeId::PASSIVE)
    );
    assert!(gpt4o.context_cache_default_enabled);
    assert_eq!(
        gpt4o.cache_usage_reporting,
        CacheUsageReportingKind::StandardUsage
    );

    let gpt5_chat = registry
        .behavior("openai", "gpt-5", OperationKind::CHAT_COMPLETION)
        .expect("gpt-5 chat behavior should exist");
    assert_eq!(gpt5_chat.tool_calls, BehaviorSupport::Supported);
    assert_eq!(
        gpt5_chat.reasoning_activation,
        ReasoningActivationKind::OpenAiReasoningEffort
    );
    assert_eq!(gpt5_chat.reasoning_output, ReasoningOutputMode::Unsupported);
    assert!(
        gpt5_chat
            .context_cache_modes
            .contains(&ContextCacheModeId::PASSIVE)
    );

    let gpt5_response = registry
        .behavior("openai", "gpt-5", OperationKind::RESPONSE)
        .expect("gpt-5 response behavior should exist");
    assert_eq!(gpt5_response.tool_calls, BehaviorSupport::Supported);
    assert_eq!(
        gpt5_response.tool_choice_required,
        BehaviorSupport::Supported
    );
    assert_eq!(
        gpt5_response.reasoning_output,
        ReasoningOutputMode::Optional
    );
    assert_eq!(
        gpt5_response.reasoning_activation,
        ReasoningActivationKind::OpenAiReasoningEffort
    );
    assert!(
        gpt5_response
            .context_cache_modes
            .contains(&ContextCacheModeId::PASSIVE)
    );
    assert!(gpt5_response.context_cache_default_enabled);
    assert_eq!(
        gpt5_response.cache_usage_reporting,
        CacheUsageReportingKind::StandardUsage
    );

    let search_preview = registry
        .behavior(
            "openai",
            "gpt-4o-search-preview",
            OperationKind::CHAT_COMPLETION,
        )
        .expect("search preview behavior should exist");
    assert_eq!(search_preview.tool_calls, BehaviorSupport::Unsupported);
    assert_eq!(
        search_preview.tool_choice_required,
        BehaviorSupport::Unsupported
    );
}

#[cfg(all(
    feature = "gateway-translation",
    feature = "provider-openai",
    feature = "cap-llm"
))]
#[tokio::test]
async fn gateway_builder_constructs_official_openai_llm() -> ditto_llm::Result<()> {
    let model = ditto_llm::gateway::translation::build_language_model(
        "openai",
        &openai_config("gpt-4.1"),
        &openai_env(),
    )
    .await?;

    assert_eq!(model.provider(), "openai");
    assert_eq!(model.model_id(), "gpt-4.1");
    Ok(())
}

#[cfg(all(
    feature = "gateway-translation",
    feature = "provider-openai",
    feature = "cap-llm"
))]
#[tokio::test]
async fn gateway_builder_constructs_official_openai_chat_only_model() -> ditto_llm::Result<()> {
    let model = ditto_llm::gateway::translation::build_language_model(
        "openai",
        &openai_config("gpt-4"),
        &openai_env(),
    )
    .await?;

    assert_eq!(model.provider(), "openai");
    assert_eq!(model.model_id(), "gpt-4");
    Ok(())
}

#[cfg(all(
    feature = "gateway-translation",
    feature = "provider-openai",
    feature = "cap-llm"
))]
#[tokio::test]
async fn gateway_builder_constructs_official_openai_legacy_completion_model()
-> ditto_llm::Result<()> {
    let model = ditto_llm::gateway::translation::build_language_model(
        "openai",
        &openai_config("davinci-002"),
        &openai_env(),
    )
    .await?;

    assert_eq!(model.provider(), "openai");
    assert_eq!(model.model_id(), "davinci-002");
    Ok(())
}

#[cfg(all(
    feature = "gateway-translation",
    feature = "provider-openai",
    feature = "cap-embedding"
))]
#[tokio::test]
async fn gateway_builder_constructs_official_openai_embeddings() -> ditto_llm::Result<()> {
    let model = ditto_llm::gateway::translation::build_embedding_model(
        "openai",
        &openai_config("text-embedding-3-small"),
        &openai_env(),
    )
    .await?
    .expect("embedding builder should return a model");

    assert_eq!(model.provider(), "openai");
    assert_eq!(model.model_id(), "text-embedding-3-small");
    Ok(())
}

#[cfg(all(
    feature = "gateway-translation",
    feature = "provider-openai",
    feature = "cap-image-generation"
))]
#[tokio::test]
async fn gateway_builder_constructs_official_openai_image_generation() -> ditto_llm::Result<()> {
    let model = ditto_llm::gateway::translation::build_image_generation_model(
        "openai",
        &openai_config("gpt-image-1"),
        &openai_env(),
    )
    .await?
    .expect("image generation builder should return a model");

    assert_eq!(model.provider(), "openai");
    assert_eq!(model.model_id(), "gpt-image-1");
    Ok(())
}

#[cfg(all(
    feature = "gateway-translation",
    feature = "provider-openai",
    feature = "cap-image-edit"
))]
#[tokio::test]
async fn gateway_builder_constructs_official_openai_image_edit() -> ditto_llm::Result<()> {
    let model = ditto_llm::gateway::translation::build_image_edit_model(
        "openai",
        &openai_config("gpt-image-1"),
        &openai_env(),
    )
    .await?
    .expect("image edit builder should return a model");

    assert_eq!(model.provider(), "openai");
    assert_eq!(model.model_id(), "gpt-image-1");
    Ok(())
}

#[cfg(all(
    feature = "gateway-translation",
    feature = "provider-openai",
    feature = "cap-audio-transcription"
))]
#[tokio::test]
async fn gateway_builder_constructs_official_openai_audio_transcription() -> ditto_llm::Result<()> {
    let model = ditto_llm::gateway::translation::build_audio_transcription_model(
        "openai",
        &openai_config("whisper-1"),
        &openai_env(),
    )
    .await?
    .expect("audio transcription builder should return a model");

    assert_eq!(model.provider(), "openai");
    assert_eq!(model.model_id(), "whisper-1");
    Ok(())
}

#[cfg(all(
    feature = "gateway-translation",
    feature = "provider-openai",
    feature = "cap-audio-speech"
))]
#[tokio::test]
async fn gateway_builder_constructs_official_openai_speech() -> ditto_llm::Result<()> {
    let model = ditto_llm::gateway::translation::build_speech_model(
        "openai",
        &openai_config("tts-1"),
        &openai_env(),
    )
    .await?
    .expect("speech builder should return a model");

    assert_eq!(model.provider(), "openai");
    assert_eq!(model.model_id(), "tts-1");
    Ok(())
}

#[cfg(all(
    feature = "gateway-translation",
    feature = "provider-openai",
    feature = "cap-moderation"
))]
#[tokio::test]
async fn gateway_builder_constructs_official_openai_moderation() -> ditto_llm::Result<()> {
    let model = ditto_llm::gateway::translation::build_moderation_model(
        "openai",
        &openai_config("omni-moderation-latest"),
        &openai_env(),
    )
    .await?
    .expect("moderation builder should return a model");

    assert_eq!(model.provider(), "openai");
    assert_eq!(model.model_id(), "omni-moderation-latest");
    Ok(())
}

#[cfg(all(
    feature = "gateway-translation",
    feature = "provider-openai",
    feature = "cap-batch"
))]
#[tokio::test]
async fn gateway_builder_constructs_official_openai_batch_client() -> ditto_llm::Result<()> {
    let client = ditto_llm::gateway::translation::build_batch_client(
        "openai",
        &openai_config("gpt-4.1"),
        &openai_env(),
    )
    .await?
    .expect("batch builder should return a client");

    assert_eq!(client.provider(), "openai");
    Ok(())
}

#[cfg(all(
    feature = "gateway-translation",
    feature = "provider-openai",
    feature = "videos"
))]
#[tokio::test]
async fn gateway_builder_constructs_official_openai_video_generation() -> ditto_llm::Result<()> {
    let model = ditto_llm::gateway::translation::build_video_generation_model(
        "openai",
        &openai_config("sora-2"),
        &openai_env(),
    )
    .await?
    .expect("video generation builder should return a model");

    assert_eq!(model.provider(), "openai");
    assert_eq!(model.model_id(), "sora-2");
    Ok(())
}

#[cfg(all(
    feature = "gateway-translation",
    feature = "provider-openai",
    feature = "cap-realtime"
))]
#[tokio::test]
async fn gateway_builder_constructs_official_openai_realtime() -> ditto_llm::Result<()> {
    let model = ditto_llm::gateway::translation::build_realtime_session_model(
        "openai",
        &openai_config("gpt-realtime"),
        &openai_env(),
    )
    .await?
    .expect("realtime builder should return a model");

    assert_eq!(model.provider(), "openai");
    assert_eq!(model.model_id(), "gpt-realtime");

    let session = model
        .prepare_session(RealtimeSessionRequest::default())
        .await?;
    assert_eq!(
        session.url,
        "wss://api.openai.com/v1/realtime?model=gpt-realtime"
    );
    assert_eq!(
        session.headers.get("openai-beta").map(String::as_str),
        Some("realtime=v1")
    );
    Ok(())
}
