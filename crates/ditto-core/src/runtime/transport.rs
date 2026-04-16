use serde::Serialize;

use crate::catalog::{CatalogRegistry, ProviderPluginDescriptor};
use crate::config::{ProviderAuth, ProviderConfig};
use crate::contracts::{
    CapabilityKind, InvocationHints, OperationKind, ResolvedEndpoint, ResolvedInvocation,
    RuntimeRoute, RuntimeRouteRequest,
};
use crate::error::Result;

use super::builtin::builtin_runtime_assembly;
use super::explain::{http_method_name, transport_name};
use super::route::resolve_runtime_route_plan;
use super::{RuntimeBaseUrlSelectionSource, RuntimeProviderSelectionSource};

/// Transport-specific request wrapper for runtime planning.
///
/// `RuntimeRouteRequest` is intentionally minimal and only carries route
/// selection hints. Transport planning also needs the full `ProviderConfig`
/// because auth, static headers, and configured query params are transport
/// concerns rather than route concerns.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeTransportRequest<'a> {
    route_request: RuntimeRouteRequest<'a>,
    provider_config: Option<&'a ProviderConfig>,
}

impl<'a> RuntimeTransportRequest<'a> {
    pub fn new(provider: &'a str, model: Option<&'a str>, operation: OperationKind) -> Self {
        Self {
            route_request: RuntimeRouteRequest::new(provider, model, operation),
            provider_config: None,
        }
    }

    pub fn with_provider_config(mut self, provider_config: &'a ProviderConfig) -> Self {
        self.provider_config = Some(provider_config);
        self.route_request = self
            .route_request
            .with_runtime_hints(provider_config.runtime_hints());
        self
    }

    pub fn with_hints(mut self, hints: InvocationHints) -> Self {
        self.route_request = self.route_request.with_hints(hints);
        self
    }

    pub fn with_required_capability(mut self, capability: CapabilityKind) -> Self {
        self.route_request = self.route_request.with_required_capability(capability);
        self
    }

