use super::*;
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};

/// The provider's `secure_random` must produce distinct output across
/// calls. A broken delegation (e.g. returning a constant) would fail.
#[test]
fn secure_random_produces_distinct_output_across_calls() {
    let p = default_provider();
    let mut a = [0u8; 32];
    let mut b = [0u8; 32];
    p.secure_random.fill(&mut a).expect("fill a");
    p.secure_random.fill(&mut b).expect("fill b");
    assert_ne!(a, b);
}

/// The provider's `key_provider` rejects obviously-malformed input
/// with the documented marker error rather than a generic failure or
/// panic. Tightened from a plain `.is_err()` so a regression that
/// returns `Err(Error::Other(...))` (or worse, that silently accepts
/// the bytes and surfaces a panic during `sign()` later) is caught
/// here at load time.
#[test]
fn key_provider_rejects_garbage_private_key() {
    let p = default_provider();
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(vec![0u8; 16]));
    match p.key_provider.load_private_key(key) {
        Err(rustls::Error::General(msg)) => assert!(
            msg.contains("unsupported private-key type"),
            "garbage rejection must use the documented marker, got {msg:?}"
        ),
        other => panic!("expected Error::General, got {other:?}"),
    }
}

/// Without `feature = "fips"`: 2 TLS 1.3 + 4 TLS 1.2 cipher suites
/// are exposed — without all of them, rustls would fail to negotiate
/// with peers that only offer a subset.
#[cfg(not(feature = "fips"))]
#[test]
fn provider_exposes_six_cipher_suites() {
    let p = default_provider();
    assert_eq!(p.cipher_suites.len(), 6);
}

/// With `feature = "fips"`: `default_provider()` is aliased to
/// `fips_provider()`, so only the 2 TLS 1.3 suites are exposed.
#[cfg(feature = "fips")]
#[test]
fn provider_under_fips_exposes_two_tls13_suites_only() {
    let p = default_provider();
    assert_eq!(p.cipher_suites.len(), 2);
    for cs in &p.cipher_suites {
        assert!(
            matches!(cs, rustls::SupportedCipherSuite::Tls13(_)),
            "fips-feature provider must contain only TLS 1.3 suites"
        );
    }
    assert!(
        p.fips(),
        "default_provider().fips() under feature=fips must be true"
    );
}

/// Both NIST P-curves are exposed.
#[test]
fn provider_exposes_two_kx_groups() {
    let p = default_provider();
    assert_eq!(p.kx_groups.len(), 2);
    // ECDHE relies on at least P-256 being available; assert both are.
    let names: Vec<_> = p.kx_groups.iter().map(|g| g.name()).collect();
    assert!(names.contains(&rustls::NamedGroup::secp256r1));
    assert!(names.contains(&rustls::NamedGroup::secp384r1));
}

/// Without `feature = "fips"`: `default_provider()` includes TLS 1.2
/// cipher suites, whose PRF is not CAVS-validated on macOS. Per
/// rustls's `CryptoProvider::fips()` (AND over every cipher suite +
/// every kx group + sig-verify + RNG + key-provider), this means
/// `default_provider().fips() == false`.
///
/// This is the honest stance — claim FIPS only via `fips_provider()`,
/// or compile with `--features fips` to flip `default_provider()`
/// itself to the FIPS path.
#[cfg(not(feature = "fips"))]
#[test]
fn default_provider_is_not_fips_due_to_tls12_prf() {
    let p = default_provider();
    // Component check first — narrows the blame on regression.
    for cs in &p.cipher_suites {
        let suite = cs.suite();
        let is_tls13 = matches!(cs, rustls::SupportedCipherSuite::Tls13(_));
        if is_tls13 {
            assert!(cs.fips(), "TLS 1.3 suite {suite:?} must claim FIPS");
        } else {
            assert!(
                !cs.fips(),
                "TLS 1.2 suite {suite:?} must NOT claim FIPS (PRF not CAVS-validated)"
            );
        }
    }
    for kx in &p.kx_groups {
        assert!(kx.fips(), "kx group {:?} not FIPS", kx.name());
    }
    assert!(p.signature_verification_algorithms.fips());
    assert!(p.secure_random.fips());
    assert!(p.key_provider.fips());
    // Overall: false, because at least one TLS 1.2 cipher suite is in
    // the set.
    assert!(
        !p.fips(),
        "default_provider() must not claim FIPS while TLS 1.2 suites are present"
    );
}

