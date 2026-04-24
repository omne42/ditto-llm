use std::path::PathBuf;

#[cfg(feature = "gateway")]
use crate::gateway::GatewayConfig;
use ditto_core::resources::RuntimeDefaultFile;

pub const CONFIG_FILE_NAME: &str = "config.toml";
#[cfg(feature = "gateway")]
pub const GATEWAY_CONFIG_FILE_NAME: &str = "gateway.json";

#[derive(Debug, Clone)]
pub struct ServerDataRoot {
    pub data_root: PathBuf,
    pub config_path: PathBuf,
    #[cfg(feature = "gateway")]
    pub gateway_config_path: PathBuf,
}

pub fn server_data_root(root: impl Into<PathBuf>) -> ServerDataRoot {
    let data_root = root.into();
    ServerDataRoot {
        config_path: data_root.join(CONFIG_FILE_NAME),
        #[cfg(feature = "gateway")]
        gateway_config_path: data_root.join(GATEWAY_CONFIG_FILE_NAME),
        data_root,
    }
}

pub fn default_server_data_root_files() -> Vec<RuntimeDefaultFile> {
    let files = vec![RuntimeDefaultFile::new(
        CONFIG_FILE_NAME,
        default_config_toml(),
    )];
    #[cfg(feature = "gateway")]
    let files = {
        let mut files = files;
        files.push(RuntimeDefaultFile::new(
            GATEWAY_CONFIG_FILE_NAME,
            default_gateway_config_json(),
        ));
        files
    };
    files
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
    fn default_server_data_root_files_expose_server_defaults() {
        let files = default_server_data_root_files();
        assert_eq!(files[0].file_name(), CONFIG_FILE_NAME);
        assert_eq!(files[0].contents(), default_config_toml());

        #[cfg(feature = "gateway")]
        {
            assert_eq!(files[1].file_name(), GATEWAY_CONFIG_FILE_NAME);
            assert!(files[1].contents().contains("\"listen\""));
        }
    }

    #[cfg(feature = "gateway")]
    #[test]
    fn inject_default_gateway_config_path_only_for_gateway_mode() {
        let data_root = server_data_root("/tmp/omne");

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
