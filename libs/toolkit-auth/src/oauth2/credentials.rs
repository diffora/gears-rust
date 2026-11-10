//! Client authentication at the token endpoint: a shared secret
//! (RFC 6749 §2.3.1) or a signed JWT assertion (`private_key_jwt`,
//! RFC 7523 §2.2, OIDC Core §9).

use std::fmt;
use std::time::Duration;

use jsonwebtoken::{Algorithm, Header};
use serde::Serialize;
use time::OffsetDateTime;
use toolkit_security::crypto::signature::{EcdsaP256KeyPair, RsaKeyPair};
use toolkit_utils::SecretString;
use url::Url;

use super::error::TokenError;

/// `client_assertion_type` for a JWT client assertion (RFC 7523 §2.2).
pub(crate) const JWT_BEARER_ASSERTION_TYPE: &str =
    "urn:ietf:params:oauth:client-assertion-type:jwt-bearer";

/// Assertion lifetime. Assertions are single-use and minted per request.
const ASSERTION_LIFETIME: Duration = Duration::from_secs(60);

/// How the client authenticates to the token endpoint, with the credential
/// that method needs.
///
/// `Debug` never prints secrets or key material.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum ClientCredentials {
    /// `client_secret_basic`: `Authorization: Basic base64(client_id:client_secret)`.
    SecretBasic(SecretString),
    /// `client_secret_post`: `client_id` and `client_secret` as form fields.
    SecretPost(SecretString),
    /// `private_key_jwt`: a JWT signed with the client's private key, sent as
    /// `client_assertion`. No shared secret goes over the wire.
    PrivateKeyJwt(PrivateKeyJwt),
}

/// RSA signature algorithm for a `private_key_jwt` assertion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RsaJwsAlg {
    /// RSASSA-PKCS1-v1_5 with SHA-256.
    Rs256,
    /// RSASSA-PSS with SHA-256.
    Ps256,
}

/// Signing key and assertion options for `private_key_jwt`.
///
/// Keys are parsed and checked by the crypto facade at construction, so a
/// value of this type always holds a usable key. Signing runs in the
/// platform's FIPS module when `toolkit-security/fips` is enabled.
///
/// The assertion carries `iss` = `sub` = `client_id`, `aud`, a random `jti`,
/// `iat` and `exp` = `iat` + 60 s. `aud` defaults to the resolved token
/// endpoint (the URL the request is sent to); override it with
/// [`with_audience`](Self::with_audience) for identity providers that expect their issuer.
#[derive(Clone)]
pub struct PrivateKeyJwt {
    key: AssertionKey,
    kid: Option<String>,
    audience: Option<String>,
}

#[derive(Clone)]
enum AssertionKey {
    Es256(EcdsaP256KeyPair),
    Rsa(RsaKeyPair, RsaJwsAlg),
}

impl PrivateKeyJwt {
    /// Sign assertions with `ES256` using a P-256 key.
    #[must_use]
    pub fn es256(key: EcdsaP256KeyPair) -> Self {
        Self::new(AssertionKey::Es256(key))
    }

    /// Sign assertions with `RS256` or `PS256` using an RSA key (2048–8192 bits).
    #[must_use]
    pub fn rsa(key: RsaKeyPair, alg: RsaJwsAlg) -> Self {
        Self::new(AssertionKey::Rsa(key, alg))
    }

    /// [`es256`](Self::es256) with a P-256 key from unencrypted PKCS#8 PEM
    /// (`-----BEGIN PRIVATE KEY-----`).
    ///
    /// # Errors
    /// [`TokenError::ConfigError`] if the PEM is not a usable P-256 PKCS#8 key.
    pub fn es256_from_pem(pem: &str) -> Result<Self, TokenError> {
        EcdsaP256KeyPair::from_pkcs8_pem(pem)
            .map(Self::es256)
            .map_err(|e| key_error("P-256", e))
    }

    /// [`rsa`](Self::rsa) with an RSA key from unencrypted PKCS#8 PEM
    /// (`-----BEGIN PRIVATE KEY-----`).
    ///
    /// # Errors
    /// [`TokenError::ConfigError`] if the PEM is not a usable RSA PKCS#8 key.
    pub fn rsa_from_pem(pem: &str, alg: RsaJwsAlg) -> Result<Self, TokenError> {
        RsaKeyPair::from_pkcs8_pem(pem)
            .map(|key| Self::rsa(key, alg))
            .map_err(|e| key_error("RSA", e))
    }

    /// Set the `kid` header so the identity provider can select the registered public key.
    #[must_use]
    pub fn with_kid(mut self, kid: impl Into<String>) -> Self {
        self.kid = Some(kid.into());
        self
    }

    /// Use this `aud` instead of the resolved token endpoint.
    #[must_use]
    pub fn with_audience(mut self, audience: impl Into<String>) -> Self {
        self.audience = Some(audience.into());
        self
    }

