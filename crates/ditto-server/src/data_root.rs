use std::io::Write as _;
use std::path::{Path, PathBuf};

#[cfg(feature = "gateway")]
use crate::gateway::GatewayConfig;
use ditto_core::error::{DittoError, Result};
use text_assets_kit::{DataRootOptions, DataRootScope, ensure_data_root};

pub const CONFIG_FILE_NAME: &str = "config.toml";
#[cfg(feature = "gateway")]
pub const GATEWAY_CONFIG_FILE_NAME: &str = "gateway.json";
pub const DATA_ROOT_DIR_NAME: &str = ".omne_data";
pub const DATA_ROOT_ENV_VAR: &str = "OMNE_DATA_DIR";

#[derive(Debug, Clone)]
pub struct ServerDataRoot {
    pub data_root: PathBuf,
    pub config_path: PathBuf,
    #[cfg(feature = "gateway")]
    pub gateway_config_path: PathBuf,
}

pub fn data_root_options(data_dir: Option<PathBuf>, scope: DataRootScope) -> DataRootOptions {
    let options = DataRootOptions::default()
        .with_dir_name(DATA_ROOT_DIR_NAME)
        .with_env_var(DATA_ROOT_ENV_VAR)
        .with_scope(scope);
    match data_dir {
        Some(data_dir) => options.with_data_dir(data_dir),
        None => options,
    }
}

pub fn sniff_data_root_options(args: &[String]) -> Result<DataRootOptions> {
    let mut data_dir = None;
    let mut scope = DataRootScope::Auto;
    let mut iter = args.iter();

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--root" => {
                let value = iter.next().ok_or_else(|| {
                    ditto_core::config_error!(
                        "error_detail.config.missing_flag_value",
                        "flag" => "--root"
                    )
                })?;
                if value.trim().is_empty() {
                    return Err(ditto_core::config_error!(
                        "error_detail.config.empty_flag_value",
                        "flag" => "--root"
                    ));
                }
                data_dir = Some(PathBuf::from(value));
            }
            "--scope" => {
                let value = iter.next().ok_or_else(|| {
                    ditto_core::config_error!(
                        "error_detail.config.missing_flag_value",
                        "flag" => "--scope"
                    )
                })?;
                scope = parse_scope(value)?;
            }
            _ => {
                if let Some(value) = arg.strip_prefix("--root=") {
                    if !value.trim().is_empty() {
                        data_dir = Some(PathBuf::from(value));
                    } else {
                        return Err(ditto_core::config_error!(
                            "error_detail.config.empty_flag_value",
                            "flag" => "--root"
                        ));
                    }
                    continue;
                }
                if let Some(value) = arg.strip_prefix("--scope=") {
                    scope = parse_scope(value)?;
                }
            }
        }
    }

    Ok(data_root_options(data_dir, scope))
}

pub fn bootstrap_server_data_root_with_options(
    options: &DataRootOptions,
) -> Result<ServerDataRoot> {
    let data_root = ensure_data_root(options).map_err(DittoError::Io)?;
    bootstrap_server_files(&data_root).map_err(DittoError::Io)?;

    Ok(ServerDataRoot {
        config_path: data_root.join(CONFIG_FILE_NAME),
        #[cfg(feature = "gateway")]
        gateway_config_path: data_root.join(GATEWAY_CONFIG_FILE_NAME),
        data_root,
    })
}

pub fn bootstrap_cli_runtime_with_options(options: &DataRootOptions) -> Result<ServerDataRoot> {
    let data_root = bootstrap_server_data_root_with_options(options)?;
    ditto_core::resources::bootstrap_runtime_assets_with_options(options)?;
    Ok(data_root)
}

pub fn bootstrap_cli_runtime_from_args(args: &[String]) -> Result<ServerDataRoot> {
    let options = sniff_data_root_options(args)?;
    bootstrap_cli_runtime_with_options(&options)
}

