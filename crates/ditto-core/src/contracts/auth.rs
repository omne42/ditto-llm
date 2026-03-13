#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuthMethodKind {
    ApiKeyHeader,
    ApiKeyQuery,
    CommandToken,
    StaticBearer,
    SigV4,
    OAuthClientCredentials,
    OAuthDeviceCode,
    OAuthBrowserPkce,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderAuthHint {
    pub method: AuthMethodKind,
    pub env_keys: &'static [&'static str],
    pub query_param: Option<&'static str>,
    pub header_name: Option<&'static str>,
    pub prefix: Option<&'static str>,
}