    /// JWS algorithm of the assertions this key produces.
    pub(crate) fn algorithm(&self) -> Algorithm {
        match self.key {
            AssertionKey::Es256(_) => Algorithm::ES256,
            AssertionKey::Rsa(_, RsaJwsAlg::Rs256) => Algorithm::RS256,
            AssertionKey::Rsa(_, RsaJwsAlg::Ps256) => Algorithm::PS256,
        }
    }

    fn new(key: AssertionKey) -> Self {
        Self {
            key,
            kid: None,
            audience: None,
        }
    }

    /// Build and sign a fresh assertion for one token request.
    pub(crate) fn client_assertion(
        &self,
        client_id: &str,
        token_endpoint: &Url,
        now: OffsetDateTime,
    ) -> Result<String, TokenError> {
        let iat = now.unix_timestamp();
        let exp = iat.saturating_add_unsigned(ASSERTION_LIFETIME.as_secs());
        let claims = AssertionClaims {
            iss: client_id,
            sub: client_id,
            aud: self.audience.as_deref().unwrap_or(token_endpoint.as_str()),
            jti: new_jti()?,
            iat,
            exp,
        };
        let mut header = Header::new(self.algorithm());
        header.kid.clone_from(&self.kid);
        let signed = match &self.key {
            AssertionKey::Es256(key) => toolkit_security::jwt::encode(&header, &claims, key),
            AssertionKey::Rsa(key, _) => toolkit_security::jwt::encode(&header, &claims, key),
        };
        signed.map_err(|e| TokenError::ClientAssertion(e.to_string()))
    }
}

/// `Debug` shows the algorithm and options; key material is never printed.
impl fmt::Debug for PrivateKeyJwt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PrivateKeyJwt")
            .field("alg", &self.algorithm())
            .field("kid", &self.kid)
            .field("audience", &self.audience)
            .finish_non_exhaustive()
    }
}

#[derive(Serialize)]
struct AssertionClaims<'a> {
    iss: &'a str,
    sub: &'a str,
    aud: &'a str,
    jti: String,
    iat: i64,
    exp: i64,
}

/// Random (version 4) UUID from the crypto facade's CSPRNG.
fn new_jti() -> Result<String, TokenError> {
    let mut bytes = [0u8; 16];
    toolkit_security::crypto::rng::fill(&mut bytes)
        .map_err(|e| TokenError::ClientAssertion(e.to_string()))?;
    Ok(uuid::Builder::from_random_bytes(bytes)
        .into_uuid()
        .to_string())
}

