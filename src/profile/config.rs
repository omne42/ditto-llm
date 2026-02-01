use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingIntensity {
    #[serde(
        alias = "none",
        alias = "disabled",
        alias = "off",
        alias = "not_supported"
    )]
    Unsupported,
    #[serde(alias = "low")]
    Small,
    #[default]
    Medium,
    High,
    #[serde(rename = "xhigh")]
    XHigh,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ModelConfig {
    #[serde(default)]
    pub thinking: ThinkingIntensity,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "max_context",
        alias = "max_context_window"
    )]
    pub context_window: Option<u64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "best_context",
        alias = "best_context_window"
    )]
    pub auto_compact_token_limit: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache: Option<bool>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderAuth {
    #[serde(rename = "api_key_env", alias = "env", alias = "api_key")]
    ApiKeyEnv {
        #[serde(default)]
        keys: Vec<String>,
    },
    #[serde(alias = "auth_command")]
    Command { command: Vec<String> },
    #[serde(alias = "header_env")]
    HttpHeaderEnv {
        header: String,
        #[serde(default)]
        keys: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prefix: Option<String>,
    },
    #[serde(alias = "header_command")]
    HttpHeaderCommand {
        header: String,
        command: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prefix: Option<String>,
    },
    #[serde(alias = "query_env", alias = "query_param")]
    QueryParamEnv {
        param: String,
        #[serde(default)]
        keys: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prefix: Option<String>,
    },
    #[serde(alias = "query_command")]
    QueryParamCommand {
        param: String,
        command: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prefix: Option<String>,
    },
    #[serde(rename = "sigv4", alias = "sig_v4")]
    SigV4 {
        #[serde(default)]
        access_keys: Vec<String>,
        #[serde(default)]
        secret_keys: Vec<String>,
        #[serde(default)]
        session_token_keys: Vec<String>,
        region: String,
        service: String,
    },
    #[serde(rename = "oauth_client_credentials", alias = "oauth")]
    OAuthClientCredentials {
        token_url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_secret: Option<String>,
        #[serde(default)]
        client_id_keys: Vec<String>,
        #[serde(default)]
        client_secret_keys: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scope: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        audience: Option<String>,
        #[serde(default)]
        extra_params: BTreeMap<String, String>,
    },
}

impl std::fmt::Debug for ProviderAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderAuth::ApiKeyEnv { keys } => {
                f.debug_struct("ApiKeyEnv").field("keys", keys).finish()
            }
            ProviderAuth::Command { command } => {
                f.debug_struct("Command").field("command", command).finish()
            }
            ProviderAuth::HttpHeaderEnv {
                header,
                keys,
                prefix,
            } => f
                .debug_struct("HttpHeaderEnv")
                .field("header", header)
                .field("keys", keys)
                .field("prefix", prefix)
                .finish(),
            ProviderAuth::HttpHeaderCommand {
                header,
                command,
                prefix,
            } => f
                .debug_struct("HttpHeaderCommand")
                .field("header", header)
                .field("command", command)
                .field("prefix", prefix)
                .finish(),
            ProviderAuth::QueryParamEnv {
                param,
                keys,
                prefix,
            } => f
                .debug_struct("QueryParamEnv")
                .field("param", param)
                .field("keys", keys)
                .field("prefix", prefix)
                .finish(),
            ProviderAuth::QueryParamCommand {
                param,
                command,
                prefix,
            } => f
                .debug_struct("QueryParamCommand")
                .field("param", param)
                .field("command", command)
                .field("prefix", prefix)
                .finish(),
            ProviderAuth::SigV4 {
                access_keys,
                secret_keys,
                session_token_keys,
                region,
                service,
            } => f
                .debug_struct("SigV4")
                .field("access_keys", access_keys)
                .field("secret_keys", secret_keys)
                .field("session_token_keys", session_token_keys)
                .field("region", region)
                .field("service", service)
                .finish(),
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
                let extra_param_keys: Vec<&str> =
                    extra_params.keys().map(|key| key.as_str()).collect();
                f.debug_struct("OAuthClientCredentials")
                    .field("token_url", token_url)
                    .field("client_id", client_id)
                    .field(
                        "client_secret",
                        &client_secret.as_ref().map(|_| "<redacted>"),
                    )
                    .field("client_id_keys", client_id_keys)
                    .field("client_secret_keys", client_secret_keys)
                    .field("scope", scope)
                    .field("audience", audience)
                    .field("extra_params", &extra_param_keys)
                    .finish()
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProviderCapabilities {
    #[serde(default)]
    pub tools: bool,
    #[serde(default)]
    pub vision: bool,
    #[serde(default)]
    pub reasoning: bool,
    #[serde(default)]
    pub json_schema: bool,
    #[serde(default)]
    pub streaming: bool,
    #[serde(default = "default_true")]
    pub prompt_cache: bool,
}

impl ProviderCapabilities {
    pub fn openai_responses() -> Self {
        Self {
            tools: true,
            vision: true,
            reasoning: true,
            json_schema: true,
            streaming: true,
            prompt_cache: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProviderConfig {
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub model_whitelist: Vec<String>,
    #[serde(default)]
    pub http_headers: BTreeMap<String, String>,
    #[serde(default)]
    pub http_query_params: BTreeMap<String, String>,
    #[serde(default)]
    pub auth: Option<ProviderAuth>,
    #[serde(default)]
    pub capabilities: Option<ProviderCapabilities>,
}

pub fn normalize_string_list(values: Vec<String>) -> Vec<String> {
    let mut out = Vec::<String>::new();
    let mut seen = BTreeSet::<String>::new();
    for value in values {
        let value = value.trim().to_string();
        if value.is_empty() {
            continue;
        }
        if seen.insert(value.clone()) {
            out.push(value);
        }
    }
    out
}

pub fn select_model_config<'a>(
    models: &'a BTreeMap<String, ModelConfig>,
    model: &str,
) -> Option<&'a ModelConfig> {
    if let Some(config) = models.get(model) {
        return Some(config);
    }
    models.get("*")
}

pub fn filter_models_whitelist(models: Vec<String>, whitelist: &[String]) -> Vec<String> {
    if whitelist.is_empty() {
        return models;
    }

    let allow = whitelist
        .iter()
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty())
        .collect::<BTreeSet<_>>();

    models
        .into_iter()
        .filter(|m| allow.contains(m))
        .collect::<Vec<_>>()
}
