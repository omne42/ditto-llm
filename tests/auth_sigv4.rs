#![cfg(feature = "auth")]

use std::collections::BTreeMap;

use ditto_llm::Result;
use ditto_llm::auth::{SigV4Signer, SigV4Timestamp};

#[test]
fn sigv4_headers_match_example_signature() -> Result<()> {
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

    let signed = signer.sign(
        "GET",
        "https://iam.amazonaws.com/?Action=ListUsers&Version=2010-05-08",
        &headers,
        b"",
        timestamp,
    )?;

    assert_eq!(
        signed.headers.authorization,
        "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/iam/aws4_request, SignedHeaders=content-type;host;x-amz-content-sha256;x-amz-date, Signature=dd479fa8a80364edf2119ec24bebde66712ee9c9cb2b0d92eb3ab9ccdc0c3947"
    );
    Ok(())
}