fn key_error(kind: &str, error: toolkit_security::crypto::CryptoError) -> TokenError {
    use toolkit_security::crypto::CryptoError;
    TokenError::ConfigError(match error {
        CryptoError::InvalidKey => format!("private_key_jwt: invalid {kind} PKCS#8 key"),
        // Backend or policy problem (e.g. no FIPS witness); the key may be fine.
        other => format!("private_key_jwt: cannot load {kind} key: {other}"),
    })
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) mod tests {
    use super::*;
    use jsonwebtoken::{DecodingKey, Validation};
    use serde_json::Value;

    pub const RSA_PEM: &str = include_str!("../../tests/fixtures/rsa-private.pem");
    const ENDPOINT: &str = "https://idp.example.com/oauth2/token";

    /// Signing key plus the matching public key for verification.
    pub fn key_and_verifier(alg: Algorithm) -> (PrivateKeyJwt, DecodingKey) {
        match alg {
            Algorithm::ES256 => {
                let pair = EcdsaP256KeyPair::generate().unwrap();
                let verifier = DecodingKey::from_ec_der(pair.public_key());
                (PrivateKeyJwt::es256(pair), verifier)
            }
            Algorithm::RS256 | Algorithm::PS256 => {
                let pair = RsaKeyPair::from_pkcs8_pem(RSA_PEM).unwrap();
                let verifier =
                    DecodingKey::from_rsa_raw_components(pair.modulus(), pair.public_exponent());
                let rsa_alg = if alg == Algorithm::RS256 {
                    RsaJwsAlg::Rs256
                } else {
                    RsaJwsAlg::Ps256
                };
                (PrivateKeyJwt::rsa(pair, rsa_alg), verifier)
            }
            other => panic!("unsupported test algorithm {other:?}"),
        }
    }

    /// Verify the signature and the RFC 7523 claim set; return the claims.
    pub fn verify(
        assertion: &str,
        verifier: &DecodingKey,
        alg: Algorithm,
        client_id: &str,
        aud: &str,
    ) -> (jsonwebtoken::Header, Value) {
        let mut validation = Validation::new(alg);
        validation.set_issuer(&[client_id]);
        validation.set_audience(&[aud]);
        validation.sub = Some(client_id.to_owned());
        validation.set_required_spec_claims(&["iss", "sub", "aud", "exp", "iat"]);
        let data =
            toolkit_security::jwt::decode::<Value>(assertion, verifier, &validation).unwrap();
        (data.header, data.claims)
    }

    fn url() -> Url {
        Url::parse(ENDPOINT).unwrap()
    }

    #[test]
    fn assertion_claims_and_header_for_every_algorithm() {
        for alg in [Algorithm::ES256, Algorithm::RS256, Algorithm::PS256] {
            let (key, verifier) = key_and_verifier(alg);
            let key = key.with_kid("kid-1");
            let now = OffsetDateTime::now_utc();
            let assertion = key.client_assertion("svc", &url(), now).unwrap();
            let (header, claims) = verify(&assertion, &verifier, alg, "svc", ENDPOINT);
            assert_eq!(header.alg, alg);
            assert_eq!(header.kid.as_deref(), Some("kid-1"));
            assert_eq!(header.typ.as_deref(), Some("JWT"));
            assert_eq!(claims["iss"], "svc");
            assert_eq!(claims["sub"], "svc");
            // A single string, not an array (audience-injection hardening).
            assert_eq!(claims["aud"], ENDPOINT);
            assert_eq!(claims["iat"], now.unix_timestamp());
            assert_eq!(claims["exp"], now.unix_timestamp() + 60);
            let jti = uuid::Uuid::parse_str(claims["jti"].as_str().unwrap()).unwrap();
            assert_eq!(jti.get_version_num(), 4);
            assert_eq!(key.algorithm(), alg);
        }
    }

    #[test]
    fn audience_override_is_used_verbatim() {
        let (key, verifier) = key_and_verifier(Algorithm::ES256);
        let key = key.with_audience("https://idp.example.com");
        let assertion = key
            .client_assertion("svc", &url(), OffsetDateTime::now_utc())
            .unwrap();
        let (header, claims) = verify(
            &assertion,
            &verifier,
            Algorithm::ES256,
            "svc",
            "https://idp.example.com",
        );
        assert_eq!(claims["aud"], "https://idp.example.com");
        assert!(header.kid.is_none());
    }

    #[test]
    fn jti_is_unique_per_assertion() {
        let (key, verifier) = key_and_verifier(Algorithm::ES256);
        let now = OffsetDateTime::now_utc();
        let jti = |assertion: String| {
            verify(&assertion, &verifier, Algorithm::ES256, "svc", ENDPOINT).1["jti"].clone()
        };
        let first = jti(key.client_assertion("svc", &url(), now).unwrap());
        let second = jti(key.client_assertion("svc", &url(), now).unwrap());
        assert_ne!(first, second);
    }

    #[test]
    fn pem_constructors_accept_matching_keys_and_reject_others() {
        let rsa = PrivateKeyJwt::rsa_from_pem(RSA_PEM, RsaJwsAlg::Ps256).unwrap();
        assert_eq!(rsa.algorithm(), Algorithm::PS256);
        let body: String = RSA_PEM.lines().filter(|l| !l.starts_with('-')).collect();
        for err in [
            PrivateKeyJwt::es256_from_pem(RSA_PEM).unwrap_err(),
            PrivateKeyJwt::rsa_from_pem("not a pem", RsaJwsAlg::Rs256).unwrap_err(),
            PrivateKeyJwt::rsa_from_pem(
                &RSA_PEM.replace("PRIVATE KEY", "RSA PRIVATE KEY"),
                RsaJwsAlg::Rs256,
            )
            .unwrap_err(),
        ] {
            assert!(matches!(err, TokenError::ConfigError(_)), "{err:?}");
            let rendered = format!("{err} {err:?}");
            assert!(rendered.contains("private_key_jwt"), "{rendered}");
            assert!(!rendered.contains(&body[..32]), "{rendered}");
        }
    }

    #[test]
    fn debug_never_prints_key_material_or_secrets() {
        let key = PrivateKeyJwt::rsa_from_pem(RSA_PEM, RsaJwsAlg::Rs256)
            .unwrap()
            .with_kid("kid-1");
        let pair = RsaKeyPair::from_pkcs8_pem(RSA_PEM).unwrap();
        let rendered = format!("{:?}", ClientCredentials::PrivateKeyJwt(key));
        assert!(rendered.contains("RS256"), "{rendered}");
        assert!(rendered.contains("kid-1"), "{rendered}");
        // Neither PEM body nor modulus bytes leak.
        let body: String = RSA_PEM.lines().filter(|l| !l.starts_with('-')).collect();
        for chunk in body.as_bytes().chunks(16) {
            assert!(!rendered.contains(std::str::from_utf8(chunk).unwrap()));
        }
        assert!(!rendered.contains(&format!("{:?}", &pair.modulus()[..8])[1..20]));
        let secret = format!(
            "{:?}",
            ClientCredentials::SecretPost(SecretString::new("hunter2"))
        );
        // No message argument: printing `secret` on failure would log it.
        assert!(!secret.contains("hunter2"));
    }
}
