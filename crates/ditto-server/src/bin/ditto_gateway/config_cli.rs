#[cfg(feature = "gateway")]
use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};

#[cfg(feature = "gateway")]
use ditto_core::config::ProviderApi;
#[cfg(feature = "gateway")]
use ditto_core::error::{DittoError, try_structured_text_from_text_args};
#[cfg(feature = "gateway")]
use ditto_server::config_editing::{
    ConfigScope, ModelDeleteRequest, ModelListRequest, ModelShowRequest, ModelUpsertRequest,
    ProviderAuthType, ProviderDeleteRequest, ProviderListRequest, ProviderNamespace,
    ProviderShowRequest, ProviderUpsertRequest, delete_model_config, delete_provider_config,
    list_model_configs, list_provider_configs, show_model_config, show_provider_config,
    upsert_model_config, upsert_provider_config,
};
#[cfg(feature = "config-interactive")]
use ditto_server::config_editing::{
    complete_model_upsert_request_interactive, complete_provider_upsert_request_interactive,
};
#[cfg(feature = "gateway")]
use i18n_kit::{Locale, TemplateArg};

#[cfg(feature = "gateway")]
use serde_json::Value;

#[cfg(feature = "gateway")]
use crate::ditto_gateway::clap_i18n::{
    LocalizedCliError, localize_clap_command, render_clap_error,
};

#[cfg(feature = "gateway")]
fn config_error(_locale: Locale, key: &'static str, args: &[TemplateArg<'_>]) -> DittoError {
    let message =
        try_structured_text_from_text_args(key, args).expect("config error args must remain valid");
    DittoError::Config(message)
}

#[cfg(all(feature = "gateway", feature = "config-interactive"))]
fn maybe_complete_provider_request_interactive(
    request: ProviderUpsertRequest,
    use_interactive: bool,
    _locale: Locale,
) -> ditto_core::error::Result<ProviderUpsertRequest> {
    if use_interactive {
        complete_provider_upsert_request_interactive(request)
    } else {
        Ok(request)
    }
}

#[cfg(all(feature = "gateway", not(feature = "config-interactive")))]
fn maybe_complete_provider_request_interactive(
    request: ProviderUpsertRequest,
    use_interactive: bool,
    locale: Locale,
) -> ditto_core::error::Result<ProviderUpsertRequest> {
    if use_interactive {
        Err(config_error(
            locale,
            "config_cli.interactive_feature_disabled",
            &[],
        ))
    } else {
        Ok(request)
    }
}

#[cfg(all(feature = "gateway", feature = "config-interactive"))]
fn maybe_complete_model_request_interactive(
    request: ModelUpsertRequest,
    use_interactive: bool,
    _locale: Locale,
) -> ditto_core::error::Result<ModelUpsertRequest> {
    if use_interactive {
        complete_model_upsert_request_interactive(request)
    } else {
        Ok(request)
    }
}

#[cfg(all(feature = "gateway", not(feature = "config-interactive")))]
fn maybe_complete_model_request_interactive(
    request: ModelUpsertRequest,
    use_interactive: bool,
    locale: Locale,
) -> ditto_core::error::Result<ModelUpsertRequest> {
    if use_interactive {
        Err(config_error(
            locale,
            "config_cli.interactive_feature_disabled",
            &[],
        ))
    } else {
        Ok(request)
    }
}

#[cfg(feature = "gateway")]
async fn maybe_resolve_provider_request_discovery(
    mut request: ProviderUpsertRequest,
    locale: Locale,
) -> ditto_core::error::Result<ProviderUpsertRequest> {
    if !request.discover_models || !request.model_whitelist.is_empty() {
        if let Some(limit) = request.model_limit {
            request.model_whitelist.truncate(limit);
        }
        return Ok(request);
    }

    // Provider model discovery is a CLI-side effect. `config_editing` only
    // consumes the resolved whitelist so L0 stays a pure config mutation layer.
    request.model_whitelist = discover_models_for_provider(&request, locale).await?;
    if let Some(limit) = request.model_limit {
        request.model_whitelist.truncate(limit);
    }
    Ok(request)
}

