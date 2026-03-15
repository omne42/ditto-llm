use std::collections::BTreeMap;

use ::secret::SecretEnvironment;

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
    fn get_secret_env(&self, key: &str) -> Option<String> {
        self.get(key)
    }
}

pub fn parse_dotenv(contents: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::<String, String>::new();

    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let line = line.strip_prefix("export ").unwrap_or(line).trim();
        let Some((raw_key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let key = raw_key.trim();
        if key.is_empty() {
            continue;
        }

        let mut value = raw_value.trim().to_string();
        if let Some(stripped) = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
        {
            value = stripped.to_string();
        }

        if value.trim().is_empty() {
            continue;
        }

        out.insert(key.to_string(), value);
    }

    out
}

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
