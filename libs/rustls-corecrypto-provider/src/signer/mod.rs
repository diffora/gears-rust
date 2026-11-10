//! Server-side private-key signing routed through Apple corecrypto.
//!
//! Implements [`rustls::crypto::KeyProvider`] over `SecKey` so that
//! [`rustls::ServerConfig`] can use this provider for server-side TLS and
//! for client-cert (mTLS) authentication. The crate previously rejected
//! every `load_private_key` call to enforce a "client-side only" scope —
//! that constraint is lifted by ADR 0004 (supersedes lines 49–52 of
//! [ADR 0001](../../../../docs/security/fips/adrs/0001-macos-fips-via-corecrypto-provider.md)).
//!
//! Dispatch in `any_supported_type` mirrors [`rustls-cng-crypto`'s `signer/mod.rs`
//! ](https://docs.rs/rustls-cng-crypto/0.1.2/rustls_cng_crypto/) so the two
//! providers expose the same API surface — try RSA first, then EC, error
//! on anything else.

use std::sync::Arc;

use rustls::Error;
use rustls::crypto::KeyProvider;
use rustls::pki_types::PrivateKeyDer;
use rustls::sign::SigningKey;
use zeroize::Zeroizing;

mod ec;
mod rsa;

pub(crate) use rsa::RSA_SCHEMES;

/// `KeyProvider` exposed by [`crate::default_provider`].
///
/// `fips()` consults the runtime witness in [`crate::oe::fips_witness_ok`]:
/// `true` when the running macOS major is inside the active Apple
/// corecrypto CMVP cert's Operational Environment (or when the
/// `CF_GEARS_FIPS_OE_OVERRIDE` env-var is set for CI), `false`
/// otherwise. This propagates honestly to `ClientConfig::fips()` /
/// `ServerConfig::fips()` rather than asserting a claim by intent —
/// mirrors `rustls-cng-crypto`'s `fips::enabled()` pattern on Windows
/// (which similarly consults the OS FIPS-mode flag at every call).
#[derive(Debug, Default)]
pub struct CoreCryptoKeyProvider;

impl KeyProvider for CoreCryptoKeyProvider {
    fn load_private_key(
        &self,
        key_der: PrivateKeyDer<'static>,
    ) -> Result<Arc<dyn SigningKey>, Error> {
        // The caller hands over its only copy of the private key DER; wipe it
        // when loading is done, as rustls's own aws-lc provider does.
        any_supported_type(&Zeroizing::new(key_der))
    }

    fn fips(&self) -> bool {
        // Runtime witness — see [`crate::oe::fips_witness_ok`]. Returns
        // `false` if the running macOS is outside the active Apple
        // corecrypto CMVP cert OE; downstream `ServerConfig::fips()` then
        // reflects that fact (rather than asserting a claim by intent).
        crate::oe::fips_witness_ok()
    }
}

fn any_supported_type(der: &PrivateKeyDer<'_>) -> Result<Arc<dyn SigningKey>, Error> {
    if let Ok(key) = rsa::RsaSigningKey::new(der) {
        return Ok(Arc::new(key));
    }
    if let Ok(key) = ec::EcSigningKey::new(der) {
        return Ok(Arc::new(key));
    }
    Err(Error::General(
        "cf-gears-rustls-corecrypto-provider: unsupported private-key type \
         (expected RSA or NIST P-256/P-384/P-521 in PKCS#1, PKCS#8 or SEC1 DER)"
            .to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::pki_types::PrivatePkcs8KeyDer;

    /// FIPS-claim contract: KeyProvider must advertise FIPS so that rustls's
    /// `ServerConfig::fips()` invariant remains true when our provider is
    /// in use. Returning `false` would silently disable rustls's FIPS
    /// assertions in downstream `tls.rs`.
    #[test]
    fn advertises_fips_to_rustls() {
        assert!(CoreCryptoKeyProvider.fips());
    }

    /// Garbage input must be rejected with the documented marker text —
    /// the dispatcher tries RSA first then EC, both fail on random bytes.
    #[test]
    fn rejects_garbage_key_bytes() {
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(vec![0u8; 16]));
        let err = CoreCryptoKeyProvider
            .load_private_key(key)
            .expect_err("garbage must fail");
        match err {
            Error::General(msg) => assert!(
                msg.contains("unsupported private-key type"),
                "error must explain unsupported key type, got {msg:?}"
            ),
            other => panic!("expected Error::General, got {other:?}"),
        }
    }
}
