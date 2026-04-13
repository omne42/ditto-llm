use std::fs;
use std::path::Path;

use omne_fs_primitives::{AtomicWriteError, AtomicWriteOptions, write_file_atomically};
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
    Write(#[from] AtomicWriteError),
}

impl GatewayStateFile {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, GatewayStateFileError> {
        let raw = fs::read_to_string(path)?;
        Ok(serde_json::from_str(&raw)?)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), GatewayStateFileError> {
        let path = path.as_ref();
        let payload = serde_json::to_vec_pretty(&GatewayStateFile {
            virtual_keys: self
                .virtual_keys
                .iter()
                .map(VirtualKeyConfig::sanitized_for_persistence)
                .collect(),
            router: self.router.clone(),
        })
        .map_err(GatewayStateFileError::Parse)?;
        let options = AtomicWriteOptions {
            unix_mode: Some(0o600),
            ..AtomicWriteOptions::default()
        };
        write_file_atomically(&payload, path, &options).map_err(GatewayStateFileError::Write)
    }
}
