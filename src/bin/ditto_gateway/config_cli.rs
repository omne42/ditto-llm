#[cfg(feature = "gateway")]
use clap::{Args, Parser, Subcommand, ValueEnum};

#[cfg(feature = "gateway")]
use ditto_llm::{
    ConfigScope, ModelDeleteRequest, ModelListRequest, ModelShowRequest, ModelUpsertRequest,
    ProviderApi, ProviderAuthType, ProviderDeleteRequest, ProviderListRequest, ProviderNamespace,
    ProviderShowRequest, ProviderUpsertRequest, complete_model_upsert_request_interactive,
    complete_provider_upsert_request_interactive, delete_model_config, delete_provider_config,
    list_model_configs, list_provider_configs, show_model_config, show_provider_config,
    upsert_model_config, upsert_provider_config,
};

#[cfg(feature = "gateway")]
use serde_json::Value;

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
    let cli = ConfigCli::try_parse_from(argv).map_err(|err| err.to_string())?;

    match cli.command {
        ConfigCommand::Provider { command } => match command {
            ProviderCommand::Add(args) => {
                let args = *args;
                let use_interactive = !args.no_interactive || args.interactive;
                let mut request = ProviderUpsertRequest {
                    name: args.name,
                    config_path: args.config_path,
                    root: args.root,
                    scope: args.scope.to_config_scope(),
                    namespace: args.namespace.to_provider_namespace(),
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
                    register_models: args.register_models,
                    model_limit: args.model_limit,
                };
                if use_interactive {
                    request = complete_provider_upsert_request_interactive(request)?;
                }
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
                let use_interactive = !args.no_interactive || args.interactive;
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
                if use_interactive {
                    request = complete_model_upsert_request_interactive(request)?;
                }
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
