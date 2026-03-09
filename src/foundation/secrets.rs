use async_trait::async_trait;

use crate::config::Env;
use crate::core::Result;

#[async_trait]
pub trait SecretResolver: Send + Sync {
    async fn resolve_secret_string(&self, spec: &str, env: &Env) -> Result<String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultSecretResolver;

#[async_trait]
impl SecretResolver for DefaultSecretResolver {
    async fn resolve_secret_string(&self, spec: &str, env: &Env) -> Result<String> {
        crate::secrets::resolve_secret_string(spec, env).await
    }
}
