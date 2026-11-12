#![allow(clippy::use_debug)]
#![allow(clippy::expect_used)] // example code, not production

//! Configuration reference: form-based auth, `private_key_jwt`, all config
//! fields, error handling.

use toolkit_auth::{
    ClientCredentials, OAuthClientConfig, PrivateKeyJwt, RsaJwsAlg, SecretString, TokenEndpoint,
    TokenError, TokenProvider,
};
use toolkit_http::HttpClientBuilder;

/// Form-based auth: some `IdPs` require credentials in the POST body
/// instead of a Basic header.
#[expect(
    dead_code,
    reason = "reference example; main calls it only when uncommented"
)]
async fn form_auth() -> Result<(), Box<dyn std::error::Error>> {
    use toolkit_auth::HttpClientBuilderExt;

    let token = TokenProvider::new(OAuthClientConfig::new(
        TokenEndpoint::Direct("https://idp.example.com/oauth/token".parse()?),
        "my-service",
        ClientCredentials::SecretPost(SecretString::new("my-secret")),
    ))?;

    let client = HttpClientBuilder::new().with_bearer_auth(token).build()?;

    let _resp = client.get("https://api.example.com/data").send().await?;

    println!("Used Form auth (client_id/client_secret in POST body)");
    Ok(())
}

/// `private_key_jwt`: the client signs a short-lived JWT with its private key
/// instead of sending a shared secret. The identity provider holds only the public key.
#[expect(
    dead_code,
    reason = "reference example; main calls it only when uncommented"
)]
async fn private_key_jwt() -> Result<(), Box<dyn std::error::Error>> {
    use toolkit_auth::HttpClientBuilderExt;

    // Unencrypted PKCS#8 PEM (`-----BEGIN PRIVATE KEY-----`). For P-256 use
    // `PrivateKeyJwt::es256_from_pem`.
    // `Zeroizing` wipes the PEM text when it goes out of scope.
    let pem =
        zeroize::Zeroizing::new(tokio::fs::read_to_string("/etc/my-service/client-key.pem").await?);
    let key = PrivateKeyJwt::rsa_from_pem(&pem, RsaJwsAlg::Ps256)?
        // `kid` registered with the IdP; optional.
        .with_kid("my-service-2026");
    // `aud` defaults to the token endpoint. Some IdPs want their issuer:
    // `.with_audience("https://idp.example.com")`.

    let token = TokenProvider::new(OAuthClientConfig::new(
        TokenEndpoint::Issuer("https://idp.example.com".parse()?),
        "my-service",
        ClientCredentials::PrivateKeyJwt(key),
    ))?;

    let client = HttpClientBuilder::new().with_bearer_auth(token).build()?;
    let _resp = client.get("https://api.example.com/data").send().await?;
    Ok(())
}

/// All config fields, with the defaults `OAuthClientConfig::new` uses.
fn config_overview() {
    println!("\n=== Configuration reference ===");

    let config = OAuthClientConfig {
        // Endpoint: Direct(token endpoint URL) or Issuer(OIDC issuer URL,
        // token endpoint discovered on first use).
        endpoint: TokenEndpoint::Direct(
            "https://idp.example.com/oauth/token"
                .parse()
                .expect("valid URL"),
        ),

        // Credentials:
        client_id: "my-service".into(),
        // SecretBasic (Authorization header), SecretPost (form body) or
        // PrivateKeyJwt (signed assertion, no shared secret):
        credentials: ClientCredentials::SecretBasic(SecretString::new("my-secret")),
        scopes: vec!["api.read".into(), "api.write".into()],

        // Vendor-specific headers (e.g. Azure requires a resource header):
        extra_headers: vec![("x-vendor-id".into(), "acme-corp".into())],

        // Refresh policy (defaults shown):
        refresh_offset: std::time::Duration::from_mins(30),
        jitter_max: std::time::Duration::from_mins(5),
        min_refresh_period: std::time::Duration::from_secs(10),
        default_ttl: std::time::Duration::from_mins(5),

        // HTTP client override (None = use defaults):
        http_config: None,
    };

    // Debug output redacts secrets:
    println!("  {config:?}");

    // Validate before use:
    match config.validate() {
        Ok(()) => println!("  Config is valid"),
        Err(e) => println!("  Config error: {e}"),
    }
}

/// Error handling patterns.
async fn error_handling() {
    println!("\n=== Error handling ===");

    let result = TokenProvider::new(OAuthClientConfig::new(
        TokenEndpoint::Direct(
            "https://unreachable.example.com/token"
                .parse()
                .expect("valid URL"),
        ),
        "my-service",
        ClientCredentials::SecretBasic(SecretString::new("my-secret")),
    ));

    let result = match result {
        Ok(provider) => provider.get().await,
        Err(error) => Err(error),
    };
    match result {
        Ok(_) => println!("  Token acquired"),
        Err(TokenError::Http(msg)) => println!("  HTTP error: {msg}"),
        Err(TokenError::InvalidResponse(msg)) => {
            println!("  Bad response: {msg}");
        }
        Err(TokenError::ConfigError(msg)) => {
            println!("  Config error: {msg}");
        }
        Err(e) => println!("  Other error: {e}"),
    }
}

/// Demonstrate provider construction and authenticated requests to an external service.
#[tokio::main]
async fn main() {
    config_overview();
    error_handling().await;

    // Requires a real IDP -- uncomment to test:
    // form_auth().await.unwrap();
    // private_key_jwt().await.unwrap();
}
