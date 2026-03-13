use super::route::RuntimeResolvedBaseUrlSource;
use crate::contracts::{RuntimeProviderApi, RuntimeProviderHints, TransportKind};
use crate::foundation::error::{ProviderResolutionError, Result};

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
) -> (
    String,
    Option<crate::session_transport::WebsocketBaseUrlRewrite>,
) {
    if transport != TransportKind::WebSocket {
        return (base_url, None);
    }

    let rewritten = crate::session_transport::resolve_websocket_base_url(&base_url);
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

pub(super) fn join_base_url(base_url: &str, path: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let path_no_leading_slash = path.strip_prefix('/').unwrap_or(path);

    if base.ends_with("/v1") {
        if path_no_leading_slash == "v1" {
            return base.to_string();
        }
        if let Some(rest) = path_no_leading_slash.strip_prefix("v1/") {
            let mut out = String::with_capacity(base.len() + 1 + rest.len());
            out.push_str(base);
            out.push('/');
            out.push_str(rest);
            return out;
        }
    }

    if path.starts_with('/') {
        let mut out = String::with_capacity(base.len() + path.len());
        out.push_str(base);
        out.push_str(path);
        out
    } else {
        let mut out = String::with_capacity(base.len() + 1 + path.len());
        out.push_str(base);
        out.push('/');
        out.push_str(path);
        out
    }
}

pub(super) fn append_query_params(base_url: String, query_params: &[(String, String)]) -> String {
    if query_params.is_empty() {
        return base_url;
    }

    let mut out = base_url;
    out.push(if out.contains('?') { '&' } else { '?' });
    for (idx, (name, value)) in query_params.iter().enumerate() {
        if idx > 0 {
            out.push('&');
        }
        out.push_str(name);
        out.push('=');
        out.push_str(value);
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
