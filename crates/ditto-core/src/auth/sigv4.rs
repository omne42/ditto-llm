pub use http_auth_kit::{SigV4Headers, SigV4SigningResult};

use std::collections::BTreeMap;

use time::OffsetDateTime;

use crate::config::{Env, ProviderAuth};
use crate::error::{DittoError, Result};

fn sigv4_format_amz_date_failed(error: impl std::fmt::Display) -> DittoError {
    crate::invalid_response!(
        "error_detail.sigv4.format_amz_date_failed",
        "error" => error.to_string()
    )
}

fn sigv4_format_date_failed(error: impl std::fmt::Display) -> DittoError {
    crate::invalid_response!(
        "error_detail.sigv4.format_date_failed",
        "error" => error.to_string()
    )
}

fn sigv4_amz_date_too_short() -> DittoError {
    crate::invalid_response!("error_detail.sigv4.amz_date_too_short")
}

fn sigv4_field_required(field: &str) -> DittoError {
    crate::invalid_response!("error_detail.sigv4.field_required", "field" => field)
}

fn sigv4_method_empty() -> DittoError {
    crate::invalid_response!("error_detail.sigv4.method_empty")
}

fn sigv4_url_invalid(url: &str, error: impl std::fmt::Display) -> DittoError {
    crate::invalid_response!(
        "error_detail.sigv4.url_invalid",
        "url" => url,
        "error" => error.to_string()
    )
}

fn sigv4_url_missing_host() -> DittoError {
    crate::invalid_response!("error_detail.sigv4.url_missing_host")
}

fn sigv4_hmac_key_invalid(error: impl std::fmt::Display) -> DittoError {
    crate::invalid_response!(
        "error_detail.sigv4.hmac_key_invalid",
        "error" => error.to_string()
    )
}

fn sigv4_expected_auth() -> DittoError {
    crate::invalid_response!("error_detail.sigv4.expected_auth")
}

fn sigv4_missing_env(label: &str, keys: &str) -> DittoError {
    crate::invalid_response!(
        "error_detail.sigv4.missing_env",
        "label" => label,
        "keys" => keys
    )
}

fn sigv4_foundation_error(error: http_auth_kit::HttpAuthError) -> DittoError {
    match error {
        http_auth_kit::HttpAuthError::FieldRequired { field } => sigv4_field_required(field),
        http_auth_kit::HttpAuthError::SigV4FormatAmzDate(error) => {
            sigv4_format_amz_date_failed(error)
        }
        http_auth_kit::HttpAuthError::SigV4FormatDate(error) => sigv4_format_date_failed(error),
        http_auth_kit::HttpAuthError::SigV4AmzDateTooShort => sigv4_amz_date_too_short(),
        http_auth_kit::HttpAuthError::SigV4MethodEmpty => sigv4_method_empty(),
        http_auth_kit::HttpAuthError::SigV4UrlInvalid { url, message } => {
            sigv4_url_invalid(&url, message)
        }
        http_auth_kit::HttpAuthError::SigV4UrlMissingHost => sigv4_url_missing_host(),
        http_auth_kit::HttpAuthError::SigV4HmacKeyInvalid(error) => sigv4_hmac_key_invalid(error),
        other => crate::invalid_response!(format!("SigV4 auth failed: {other}")),
    }
}

#[derive(Debug, Clone)]
pub struct SigV4Timestamp {
    pub amz_date: String,
    pub date: String,
}

impl SigV4Timestamp {
    pub fn now() -> Result<Self> {
        let inner = http_auth_kit::SigV4Timestamp::now().map_err(sigv4_foundation_error)?;
        Ok(Self::from(inner))
    }

    pub fn from_datetime(datetime: OffsetDateTime) -> Result<Self> {
        let inner = http_auth_kit::SigV4Timestamp::from_datetime(datetime)
            .map_err(sigv4_foundation_error)?;
        Ok(Self::from(inner))
    }

    pub fn from_amz_date(amz_date: &str) -> Result<Self> {
        let inner = http_auth_kit::SigV4Timestamp::from_amz_date(amz_date)
            .map_err(sigv4_foundation_error)?;
        Ok(Self::from(inner))
    }

    fn to_foundation(&self) -> http_auth_kit::SigV4Timestamp {
        http_auth_kit::SigV4Timestamp {
            amz_date: self.amz_date.clone(),
            date: self.date.clone(),
        }
    }
}

impl From<http_auth_kit::SigV4Timestamp> for SigV4Timestamp {
    fn from(value: http_auth_kit::SigV4Timestamp) -> Self {
        Self {
            amz_date: value.amz_date,
            date: value.date,
        }
    }
}

#[derive(Clone)]
pub struct SigV4Signer {
    inner: http_auth_kit::SigV4Signer,
}

impl std::fmt::Debug for SigV4Signer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.inner.fmt(f)
    }
}

impl SigV4Signer {
    pub fn new(
        access_key: impl Into<String>,
        secret_key: impl Into<String>,
        session_token: Option<String>,
        region: impl Into<String>,
        service: impl Into<String>,
    ) -> Result<Self> {
        let inner =
            http_auth_kit::SigV4Signer::new(access_key, secret_key, session_token, region, service)
                .map_err(sigv4_foundation_error)?;
        Ok(Self { inner })
    }

