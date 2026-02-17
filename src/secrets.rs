use std::collections::BTreeMap;
use std::time::Duration;

use crate::profile::Env;
use crate::{DittoError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretSpec {
    Env {
        key: String,
    },
    File {
        path: String,
    },
    VaultCli {
        path: String,
        field: String,
        namespace: Option<String>,
    },
    AwsSecretsManagerCli {
        secret_id: String,
        region: Option<String>,
        profile: Option<String>,
        json_key: Option<String>,
    },
    GcpSecretManagerCli {
        secret: String,
        project: Option<String>,
        version: String,
        json_key: Option<String>,
    },
    AzureKeyVaultCli {
        vault: String,
        name: String,
        version: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretCommand {
    pub program: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub json_key: Option<String>,
}

impl SecretSpec {
    pub fn parse(input: &str) -> Result<Self> {
        let input = input.trim();
        let rest = input.strip_prefix("secret://").ok_or_else(|| {
            DittoError::InvalidResponse("secret must start with secret://".into())
        })?;

        let (head, query) = rest.split_once('?').unwrap_or((rest, ""));
        let query = parse_query(query);

        let (provider, tail) = head
            .split_once('/')
            .map(|(provider, tail)| (provider, Some(tail)))
            .unwrap_or((head, None));
        let provider = provider.trim();

        match provider {
            "env" => {
                let key = tail.unwrap_or_default().trim();
                if key.is_empty() {
                    return Err(DittoError::InvalidResponse(
                        "secret://env/<KEY> requires a key".into(),
                    ));
                }
                Ok(Self::Env {
                    key: key.to_string(),
                })
            }
            "file" => {
                let path = query
                    .get("path")
                    .cloned()
                    .or_else(|| tail.map(|v| v.to_string()))
                    .unwrap_or_default();
                let path = path.trim();
                if path.is_empty() {
                    return Err(DittoError::InvalidResponse(
                        "secret://file requires ?path=... or secret://file/<path>".into(),
                    ));
                }
                Ok(Self::File {
                    path: path.to_string(),
                })
            }
            "vault" => {
                let path = tail.unwrap_or_default().trim();
                if path.is_empty() {
                    return Err(DittoError::InvalidResponse(
                        "secret://vault/<path> requires a path".into(),
                    ));
                }
                let field = query
                    .get("field")
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
                    .unwrap_or_else(|| "token".to_string());
                let namespace = query
                    .get("namespace")
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty());
                Ok(Self::VaultCli {
                    path: path.to_string(),
                    field,
                    namespace,
                })
            }
            "aws-sm" | "aws-secrets-manager" => {
                let secret_id = tail.unwrap_or_default().trim();
                if secret_id.is_empty() {
                    return Err(DittoError::InvalidResponse(
                        "secret://aws-sm/<secret_id> requires a secret id".into(),
                    ));
                }
                Ok(Self::AwsSecretsManagerCli {
                    secret_id: secret_id.to_string(),
                    region: query
                        .get("region")
                        .cloned()
                        .filter(|v| !v.trim().is_empty()),
                    profile: query
                        .get("profile")
                        .cloned()
                        .filter(|v| !v.trim().is_empty()),
                    json_key: query
                        .get("json_key")
                        .cloned()
                        .filter(|v| !v.trim().is_empty()),
                })
            }
            "gcp-sm" | "gcp-secret-manager" => {
                let secret = tail.unwrap_or_default().trim();
                if secret.is_empty() {
                    return Err(DittoError::InvalidResponse(
                        "secret://gcp-sm/<secret> requires a secret name".into(),
                    ));
                }
                Ok(Self::GcpSecretManagerCli {
                    secret: secret.to_string(),
                    project: query
                        .get("project")
                        .cloned()
                        .filter(|v| !v.trim().is_empty()),
                    version: query
                        .get("version")
                        .cloned()
                        .filter(|v| !v.trim().is_empty())
                        .unwrap_or_else(|| "latest".to_string()),
                    json_key: query
                        .get("json_key")
                        .cloned()
                        .filter(|v| !v.trim().is_empty()),
                })
            }
            "azure-kv" | "azure-key-vault" => {
                let tail = tail.unwrap_or_default();
                let (vault, name) = tail.split_once('/').ok_or_else(|| {
                    DittoError::InvalidResponse(
                        "secret://azure-kv/<vault>/<name> requires vault and name".into(),
                    )
                })?;
                let vault = vault.trim();
                let name = name.trim();
                if vault.is_empty() || name.is_empty() {
                    return Err(DittoError::InvalidResponse(
                        "secret://azure-kv/<vault>/<name> requires vault and name".into(),
                    ));
                }
                Ok(Self::AzureKeyVaultCli {
                    vault: vault.to_string(),
                    name: name.to_string(),
                    version: query
                        .get("version")
                        .cloned()
                        .filter(|v| !v.trim().is_empty()),
                })
            }
            other => Err(DittoError::InvalidResponse(format!(
                "unsupported secret provider: {other}"
            ))),
        }
    }

    pub fn build_command(&self) -> Option<SecretCommand> {
        match self {
            SecretSpec::Env { .. } | SecretSpec::File { .. } => None,
            SecretSpec::VaultCli {
                path,
                field,
                namespace,
            } => {
                let mut env = BTreeMap::new();
                if let Some(namespace) = namespace.as_deref() {
                    env.insert("VAULT_NAMESPACE".to_string(), namespace.to_string());
                }
                Some(SecretCommand {
                    program: "vault".to_string(),
                    args: vec![
                        "kv".to_string(),
                        "get".to_string(),
                        format!("-field={field}"),
                        path.to_string(),
                    ],
                    env,
                    json_key: None,
                })
            }
            SecretSpec::AwsSecretsManagerCli {
                secret_id,
                region,
                profile,
                json_key,
            } => {
                let mut args = vec![
                    "secretsmanager".to_string(),
                    "get-secret-value".to_string(),
                    "--secret-id".to_string(),
                    secret_id.to_string(),
                    "--query".to_string(),
                    "SecretString".to_string(),
                    "--output".to_string(),
                    "text".to_string(),
                ];
                if let Some(region) = region.as_deref() {
                    args.push("--region".to_string());
                    args.push(region.to_string());
                }
                if let Some(profile) = profile.as_deref() {
                    args.push("--profile".to_string());
                    args.push(profile.to_string());
                }
                Some(SecretCommand {
                    program: "aws".to_string(),
                    args,
                    env: BTreeMap::new(),
                    json_key: json_key.clone(),
                })
            }
            SecretSpec::GcpSecretManagerCli {
                secret,
                project,
                version,
                json_key,
            } => {
                let mut args = vec![
                    "secrets".to_string(),
                    "versions".to_string(),
                    "access".to_string(),
                    version.to_string(),
                    "--secret".to_string(),
                    secret.to_string(),
                ];
                if let Some(project) = project.as_deref() {
                    args.push("--project".to_string());
                    args.push(project.to_string());
                }
                Some(SecretCommand {
                    program: "gcloud".to_string(),
                    args,
                    env: BTreeMap::new(),
                    json_key: json_key.clone(),
                })
            }
            SecretSpec::AzureKeyVaultCli {
                vault,
                name,
                version,
            } => {
                let mut args = vec![
                    "keyvault".to_string(),
                    "secret".to_string(),
                    "show".to_string(),
                    "--vault-name".to_string(),
                    vault.to_string(),
                    "--name".to_string(),
                    name.to_string(),
                    "--query".to_string(),
                    "value".to_string(),
                    "-o".to_string(),
                    "tsv".to_string(),
                ];
                if let Some(version) = version.as_deref() {
                    args.push("--version".to_string());
                    args.push(version.to_string());
                }
                Some(SecretCommand {
                    program: "az".to_string(),
                    args,
                    env: BTreeMap::new(),
                    json_key: None,
                })
            }
        }
    }

    pub async fn resolve(&self, env: &Env) -> Result<String> {
        match self {
            SecretSpec::Env { key } => env
                .get(key)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| DittoError::AuthCommand(format!("missing env var: {key}"))),
            SecretSpec::File { path } => {
                let contents = tokio::fs::read_to_string(path).await?;
                let value = contents.trim().to_string();
                if value.is_empty() {
                    return Err(DittoError::InvalidResponse(format!(
                        "secret file is empty: {path}"
                    )));
                }
                Ok(value)
            }
            other => {
                let cmd = other.build_command().ok_or_else(|| {
                    DittoError::InvalidResponse("secret is not resolvable".into())
                })?;
                let value = run_secret_command(&cmd, env).await?;
                if let Some(json_key) = cmd.json_key.as_deref() {
                    let extracted = extract_json_key(&value, json_key)?;
                    return Ok(extracted);
                }
                Ok(value)
            }
        }
    }
}

const DEFAULT_SECRET_COMMAND_TIMEOUT_SECS: u64 = 15;
const MAX_SECRET_COMMAND_TIMEOUT_SECS: u64 = 300;

fn secret_command_timeout(env: &Env) -> Duration {
    let ms = env
        .get("DITTO_SECRET_COMMAND_TIMEOUT_MS")
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0);
    if let Some(ms) = ms {
        return Duration::from_millis(ms);
    }

    let secs = env
        .get("DITTO_SECRET_COMMAND_TIMEOUT_SECS")
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_SECRET_COMMAND_TIMEOUT_SECS)
        .min(MAX_SECRET_COMMAND_TIMEOUT_SECS);
    Duration::from_secs(secs)
}

