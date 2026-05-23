use super::*;
use security_framework::key::{GenerateKeyOptions, KeyType, SecKey};

fn gen_ec(size_bits: u32) -> SecKey {
    let mut opts = GenerateKeyOptions::default();
    opts.set_key_type(KeyType::ec());
    opts.set_size_in_bits(size_bits);
    SecKey::new(&opts).expect("EC keygen")
}

fn gen_rsa(size_bits: u32) -> SecKey {
    let mut opts = GenerateKeyOptions::default();
    opts.set_key_type(KeyType::rsa());
    opts.set_size_in_bits(size_bits);
    SecKey::new(&opts).expect("RSA keygen")
}

fn pub_bytes(k: &SecKey) -> Vec<u8> {
    k.public_key()
        .expect("public_key")
        .external_representation()
        .expect("external_representation")
        .bytes()
        .to_vec()
}

/// ECDSA P-256 SHA-256 roundtrip: Apple-signed signature must verify
/// through our trait impl.
#[test]
fn ecdsa_p256_sha256_roundtrip() {
    let key = gen_ec(256);
    let msg = b"the quick brown fox jumps over the lazy dog";
    let sig = key
        .create_signature(Algorithm::ECDSASignatureMessageX962SHA256, msg)
        .expect("sign");
    let pk = pub_bytes(&key);
    EcdsaP256Sha256.verify_signature(&pk, msg, &sig).unwrap();
}

/// ECDSA P-384 SHA-384 roundtrip.
#[test]
fn ecdsa_p384_sha384_roundtrip() {
    let key = gen_ec(384);
    let msg = b"another message";
    let sig = key
        .create_signature(Algorithm::ECDSASignatureMessageX962SHA384, msg)
        .expect("sign");
    let pk = pub_bytes(&key);
    EcdsaP384Sha384.verify_signature(&pk, msg, &sig).unwrap();
}

/// ECDSA P-521 SHA-512 roundtrip. Required for cross-OS parity
/// (rustls-cng-crypto on Windows exposes P-521); FIPS-claim unaffected
/// since Apple corecrypto's CMVP cert covers P-521.
#[test]
fn ecdsa_p521_sha512_roundtrip() {
    let key = gen_ec(521);
    let msg = b"p-521 sha-512 roundtrip message";
    let sig = key
        .create_signature(Algorithm::ECDSASignatureMessageX962SHA512, msg)
        .expect("sign");
    let pk = pub_bytes(&key);
    EcdsaP521Sha512.verify_signature(&pk, msg, &sig).unwrap();
}

/// Tampered message must fail verification.
#[test]
fn ecdsa_p256_tampered_message_fails() {
    let key = gen_ec(256);
    let msg = b"original message";
    let sig = key
        .create_signature(Algorithm::ECDSASignatureMessageX962SHA256, msg)
        .expect("sign");
    let pk = pub_bytes(&key);
    let bad = b"different message";
    assert!(EcdsaP256Sha256.verify_signature(&pk, bad, &sig).is_err());
}

/// Tampered signature must fail verification.
#[test]
fn ecdsa_p256_tampered_signature_fails() {
    let key = gen_ec(256);
    let msg = b"a message";
    let mut sig = key
        .create_signature(Algorithm::ECDSASignatureMessageX962SHA256, msg)
        .expect("sign");
    // Flip a bit in the signature DER value (avoid the leading SEQUENCE tag).
    let last = sig.len() - 1;
    sig[last] ^= 0x01;
    let pk = pub_bytes(&key);
    assert!(EcdsaP256Sha256.verify_signature(&pk, msg, &sig).is_err());
}

/// Verification with a DIFFERENT key must fail.
#[test]
fn ecdsa_p256_wrong_key_fails() {
    let signer = gen_ec(256);
    let other = gen_ec(256);
    let msg = b"a message";
    let sig = signer
        .create_signature(Algorithm::ECDSASignatureMessageX962SHA256, msg)
        .expect("sign");
    let wrong_pk = pub_bytes(&other);
    assert!(
        EcdsaP256Sha256
            .verify_signature(&wrong_pk, msg, &sig)
            .is_err()
    );
}

/// Malformed public-key bytes must fail rather than panic.
#[test]
fn ecdsa_p256_malformed_public_key_fails() {
    let bad_pk = vec![0u8; 10];
    let result = EcdsaP256Sha256.verify_signature(&bad_pk, b"msg", b"sig");
    assert!(result.is_err());
}

/// RSA-PSS SHA-256 roundtrip.
#[test]
fn rsa_pss_sha256_roundtrip() {
    let key = gen_rsa(2048);
    let msg = b"rsa-pss test message";
    let sig = key
        .create_signature(Algorithm::RSASignatureMessagePSSSHA256, msg)
        .expect("sign");
    let pk = pub_bytes(&key);
    RsaPssSha256.verify_signature(&pk, msg, &sig).unwrap();
}

/// RSA-PSS SHA-384 roundtrip.
#[test]
fn rsa_pss_sha384_roundtrip() {
    let key = gen_rsa(2048);
    let msg = b"rsa-pss-384";
    let sig = key
        .create_signature(Algorithm::RSASignatureMessagePSSSHA384, msg)
        .expect("sign");
    let pk = pub_bytes(&key);
    RsaPssSha384.verify_signature(&pk, msg, &sig).unwrap();
}

