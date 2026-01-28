use std::fs::{OpenOptions, create_dir_all};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Result;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DevtoolsRecord {
    pub ts_ms: u64,
    pub kind: String,
    pub payload: Value,
}

impl DevtoolsRecord {
    pub fn new(kind: impl Into<String>, payload: Value) -> Self {
        Self {
            ts_ms: now_millis(),
            kind: kind.into(),
            payload,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DevtoolsLogger {
    path: PathBuf,
}

impl DevtoolsLogger {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn log_event(&self, kind: impl Into<String>, payload: Value) -> Result<()> {
        let record = DevtoolsRecord::new(kind, payload);
        self.log_record(&record)
    }

    pub fn log_record(&self, record: &DevtoolsRecord) -> Result<()> {
        self.write_json_line(record)
    }

    fn write_json_line<T: Serialize>(&self, value: &T) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                create_dir_all(parent)?;
            }
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer(&mut writer, value)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        Ok(())
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn devtools_writes_json_line() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("devtools.jsonl");
        let logger = DevtoolsLogger::new(&path);
        logger
            .log_event("stream", json!({"ok": true}))
            .expect("write");

        let contents = std::fs::read_to_string(&path).expect("read");
        let mut lines = contents.lines();
        let line = lines.next().expect("line");
        let value: Value = serde_json::from_str(line).expect("json");
        assert_eq!(value["kind"], "stream");
        assert_eq!(value["payload"], json!({"ok": true}));
        assert!(value.get("ts_ms").and_then(|v| v.as_u64()).is_some());
    }
}