async fn run_secret_command(cmd: &SecretCommand, env: &Env) -> Result<String> {
    let timeout = secret_command_timeout(env);

    let mut command = tokio::process::Command::new(cmd.program.as_str());
    command.args(&cmd.args);
    for (key, value) in &cmd.env {
        command.env(key, value);
    }

    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());
    command.kill_on_drop(true);

    let mut child = command
        .spawn()
        .map_err(|err| DittoError::AuthCommand(format!("spawn {}: {err}", cmd.program)))?;
    let stdout = child.stdout.take().ok_or_else(|| {
        DittoError::AuthCommand(format!("command {} did not capture stdout", cmd.program))
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        DittoError::AuthCommand(format!("command {} did not capture stderr", cmd.program))
    })?;

    const MAX_SECRET_COMMAND_OUTPUT_BYTES: usize = 64 * 1024;

    let stdout_task = tokio::spawn(read_limited(stdout, MAX_SECRET_COMMAND_OUTPUT_BYTES));
    let stderr_task = tokio::spawn(read_limited(stderr, MAX_SECRET_COMMAND_OUTPUT_BYTES));

    let status = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(status) => {
            status.map_err(|err| DittoError::AuthCommand(format!("wait {}: {err}", cmd.program)))?
        }
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(DittoError::AuthCommand(format!(
                "command {} timed out after {}ms",
                cmd.program,
                timeout.as_millis()
            )));
        }
    };

    let stdout = stdout_task
        .await
        .map_err(|err| DittoError::AuthCommand(format!("join stdout reader: {err}")))??;
    let stderr = stderr_task
        .await
        .map_err(|err| DittoError::AuthCommand(format!("join stderr reader: {err}")))??;

    if !status.success() {
        let stderr = String::from_utf8_lossy(&stderr);
        let stderr = stderr.trim();
        if stderr.is_empty() {
            return Err(DittoError::AuthCommand(format!(
                "command {} failed with status {}",
                cmd.program, status
            )));
        }

        let preview = stderr
            .chars()
            .take(200)
            .collect::<String>()
            .trim()
            .to_string();
        return Err(DittoError::AuthCommand(format!(
            "command {} failed with status {}: {}",
            cmd.program, status, preview
        )));
    }

    let stdout = String::from_utf8_lossy(&stdout);
    let value = stdout.trim().to_string();
    if value.is_empty() {
        return Err(DittoError::AuthCommand(format!(
            "command {} returned empty stdout",
            cmd.program
        )));
    }
    Ok(value)
}

