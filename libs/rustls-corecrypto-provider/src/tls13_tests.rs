use super::*;
use rustls::crypto::cipher::OutboundChunks;

fn aead_key_256() -> AeadKey {
    // rustls's public `AeadKey::from([u8; 32])` always yields a 32-byte
    // key; AES-128 unit-tests aren't expressible at the unit level via
    // public APIs (rustls reserves the shorter variant for its own
    // internal use). AES-128 is exercised end-to-end by handshake_smoke.
    AeadKey::from([0x22u8; 32])
}
fn iv12() -> Iv {
    Iv::copy(&[0x33u8; 12])
}

/// Encrypted-then-decrypted record must reproduce the original payload
/// and inner content type. This catches: wrong AAD construction, wrong
/// nonce derivation from (iv, seq), incorrect tag placement, broken
/// TLS 1.3 padding/contentType handling on decrypt.
#[test]
fn aes256_gcm_record_roundtrip() {
    let mut enc = AES_256_GCM.encrypter(aead_key_256(), iv12());
    let mut dec = AES_256_GCM.decrypter(aead_key_256(), iv12());

    let payload: &[u8] = b"hello tls 1.3 aes-256";
    let msg = OutboundPlainMessage {
        typ: ContentType::Handshake,
        version: ProtocolVersion::TLSv1_2,
        payload: OutboundChunks::Single(payload),
    };
    let opaque = enc.encrypt(msg, 42).expect("encrypt");

    // Strip the 5-byte TLS record header to get the encrypted payload.
    let wire = opaque.encode();
    assert!(wire.len() > 5 + payload.len() + TAG_LEN);
    let mut body = wire[5..].to_vec();

    let inbound = InboundOpaqueMessage::new(
        ContentType::ApplicationData,
        ProtocolVersion::TLSv1_2,
        body.as_mut_slice(),
    );
    let plain = dec.decrypt(inbound, 42).expect("decrypt");
    assert_eq!(plain.payload, payload);
    assert_eq!(plain.typ, ContentType::Handshake);
}

/// Wrong sequence number on decrypt must fail tag verification. Catches
/// a bug where `seq` is not actually folded into the nonce via XOR.
#[test]
fn aes256_gcm_wrong_seq_fails() {
    let mut enc = AES_256_GCM.encrypter(aead_key_256(), iv12());
    let mut dec = AES_256_GCM.decrypter(aead_key_256(), iv12());

    let payload: &[u8] = b"x";
    let msg = OutboundPlainMessage {
        typ: ContentType::ApplicationData,
        version: ProtocolVersion::TLSv1_2,
        payload: OutboundChunks::Single(payload),
    };
    let opaque = enc.encrypt(msg, 7).expect("encrypt");
    let wire = opaque.encode();
    let mut body = wire[5..].to_vec();

    let inbound = InboundOpaqueMessage::new(
        ContentType::ApplicationData,
        ProtocolVersion::TLSv1_2,
        body.as_mut_slice(),
    );
    // Decrypt with seq=0 instead of 7 — must fail (different nonce).
    assert!(dec.decrypt(inbound, 0).is_err());
}

/// `extract_keys` returns the appropriate `ConnectionTrafficSecrets`
/// variant — required for callers exporting keys (e.g. for kTLS or
/// QUIC interop).
#[test]
fn extract_keys_aes128_returns_correct_variant() {
    let secrets =
        Tls13AeadAlgorithm::extract_keys(&AES_128_GCM, aead_key_256(), iv12()).expect("extract");
    assert!(matches!(
        secrets,
        rustls::ConnectionTrafficSecrets::Aes128Gcm { .. }
    ));
}

#[test]
fn extract_keys_aes256_returns_correct_variant() {
    let secrets =
        Tls13AeadAlgorithm::extract_keys(&AES_256_GCM, aead_key_256(), iv12()).expect("extract");
    assert!(matches!(
        secrets,
        rustls::ConnectionTrafficSecrets::Aes256Gcm { .. }
    ));
}

/// `key_len()` contract: must match the AES key size for which this
/// AEAD is registered. A mismatch would cause rustls to derive the
/// wrong key length from the HKDF schedule.
#[test]
fn key_len_contract() {
    assert_eq!(Tls13AeadAlgorithm::key_len(&AES_128_GCM), 16);
    assert_eq!(Tls13AeadAlgorithm::key_len(&AES_256_GCM), 32);
}

/// FIPS-claim contract for both AEAD variants.
#[test]
fn fips_contract() {
    assert!(Tls13AeadAlgorithm::fips(&AES_128_GCM));
    assert!(Tls13AeadAlgorithm::fips(&AES_256_GCM));
}

/// `encrypted_payload_len(N)` exactly equals `N + 1` (inner ContentType
/// byte) plus `16` (GCM tag). Catches a math drift between this
/// accessor and the actual `encrypt` output length.
#[test]
fn encrypted_payload_len_matches_encrypt_output() {
    let mut enc = AES_256_GCM.encrypter(aead_key_256(), iv12());
    let payload: &[u8] = b"abc";

    let predicted = enc.encrypted_payload_len(payload.len());

    let msg = OutboundPlainMessage {
        typ: ContentType::ApplicationData,
        version: ProtocolVersion::TLSv1_2,
        payload: OutboundChunks::Single(payload),
    };
    let opaque = enc.encrypt(msg, 0).expect("encrypt");
    let body_len = opaque.encode().len() - 5; // strip record header

    assert_eq!(predicted, body_len);
    assert_eq!(predicted, payload.len() + 1 + TAG_LEN);
}

/// Decryption of a too-short payload (< tag size) must error rather than
/// panic — boundary safety against malformed records.
#[test]
fn aes256_gcm_too_short_payload_errors() {
    let mut dec = AES_256_GCM.decrypter(aead_key_256(), iv12());
    let mut buf = [0u8; 5];
    let inbound = InboundOpaqueMessage::new(
        ContentType::ApplicationData,
        ProtocolVersion::TLSv1_2,
        &mut buf[..],
    );
    assert!(dec.decrypt(inbound, 0).is_err());
}