#[cfg(feature = "gateway")]
async fn discover_models_for_provider(
    request: &ProviderUpsertRequest,
    locale: Locale,
) -> ditto_core::error::Result<Vec<String>> {
    let base_url = request
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| config_error(locale, "config_cli.discover.base_url_required", &[]))?;
    let api = infer_discovery_api(request);
    if matches!(api, ProviderApi::AnthropicMessages) {
        return Err(config_error(
            locale,
            "config_cli.discover.anthropic_unimplemented",
            &[],
        ));
    }

    let key = resolve_discovery_key(request, locale)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(DittoError::Http)?;
    let endpoint = format!("{}/models", base_url.trim_end_matches('/'));
    let mut http_request = client.get(endpoint);

    match request.auth_type {
        ProviderAuthType::ApiKeyEnv => {
            http_request = http_request.bearer_auth(&key);
        }
        ProviderAuthType::QueryParamEnv => {
            let param = request
                .auth_param
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("key");
            http_request = http_request.query(&[(param, key.as_str())]);
        }
        ProviderAuthType::HttpHeaderEnv => {
            let header = request
                .auth_header
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("Authorization");
            let auth_value = if let Some(prefix) = request
                .auth_prefix
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                format!("{prefix}{key}")
            } else if header.eq_ignore_ascii_case("authorization") {
                format!("Bearer {key}")
            } else {
                key.clone()
            };
            http_request = http_request.header(header, auth_value);
        }
        ProviderAuthType::Command => {
            http_request = http_request.bearer_auth(&key);
        }
    }

    let response = http_request.send().await.map_err(DittoError::Http)?;
    let response = response.error_for_status().map_err(DittoError::Http)?;
    let payload = response.json::<Value>().await.map_err(DittoError::Http)?;

    let mut models = match api {
        ProviderApi::OpenaiChatCompletions | ProviderApi::OpenaiResponses => {
            parse_openai_models(&payload)
        }
        ProviderApi::GeminiGenerateContent => parse_gemini_models(&payload),
        ProviderApi::AnthropicMessages => Vec::new(),
    };
    models.sort_unstable();
    models.dedup();
    Ok(models)
}

#[cfg(feature = "gateway")]
fn infer_discovery_api(request: &ProviderUpsertRequest) -> ProviderApi {
    request.upstream_api.unwrap_or(match request.namespace {
        ProviderNamespace::Google | ProviderNamespace::Gemini => ProviderApi::GeminiGenerateContent,
        ProviderNamespace::Claude | ProviderNamespace::Anthropic => ProviderApi::AnthropicMessages,
        ProviderNamespace::Openai => ProviderApi::OpenaiChatCompletions,
    })
}

#[cfg(feature = "gateway")]
fn resolve_discovery_key(
    request: &ProviderUpsertRequest,
    locale: Locale,
) -> ditto_core::error::Result<String> {
    if let Some(api_key) = request
        .discovery_api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(api_key.to_string());
    }

    for key in &request.auth_keys {
        if let Ok(value) = std::env::var(key) {
            let value = value.trim();
            if !value.is_empty() {
                return Ok(value.to_string());
            }
        }
    }

    Err(config_error(
        locale,
        "config_cli.discover.auth_missing",
        &[],
    ))
}

#[cfg(feature = "gateway")]
fn parse_openai_models(payload: &Value) -> Vec<String> {
    payload
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("id").and_then(Value::as_str))
        .filter_map(|id| {
            let id = id.trim();
            if id.is_empty() {
                None
            } else {
                Some(id.to_string())
            }
        })
        .collect()
}

#[cfg(feature = "gateway")]
fn parse_gemini_models(payload: &Value) -> Vec<String> {
    let mut out = Vec::<String>::new();
    for item in payload
        .get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let supports_generate_content = item
            .get("supportedGenerationMethods")
            .and_then(Value::as_array)
            .is_none_or(|methods| {
                methods
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|method| method.eq_ignore_ascii_case("generateContent"))
            });
        if !supports_generate_content {
            continue;
        }

        let Some(name) = item.get("name").and_then(Value::as_str) else {
            continue;
        };
        let name = name.trim().trim_start_matches("models/");
        if !name.is_empty() {
            out.push(name.to_string());
        }
    }
    out
}

#[cfg(feature = "gateway")]
#[derive(Debug, Parser)]
#[command(name = "ditto-gateway")]
struct ConfigCli {
    #[command(subcommand)]
    command: ConfigCommand,
}