    pub fn sign(
        &self,
        method: &str,
        url: &str,
        headers: &BTreeMap<String, String>,
        payload: &[u8],
        timestamp: SigV4Timestamp,
    ) -> Result<SigV4SigningResult> {
        self.inner
            .sign(method, url, headers, payload, timestamp.to_foundation())
            .map_err(sigv4_foundation_error)
    }
}

pub fn resolve_sigv4_signer(auth: &ProviderAuth, env: &Env) -> Result<SigV4Signer> {
    let ProviderAuth::SigV4 {
        access_keys,
        secret_keys,
        session_token_keys,
        region,
        service,
    } = auth
    else {
        return Err(sigv4_expected_auth());
    };

    let access_key = resolve_required_env(
        env,
        access_keys,
        &["AWS_ACCESS_KEY_ID", "AWS_ACCESS_KEY"],
        "access_key",
    )?;
    let secret_key = resolve_required_env(
        env,
        secret_keys,
        &["AWS_SECRET_ACCESS_KEY", "AWS_SECRET_KEY"],
        "secret_key",
    )?;
    let session_token = resolve_optional_env(env, session_token_keys, &["AWS_SESSION_TOKEN"]);

    SigV4Signer::new(
        access_key,
        secret_key,
        session_token,
        region.to_string(),
        service.to_string(),
    )
}

fn resolve_required_env(
    env: &Env,
    keys: &[String],
    defaults: &[&str],
    label: &str,
) -> Result<String> {
    let candidate_keys: Vec<String> = if keys.is_empty() {
        defaults.iter().map(|key| key.to_string()).collect()
    } else {
        keys.to_vec()
    };

    for key in &candidate_keys {
        if let Some(value) = env.get(key.as_str()) {
            return Ok(value);
        }
    }
    Err(sigv4_missing_env(label, &candidate_keys.join(", ")))
}

fn resolve_optional_env(env: &Env, keys: &[String], defaults: &[&str]) -> Option<String> {
    let candidate_keys: Vec<String> = if keys.is_empty() {
        defaults.iter().map(|key| key.to_string()).collect()
    } else {
        keys.to_vec()
    };
    for key in &candidate_keys {
        if let Some(value) = env.get(key.as_str()) {
            return Some(value);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn signs_canonical_sigv4_headers() -> Result<()> {
        let signer = SigV4Signer::new(
            "AKIDEXAMPLE",
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            None,
            "us-east-1",
            "iam",
        )?;
        let mut headers = BTreeMap::new();
        headers.insert(
            "Content-Type".to_string(),
            "application/x-www-form-urlencoded; charset=utf-8".to_string(),
        );

        let timestamp = SigV4Timestamp::from_amz_date("20150830T123600Z")?;
        let result = signer.sign(
            "GET",
            "https://iam.amazonaws.com/?Action=ListUsers&Version=2010-05-08",
            &headers,
            b"",
            timestamp,
        )?;

        let expected_canonical = [
            "GET",
            "/",
            "Action=ListUsers&Version=2010-05-08",
            "content-type:application/x-www-form-urlencoded; charset=utf-8",
            "host:iam.amazonaws.com",
            "x-amz-content-sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "x-amz-date:20150830T123600Z",
            "",
            "content-type;host;x-amz-content-sha256;x-amz-date",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ]
        .join("\n");

        assert_eq!(result.canonical_request, expected_canonical);
        assert_eq!(
            result.headers.authorization,
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/iam/aws4_request, SignedHeaders=content-type;host;x-amz-content-sha256;x-amz-date, Signature=dd479fa8a80364edf2119ec24bebde66712ee9c9cb2b0d92eb3ab9ccdc0c3947"
        );
        Ok(())
    }

    #[test]
    fn resolves_sigv4_from_env() -> Result<()> {
        let env = Env {
            dotenv: BTreeMap::from([
                ("AWS_ACCESS_KEY_ID".to_string(), "AKIDEXAMPLE".to_string()),
                (
                    "AWS_SECRET_ACCESS_KEY".to_string(),
                    "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".to_string(),
                ),
            ]),
        };
        let auth = ProviderAuth::SigV4 {
            access_keys: Vec::new(),
            secret_keys: Vec::new(),
            session_token_keys: Vec::new(),
            region: "us-east-1".to_string(),
            service: "iam".to_string(),
        };

        let signer = resolve_sigv4_signer(&auth, &env)?;
        let timestamp = SigV4Timestamp::from_amz_date("20150830T123600Z")?;
        let result = signer.sign(
            "GET",
            "https://iam.amazonaws.com/?Action=ListUsers&Version=2010-05-08",
            &BTreeMap::new(),
            b"",
            timestamp,
        )?;
        assert!(
            result
                .headers
                .authorization
                .contains("Credential=AKIDEXAMPLE/20150830/us-east-1/iam/aws4_request")
        );
        Ok(())
    }
}
