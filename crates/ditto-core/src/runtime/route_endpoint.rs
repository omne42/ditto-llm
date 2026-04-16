use super::route::RuntimeResolvedBaseUrlSource;
use crate::contracts::{RuntimeProviderApi, RuntimeProviderHints, TransportKind};
use crate::error::{ProviderResolutionError, Result};

pub(super) fn resolve_runtime_base_url(
    endpoint_override: Option<&str>,
    provider_hints: RuntimeProviderHints<'_>,
    plugin_default: Option<&str>,
) -> Result<(String, RuntimeResolvedBaseUrlSource)> {
    if let Some(value) = endpoint_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok((
            value.to_string(),
            RuntimeResolvedBaseUrlSource::EndpointOverride,
        ));
    }

    if let Some(value) = provider_hints
        .base_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok((
            value.to_string(),
            RuntimeResolvedBaseUrlSource::ProviderConfig,
        ));
    }

    if let Some(value) = plugin_default
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok((
            value.to_string(),
            RuntimeResolvedBaseUrlSource::PluginDefault,
        ));
    }

    inferred_runtime_base_url(provider_hints)
        .ok_or_else(|| ProviderResolutionError::RuntimeRouteBaseUrlMissing.into())
}

pub(super) fn adapt_runtime_base_url_for_transport(
    base_url: String,
    transport: TransportKind,
) -> (String, Option<http_kit::WebsocketBaseUrlRewrite>) {
    if transport != TransportKind::WebSocket {
        return (base_url, None);
    }

    let rewritten = http_kit::resolve_websocket_base_url(&base_url);
    (rewritten.base_url, rewritten.rewrite)
}

pub(super) fn merge_runtime_query_params(
    endpoint_query_params: &[(String, String)],
    provider_hints: RuntimeProviderHints<'_>,
) -> Vec<(String, String)> {
    let mut out = endpoint_query_params.to_vec();
    if let Some(query_params) = provider_hints.http_query_params {
        for (name, value) in query_params {
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            out.push((name.to_string(), value.clone()));
        }
    }
    out
}

fn inferred_runtime_base_url(
    provider_hints: RuntimeProviderHints<'_>,
) -> Option<(String, RuntimeResolvedBaseUrlSource)> {
    match provider_hints.upstream_api {
        Some(RuntimeProviderApi::GeminiGenerateContent) => {
            return Some((
                "https://generativelanguage.googleapis.com/v1beta".to_string(),
                RuntimeResolvedBaseUrlSource::InferredFromUpstreamApi,
            ));
        }
        Some(RuntimeProviderApi::AnthropicMessages) => {
            return Some((
                "https://api.anthropic.com/v1".to_string(),
                RuntimeResolvedBaseUrlSource::InferredFromUpstreamApi,
            ));
        }
        Some(RuntimeProviderApi::OpenaiChatCompletions)
        | Some(RuntimeProviderApi::OpenaiResponses)
        | None => {}
    }
    None
}