#[cfg(feature = "gateway")]
#[derive(Debug, Subcommand)]
enum ConfigCommand {
    Provider {
        #[command(subcommand)]
        command: ProviderCommand,
    },
    Model {
        #[command(subcommand)]
        command: ModelCommand,
    },
}

#[cfg(feature = "gateway")]
#[derive(Debug, Subcommand)]
enum ProviderCommand {
    #[command(alias = "set")]
    Add(Box<ProviderAddArgs>),
    #[command(alias = "ls")]
    List(ProviderListArgs),
    Show(Box<ProviderShowArgs>),
    #[command(alias = "rm")]
    Delete(ProviderDeleteArgs),
}

#[cfg(feature = "gateway")]
#[derive(Debug, Subcommand)]
enum ModelCommand {
    #[command(alias = "set")]
    Add(Box<ModelAddArgs>),
    #[command(alias = "ls")]
    List(ModelListArgs),
    Show(Box<ModelShowArgs>),
    #[command(alias = "rm")]
    Delete(ModelDeleteArgs),
}

#[cfg(feature = "gateway")]
#[derive(Debug, Clone, Copy, ValueEnum)]
#[value(rename_all = "snake_case")]
enum ProviderApiArg {
    OpenaiChatCompletions,
    OpenaiResponses,
    GeminiGenerateContent,
    AnthropicMessages,
}

#[cfg(feature = "gateway")]
impl ProviderApiArg {
    const fn to_provider_api(self) -> ProviderApi {
        match self {
            Self::OpenaiChatCompletions => ProviderApi::OpenaiChatCompletions,
            Self::OpenaiResponses => ProviderApi::OpenaiResponses,
            Self::GeminiGenerateContent => ProviderApi::GeminiGenerateContent,
            Self::AnthropicMessages => ProviderApi::AnthropicMessages,
        }
    }
}

#[cfg(feature = "gateway")]
#[derive(Debug, Clone, Copy, ValueEnum)]
#[value(rename_all = "snake_case")]
enum ProviderNamespaceArg {
    Openai,
    Google,
    Gemini,
    Claude,
    Anthropic,
}

#[cfg(feature = "gateway")]
impl ProviderNamespaceArg {
    const fn to_provider_namespace(self) -> ProviderNamespace {
        match self {
            Self::Openai => ProviderNamespace::Openai,
            Self::Google => ProviderNamespace::Google,
            Self::Gemini => ProviderNamespace::Gemini,
            Self::Claude => ProviderNamespace::Claude,
            Self::Anthropic => ProviderNamespace::Anthropic,
        }
    }
}

#[cfg(feature = "gateway")]
#[derive(Debug, Clone, Copy, ValueEnum)]
#[value(rename_all = "snake_case")]
enum ProviderAuthTypeArg {
    ApiKeyEnv,
    QueryParamEnv,
    HttpHeaderEnv,
    Command,
}

#[cfg(feature = "gateway")]
impl ProviderAuthTypeArg {
    const fn to_provider_auth_type(self) -> ProviderAuthType {
        match self {
            Self::ApiKeyEnv => ProviderAuthType::ApiKeyEnv,
            Self::QueryParamEnv => ProviderAuthType::QueryParamEnv,
            Self::HttpHeaderEnv => ProviderAuthType::HttpHeaderEnv,
            Self::Command => ProviderAuthType::Command,
        }
    }
}

#[cfg(feature = "gateway")]
#[derive(Debug, Clone, Copy, ValueEnum)]
#[value(rename_all = "snake_case")]
enum ScopeArg {
    Auto,
    Workspace,
    Global,
}

#[cfg(feature = "gateway")]
impl ScopeArg {
    const fn to_config_scope(self) -> ConfigScope {
        match self {
            Self::Auto => ConfigScope::Auto,
            Self::Workspace => ConfigScope::Workspace,
            Self::Global => ConfigScope::Global,
        }
    }
}

#[cfg(feature = "gateway")]
#[derive(Debug, Args)]
struct ProviderAddArgs {
    name: String,

    #[arg(long)]
    config_path: Option<std::path::PathBuf>,

    #[arg(long)]
    root: Option<std::path::PathBuf>,

    #[arg(long, value_enum, default_value_t = ScopeArg::Auto)]
    scope: ScopeArg,

    #[arg(long, value_enum, default_value_t = ProviderNamespaceArg::Openai)]
    namespace: ProviderNamespaceArg,

