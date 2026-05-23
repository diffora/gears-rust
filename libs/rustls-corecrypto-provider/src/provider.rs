//! Final assembly of the [`rustls::crypto::CryptoProvider`].
//!
//! Combines:
//! - 2 TLS 1.3 cipher suites (AES-128/256-GCM with SHA-256/384)
//! - 4 TLS 1.2 GCM cipher suites (ECDHE_ECDSA / ECDHE_RSA × AES-128/256)
//! - 2 key exchange groups (P-256, P-384)
//! - 9 signature verification algorithms (ECDSA P-256/384/521 + RSA-PSS + RSA-PKCS#1)
//! - Apple `SecRandom` as `SecureRandom`
//! - `KeyProvider` for server-side TLS / mTLS (RSA + ECDSA P-256/384/521;
//!   see [`crate::signer`] and ADR 0004).
//!
//! All operations route through Apple corecrypto (FIPS-validated module).

use std::sync::{Arc, OnceLock};

use rustls::crypto::CipherSuiteCommon;
use rustls::crypto::CryptoProvider;
use rustls::crypto::KeyExchangeAlgorithm;
use rustls::{
    CipherSuite, SignatureScheme, SupportedCipherSuite, Tls12CipherSuite, Tls13CipherSuite,
};

use crate::hash::{SHA256, SHA384};
use crate::hkdf::{HKDF_SHA256, HKDF_SHA384};
use crate::kx::{SECP256R1, SECP384R1};
use crate::random::CoreCryptoRandom;
use crate::signer::{CoreCryptoKeyProvider, RSA_SCHEMES};
use crate::tls12;
use crate::tls13;
use crate::verify::SUPPORTED_SIG_ALGS;

// =========================================================================
// TLS 1.3 cipher suites
// =========================================================================

pub static TLS13_AES_128_GCM_SHA256: SupportedCipherSuite =
    SupportedCipherSuite::Tls13(&Tls13CipherSuite {
        common: CipherSuiteCommon {
            suite: CipherSuite::TLS13_AES_128_GCM_SHA256,
            hash_provider: &SHA256,
            // RFC 8446 §5.5 / TLS WG guidance for AES-GCM: limit to 2^23.5
            // records before rekey. We use the conservative 2^23.
            confidentiality_limit: 1 << 23,
        },
        hkdf_provider: &HKDF_SHA256,
        aead_alg: &tls13::AES_128_GCM,
        quic: None,
    });

pub static TLS13_AES_256_GCM_SHA384: SupportedCipherSuite =
    SupportedCipherSuite::Tls13(&Tls13CipherSuite {
        common: CipherSuiteCommon {
            suite: CipherSuite::TLS13_AES_256_GCM_SHA384,
            hash_provider: &SHA384,
            confidentiality_limit: 1 << 23,
        },
        hkdf_provider: &HKDF_SHA384,
        aead_alg: &tls13::AES_256_GCM,
        quic: None,
    });

// =========================================================================
// TLS 1.2 cipher suites
// =========================================================================

pub static TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256: SupportedCipherSuite =
    SupportedCipherSuite::Tls12(&Tls12CipherSuite {
        common: CipherSuiteCommon {
            suite: CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
            hash_provider: &SHA256,
            confidentiality_limit: 1 << 23,
        },
        kx: KeyExchangeAlgorithm::ECDHE,
        sign: ECDSA_SIG_SCHEMES,
        aead_alg: &tls12::AES_128_GCM,
        prf_provider: &tls12::PRF_SHA256,
    });

pub static TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384: SupportedCipherSuite =
    SupportedCipherSuite::Tls12(&Tls12CipherSuite {
        common: CipherSuiteCommon {
            suite: CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
            hash_provider: &SHA384,
            confidentiality_limit: 1 << 23,
        },
        kx: KeyExchangeAlgorithm::ECDHE,
        sign: ECDSA_SIG_SCHEMES,
        aead_alg: &tls12::AES_256_GCM,
        prf_provider: &tls12::PRF_SHA384,
    });

pub static TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256: SupportedCipherSuite =
    SupportedCipherSuite::Tls12(&Tls12CipherSuite {
        common: CipherSuiteCommon {
            suite: CipherSuite::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256,
            hash_provider: &SHA256,
            confidentiality_limit: 1 << 23,
        },
        kx: KeyExchangeAlgorithm::ECDHE,
        sign: RSA_SCHEMES,
        aead_alg: &tls12::AES_128_GCM,
        prf_provider: &tls12::PRF_SHA256,
    });