async fn read_limited<R>(mut reader: R, max_bytes: usize) -> Result<Vec<u8>>
where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncReadExt as _;

    let mut out = Vec::<u8>::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        if out.len().saturating_add(n) > max_bytes {
            return Err(DittoError::AuthCommand(format!(
                "command output exceeds {} bytes",
                max_bytes
            )));
        }
        out.extend_from_slice(&buf[..n]);
    }
    Ok(out)
}

pub async fn resolve_secret_string(spec: &str, env: &Env) -> Result<String> {
    let spec = SecretSpec::parse(spec)?;
    spec.resolve(env).await
}

fn parse_query(query: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::<String, String>::new();
    for pair in query.split('&') {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        out.insert(key.to_string(), value.trim().to_string());
    }
    out
}

fn extract_json_key(json: &str, key: &str) -> Result<String> {
    let value: serde_json::Value = serde_json::from_str(json)?;
    let mut cursor = &value;
    for part in key.split('.').map(str::trim).filter(|p| !p.is_empty()) {
        cursor = cursor.get(part).ok_or_else(|| {
            DittoError::InvalidResponse(format!("secret json missing key: {key}"))
        })?;
    }
    match cursor {
        serde_json::Value::String(value) => Ok(value.clone()),
        serde_json::Value::Number(value) => Ok(value.to_string()),
        serde_json::Value::Bool(value) => Ok(value.to_string()),
        other => Ok(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn resolves_env_secret() -> Result<()> {
        let env = Env {
            dotenv: BTreeMap::from([("TEST_SECRET".to_string(), "ok".to_string())]),
        };
        let value = resolve_secret_string("secret://env/TEST_SECRET", &env).await?;
        assert_eq!(value, "ok");
        Ok(())
    }

    #[tokio::test]
    async fn resolves_file_secret() -> Result<()> {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("secret.txt");
        tokio::fs::write(&path, "  hello  \n").await?;
        let env = Env {
            dotenv: BTreeMap::new(),
        };
        let value = resolve_secret_string(
            &format!("secret://file?path={}", path.to_string_lossy()),
            &env,
        )
        .await?;
        assert_eq!(value, "hello");
        Ok(())
    }

    #[test]
    fn parses_command_specs() -> Result<()> {
        let spec = SecretSpec::parse(
            "secret://aws-sm/mysecret?region=us-east-1&profile=dev&json_key=token",
        )?;
        let cmd = spec.build_command().expect("command");
        assert_eq!(cmd.program, "aws");
        assert!(cmd.args.iter().any(|arg| arg == "secretsmanager"));
        assert_eq!(cmd.json_key.as_deref(), Some("token"));

        let spec = SecretSpec::parse("secret://azure-kv/myvault/mysecret")?;
        let cmd = spec.build_command().expect("command");
        assert_eq!(cmd.program, "az");

        let spec = SecretSpec::parse("secret://vault/secret/openai?field=api_key&namespace=team")?;
        let cmd = spec.build_command().expect("command");
        assert_eq!(cmd.program, "vault");
        assert_eq!(
            cmd.env.get("VAULT_NAMESPACE").map(String::as_str),
            Some("team")
        );
        Ok(())
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn secret_command_runner_returns_stdout() -> Result<()> {
        let env = Env {
            dotenv: BTreeMap::new(),
        };
        let cmd = SecretCommand {
            program: "sh".to_string(),
            args: vec!["-c".to_string(), "echo ok".to_string()],
            env: BTreeMap::new(),
            json_key: None,
        };
        let value = run_secret_command(&cmd, &env).await?;
        assert_eq!(value, "ok");
        Ok(())
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn secret_command_runner_times_out() -> Result<()> {
        let env = Env {
            dotenv: BTreeMap::from([(
                "DITTO_SECRET_COMMAND_TIMEOUT_MS".to_string(),
                "10".to_string(),
            )]),
        };
        let cmd = SecretCommand {
            program: "sh".to_string(),
            args: vec!["-c".to_string(), "sleep 1; echo ok".to_string()],
            env: BTreeMap::new(),
            json_key: None,
        };
        let err = run_secret_command(&cmd, &env).await.unwrap_err();
        assert!(matches!(err, DittoError::AuthCommand(_)));
        assert!(err.to_string().contains("timed out"));
        Ok(())
    }

    #[cfg(all(unix, target_os = "linux"))]
    #[tokio::test]
    async fn secret_command_runner_cancellation_kills_child_process() -> Result<()> {
        fn process_terminated_or_zombie(pid: u32) -> bool {
            let status_path = format!("/proc/{pid}/status");
            match std::fs::read_to_string(status_path) {
                Ok(status) => status
                    .lines()
                    .find(|line| line.starts_with("State:"))
                    .map(|line| line.contains("\tZ") || line.contains(" zombie"))
                    .unwrap_or(false),
                Err(err) => err.kind() == std::io::ErrorKind::NotFound,
            }
        }

        let dir = tempfile::tempdir().expect("tempdir");
        let pid_file = dir.path().join("secret-command.pid");
        let script = format!("echo $$ > '{}'; exec sleep 30", pid_file.display());
        let cmd = SecretCommand {
            program: "sh".to_string(),
            args: vec!["-c".to_string(), script],
            env: BTreeMap::new(),
            json_key: None,
        };
        let env = Env {
            dotenv: BTreeMap::new(),
        };

        let handle = tokio::spawn(async move {
            let _ = run_secret_command(&cmd, &env).await;
        });

        let mut pid: Option<u32> = None;
        for _ in 0..100 {
            if let Ok(raw) = tokio::fs::read_to_string(&pid_file).await {
                let parsed = raw.trim().parse::<u32>().ok();
                if parsed.is_some() {
                    pid = parsed;
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let pid = pid.expect("pid file should be written");

        handle.abort();
        let _ = handle.await;

        let mut gone = false;
        for _ in 0..300 {
            if process_terminated_or_zombie(pid) {
                gone = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            gone,
            "secret command child process should be killed on cancellation"
        );
        Ok(())
    }
}
