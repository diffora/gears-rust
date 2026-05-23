//! ECDHE key exchange on NIST P-256 and P-384.
//!
//! Both curves are implemented through Apple `SecKey`:
//! - **start**: `SecKey::new(GenerateKeyOptions { ec, 256|384 })` mints an
//!   ephemeral key pair; we export the public point in uncompressed X9.63
//!   form (`0x04 || X || Y`) for transmission.
//! - **complete**: the peer's bytes are imported via `SecKeyCreateWithData`
//!   (see [`crate::ffi::security`]) and `SecKey::key_exchange` is invoked
//!   with `EcdhKeyExchangeStandard`, which returns the raw shared X
//!   coordinate — exactly what TLS 1.2/1.3 ECDHE expects.

use rustls::crypto::{ActiveKeyExchange, SharedSecret, SupportedKxGroup};
use rustls::{Error, NamedGroup};
use security_framework::key::{Algorithm, GenerateKeyOptions, KeyType, SecKey};

use crate::ffi::security::{PublicKeyKind, import_public_key};

/// SECP256R1 (NIST P-256) key exchange.
#[derive(Debug)]
pub struct P256KxGroup;

/// SECP384R1 (NIST P-384) key exchange.
#[derive(Debug)]
pub struct P384KxGroup;

pub static SECP256R1: P256KxGroup = P256KxGroup;
pub static SECP384R1: P384KxGroup = P384KxGroup;

/// Curve discriminant — keeps every dispatch on a closed `match` so there
/// are no unreachable `_` arms (and therefore no unreachable error paths
/// inflating the coverage denominator).
#[derive(Copy, Clone)]
enum Curve {
    P256,
    P384,
}

impl Curve {
    const fn size_bits(self) -> u32 {
        match self {
            Self::P256 => 256,
            Self::P384 => 384,
        }
    }
    const fn coord_len(self) -> usize {
        match self {
            Self::P256 => 32,
            Self::P384 => 48,
        }
    }
    const fn group(self) -> NamedGroup {
        match self {
            Self::P256 => NamedGroup::secp256r1,
            Self::P384 => NamedGroup::secp384r1,
        }
    }
    const fn kind(self) -> PublicKeyKind {
        match self {
            Self::P256 => PublicKeyKind::EcSecPrimeRandomP256,
            Self::P384 => PublicKeyKind::EcSecPrimeRandomP384,
        }
    }
}

impl SupportedKxGroup for P256KxGroup {
    fn start(&self) -> Result<Box<dyn ActiveKeyExchange>, Error> {
        start_active(Curve::P256)
    }
    fn name(&self) -> NamedGroup {
        NamedGroup::secp256r1
    }
    fn fips(&self) -> bool {
        // Runtime witness — see [`crate::oe::fips_witness_ok`].
        crate::oe::fips_witness_ok()
    }
}

impl SupportedKxGroup for P384KxGroup {
    fn start(&self) -> Result<Box<dyn ActiveKeyExchange>, Error> {
        start_active(Curve::P384)
    }
    fn name(&self) -> NamedGroup {
        NamedGroup::secp384r1
    }
    fn fips(&self) -> bool {
        // Runtime witness — see [`crate::oe::fips_witness_ok`].
        crate::oe::fips_witness_ok()
    }
}

