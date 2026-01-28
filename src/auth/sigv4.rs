use std::collections::BTreeMap;

use hmac::{Hmac, Mac};
use reqwest::Url;
use sha2::{Digest, Sha256};
use time::{OffsetDateTime, format_description::FormatItem, macros::format_description};

use crate::profile::{Env, ProviderAuth};
use crate::{DittoError, Result};

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone)]
pub struct SigV4Timestamp {
    pub amz_date: String,
    pub date: String,
}

impl SigV4Timestamp {
    pub fn now() -> Result<Self> {
        let now = OffsetDateTime::now_utc();
        Ok(Self::from_datetime(now)?)
    }

    pub fn from_datetime(datetime: OffsetDateTime) -> Result<Self> {
        const AMZ_FORMAT: &[FormatItem<'_>] =
            format_description!("[year][month][day]T[hour][minute][second]Z");
        const DATE_FORMAT: &[FormatItem<'_>] = format_description!("[year][month][day]");

        let amz_date = datetime.format(AMZ_FORMAT).map_err(|err| {
            DittoError::InvalidResponse(format!("failed to format sigv4 amz date: {err}"))
        })?;
        let date = datetime.format(DATE_FORMAT).map_err(|err| {
            DittoError::InvalidResponse(format!("failed to format sigv4 date: {err}"))
        })?;
        Ok(Self { amz_date, date })
    }

    pub fn from_amz_date(amz_date: &str) -> Result<Self> {
        let amz_date = amz_date.trim();
        if amz_date.len() < 8 {
            return Err(DittoError::InvalidResponse(
                "sigv4 amz date must be at least 8 chars".to_string(),
            ));
        }
        Ok(Self {
            amz_date: amz_date.to_string(),
            date: amz_date[..8].to_string(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct SigV4Signer {
    access_key: String,
    secret_key: String,
    session_token: Option<String>,
    region: String,
    service: String,
}

impl SigV4Signer {
    pub fn new(
        access_key: impl Into<String>,
        secret_key: impl Into<String>,
        session_token: Option<String>,
        region: impl Into<String>,
        service: impl Into<String>,
    ) -> Result<Self> {
        let access_key = access_key.into();
        let secret_key = secret_key.into();
        let region = region.into();
        let service = service.into();

        if access_key.trim().is_empty() {
            return Err(DittoError::InvalidResponse(
                "sigv4 access_key is required".to_string(),
            ));
        }
        if secret_key.trim().is_empty() {
            return Err(DittoError::InvalidResponse(
                "sigv4 secret_key is required".to_string(),
            ));
        }
        if region.trim().is_empty() {
            return Err(DittoError::InvalidResponse(
                "sigv4 region is required".to_string(),
            ));
        }
        if service.trim().is_empty() {
            return Err(DittoError::InvalidResponse(
                "sigv4 service is required".to_string(),
            ));
        }

        Ok(Self {
            access_key,
            secret_key,
            session_token,
            region,
            service,
        })
    }

    pub fn sign(
        &self,
        method: &str,
        url: &str,
        headers: &BTreeMap<String, String>,
        payload: &[u8],
        timestamp: SigV4Timestamp,
    ) -> Result<SigV4SigningResult> {
        let method = method.trim();
        if method.is_empty() {
            return Err(DittoError::InvalidResponse(
                "sigv4 method must be non-empty".to_string(),
            ));
        }

        let url = Url::parse(url).map_err(|err| {
            DittoError::InvalidResponse(format!("sigv4 invalid url {url:?}: {err}"))
        })?;
        let host = url
            .host_str()
            .ok_or_else(|| DittoError::InvalidResponse("sigv4 url missing host".to_string()))?;
        let host = match url.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_string(),
        };

        let payload_hash = sha256_hex(payload);
        let canonical_headers_map = prepare_headers(
            headers,
            &host,
            &timestamp.amz_date,
            &payload_hash,
            self.session_token.as_deref(),
        );
        let (canonical_headers, signed_headers) = canonical_headers(&canonical_headers_map);
        let canonical_request = format!(
            "{}\n{}\n{}\n{}\n{}\n{}",
            method,
            canonical_uri(&url),
            canonical_query(&url),
            canonical_headers,
            signed_headers,
            payload_hash
        );

        let scope = format!(
            "{}/{}/{}/aws4_request",
            timestamp.date, self.region, self.service
        );
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{}\n{}\n{}",
            timestamp.amz_date,
            scope,
            sha256_hex(canonical_request.as_bytes())
        );
        let signature = self.sign_string(&timestamp.date, &string_to_sign)?;
        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
            self.access_key, scope, signed_headers, signature
        );

        let headers = SigV4Headers {
            authorization,
            amz_date: timestamp.amz_date,
            content_sha256: payload_hash,
            host,
            security_token: self.session_token.clone(),
        };

        Ok(SigV4SigningResult {
            headers,
            signed_headers,
            signature,
            canonical_request,
            string_to_sign,
        })
    }

    fn sign_string(&self, date: &str, string_to_sign: &str) -> Result<String> {
        let k_date = hmac_sha256(format!("AWS4{}", self.secret_key).as_bytes(), date)?;
        let k_region = hmac_sha256(&k_date, self.region.as_str())?;
        let k_service = hmac_sha256(&k_region, self.service.as_str())?;
        let k_signing = hmac_sha256(&k_service, "aws4_request")?;
        let signature = hmac_sha256(&k_signing, string_to_sign)?;
        Ok(hex_encode(&signature))
    }
}

#[derive(Debug, Clone)]
pub struct SigV4Headers {
    pub authorization: String,
    pub amz_date: String,
    pub content_sha256: String,
    pub host: String,
    pub security_token: Option<String>,
}

impl SigV4Headers {
    pub fn apply(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let req = req
            .header("authorization", &self.authorization)
            .header("x-amz-date", &self.amz_date)
            .header("x-amz-content-sha256", &self.content_sha256)
            .header("host", &self.host);
        if let Some(token) = self.security_token.as_ref() {
            req.header("x-amz-security-token", token)
        } else {
            req
        }
    }
}

#[derive(Debug, Clone)]
pub struct SigV4SigningResult {
    pub headers: SigV4Headers,
    pub signed_headers: String,
    pub signature: String,
    pub canonical_request: String,
    pub string_to_sign: String,
}

fn prepare_headers(
    headers: &BTreeMap<String, String>,
    host: &str,
    amz_date: &str,
    payload_hash: &str,
    session_token: Option<&str>,
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::<String, String>::new();
    for (name, value) in headers {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        let key = name.to_ascii_lowercase();
        let value = normalize_header_value(value);
        if let Some(existing) = out.get_mut(&key) {
            if !existing.is_empty() {
                existing.push(',');
            }
            existing.push_str(&value);
        } else {
            out.insert(key, value);
        }
    }

    out.entry("host".to_string())
        .or_insert_with(|| host.to_string());
    out.insert("x-amz-date".to_string(), amz_date.to_string());
    out.entry("x-amz-content-sha256".to_string())
        .or_insert_with(|| payload_hash.to_string());
    if let Some(token) = session_token {
        out.insert(
            "x-amz-security-token".to_string(),
            normalize_header_value(token),
        );
    }
    out
}

fn canonical_headers(headers: &BTreeMap<String, String>) -> (String, String) {
    let mut canonical_headers = String::new();
    let mut signed_headers = Vec::<String>::new();

    for (name, value) in headers {
        canonical_headers.push_str(name);
        canonical_headers.push(':');
        canonical_headers.push_str(value);
        canonical_headers.push('\n');
        signed_headers.push(name.clone());
    }

    (canonical_headers, signed_headers.join(";"))
}

fn canonical_uri(url: &Url) -> String {
    let path = url.path();
    if path.is_empty() {
        "/".to_string()
    } else {
        aws_percent_encode(path, false)
    }
}

fn canonical_query(url: &Url) -> String {
    let mut pairs = Vec::<(String, String)>::new();
    for (name, value) in url.query_pairs() {
        pairs.push((
            aws_percent_encode(&name, true),
            aws_percent_encode(&value, true),
        ));
    }
    pairs.sort();
    pairs
        .into_iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("&")
}

fn aws_percent_encode(value: &str, encode_slash: bool) -> String {
    let mut out = String::new();
    for &byte in value.as_bytes() {
        let is_unreserved =
            matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~');
        if is_unreserved || (!encode_slash && byte == b'/') {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push(HEX_CHARS[(byte >> 4) as usize] as char);
            out.push(HEX_CHARS[(byte & 0x0f) as usize] as char);
        }
    }
    out
}

fn normalize_header_value(value: &str) -> String {
    let mut out = String::new();
    let mut last_space = false;
    for ch in value.chars() {
        if ch.is_whitespace() {
            if !last_space {
                out.push(' ');
                last_space = true;
            }
        } else {
            out.push(ch);
            last_space = false;
        }
    }
    out.trim().to_string()
}

fn hmac_sha256(key: &[u8], data: &str) -> Result<Vec<u8>> {
    let mut mac = HmacSha256::new_from_slice(key)
        .map_err(|err| DittoError::InvalidResponse(format!("sigv4 invalid hmac key: {err}")))?;
    mac.update(data.as_bytes());
    Ok(mac.finalize().into_bytes().to_vec())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    hex_encode(&digest)
}

const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX_CHARS[(byte >> 4) as usize] as char);
        out.push(HEX_CHARS[(byte & 0x0f) as usize] as char);
    }
    out
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
        return Err(DittoError::InvalidResponse(
            "expected sigv4 auth".to_string(),
        ));
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
    Err(DittoError::InvalidResponse(format!(
        "missing sigv4 {} (tried: {})",
        label,
        candidate_keys.join(", ")
    )))
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
