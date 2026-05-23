use super::*;
use rustls::crypto::cipher::OutboundChunks;
use rustls::{ContentType, ProtocolVersion};

fn aead_key_256() -> AeadKey {
    // See tls13.rs tests — public API only constructs 32-byte AeadKey,
    // so AES-128 unit-tests aren't expressible here. AES-128 is
    // exercised end-to-end by handshake_smoke.
    AeadKey::from([0x99u8; 32])
}
fn implicit_iv() -> [u8; 4] {
    [0x77u8; 4]
}

/// TLS 1.2 record encrypt → decrypt roundtrip for AES-256-GCM with the
/// explicit-nonce wire format. Catches: wrong AAD construction (TLS 1.2
/// uses seq||type||version||length, different from TLS 1.3), wrong
/// nonce assembly (implicit_iv(4) || explicit_seq(8)), broken extraction
/// of explicit-nonce prefix on decrypt.
#[test]
fn aes256_gcm_record_roundtrip() {
    let mut enc =
        Tls12AeadAlgorithm::encrypter(&AES_256_GCM, aead_key_256(), &implicit_iv(), &[0u8; 8]);
    let mut dec = Tls12AeadAlgorithm::decrypter(&AES_256_GCM, aead_key_256(), &implicit_iv());

    let payload: &[u8] = b"tls 1.2 application data";
    let msg = OutboundPlainMessage {
        typ: ContentType::ApplicationData,
        version: ProtocolVersion::TLSv1_2,
        payload: OutboundChunks::Single(payload),
    };
    let opaque = enc.encrypt(msg, 100).expect("encrypt");

    let wire = opaque.encode();
    let mut body = wire[5..].to_vec(); // strip 5-byte record header

    let inbound = InboundOpaqueMessage::new(
        ContentType::ApplicationData,
        ProtocolVersion::TLSv1_2,
        body.as_mut_slice(),
    );
    let plain = dec.decrypt(inbound, 100).expect("decrypt");
    assert_eq!(plain.payload, payload);
    assert_eq!(plain.typ, ContentType::ApplicationData);
}

/// Wrong sequence number on decrypt must fail — the seq feeds both the
/// nonce (via explicit prefix on wire == seq on rustls side) and the AAD.
#[test]
fn aes256_gcm_wrong_seq_fails() {
    let mut enc =
        Tls12AeadAlgorithm::encrypter(&AES_256_GCM, aead_key_256(), &implicit_iv(), &[0u8; 8]);
    let mut dec = Tls12AeadAlgorithm::decrypter(&AES_256_GCM, aead_key_256(), &implicit_iv());

    let payload: &[u8] = b"x";
    let msg = OutboundPlainMessage {
        typ: ContentType::ApplicationData,
        version: ProtocolVersion::TLSv1_2,
        payload: OutboundChunks::Single(payload),
    };
    let opaque = enc.encrypt(msg, 11).expect("encrypt");
    let wire = opaque.encode();
    let mut body = wire[5..].to_vec();
    let inbound = InboundOpaqueMessage::new(
        ContentType::ApplicationData,
        ProtocolVersion::TLSv1_2,
        body.as_mut_slice(),
    );
    // seq=99 ≠ 11 → AAD mismatch → tag mismatch.
    assert!(dec.decrypt(inbound, 99).is_err());
}

/// `encrypted_payload_len(N)` exactly equals N + 8 (explicit nonce) +
/// 16 (tag). Catches drift between this accessor and `encrypt` output.
#[test]
fn encrypted_payload_len_matches_encrypt_output() {
    let mut enc =
        Tls12AeadAlgorithm::encrypter(&AES_256_GCM, aead_key_256(), &implicit_iv(), &[0u8; 8]);
    let payload: &[u8] = b"abcdef";

    let predicted = enc.encrypted_payload_len(payload.len());

    let msg = OutboundPlainMessage {
        typ: ContentType::ApplicationData,
        version: ProtocolVersion::TLSv1_2,
        payload: OutboundChunks::Single(payload),
    };
    let opaque = enc.encrypt(msg, 1).expect("encrypt");
    let body_len = opaque.encode().len() - 5;

    assert_eq!(predicted, body_len);
    assert_eq!(predicted, payload.len() + EXPLICIT_NONCE_LEN + TAG_LEN);
}

