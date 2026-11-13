use std::fmt;
use std::time::Duration;
use url::Url;

use super::credentials::ClientCredentials;
use super::error::TokenError;

/// Where the token endpoint comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenEndpoint {
    /// Token endpoint URL, used as is.
    Direct(Url),
    /// OIDC issuer. The token endpoint is resolved on first use from
    /// `{issuer}/.well-known/openid-configuration`.
    Issuer(Url),
}

/// Configuration for an outbound `OAuth2` client credentials flow.
///
/// Built with [`new`](Self::new) from the required parts; optional fields are
/// set with struct update syntax:
///
/// ```
/// use toolkit_auth::{ClientCredentials, OAuthClientConfig, SecretString, TokenEndpoint};
///
/// let config = OAuthClientConfig {
///     scopes: vec!["api.read".into()],
///     ..OAuthClientConfig::new(
///         TokenEndpoint::Direct("https://idp.example.com/oauth/token".parse().unwrap()),
///         "my-service",
///         ClientCredentials::SecretBasic(SecretString::new("my-secret")),
///     )
/// };
/// assert!(config.validate().is_ok());
/// ```
///
/// `Debug` is manually implemented; credentials and extra header values are
/// redacted.
pub struct OAuthClientConfig {
    // ---- endpoint resolution ------------------------------------------------
    /// Token endpoint, given directly or discovered from an OIDC issuer.
    pub endpoint: TokenEndpoint,

    // ---- credentials --------------------------------------------------------
    /// `OAuth2` client identifier.
    pub client_id: String,

    /// Requested scopes (normalized once, stable order).
    pub scopes: Vec<String>,

    /// Client authentication method and its credential (secret or signing
    /// key). Redacted in `Debug` output.
    pub credentials: ClientCredentials,

    /// Extra headers attached to every token request (vendor quirks).
    pub extra_headers: Vec<(String, String)>,

    // ---- refresh policy -----------------------------------------------------
    /// How far before expiry the token should be refreshed (default: 30 min).
    pub refresh_offset: Duration,

    /// Maximum random jitter added to the refresh offset (default: 5 min).
    pub jitter_max: Duration,

    /// Minimum period between consecutive refresh attempts (default: 10 s).
    pub min_refresh_period: Duration,

    /// Fallback TTL when the token endpoint omits `expires_in` (default: 5 min).
    pub default_ttl: Duration,

    // ---- HTTP client --------------------------------------------------------
    /// Override for the internal HTTP client configuration.
    /// When `None`,
    /// [`HttpClientConfig::token_endpoint()`](toolkit_http::HttpClientConfig::token_endpoint)
    /// is used.
    ///
    /// With [`ClientCredentials::PrivateKeyJwt`] the HTTP retry policy is
    /// cleared: a retry would resend a single-use assertion. Failed requests
    /// are retried by the token watcher, each with a new assertion.
    pub http_config: Option<toolkit_http::HttpClientConfig>,
}

impl OAuthClientConfig {
    /// Configuration with the required parts and default policy: no scopes,
    /// no extra headers, refresh 30 min before expiry with up to 5 min of
    /// jitter, at most one refresh per 10 s, 5 min TTL when the server omits
    /// `expires_in`, and the default token-endpoint HTTP client.
    #[must_use]
    pub fn new(
        endpoint: TokenEndpoint,
        client_id: impl Into<String>,
        credentials: ClientCredentials,
    ) -> Self {
        Self {
            endpoint,
            client_id: client_id.into(),
            credentials,
            scopes: Vec::new(),
            extra_headers: Vec::new(),
            refresh_offset: Duration::from_mins(30),
            jitter_max: Duration::from_mins(5),
            min_refresh_period: Duration::from_secs(10),
            default_ttl: Duration::from_mins(5),
            http_config: None,
        }
    }

    /// Validate the values the types cannot rule out.
    ///
    /// # Errors
    ///
    /// Returns [`TokenError::ConfigError`] if `client_id` is empty or a
    /// `SecretBasic` / `SecretPost` secret is empty.
    pub fn validate(&self) -> Result<(), TokenError> {
        if self.client_id.trim().is_empty() {
            return Err(TokenError::ConfigError(
                "client_id must not be empty".into(),
            ));
        }
        if let ClientCredentials::SecretBasic(secret) | ClientCredentials::SecretPost(secret) =
            &self.credentials
            && secret.expose().is_empty()
        {
            return Err(TokenError::ConfigError(
                "client_secret must not be empty".into(),
            ));
        }
        Ok(())
    }
}

impl Clone for OAuthClientConfig {
    fn clone(&self) -> Self {
        Self {
            endpoint: self.endpoint.clone(),
            client_id: self.client_id.clone(),
            credentials: self.credentials.clone(),
            scopes: self.scopes.clone(),
            extra_headers: self.extra_headers.clone(),
            refresh_offset: self.refresh_offset,
            jitter_max: self.jitter_max,
            min_refresh_period: self.min_refresh_period,
            default_ttl: self.default_ttl,
            http_config: self.http_config.clone(),
        }
    }
}