#[cfg(feature = "gateway")]
pub fn inject_default_gateway_config_path(
    mut args: Vec<String>,
    data_root: &ServerDataRoot,
) -> Vec<String> {
    match args.first().map(String::as_str) {
        Some("provider" | "model") => args,
        Some(first) if !first.starts_with('-') => args,
        _ => {
            let mut normalized = Vec::with_capacity(args.len() + 1);
            normalized.push(data_root.gateway_config_path.display().to_string());
            normalized.append(&mut args);
            normalized
        }
    }
}

fn parse_scope(value: &str) -> Result<DataRootScope> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Ok(DataRootScope::Auto),
        "workspace" => Ok(DataRootScope::Workspace),
        "global" => Ok(DataRootScope::Global),
        _ => Err(ditto_core::config_error!(
            "error_detail.config.invalid_flag_value",
            "flag" => "--scope",
            "value" => value
        )),
    }
}

fn bootstrap_server_files(data_root: &Path) -> std::io::Result<()> {
    write_if_missing(data_root.join(CONFIG_FILE_NAME), default_config_toml())?;

    #[cfg(feature = "gateway")]
    write_if_missing(
        data_root.join(GATEWAY_CONFIG_FILE_NAME),
        &default_gateway_config_json(),
    )?;

    Ok(())
}

fn write_if_missing(path: PathBuf, contents: &str) -> std::io::Result<()> {
    match std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
    {
        Ok(mut file) => file.write_all(contents.as_bytes()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error),
    }
}

fn default_config_toml() -> &'static str {
    "# Ditto provider and model overrides live here.\n# This file is created automatically and can be edited safely.\n"
}

#[cfg(feature = "gateway")]
fn default_gateway_config_json() -> String {
    serde_json::to_string_pretty(&GatewayConfig::default())
        .expect("gateway default config JSON must be serializable")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff_data_root_options_reads_root_and_scope_flags() {
        let options = sniff_data_root_options(&[
            "--root".to_string(),
            "/tmp/omne".to_string(),
            "--scope".to_string(),
            "workspace".to_string(),
        ])
        .expect("parse data root options");

        let debug = format!("{options:?}");
        assert!(debug.contains("data_dir: Some(\"/tmp/omne\")"));
        assert!(debug.contains("scope: Workspace"));
    }

    #[test]
    fn sniff_data_root_options_rejects_missing_root_value() {
        let err = sniff_data_root_options(&["--root".to_string()]).expect_err("missing value");
        let DittoError::Config(message) = err else {
            panic!("expected config error");
        };
        assert_eq!(
            message.as_catalog().map(|message| message.code()),
            Some("error_detail.config.missing_flag_value")
        );
    }

    #[test]
    fn sniff_data_root_options_rejects_invalid_scope() {
        let err =
            sniff_data_root_options(&["--scope=sideways".to_string()]).expect_err("invalid scope");
        let DittoError::Config(message) = err else {
            panic!("expected config error");
        };
        assert_eq!(
            message.as_catalog().map(|message| message.code()),
            Some("error_detail.config.invalid_flag_value")
        );
    }

    #[cfg(feature = "gateway")]
    #[test]
    fn inject_default_gateway_config_path_only_for_gateway_mode() {
        let data_root = ServerDataRoot {
            data_root: PathBuf::from("/tmp/omne"),
            config_path: PathBuf::from("/tmp/omne/config.toml"),
            gateway_config_path: PathBuf::from("/tmp/omne/gateway.json"),
        };

        assert_eq!(
            inject_default_gateway_config_path(Vec::new(), &data_root),
            vec!["/tmp/omne/gateway.json".to_string()]
        );
        assert_eq!(
            inject_default_gateway_config_path(vec!["--help".to_string()], &data_root),
            vec!["/tmp/omne/gateway.json".to_string(), "--help".to_string()]
        );
        assert_eq!(
            inject_default_gateway_config_path(vec!["provider".to_string()], &data_root),
            vec!["provider".to_string()]
        );
    }
}