/// RSA-PSS SHA-512 roundtrip.
#[test]
fn rsa_pss_sha512_roundtrip() {
    let key = gen_rsa(2048);
    let msg = b"rsa-pss-512";
    let sig = key
        .create_signature(Algorithm::RSASignatureMessagePSSSHA512, msg)
        .expect("sign");
    let pk = pub_bytes(&key);
    RsaPssSha512.verify_signature(&pk, msg, &sig).unwrap();
}

/// RSA-PKCS1 v1.5 SHA-256 roundtrip.
#[test]
fn rsa_pkcs1_sha256_roundtrip() {
    let key = gen_rsa(2048);
    let msg = b"rsa-pkcs1-256";
    let sig = key
        .create_signature(Algorithm::RSASignatureMessagePKCS1v15SHA256, msg)
        .expect("sign");
    let pk = pub_bytes(&key);
    RsaPkcs1Sha256.verify_signature(&pk, msg, &sig).unwrap();
}

/// RSA-PKCS1 v1.5 SHA-384 roundtrip.
#[test]
fn rsa_pkcs1_sha384_roundtrip() {
    let key = gen_rsa(2048);
    let msg = b"rsa-pkcs1-384";
    let sig = key
        .create_signature(Algorithm::RSASignatureMessagePKCS1v15SHA384, msg)
        .expect("sign");
    let pk = pub_bytes(&key);
    RsaPkcs1Sha384.verify_signature(&pk, msg, &sig).unwrap();
}

/// RSA-PKCS1 v1.5 SHA-512 roundtrip.
#[test]
fn rsa_pkcs1_sha512_roundtrip() {
    let key = gen_rsa(2048);
    let msg = b"rsa-pkcs1-512";
    let sig = key
        .create_signature(Algorithm::RSASignatureMessagePKCS1v15SHA512, msg)
        .expect("sign");
    let pk = pub_bytes(&key);
    RsaPkcs1Sha512.verify_signature(&pk, msg, &sig).unwrap();
}

/// Tampered RSA signature must fail.
#[test]
fn rsa_pss_sha256_tampered_signature_fails() {
    let key = gen_rsa(2048);
    let msg = b"rsa";
    let mut sig = key
        .create_signature(Algorithm::RSASignatureMessagePSSSHA256, msg)
        .expect("sign");
    sig[0] ^= 0x01;
    let pk = pub_bytes(&key);
    assert!(RsaPssSha256.verify_signature(&pk, msg, &sig).is_err());
}

/// Cross-algorithm reject: PKCS#1 v1.5 signature must NOT verify as PSS.
#[test]
fn pkcs1_sig_does_not_verify_as_pss() {
    let key = gen_rsa(2048);
    let msg = b"hybrid test";
    let pkcs1_sig = key
        .create_signature(Algorithm::RSASignatureMessagePKCS1v15SHA256, msg)
        .expect("sign");
    let pk = pub_bytes(&key);
    assert!(RsaPssSha256.verify_signature(&pk, msg, &pkcs1_sig).is_err());
}

/// Public/signature alg-id pairs match the X.509 OIDs they advertise.
/// rustls uses these IDs to match TLS sig_alg negotiation with cert
/// SubjectPublicKeyInfo — a mismatch silently breaks cert validation.
#[test]
fn alg_id_contract() {
    assert_eq!(EcdsaP256Sha256.public_key_alg_id(), alg_id::ECDSA_P256);
    assert_eq!(EcdsaP256Sha256.signature_alg_id(), alg_id::ECDSA_SHA256);
    assert_eq!(EcdsaP384Sha384.public_key_alg_id(), alg_id::ECDSA_P384);
    assert_eq!(EcdsaP384Sha384.signature_alg_id(), alg_id::ECDSA_SHA384);
    assert_eq!(EcdsaP521Sha512.public_key_alg_id(), alg_id::ECDSA_P521);
    assert_eq!(EcdsaP521Sha512.signature_alg_id(), alg_id::ECDSA_SHA512);
    assert_eq!(RsaPssSha256.public_key_alg_id(), alg_id::RSA_ENCRYPTION);
    assert_eq!(RsaPssSha256.signature_alg_id(), alg_id::RSA_PSS_SHA256);
    assert_eq!(RsaPkcs1Sha512.signature_alg_id(), alg_id::RSA_PKCS1_SHA512);
}

/// All eight algorithms claim FIPS — required for downstream
/// `ClientConfig::fips()` invariant.
#[test]
fn all_algorithms_advertise_fips() {
    for alg in SUPPORTED_SIG_ALGS.all {
        assert!(
            alg.fips(),
            "every supported signature algorithm must advertise FIPS, but {alg:?} does not"
        );
    }
}

/// `mapping` table must reference exactly one algorithm per scheme we
/// advertise. Catches accidental omissions when a new scheme is added.
#[test]
fn mapping_table_complete() {
    let schemes_in_mapping: Vec<SignatureScheme> =
        SUPPORTED_SIG_ALGS.mapping.iter().map(|(s, _)| *s).collect();
    let expected = [
        SignatureScheme::ECDSA_NISTP256_SHA256,
        SignatureScheme::ECDSA_NISTP384_SHA384,
        SignatureScheme::ECDSA_NISTP521_SHA512,
        SignatureScheme::RSA_PSS_SHA256,
        SignatureScheme::RSA_PSS_SHA384,
        SignatureScheme::RSA_PSS_SHA512,
        SignatureScheme::RSA_PKCS1_SHA256,
        SignatureScheme::RSA_PKCS1_SHA384,
        SignatureScheme::RSA_PKCS1_SHA512,
    ];
    for e in expected {
        assert!(
            schemes_in_mapping.contains(&e),
            "mapping missing scheme {e:?}"
        );
    }
}