    pub fn route_request(self) -> RuntimeRouteRequest<'a> {
        self.route_request
    }

    pub fn provider_config(self) -> Option<&'a ProviderConfig> {
        self.provider_config
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeTransportAuthSelectionSource {
    ProviderConfig,
    ProviderDefault,
    Unconfigured,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuntimeTransportCredentialSource {
    Env { keys: Vec<String> },
    Command { command: Vec<String> },
    Inline,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuntimeTransportAuthPlan {
    None {
        source: RuntimeTransportAuthSelectionSource,
    },
    HttpHeader {
        source: RuntimeTransportAuthSelectionSource,
        header_name: String,
        prefix: Option<String>,
        credential: RuntimeTransportCredentialSource,
    },
    QueryParam {
        source: RuntimeTransportAuthSelectionSource,
        param: String,
        prefix: Option<String>,
        credential: RuntimeTransportCredentialSource,
    },
    SigV4 {
        source: RuntimeTransportAuthSelectionSource,
        region: String,
        service: String,
        access_key: RuntimeTransportCredentialSource,
        secret_key: RuntimeTransportCredentialSource,
        session_token: Option<RuntimeTransportCredentialSource>,
    },
    OAuthClientCredentials {
        source: RuntimeTransportAuthSelectionSource,
        token_url: String,
        client_id: Option<RuntimeTransportCredentialSource>,
        client_secret: Option<RuntimeTransportCredentialSource>,
        scope: Option<String>,
        audience: Option<String>,
        extra_param_keys: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeTransportBaseUrlRewrite {
    HttpToWebsocket,
    HttpsToSecureWebsocket,
}

/// Machine-readable transport plan derived from catalog + config.
///
/// This is the L0 view of how a request will be transported after route
/// selection, without constructing any HTTP client or resolving secrets.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeTransportPlan {
    pub provider_hint: String,
    pub resolved_provider: &'static str,
    pub provider_source: RuntimeProviderSelectionSource,
    pub requested_model: Option<String>,
    pub resolved_model: String,
    pub operation: &'static str,
    pub transport: &'static str,
    pub http_method: Option<&'static str>,
    pub origin_base_url: String,
    pub base_url: String,
    pub base_url_source: RuntimeBaseUrlSelectionSource,
    pub base_url_rewrite: Option<RuntimeTransportBaseUrlRewrite>,
    pub path: String,
    pub url: String,
    pub endpoint_query_params: Vec<(String, String)>,
    pub configured_query_params: Vec<(String, String)>,
    pub query_params: Vec<(String, String)>,
    pub configured_http_headers: Vec<String>,
    pub auth: RuntimeTransportAuthPlan,
}

pub(crate) fn plan_runtime_transport(
    registry: CatalogRegistry,
    request: RuntimeTransportRequest<'_>,
) -> Result<RuntimeTransportPlan> {
    let route_request = request.route_request();
    let provider_config = request.provider_config();
    let plan = resolve_runtime_route_plan(&registry, route_request)?;
    let plugin = registry
        .plugin(plan.resolved_provider)
        .expect("resolved runtime provider must exist in the registry");
    let super::route::RuntimeRouteExplainPlan {
        provider_hint,
        resolved_provider,
        provider_source,
        requested_model,
        resolved_model,
        operation,
        origin_base_url,
        base_url_source,
        base_url_rewrite,
        route,
        ..
    } = plan;
    let RuntimeRoute {
        invocation,
        base_url,
        url,
        query_params,
    } = route;
    let ResolvedInvocation { endpoint, .. } = invocation;
    let ResolvedEndpoint {
        transport,
        http_method,
        path,
        query_params: endpoint_query_params,
        ..
    } = endpoint;

    Ok(RuntimeTransportPlan {
        provider_hint,
        resolved_provider,
        provider_source: provider_source.into(),
        requested_model,
        resolved_model,
        operation: operation.as_str(),
        transport: transport_name(transport),
        http_method: http_method.map(http_method_name),
        origin_base_url,
        base_url,
        base_url_source: base_url_source.into(),
        base_url_rewrite: base_url_rewrite.map(Into::into),
        path,
        url,
        endpoint_query_params,
        configured_query_params: configured_query_params(provider_config, route_request),
        query_params,
        configured_http_headers: configured_http_headers(provider_config),
        auth: resolve_transport_auth_plan(plugin, provider_config)?,
    })
}

pub fn plan_builtin_runtime_transport(
    request: RuntimeTransportRequest<'_>,
) -> Result<RuntimeTransportPlan> {
    let runtime = builtin_runtime_assembly();
    plan_runtime_transport(runtime.catalog(), request)
}

impl From<http_kit::WebsocketBaseUrlRewrite> for RuntimeTransportBaseUrlRewrite {
    fn from(value: http_kit::WebsocketBaseUrlRewrite) -> Self {
        match value {
            http_kit::WebsocketBaseUrlRewrite::HttpToWebsocket => Self::HttpToWebsocket,
            http_kit::WebsocketBaseUrlRewrite::HttpsToSecureWebsocket => {
                Self::HttpsToSecureWebsocket
            }
        }
    }
}

fn resolve_transport_auth_plan(
    plugin: &ProviderPluginDescriptor,
    provider_config: Option<&ProviderConfig>,
) -> Result<RuntimeTransportAuthPlan> {
    if let Some(auth) = provider_config.and_then(|config| config.auth.as_ref()) {
        return transport_auth_plan_from_provider_auth(
            plugin,
            auth,
            RuntimeTransportAuthSelectionSource::ProviderConfig,
        );
    }

    if let Some(hint) = plugin.auth_hint {
        return transport_auth_plan_from_hint(
            hint,
            RuntimeTransportAuthSelectionSource::ProviderDefault,
        );
    }

    Ok(RuntimeTransportAuthPlan::None {
        source: RuntimeTransportAuthSelectionSource::Unconfigured,
    })
}

fn transport_auth_plan_from_provider_auth(
    plugin: &ProviderPluginDescriptor,
    auth: &ProviderAuth,
    source: RuntimeTransportAuthSelectionSource,
) -> Result<RuntimeTransportAuthPlan> {
    match auth {
        ProviderAuth::ApiKeyEnv { keys } => {
            let credential = RuntimeTransportCredentialSource::Env {
                keys: env_keys_or_default(keys, plugin),
            };
            apply_default_auth_target(plugin, source, credential)
        }
        ProviderAuth::Command { command } => apply_default_auth_target(
            plugin,
            source,
            RuntimeTransportCredentialSource::Command {
                command: command.clone(),
            },
        ),
        ProviderAuth::HttpHeaderEnv {
            header,
            keys,
            prefix,
        } => Ok(RuntimeTransportAuthPlan::HttpHeader {
            source,
            header_name: header.trim().to_string(),
            prefix: prefix.clone(),
            credential: RuntimeTransportCredentialSource::Env {
                keys: env_keys_or_default(keys, plugin),
            },
        }),
        ProviderAuth::HttpHeaderCommand {
            header,
            command,
            prefix,
        } => Ok(RuntimeTransportAuthPlan::HttpHeader {
            source,
            header_name: header.trim().to_string(),
            prefix: prefix.clone(),
            credential: RuntimeTransportCredentialSource::Command {
                command: command.clone(),
            },
        }),
        ProviderAuth::QueryParamEnv {
            param,
            keys,
            prefix,
        } => Ok(RuntimeTransportAuthPlan::QueryParam {
            source,
            param: param.trim().to_string(),
            prefix: prefix.clone(),
            credential: RuntimeTransportCredentialSource::Env {
                keys: env_keys_or_default(keys, plugin),
            },
        }),
        ProviderAuth::QueryParamCommand {
            param,
            command,
            prefix,
        } => Ok(RuntimeTransportAuthPlan::QueryParam {
            source,
            param: param.trim().to_string(),
            prefix: prefix.clone(),
            credential: RuntimeTransportCredentialSource::Command {
                command: command.clone(),
            },
        }),
        ProviderAuth::SigV4 {
            access_keys,
            secret_keys,
            session_token_keys,
            region,
            service,
        } => Ok(RuntimeTransportAuthPlan::SigV4 {
            source,
            region: region.clone(),
            service: service.clone(),
            access_key: RuntimeTransportCredentialSource::Env {
                keys: access_keys.clone(),
            },
            secret_key: RuntimeTransportCredentialSource::Env {
                keys: secret_keys.clone(),
            },
            session_token: (!session_token_keys.is_empty()).then_some(
                RuntimeTransportCredentialSource::Env {
                    keys: session_token_keys.clone(),
                },
            ),
        }),
        ProviderAuth::OAuthClientCredentials {
            token_url,
            client_id,
            client_secret,
            client_id_keys,
            client_secret_keys,
            scope,
            audience,
            extra_params,
        } => {
            let mut extra_param_keys: Vec<_> = extra_params.keys().cloned().collect();
            extra_param_keys.sort();
            Ok(RuntimeTransportAuthPlan::OAuthClientCredentials {
                source,
                token_url: token_url.clone(),
                client_id: oauth_field_source(client_id, client_id_keys),
                client_secret: oauth_field_source(client_secret, client_secret_keys),
                scope: scope.clone(),
                audience: audience.clone(),
                extra_param_keys,
            })
        }
    }
}

fn transport_auth_plan_from_hint(
    hint: crate::contracts::ProviderAuthHint,
    source: RuntimeTransportAuthSelectionSource,
) -> Result<RuntimeTransportAuthPlan> {
    let credential = RuntimeTransportCredentialSource::Env {
        keys: hint.env_keys.iter().map(|key| (*key).to_string()).collect(),
    };

    match (hint.header_name, hint.query_param) {
        (Some(header_name), _) => Ok(RuntimeTransportAuthPlan::HttpHeader {
            source,
            header_name: header_name.to_string(),
            prefix: hint.prefix.map(str::to_string),
            credential,
        }),
        (None, Some(param)) => Ok(RuntimeTransportAuthPlan::QueryParam {
            source,
            param: param.to_string(),
            prefix: hint.prefix.map(str::to_string),
            credential,
        }),
        _ => Err(crate::invalid_response!(
            "error_detail.runtime_transport.auth_hint_target_missing",
            "provider" => source_provider_label(source)
        )),
    }
}

fn apply_default_auth_target(
    plugin: &ProviderPluginDescriptor,
    source: RuntimeTransportAuthSelectionSource,
    credential: RuntimeTransportCredentialSource,
) -> Result<RuntimeTransportAuthPlan> {
    let Some(hint) = plugin.auth_hint else {
        return Err(crate::invalid_response!(
            "error_detail.runtime_transport.default_auth_target_unavailable",
            "provider" => plugin.id
        ));
    };

    match (hint.header_name, hint.query_param) {
        (Some(header_name), _) => Ok(RuntimeTransportAuthPlan::HttpHeader {
            source,
            header_name: header_name.to_string(),
            prefix: hint.prefix.map(str::to_string),
            credential,
        }),
        (None, Some(param)) => Ok(RuntimeTransportAuthPlan::QueryParam {
            source,
            param: param.to_string(),
            prefix: hint.prefix.map(str::to_string),
            credential,
        }),
        _ => Err(crate::invalid_response!(
            "error_detail.runtime_transport.default_auth_hint_target_missing",
            "provider" => plugin.id
        )),
    }
}

fn env_keys_or_default(keys: &[String], plugin: &ProviderPluginDescriptor) -> Vec<String> {
    if !keys.is_empty() {
        return keys.to_vec();
    }

    plugin
        .auth_hint
        .map(|hint| hint.env_keys.iter().map(|key| (*key).to_string()).collect())
        .unwrap_or_default()
}

fn oauth_field_source(
    inline: &Option<String>,
    keys: &[String],
) -> Option<RuntimeTransportCredentialSource> {
    if inline.is_some() {
        return Some(RuntimeTransportCredentialSource::Inline);
    }
    if keys.is_empty() {
        return None;
    }
    Some(RuntimeTransportCredentialSource::Env {
        keys: keys.to_vec(),
    })
}

fn configured_http_headers(provider_config: Option<&ProviderConfig>) -> Vec<String> {
    let Some(provider_config) = provider_config else {
        return Vec::new();
    };

    provider_config
        .http_headers
        .keys()
        .map(|name| name.trim())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

fn configured_query_params(
    provider_config: Option<&ProviderConfig>,
    route_request: RuntimeRouteRequest<'_>,
) -> Vec<(String, String)> {
    if let Some(provider_config) = provider_config {
        return provider_config
            .http_query_params
            .iter()
            .filter_map(|(name, value)| {
                let name = name.trim();
                (!name.is_empty()).then(|| (name.to_string(), value.clone()))
            })
            .collect();
    }

    route_request
        .provider_hints
        .http_query_params
        .map(|params| {
            params
                .iter()
                .filter_map(|(name, value)| {
                    let name = name.trim();
                    (!name.is_empty()).then(|| (name.to_string(), value.clone()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn source_provider_label(source: RuntimeTransportAuthSelectionSource) -> &'static str {
    match source {
        RuntimeTransportAuthSelectionSource::ProviderConfig => "provider_config",
        RuntimeTransportAuthSelectionSource::ProviderDefault => "provider_default",
        RuntimeTransportAuthSelectionSource::Unconfigured => "unconfigured",
    }
}