/// `Debug` redacts credentials and header values to prevent accidental
/// exposure in logs.
impl fmt::Debug for OAuthClientConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let redacted_headers: Vec<_> = self
            .extra_headers
            .iter()
            .map(|(k, _)| (k.as_str(), "[REDACTED]"))
            .collect();
        f.debug_struct("OAuthClientConfig")
            .field("endpoint", &self.endpoint)
            .field("client_id", &self.client_id)
            .field("credentials", &self.credentials)
            .field("scopes", &self.scopes)
            .field("extra_headers", &redacted_headers)
            .field("refresh_offset", &self.refresh_offset)
            .field("jitter_max", &self.jitter_max)
            .field("min_refresh_period", &self.min_refresh_period)
            .field("default_ttl", &self.default_ttl)
            .field("http_config", &self.http_config)
            .finish()
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::oauth2::types::SecretString;

    fn endpoint() -> TokenEndpoint {
        TokenEndpoint::Direct(Url::parse("https://auth.example.com/token").unwrap())
    }

    fn secret(value: &str) -> ClientCredentials {
        ClientCredentials::SecretBasic(SecretString::new(value))
    }

    // ---- validate -----------------------------------------------------------

    #[test]
    fn validate_ok_with_either_endpoint_kind() {
        for endpoint in [
            endpoint(),
            TokenEndpoint::Issuer(Url::parse("https://auth.example.com").unwrap()),
        ] {
            OAuthClientConfig::new(endpoint, "my-client", secret("my-secret"))
                .validate()
                .unwrap();
        }
    }

    #[test]
    fn validate_err_when_client_id_empty_or_whitespace() {
        for client_id in ["", "   "] {
            let err = OAuthClientConfig::new(endpoint(), client_id, secret("my-secret"))
                .validate()
                .unwrap_err();
            assert!(
                err.to_string().contains("client_id"),
                "unexpected error: {err}"
            );
        }
    }

    #[test]
    fn validate_err_when_secret_empty_for_basic_and_post() {
        for credentials in [
            secret(""),
            ClientCredentials::SecretPost(SecretString::new("")),
        ] {
            let err = OAuthClientConfig::new(endpoint(), "my-client", credentials)
                .validate()
                .unwrap_err();
            assert!(
                err.to_string().contains("client_secret"),
                "unexpected error: {err}"
            );
        }
    }

    #[test]
    fn validate_ok_with_private_key_jwt_and_no_secret() {
        let key = super::super::PrivateKeyJwt::rsa_from_pem(
            super::super::credentials::tests::RSA_PEM,
            super::super::RsaJwsAlg::Rs256,
        )
        .unwrap();
        let cfg = OAuthClientConfig::new(
            endpoint(),
            "my-client",
            ClientCredentials::PrivateKeyJwt(key),
        );
        cfg.validate().unwrap();
        assert!(matches!(
            &cfg.credentials,
            ClientCredentials::PrivateKeyJwt(key) if key.algorithm() == jsonwebtoken::Algorithm::RS256
        ));
    }

    // ---- Debug redaction ----------------------------------------------------

    #[test]
    fn debug_redacts_client_secret() {
        let cfg = OAuthClientConfig::new(endpoint(), "my-client", secret("super-secret"));
        let dbg = format!("{cfg:?}");
        assert!(dbg.contains("[REDACTED]"), "Debug must contain [REDACTED]");
        assert!(
            !dbg.contains("super-secret"),
            "Debug must not contain the raw secret"
        );
        assert!(dbg.contains("my-client"), "Debug should contain client_id");
    }

    #[test]
    fn debug_redacts_extra_header_values() {
        let cfg = OAuthClientConfig {
            extra_headers: vec![("x-api-key".into(), "secret-api-key-value".into())],
            ..OAuthClientConfig::new(endpoint(), "my-client", secret("s"))
        };
        let dbg = format!("{cfg:?}");
        assert!(
            dbg.contains("x-api-key"),
            "Debug should contain header name"
        );
        assert!(
            !dbg.contains("secret-api-key-value"),
            "Debug must not contain header value"
        );
    }

    // ---- new ----------------------------------------------------------------

    #[test]
    fn new_uses_default_policy() {
        let cfg = OAuthClientConfig::new(endpoint(), "my-client", secret("s"));
        assert_eq!(cfg.endpoint, endpoint());
        assert_eq!(cfg.client_id, "my-client");
        assert!(cfg.scopes.is_empty());
        assert!(cfg.extra_headers.is_empty());
        assert_eq!(cfg.refresh_offset, Duration::from_mins(30));
        assert_eq!(cfg.jitter_max, Duration::from_mins(5));
        assert_eq!(cfg.min_refresh_period, Duration::from_secs(10));
        assert_eq!(cfg.default_ttl, Duration::from_mins(5));
        assert!(cfg.http_config.is_none());
    }
}