pub static TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384: SupportedCipherSuite =
    SupportedCipherSuite::Tls12(&Tls12CipherSuite {
        common: CipherSuiteCommon {
            suite: CipherSuite::TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,
            hash_provider: &SHA384,
            confidentiality_limit: 1 << 23,
        },
        kx: KeyExchangeAlgorithm::ECDHE,
        sign: RSA_SCHEMES,
        aead_alg: &tls12::AES_256_GCM,
        prf_provider: &tls12::PRF_SHA384,
    });

const ECDSA_SIG_SCHEMES: &[SignatureScheme] = &[
    SignatureScheme::ECDSA_NISTP256_SHA256,
    SignatureScheme::ECDSA_NISTP384_SHA384,
    SignatureScheme::ECDSA_NISTP521_SHA512,
];

// RSA scheme list lives in `crate::signer::rsa::RSA_SCHEMES` so the signer
// and the TLS 1.2 cipher-suite definitions stay in sync (one source of
// truth — the constant is re-exported through `signer/mod.rs`).

// =========================================================================
// Default cipher-suite list
// =========================================================================

/// Full cipher-suite list including TLS 1.2 fallback. Only consumed by
/// [`default_provider`] when `feature = "fips"` is *not* active —
/// `fips_provider`-only builds skip this in favour of [`FIPS_CIPHER_SUITES`].
#[cfg(not(feature = "fips"))]
pub static ALL_CIPHER_SUITES: &[SupportedCipherSuite] = &[
    // TLS 1.3 preferred.
    TLS13_AES_256_GCM_SHA384,
    TLS13_AES_128_GCM_SHA256,
    // TLS 1.2 fallback, ECDSA first then RSA, AES-256 first.
    TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
    TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
    TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,
    TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256,
];

/// TLS 1.3-only cipher-suite list, used by [`fips_provider`].
///
/// TLS 1.2 cipher suites are excluded because the TLS 1.2 PRF on this
/// provider is a generic HMAC-P_hash composition without a dedicated
/// CAVS validation on macOS (Apple corecrypto exposes only HMAC + hash
/// primitives, not a separately validated TLS PRF). Including any TLS
/// 1.2 cipher suite would make `CryptoProvider::fips()` return `false`
/// (it is the AND of every cipher suite's `fips()`), which in turn
/// poisons `ClientConfig::fips()` / `ServerConfig::fips()`.
pub static FIPS_CIPHER_SUITES: &[SupportedCipherSuite] =
    &[TLS13_AES_256_GCM_SHA384, TLS13_AES_128_GCM_SHA256];

// =========================================================================
// CryptoProvider construction
// =========================================================================

static SECURE_RANDOM: CoreCryptoRandom = CoreCryptoRandom;
static KEY_PROVIDER: CoreCryptoKeyProvider = CoreCryptoKeyProvider;

/// Process-wide cached `CryptoProvider` arcs. `CryptoProvider` contains
/// `Vec<SupportedCipherSuite>` and `Vec<&dyn SupportedKxGroup>` which
/// allocate on construction; rustls expects `Arc<CryptoProvider>`
/// downstream anyway, so caching here lets repeated `default_provider()`
/// / `fips_provider()` calls hand out cheap `Arc::clone`s instead of
/// re-allocating six `SupportedCipherSuite` slots every time.
///
/// The provider value itself is **immutable** (all components are
/// `&'static`), so a `OnceLock` is safe and race-free across threads.
#[cfg(not(feature = "fips"))]
static DEFAULT_PROVIDER_CACHE: OnceLock<Arc<CryptoProvider>> = OnceLock::new();
static FIPS_PROVIDER_CACHE: OnceLock<Arc<CryptoProvider>> = OnceLock::new();

#[cfg(not(feature = "fips"))]
fn build_default() -> CryptoProvider {
    CryptoProvider {
        cipher_suites: ALL_CIPHER_SUITES.to_vec(),
        kx_groups: vec![&SECP256R1, &SECP384R1],
        signature_verification_algorithms: SUPPORTED_SIG_ALGS,
        secure_random: &SECURE_RANDOM,
        key_provider: &KEY_PROVIDER,
    }
}

fn build_fips() -> CryptoProvider {
    CryptoProvider {
        cipher_suites: FIPS_CIPHER_SUITES.to_vec(),
        kx_groups: vec![&SECP256R1, &SECP384R1],
        signature_verification_algorithms: SUPPORTED_SIG_ALGS,
        secure_random: &SECURE_RANDOM,
        key_provider: &KEY_PROVIDER,
    }
}

