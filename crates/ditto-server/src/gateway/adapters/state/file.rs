use std::fs;
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{RouterConfig, VirtualKeyConfig};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GatewayStateFile {
    #[serde(default)]
    pub virtual_keys: Vec<VirtualKeyConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub router: Option<RouterConfig>,
}

#[derive(Debug, Error)]
pub enum GatewayStateFileError {
    #[error("read state file failed: {0}")]
    Read(#[from] std::io::Error),
    #[error("parse state file failed: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("write state file failed: {0}")]
    Write(std::io::Error),
}

impl GatewayStateFile {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, GatewayStateFileError> {
        let raw = fs::read_to_string(path)?;
        Ok(serde_json::from_str(&raw)?)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), GatewayStateFileError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                if let Err(err) = fs::create_dir_all(parent) {
                    return Err(GatewayStateFileError::Write(err));
                }
            }
        }

        let payload = serde_json::to_vec_pretty(&GatewayStateFile {
            virtual_keys: self
                .virtual_keys
                .iter()
                .map(VirtualKeyConfig::sanitized_for_persistence)
                .collect(),
            router: self.router.clone(),
        })
        .map_err(GatewayStateFileError::Parse)?;
        let tmp_path = path.with_extension("tmp");

        if write_restricted_file(&tmp_path, &payload).is_err() {
            write_restricted_file(path, &payload).map_err(GatewayStateFileError::Write)?;
            return Ok(());
        }

        match fs::rename(&tmp_path, path) {
            Ok(()) => {
                set_restricted_permissions(path).map_err(GatewayStateFileError::Write)?;
                Ok(())
            }
            Err(_) => {
                write_restricted_file(path, &payload).map_err(GatewayStateFileError::Write)?;
                let _ = fs::remove_file(&tmp_path);
                Ok(())
            }
        }
    }
}

fn write_restricted_file(path: &Path, payload: &[u8]) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    {
        use std::io::Write as _;

        let mut file = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(payload)?;
        file.sync_all()?;
        Ok(())
    }

    #[cfg(not(unix))]
    {
        fs::write(path, payload)
    }
}

fn set_restricted_permissions(path: &Path) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    {
        let permissions = fs::Permissions::from_mode(0o600);
        fs::set_permissions(path, permissions)?;
    }

    Ok(())
}