/// `key_block_shape` contract: AES-128 / AES-256 differ only in
/// `enc_key_len`; both use the same 4-byte implicit IV + 8-byte
/// explicit nonce. rustls derives the TLS 1.2 key_block layout from
/// these numbers.
#[test]
fn key_block_shape_contract() {
    let s128 = Tls12AeadAlgorithm::key_block_shape(&AES_128_GCM);
    let s256 = Tls12AeadAlgorithm::key_block_shape(&AES_256_GCM);
    assert_eq!(s128.enc_key_len, 16);
    assert_eq!(s256.enc_key_len, 32);
    assert_eq!(s128.fixed_iv_len, IMPLICIT_IV_LEN);
    assert_eq!(s256.fixed_iv_len, IMPLICIT_IV_LEN);
    assert_eq!(s128.explicit_nonce_len, EXPLICIT_NONCE_LEN);
    assert_eq!(s256.explicit_nonce_len, EXPLICIT_NONCE_LEN);
}

/// `extract_keys` returns the right `ConnectionTrafficSecrets` variant
/// for both AES widths. Required by callers exporting keys.
#[test]
fn extract_keys_aes128_variant() {
    let secrets =
        Tls12AeadAlgorithm::extract_keys(&AES_128_GCM, aead_key_256(), &implicit_iv(), &[0u8; 8])
            .expect("extract");
    assert!(matches!(
        secrets,
        rustls::ConnectionTrafficSecrets::Aes128Gcm { .. }
    ));
}

#[test]
fn extract_keys_aes256_variant() {
    let secrets =
        Tls12AeadAlgorithm::extract_keys(&AES_256_GCM, aead_key_256(), &implicit_iv(), &[0u8; 8])
            .expect("extract");
    assert!(matches!(
        secrets,
        rustls::ConnectionTrafficSecrets::Aes256Gcm { .. }
    ));
}

/// FIPS-claim contract for both AEAD variants.
#[test]
fn aead_fips_contract() {
    assert!(Tls12AeadAlgorithm::fips(&AES_128_GCM));
    assert!(Tls12AeadAlgorithm::fips(&AES_256_GCM));
}

/// PRF FIPS contract: our `Prf` impls do NOT override `fips()`, so
/// they inherit the trait default `false`. This is the honest stance —
/// TLS 1.2 PRF is a generic HMAC-P_hash composition; corecrypto does
/// not expose a separately CAVS-validated TLS PRF primitive (unlike
/// aws-lc-fips, which has one). A regression that re-introduces a
/// `fips() = true` override here would silently re-claim FIPS for
/// TLS 1.2 cipher suites and poison `ServerConfig::fips()` /
/// `ClientConfig::fips()` for any TLS-1.2-negotiated connection.
#[test]
fn prf_fips_contract_is_intentionally_false() {
    assert!(
        !PRF_SHA256.fips(),
        "PRF must NOT claim FIPS -- generic HMAC P_hash is not CAVS-validated"
    );
    assert!(!PRF_SHA384.fips(), "same as PRF_SHA256");
}

