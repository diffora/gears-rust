//! Direct token endpoint -- the simplest `OAuth2` client-credentials flow.
//!
//! This is the Rust equivalent of the Go pattern:
//!
//! ```go
//! tokenProvider, _ := tokenprovider.New(tokenURL, clientID, clientSecret, ...)
//! resolverClient, _ := tenantsresolver.NewClient(address, tokenProvider, ...)
//! ```
//!
//! NOTE: Requires a running IDP. Meant as an API reference, not a runnable demo.

use toolkit_auth::{
    ClientCredentials, HttpClientBuilderExt, OAuthClientConfig, SecretString, TokenEndpoint,
    TokenProvider,
};
use toolkit_http::HttpClientBuilder;

/// Demonstrate provider construction and authenticated requests to an external service.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Step 1 -- create token provider (like tokenprovider.New in Go)
    let token = TokenProvider::new(OAuthClientConfig {
        scopes: vec!["tenants.read".into(), "tenants.write".into()],
        ..OAuthClientConfig::new(
            TokenEndpoint::Direct("https://idp.example.com/oauth/token".parse()?),
            "my-service",
            ClientCredentials::SecretBasic(SecretString::new("my-secret")),
        )
    })?;

    // Step 2 -- create HTTP client with automatic Bearer injection
    // (like tenantsresolver.NewClient(addr, tokenProvider, ...) in Go)
    let client = HttpClientBuilder::new().with_bearer_auth(token).build()?;

    // Step 3 -- every request gets Authorization: Bearer <token> automatically
    let _resp = client
        .get("https://resolver.example.com/api/v1/tenants")
        .send()
        .await?;

    println!("Request sent with auto-injected Bearer token");
    Ok(())
}