fn start_active(curve: Curve) -> Result<Box<dyn ActiveKeyExchange>, Error> {
    let mut opts = GenerateKeyOptions::default();
    opts.set_key_type(KeyType::ec());
    opts.set_size_in_bits(curve.size_bits());

    let private = SecKey::new(&opts).map_err(|e| {
        Error::General(format!(
            "cyberware-rustls-corecrypto-provider: SecKey::new failed for P-{}: domain={} code={}",
            curve.size_bits(),
            e.domain(),
            e.code()
        ))
    })?;

    // The X9.63 uncompressed form is `0x04 || X || Y`, length = 1 + 2 · coord.
    // Apple's `SecKey` always produces these on a freshly-minted EC key; the
    // `ok_or_else` branches below are defense-in-depth for hypothetical
    // future regressions — not reachable in normal operation.
    let public = private.public_key().ok_or_else(|| {
        Error::General(
            "cyberware-rustls-corecrypto-provider: generated SecKey has no associated public key"
                .to_owned(),
        )
    })?;
    let cf_data = public.external_representation().ok_or_else(|| {
        Error::General(
            "cyberware-rustls-corecrypto-provider: public_key.external_representation() returned None"
                .to_owned(),
        )
    })?;
    let public_bytes = cf_data.bytes().to_vec();
    debug_assert_eq!(
        public_bytes.len(),
        1 + 2 * curve.coord_len(),
        "Apple SecKey produced unexpected public-key length"
    );

    Ok(Box::new(ActiveEcdh {
        curve,
        private,
        public_bytes,
    }))
}

/// `Send + Sync` are inferred from the fields: `Curve` is a plain enum,
/// `Vec<u8>` is `Send + Sync`, and `security_framework::key::SecKey`
/// upstream carries its own `unsafe impl Send + Sync` (3.7's
/// [`key.rs:129-130`](https://docs.rs/security-framework/3.7.0/src/security_framework/key.rs.html#129-130)) —
/// `SecKey` is a `CFTypeRef` newtype, and Apple documents the methods
/// we call (`key_exchange_result`, `public_key`, `external_representation`)
/// as thread-safe read operations (see Apple's "Concurrency Programming
/// Guide" on Core Foundation thread-safety). No interior mutability is
/// introduced by this wrapper. We deliberately do *not* assert an
/// explicit `unsafe impl Send + Sync` here so the auto-trait answer
/// stays anchored in the upstream `SecKey` impl rather than being
/// re-declared and possibly drifting.
struct ActiveEcdh {
    curve: Curve,
    private: SecKey,
    public_bytes: Vec<u8>,
}

impl ActiveKeyExchange for ActiveEcdh {
    fn complete(self: Box<Self>, peer_pub_key: &[u8]) -> Result<SharedSecret, Error> {
        // Reject malformed peer keys quickly; SecKey would also reject but
        // with a less specific error.
        let expected = 1 + 2 * self.curve.coord_len();
        if peer_pub_key.len() != expected || peer_pub_key[0] != 0x04 {
            return Err(Error::General(format!(
                "cyberware-rustls-corecrypto-provider: invalid peer public key (len {}, expected {} with 0x04 prefix)",
                peer_pub_key.len(),
                expected,
            )));
        }

        let peer = import_public_key(peer_pub_key, self.curve.kind()).map_err(|e| {
            // Both variants land in `Error::General` with the inner detail
            // preserved — keeping them in a single closure keeps the error
            // mapping ungated by branching (Apple typically returns the
            // `CoreFoundation` variant for malformed input; `NullKey` is a
            // defensive belt that we do not deliberately exercise).
            Error::General(format!(
                "cyberware-rustls-corecrypto-provider: peer public-key import failed: {e}"
            ))
        })?;

        let shared = self
            .private
            .key_exchange(
                Algorithm::ECDHKeyExchangeStandard,
                &peer,
                self.curve.coord_len(),
                None,
            )
            .map_err(|e| {
                Error::General(format!(
                    "cyberware-rustls-corecrypto-provider: SecKey::key_exchange failed: domain={} code={}",
                    e.domain(),
                    e.code()
                ))
            })?;

        debug_assert_eq!(
            shared.len(),
            self.curve.coord_len(),
            "ECDH shared secret length mismatch"
        );

        Ok(SharedSecret::from(shared.as_slice()))
    }

    fn pub_key(&self) -> &[u8] {
        &self.public_bytes
    }

    fn group(&self) -> NamedGroup {
        self.curve.group()
    }
}

#[cfg(test)]
#[path = "kx_tests.rs"]
mod tests;