/// Construct the corecrypto-backed [`CryptoProvider`].
///
/// **Without `feature = "fips"`** (default): TLS 1.2 + TLS 1.3 cipher
/// suites, `CryptoProvider::fips() == false` (because TLS 1.2 PRF is not
/// CAVS-validated on macOS — see [`fips_provider`] / ADR 0004). Use for
/// general-purpose outbound TLS where TLS 1.2 fallback is needed for
/// interop with older endpoints.
///
/// **With `feature = "fips"`**: this function returns the same value as
/// [`fips_provider`] — TLS 1.3 only, `CryptoProvider::fips() == true`.
/// Mirrors the feature-flag pattern in `rustls-cng-crypto`: downstream
/// callers compiled with `--features fips` get the FIPS-claim provider
/// automatically without having to switch factory calls.
///
/// All cryptographic operations route through Apple corecrypto in both
/// modes. Returns a fresh `CryptoProvider` value (rustls's contract);
/// internally a cached process-wide `Arc<CryptoProvider>` is cloned, so
/// this is allocation-free after the first call. Callers that want the
/// cached `Arc` directly can use [`default_provider_arc`].
pub fn default_provider() -> CryptoProvider {
    (*default_provider_arc()).clone()
}

/// Same as [`default_provider`] but returns the process-wide cached
/// `Arc<CryptoProvider>` directly, avoiding even the per-call
/// `CryptoProvider::clone`.
pub fn default_provider_arc() -> Arc<CryptoProvider> {
    // Under `feature = "fips"`, `default_provider*` is aliased to
    // `fips_provider*` — same cached Arc, same TLS-1.3-only set.
    #[cfg(feature = "fips")]
    {
        fips_provider_arc()
    }
    #[cfg(not(feature = "fips"))]
    {
        Arc::clone(DEFAULT_PROVIDER_CACHE.get_or_init(|| {
            // Prime the OE witness so its one-time `tracing::warn!` fires
            // here even if the caller never asks for the FIPS factory.
            // Per ADR 0004 + the FIPS-witness rework: no panic.
            let _ = crate::oe::fips_witness_ok();
            Arc::new(build_default())
        }))
    }
}

/// Construct the corecrypto-backed [`CryptoProvider`] restricted to
/// TLS 1.3 cipher suites only.
///
/// `CryptoProvider::fips()` returns `true` for this provider — every
/// cipher suite, key-exchange group, signature-verification algorithm,
/// RNG and key-provider component routes through a FIPS-validated
/// primitive. Downstream `ClientConfig::fips()` / `ServerConfig::fips()`
/// is `true` when the negotiated protocol is TLS 1.3, which is
/// guaranteed when constructed via
/// `builder_with_provider(fips_provider()).with_protocol_versions(...)`
/// restricting to TLS 1.3 (and, for TLS 1.2 fallback eventually, setting
/// `require_ems = true`).
///
/// **The FIPS claim still depends on the running macOS version being
/// covered by the current Apple corecrypto CMVP certificate** — see the
/// crate README's "Open questions / TODO" section and the per-OS-version
/// CMVP search referenced there.
pub fn fips_provider() -> CryptoProvider {
    (*fips_provider_arc()).clone()
}

/// Same as [`fips_provider`] but returns the process-wide cached
/// `Arc<CryptoProvider>` directly.
///
/// **Side effect on first call**: primes [`crate::oe::fips_witness_ok`]
/// so the one-time `tracing::warn!` is emitted on OE-validation
/// failure. The provider itself is still constructed and usable; the
/// runtime FIPS witness simply reports `false` everywhere on a host
/// whose macOS major is outside [`crate::oe::SUPPORTED_OE_MACOS_MAJOR`].
///
/// This crate **does not panic** on OE failure (per the C-2 rework).
/// The downstream signal is `CryptoProvider::fips() == false`, mirroring
/// `rustls-cng-crypto`'s posture on Windows when the OS FIPS-mode flag
/// is not set. The [`crate::oe::OE_OVERRIDE_ENV`] env-var forces the
/// witness back to `true` for CI on pre-release macOS — never for
/// production.
pub fn fips_provider_arc() -> Arc<CryptoProvider> {
    Arc::clone(FIPS_PROVIDER_CACHE.get_or_init(|| {
        // Prime the witness on first construction so OE telemetry surfaces
        // exactly once. The return value is consulted later by every
        // `fips()` impl across the crate.
        let _ = crate::oe::fips_witness_ok();
        Arc::new(build_fips())
    }))
}

#[cfg(test)]
#[path = "provider_tests.rs"]
mod tests;