/// **M-2 invariant.** Each TLS 1.2 cipher suite's `fips()` MUST be
/// `false` regardless of its AEAD's individual `fips()`. rustls's
/// `Tls12CipherSuite::fips()` is the AND of `hash.fips() &&
/// aead.fips() && prf.fips()`; our PRF's `false` carries the whole
/// suite to `false`. If a future refactor accidentally overrides
/// `PrfUsingHmac::fips()` to `true` (or rustls upstream changes its
/// default), this test surfaces the regression at the cipher-suite
/// layer rather than letting it propagate silently to
/// `default_provider().fips() = true` on TLS 1.2 paths.
#[test]
fn tls12_cipher_suite_fips_is_false_due_to_prf() {
    use crate::provider::{
        TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256, TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
        TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256, TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,
    };
    for cs in [
        TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
        TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
        TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256,
        TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,
    ] {
        let suite = cs.suite();
        assert!(
            !cs.fips(),
            "TLS 1.2 cipher suite {suite:?} unexpectedly claims FIPS -- PRF gap should keep it false"
        );
    }
}

/// **M-4 regression (explicit-nonce policy).** With a non-zero `extra`,
/// the explicit nonce on the wire must be `extra XOR seq.to_be_bytes()`,
/// matching rustls's aws-lc-rs and ring TLS 1.2 nonce construction.
/// The wire bytes are the first 8 of the encrypted body (record-header
/// stripped).
#[test]
fn tls12_explicit_nonce_is_extra_xor_seq() {
    let extra: [u8; 8] = [0xa5, 0x5a, 0xff, 0x00, 0x12, 0x34, 0x56, 0x78];
    let mut enc =
        Tls12AeadAlgorithm::encrypter(&AES_256_GCM, aead_key_256(), &implicit_iv(), &extra);
    let payload: &[u8] = b"x";
    let msg = OutboundPlainMessage {
        typ: ContentType::ApplicationData,
        version: ProtocolVersion::TLSv1_2,
        payload: OutboundChunks::Single(payload),
    };
    let seq: u64 = 0x0102_0304_0506_0708;
    let opaque = enc.encrypt(msg, seq).expect("encrypt");
    let wire = opaque.encode();
    let body = &wire[5..]; // strip 5-byte record header

    let mut expected_explicit = extra;
    let seq_be = seq.to_be_bytes();
    for i in 0..8 {
        expected_explicit[i] ^= seq_be[i];
    }
    assert_eq!(
        &body[..EXPLICIT_NONCE_LEN],
        &expected_explicit,
        "explicit nonce must be extra XOR seq"
    );

    // Two distinct `extra` values must produce distinct explicit
    // nonces for the same seq — defends against a regression that
    // accidentally ignores `extra`.
    let mut enc2 =
        Tls12AeadAlgorithm::encrypter(&AES_256_GCM, aead_key_256(), &implicit_iv(), &[0u8; 8]);
    let opaque2 = enc2
        .encrypt(
            OutboundPlainMessage {
                typ: ContentType::ApplicationData,
                version: ProtocolVersion::TLSv1_2,
                payload: OutboundChunks::Single(payload),
            },
            seq,
        )
        .expect("encrypt");
    let wire2 = opaque2.encode();
    assert_ne!(
        &wire[5..5 + EXPLICIT_NONCE_LEN],
        &wire2[5..5 + EXPLICIT_NONCE_LEN],
        "distinct `extra` must yield distinct explicit nonces"
    );
}

/// Decryption of a too-short payload (< explicit_nonce + tag) must
/// error rather than panic — boundary safety against malformed records.
#[test]
fn aes256_gcm_too_short_payload_errors() {
    let mut dec = Tls12AeadAlgorithm::decrypter(&AES_256_GCM, aead_key_256(), &implicit_iv());
    let mut buf = [0u8; 10]; // < EXPLICIT_NONCE_LEN + TAG_LEN
    let inbound = InboundOpaqueMessage::new(
        ContentType::ApplicationData,
        ProtocolVersion::TLSv1_2,
        &mut buf[..],
    );
    assert!(dec.decrypt(inbound, 0).is_err());
}

