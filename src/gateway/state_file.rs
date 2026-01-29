use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::VirtualKeyConfig;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GatewayStateFile {
    #[serde(default)]
    pub virtual_keys: Vec<VirtualKeyConfig>,
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

        let payload = serde_json::to_vec_pretty(self).map_err(GatewayStateFileError::Parse)?;
        let tmp_path = path.with_extension("tmp");

        if fs::write(&tmp_path, payload).is_err() {
            let payload = serde_json::to_vec_pretty(self).map_err(GatewayStateFileError::Parse)?;
            fs::write(path, payload).map_err(GatewayStateFileError::Write)?;
            return Ok(());
        }

        match fs::rename(&tmp_path, path) {
            Ok(()) => Ok(()),
            Err(_) => {
                let payload =
                    serde_json::to_vec_pretty(self).map_err(GatewayStateFileError::Parse)?;
                fs::write(path, payload).map_err(GatewayStateFileError::Write)?;
                let _ = fs::remove_file(&tmp_path);
                Ok(())
            }
        }
    }
}