    #[arg(long)]
    provider: Option<String>,

    #[arg(long = "enabled-capability", value_delimiter = ',')]
    enabled_capabilities: Vec<String>,

    #[arg(long)]
    base_url: Option<String>,

    #[arg(long)]
    default_model: Option<String>,

    #[arg(long, value_enum)]
    upstream_api: Option<ProviderApiArg>,

    #[arg(long, value_enum)]
    normalize_to: Option<ProviderApiArg>,

    #[arg(long)]
    normalize_endpoint: Option<String>,

    #[arg(long, value_enum, default_value_t = ProviderAuthTypeArg::ApiKeyEnv)]
    auth_type: ProviderAuthTypeArg,

    #[arg(long = "auth-key", value_delimiter = ',')]
    auth_keys: Vec<String>,

    #[arg(long)]
    auth_param: Option<String>,

    #[arg(long)]
    auth_header: Option<String>,

    #[arg(long)]
    auth_prefix: Option<String>,

    #[arg(long = "auth-command", value_delimiter = ',')]
    auth_command: Vec<String>,

    #[arg(long, default_value_t = false)]
    set_default: bool,

    #[arg(long, default_value_t = false)]
    set_default_model: bool,

    #[arg(long)]
    tools: Option<bool>,

    #[arg(long)]
    vision: Option<bool>,

    #[arg(long)]
    reasoning: Option<bool>,

    #[arg(long)]
    json_schema: Option<bool>,

    #[arg(long)]
    streaming: Option<bool>,

    #[arg(long)]
    prompt_cache: Option<bool>,

    #[arg(long, default_value_t = false)]
    discover_models: bool,

    #[arg(long)]
    api_key: Option<String>,

    #[arg(long, default_value_t = false)]
    register_models: bool,

    #[arg(long)]
    model_limit: Option<usize>,

    #[arg(long, default_value_t = false)]
    interactive: bool,

    #[arg(long, default_value_t = false, conflicts_with = "interactive")]
    no_interactive: bool,

    #[arg(long, default_value_t = false)]
    json: bool,
}

#[cfg(feature = "gateway")]
#[derive(Debug, Args)]
struct ProviderListArgs {
    #[arg(long)]
    config_path: Option<std::path::PathBuf>,

    #[arg(long)]
    root: Option<std::path::PathBuf>,

    #[arg(long, value_enum, default_value_t = ScopeArg::Auto)]
    scope: ScopeArg,

    #[arg(long, value_enum)]
    namespace: Option<ProviderNamespaceArg>,

    #[arg(long, default_value_t = false)]
    json: bool,
}

#[cfg(feature = "gateway")]
#[derive(Debug, Args)]
struct ProviderShowArgs {
    name: String,

    #[arg(long)]
    config_path: Option<std::path::PathBuf>,

    #[arg(long)]
    root: Option<std::path::PathBuf>,

    #[arg(long, value_enum, default_value_t = ScopeArg::Auto)]
    scope: ScopeArg,

    #[arg(long, value_enum, default_value_t = ProviderNamespaceArg::Openai)]
    namespace: ProviderNamespaceArg,

    #[arg(long, default_value_t = false)]
    json: bool,
}

#[cfg(feature = "gateway")]
#[derive(Debug, Args)]
struct ProviderDeleteArgs {
    name: String,

    #[arg(long)]
    config_path: Option<std::path::PathBuf>,

    #[arg(long)]
    root: Option<std::path::PathBuf>,

    #[arg(long, value_enum, default_value_t = ScopeArg::Auto)]
    scope: ScopeArg,

    #[arg(long, value_enum, default_value_t = ProviderNamespaceArg::Openai)]
    namespace: ProviderNamespaceArg,

    #[arg(long, default_value_t = false)]
    json: bool,
}

#[cfg(feature = "gateway")]
#[derive(Debug, Args)]
struct ModelAddArgs {
    name: String,

    #[arg(long)]
    config_path: Option<std::path::PathBuf>,

    #[arg(long)]
    root: Option<std::path::PathBuf>,

    #[arg(long, value_enum, default_value_t = ScopeArg::Auto)]
    scope: ScopeArg,

    #[arg(long)]
    provider: Option<String>,

    #[arg(long = "fallback-provider", value_delimiter = ',')]
    fallback_providers: Vec<String>,

    #[arg(long, default_value_t = false)]
    set_default: bool,