/// C-3 regression: a wrong-length implicit IV must surface as an
/// explicit panic with a descriptive message, not a silent
/// `copy_from_slice` panic deeper in the call stack. The contract is
/// that rustls always passes 4-byte implicit IVs; if that ever
/// breaks, the error should be obvious to debug.
#[test]
#[should_panic(expected = "TLS 1.2 implicit IV must be 4 bytes")]
fn build_iv_panics_on_wrong_iv_length() {
    let _ = build_iv(&[0u8; 3], &[0u8; 8]);
}

#[test]
#[should_panic(expected = "TLS 1.2 explicit nonce must be 8 bytes")]
fn build_iv_panics_on_wrong_explicit_length() {
    let _ = build_iv(&[0u8; 4], &[0u8; 7]);
}

#[test]
#[should_panic(expected = "TLS 1.2 implicit IV must be 4 bytes")]
fn make_encrypter_panics_on_wrong_iv_length() {
    let _ = make_encrypter(AeadKey::from([0u8; 32]), &[0u8; 5], &[0u8; 8]);
}

/// PRF wire-correctness regression. Constructs the P_hash reference
/// output by hand per RFC 5246 §5:
///
///   A(0) = seed (where "seed" here = label || actual_seed)
///   A(i) = HMAC(secret, A(i-1))
///   P_hash output = HMAC(secret, A(1) || seed) ||
///                   HMAC(secret, A(2) || seed) || ...
///   PRF(secret, label, actual_seed) = P_hash(secret, label || actual_seed)
///
/// then compares to our `PRF_SHA256::for_secret` output. If a future
/// refactor breaks the label/seed concatenation order or the A(i)
/// recurrence, this test fails. Together with the existing HMAC
/// RFC 4231 KAT tests, this gives end-to-end wire correctness for
/// our TLS 1.2 PRF construction. (The PRF itself is not CAVS-
/// validated on macOS — see ADR 0004 — but it MUST still produce
/// RFC 5246-correct output to interoperate with peers.)
#[test]
fn prf_sha256_matches_manual_p_hash_per_rfc5246() {
    use rustls::crypto::hmac::Hmac;

    let secret = b"my-master-secret-bytes";
    let label = b"key expansion";
    let seed = b"server_random || client_random";
    let mut out = [0u8; 96]; // 3 SHA-256 blocks (= 3 P_hash iterations)
    PRF_SHA256.for_secret(&mut out, secret, label, seed);

    // Manual P_hash(secret, label || seed):
    let mac = HMAC_SHA256.with_key(secret);
    let combined_seed: Vec<u8> = label.iter().chain(seed.iter()).copied().collect();

    let mut a_prev = mac.sign(&[&combined_seed]); // A(1) = HMAC(secret, A(0)=seed)
    let mut expected = Vec::<u8>::with_capacity(96);
    for _ in 0..3 {
        // P_hash block = HMAC(secret, A(i) || label || seed)
        let block = mac.sign(&[a_prev.as_ref(), label, seed]);
        expected.extend_from_slice(block.as_ref());
        // A(i+1) = HMAC(secret, A(i))
        a_prev = mac.sign(&[a_prev.as_ref()]);
    }

    assert_eq!(
        out.as_slice(),
        &expected[..96],
        "PRF output diverges from manual P_hash reference — wire-incorrect"
    );
}

/// PRF `for_secret`: HMAC-based P_hash must produce deterministic
/// output. Catches a bug where PRF accidentally uses non-deterministic
/// state (e.g. uninit memory).
#[test]
fn prf_for_secret_is_deterministic() {
    let secret = b"premaster_secret_dummy";
    let label = b"key expansion";
    let seed = b"server_random || client_random";
    let mut a = [0u8; 48];
    let mut b = [0u8; 48];
    PRF_SHA256.for_secret(&mut a, secret, label, seed);
    PRF_SHA256.for_secret(&mut b, secret, label, seed);
    assert_eq!(a, b);
    // Output must not be all zeros (a broken PRF that returns the
    // initial buffer would).
    assert!(a.iter().any(|&x| x != 0));
}