/// `fips_provider()` restricts to TLS 1.3 cipher suites only — every
/// component is FIPS, so `CryptoProvider::fips() == true`.
#[test]
fn fips_provider_claims_fips() {
    let p = fips_provider();
    assert_eq!(
        p.cipher_suites.len(),
        2,
        "fips_provider must expose exactly the two TLS 1.3 GCM suites"
    );
    for cs in &p.cipher_suites {
        assert!(
            matches!(cs, rustls::SupportedCipherSuite::Tls13(_)),
            "fips_provider must contain only TLS 1.3 suites"
        );
        assert!(cs.fips(), "TLS 1.3 suite must claim FIPS");
    }
    assert!(p.fips(), "fips_provider().fips() must be true");
}

/// A `ClientConfig` built on `fips_provider()` + EMS-required must
/// advertise FIPS. Catches regressions where any component flips
/// `fips()` to false.
#[test]
fn client_config_on_fips_provider_with_ems_advertises_fips() {
    let mut config = rustls::ClientConfig::builder_with_provider(fips_provider().into())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("protocol versions")
        .with_root_certificates(rustls::RootCertStore::empty())
        .with_no_client_auth();
    config.require_ems = true;

    assert!(
        config.fips(),
        "ClientConfig on fips_provider() with require_ems=true must claim FIPS"
    );
}

/// **A-2 regression.** Every cipher suite in our static list must
/// carry the documented AES-GCM `confidentiality_limit` of `1 << 23`
/// records. rustls's `CipherSuiteCommon` doc (rustls 0.23) cites
/// `2^24` as the AES-GCM bound to keep attack probability ≤ 2^-60
/// (see AEBounds / draft-irtf-cfrg-aead-limits-08); we run at the
/// slightly more conservative `2^23` per CipherSuiteCommon-construction
/// in `provider.rs`. This test locks the constant so an accidental
/// future drift (e.g. `1 << 27` for a perf claim) is caught here
/// rather than going unnoticed into production.
///
/// rustls's TLS-side `CipherSuiteCommon` has no `integrity_limit`
/// field — that exists on the QUIC path only — so we cannot pin one
/// here; the spirit of the A-2 review item (lock AEAD bounds) is
/// addressed by the confidentiality side.
#[test]
fn cipher_suites_use_documented_aes_gcm_confidentiality_limit() {
    const EXPECTED: u64 = 1 << 23;
    let suites: &[SupportedCipherSuite] = &[
        TLS13_AES_128_GCM_SHA256,
        TLS13_AES_256_GCM_SHA384,
        TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
        TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
        TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256,
        TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,
    ];
    for cs in suites {
        let limit = match cs {
            SupportedCipherSuite::Tls13(s) => s.common.confidentiality_limit,
            SupportedCipherSuite::Tls12(s) => s.common.confidentiality_limit,
        };
        assert_eq!(
            limit,
            EXPECTED,
            "cipher suite {:?} drifted from documented AES-GCM confidentiality_limit",
            cs.suite()
        );
    }
}

/// And the contrapositive (only without `feature = "fips"`): a
/// `ClientConfig` on `default_provider()` must NOT claim FIPS, no
/// matter the protocol-version restriction. rustls evaluates
/// `provider.fips()` once at config build time, so having TLS 1.2
/// suites in the provider poisons the claim even with
/// `with_protocol_versions(&[TLS13])`.
#[cfg(not(feature = "fips"))]
#[test]
fn client_config_on_default_provider_does_not_claim_fips() {
    let mut config = rustls::ClientConfig::builder_with_provider(default_provider().into())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("protocol versions")
        .with_root_certificates(rustls::RootCertStore::empty())
        .with_no_client_auth();
    config.require_ems = true;

    assert!(
        !config.fips(),
        "default_provider's TLS 1.2 suites must keep ClientConfig::fips() false"
    );
}