    #[arg(long)]
    thinking: Option<String>,

    #[arg(long)]
    context_window: Option<u64>,

    #[arg(long)]
    auto_compact_token_limit: Option<u64>,

    #[arg(long)]
    prompt_cache: Option<bool>,

    #[arg(long, default_value_t = false)]
    interactive: bool,

    #[arg(long, default_value_t = false, conflicts_with = "interactive")]
    no_interactive: bool,

    #[arg(long, default_value_t = false)]
    json: bool,
}

#[cfg(feature = "gateway")]
#[derive(Debug, Args)]
struct ModelListArgs {
    #[arg(long)]
    config_path: Option<std::path::PathBuf>,

    #[arg(long)]
    root: Option<std::path::PathBuf>,

    #[arg(long, value_enum, default_value_t = ScopeArg::Auto)]
    scope: ScopeArg,

    #[arg(long, default_value_t = false)]
    json: bool,
}

#[cfg(feature = "gateway")]
#[derive(Debug, Args)]
struct ModelShowArgs {
    name: String,

    #[arg(long)]
    config_path: Option<std::path::PathBuf>,

    #[arg(long)]
    root: Option<std::path::PathBuf>,

    #[arg(long, value_enum, default_value_t = ScopeArg::Auto)]
    scope: ScopeArg,

    #[arg(long, default_value_t = false)]
    json: bool,
}

#[cfg(feature = "gateway")]
#[derive(Debug, Args)]
struct ModelDeleteArgs {
    name: String,

    #[arg(long)]
    config_path: Option<std::path::PathBuf>,

    #[arg(long)]
    root: Option<std::path::PathBuf>,

    #[arg(long, value_enum, default_value_t = ScopeArg::Auto)]
    scope: ScopeArg,

    #[arg(long, default_value_t = false)]
    json: bool,
}

