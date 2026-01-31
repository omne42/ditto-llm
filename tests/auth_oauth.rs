#![cfg(feature = "auth")]

use ditto_llm::Result;
use ditto_llm::auth::OAuthClientCredentials;
use httpmock::{Method::POST, MockServer};

#[tokio::test]
async fn oauth_client_credentials_fetches_token() -> Result<()> {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return Ok(());
    }
    let server = MockServer::start_async().await;
    let mock = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/token")
                .body_includes("grant_type=client_credentials")
                .body_includes("client_id=client-a")
                .body_includes("client_secret=secret-a")
                .body_includes("scope=scope-a");
            then.status(200)
                .header("content-type", "application/json")
                .body(r#"{"access_token":"token-abc","token_type":"Bearer"}"#);
        })
        .await;

    let http = reqwest::Client::new();
    let oauth = OAuthClientCredentials::new(server.url("/token"), "client-a", "secret-a")?
        .with_scope("scope-a");
    let token = oauth.fetch_token(&http).await?;
    mock.assert_async().await;

    assert_eq!(token.access_token, "token-abc");
    assert_eq!(token.token_type, "Bearer");
    Ok(())
}
