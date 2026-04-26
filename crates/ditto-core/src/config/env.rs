use std::collections::BTreeMap;

use ::secret_kit::SecretString;
use ::secret_kit::runtime::{SecretCommandRuntime, SecretEnvironment};
pub use config_kit::parse_dotenv;

#[derive(Clone, Default)]
pub struct Env {
    pub dotenv: BTreeMap<String, String>,
}

impl std::fmt::Debug for Env {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let keys: Vec<&str> = self.dotenv.keys().map(|key| key.as_str()).collect();
        f.debug_struct("Env").field("dotenv_keys", &keys).finish()
    }
}

impl Env {
    pub fn parse_dotenv(contents: &str) -> Self {
        Self {
            dotenv: parse_dotenv(contents),
        }
    }

    pub fn get(&self, key: &str) -> Option<String> {
        if let Some(value) = self.dotenv.get(key) {
            return Some(value.clone());
        }
        std::env::var(key)
            .ok()
            .filter(|value| !value.trim().is_empty())
    }
}

impl SecretEnvironment for Env {
    fn get_secret(&self, key: &str) -> Option<SecretString> {
        self.get(key).map(SecretString::from)
    }
}

impl SecretCommandRuntime for Env {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dotenv_basic() {
        let parsed = parse_dotenv(
            r#"
# comment
export OPENAI_COMPAT_API_KEY="sk-test"
FOO=bar
EMPTY=
"#,
        );
        assert_eq!(
            parsed.get("OPENAI_COMPAT_API_KEY").map(String::as_str),
            Some("sk-test")
        );
        assert_eq!(parsed.get("FOO").map(String::as_str), Some("bar"));
        assert_eq!(parsed.get("EMPTY"), None);
    }
}