#[cfg(feature = "gateway")]
pub(crate) async fn maybe_run_config_cli(
    args: Vec<String>,
    locale: Locale,
) -> Result<bool, Box<dyn std::error::Error>> {
    let Some(first) = args.first().map(String::as_str) else {
        return Ok(false);
    };
    if first != "provider" && first != "model" {
        return Ok(false);
    }

    let argv = std::iter::once("ditto-gateway".to_string())
        .chain(args)
        .collect::<Vec<_>>();
    let mut command = localize_clap_command(ConfigCli::command(), locale);
    let mut matches = match command.try_get_matches_from_mut(argv) {
        Ok(matches) => matches,
        Err(err) => {
            let kind = err.kind();
            let rendered = render_clap_error(&mut command, err, locale);
            if matches!(
                kind,
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) {
                print!("{rendered}");
                return Ok(true);
            }
            return Err(Box::new(LocalizedCliError::new(rendered)));
        }
    };
    let cli = ConfigCli::from_arg_matches_mut(&mut matches).map_err(|err| {
        Box::new(LocalizedCliError::new(render_clap_error(
            &mut command,
            err,
            locale,
        ))) as Box<dyn std::error::Error>
    })?;

    match cli.command {
        ConfigCommand::Provider { command } => match command {
            ProviderCommand::Add(args) => {
                let args = *args;
                let use_interactive = if cfg!(feature = "config-interactive") {
                    !args.no_interactive || args.interactive
                } else {
                    args.interactive
                };
                let mut request = ProviderUpsertRequest {
                    name: args.name,
                    config_path: args.config_path,
                    root: args.root,
                    scope: args.scope.to_config_scope(),
                    namespace: args.namespace.to_provider_namespace(),
                    provider: args.provider,
                    enabled_capabilities: args.enabled_capabilities,
                    base_url: args.base_url,
                    default_model: args.default_model,
                    upstream_api: args.upstream_api.map(ProviderApiArg::to_provider_api),
                    normalize_to: args.normalize_to.map(ProviderApiArg::to_provider_api),
                    normalize_endpoint: args.normalize_endpoint,
                    auth_type: args.auth_type.to_provider_auth_type(),
                    auth_keys: args.auth_keys,
                    auth_param: args.auth_param,
                    auth_header: args.auth_header,
                    auth_prefix: args.auth_prefix,
                    auth_command: args.auth_command,
                    set_default: args.set_default,
                    set_default_model: args.set_default_model,
                    tools: args.tools,
                    vision: args.vision,
                    reasoning: args.reasoning,
                    json_schema: args.json_schema,
                    streaming: args.streaming,
                    prompt_cache: args.prompt_cache,
                    discover_models: args.discover_models,
                    discovery_api_key: args.api_key,
                    model_whitelist: Vec::new(),
                    register_models: args.register_models,
                    model_limit: args.model_limit,
                };
                request =
                    maybe_complete_provider_request_interactive(request, use_interactive, locale)?;
                request = maybe_resolve_provider_request_discovery(request, locale).await?;
                let report = upsert_provider_config(request).await?;
                print_json_or_pretty(args.json, &serde_json::to_value(report)?)?;
            }
            ProviderCommand::List(args) => {
                let report = list_provider_configs(ProviderListRequest {
                    config_path: args.config_path,
                    root: args.root,
                    scope: args.scope.to_config_scope(),
                    namespace: args
                        .namespace
                        .map(ProviderNamespaceArg::to_provider_namespace),
                })
                .await?;
                print_json_or_pretty(args.json, &serde_json::to_value(report)?)?;
            }
            ProviderCommand::Show(args) => {
                let args = *args;
                let report = show_provider_config(ProviderShowRequest {
                    name: args.name,
                    config_path: args.config_path,
                    root: args.root,
                    scope: args.scope.to_config_scope(),
                    namespace: args.namespace.to_provider_namespace(),
                })
                .await?;
                print_json_or_pretty(args.json, &serde_json::to_value(report)?)?;
            }
            ProviderCommand::Delete(args) => {
                let report = delete_provider_config(ProviderDeleteRequest {
                    name: args.name,
                    config_path: args.config_path,
                    root: args.root,
                    scope: args.scope.to_config_scope(),
                    namespace: args.namespace.to_provider_namespace(),
                })
                .await?;
                print_json_or_pretty(args.json, &serde_json::to_value(report)?)?;
            }
        },
        ConfigCommand::Model { command } => match command {
            ModelCommand::Add(args) => {
                let args = *args;
                let use_interactive = if cfg!(feature = "config-interactive") {
                    !args.no_interactive || args.interactive
                } else {
                    args.interactive
                };
                let mut request = ModelUpsertRequest {
                    name: args.name,
                    config_path: args.config_path,
                    root: args.root,
                    scope: args.scope.to_config_scope(),
                    provider: args.provider,
                    fallback_providers: args.fallback_providers,
                    set_default: args.set_default,
                    thinking: args.thinking,
                    context_window: args.context_window,
                    auto_compact_token_limit: args.auto_compact_token_limit,
                    prompt_cache: args.prompt_cache,
                };
                request =
                    maybe_complete_model_request_interactive(request, use_interactive, locale)?;
                let report = upsert_model_config(request).await?;
                print_json_or_pretty(args.json, &serde_json::to_value(report)?)?;
            }
            ModelCommand::List(args) => {
                let report = list_model_configs(ModelListRequest {
                    config_path: args.config_path,
                    root: args.root,
                    scope: args.scope.to_config_scope(),
                })
                .await?;
                print_json_or_pretty(args.json, &serde_json::to_value(report)?)?;
            }
            ModelCommand::Show(args) => {
                let args = *args;
                let report = show_model_config(ModelShowRequest {
                    name: args.name,
                    config_path: args.config_path,
                    root: args.root,
                    scope: args.scope.to_config_scope(),
                })
                .await?;
                print_json_or_pretty(args.json, &serde_json::to_value(report)?)?;
            }
            ModelCommand::Delete(args) => {
                let report = delete_model_config(ModelDeleteRequest {
                    name: args.name,
                    config_path: args.config_path,
                    root: args.root,
                    scope: args.scope.to_config_scope(),
                })
                .await?;
                print_json_or_pretty(args.json, &serde_json::to_value(report)?)?;
            }
        },
    }

    Ok(true)
}

#[cfg(feature = "gateway")]
fn print_json_or_pretty(json: bool, value: &Value) -> Result<(), Box<dyn std::error::Error>> {
    if json {
        println!("{}", serde_json::to_string_pretty(value)?);
        return Ok(());
    }
    match value {
        Value::Object(_) | Value::Array(_) => {
            println!("{}", serde_json::to_string_pretty(value)?);
        }
        _ => println!("{value}"),
    }
    Ok(())
}
