use super::*;

/// NIST GCM CAVS Test Case 13 (AES-128-GCM, 96-bit IV).
/// Key:   feffe9928665731c6d6a8f9467308308
/// IV:    cafebabefacedbaddecaf888
/// PT:    d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a721c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b39
/// AAD:   feedfacedeadbeeffeedfacedeadbeefabaddad2
/// CT:    42831ec2217774244b7221b784d0d49ce3aa212f2c02a4e035c17e2329aca12e21d514b25466931c7d8f6a5aac84aa051ba30b396a0aac973d58e091
/// Tag:   5bc94fbc3221a5db94fae95ae7121a47
#[test]
fn aes128_gcm_nist_case13() {
    let key = hex::decode("feffe9928665731c6d6a8f9467308308").unwrap();
    let iv = hex::decode("cafebabefacedbaddecaf888").unwrap();
    let aad = hex::decode("feedfacedeadbeeffeedfacedeadbeefabaddad2").unwrap();
    let pt = hex::decode(
        "d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a721c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b39",
    )
    .unwrap();
    let ct_expected = hex::decode(
        "42831ec2217774244b7221b784d0d49ce3aa212f2c02a4e035c17e2329aca12e21d514b25466931c7d8f6a5aac84aa051ba30b396a0aac973d58e091",
    )
    .unwrap();
    let tag_expected = hex::decode("5bc94fbc3221a5db94fae95ae7121a47").unwrap();

    let mut ct = vec![0u8; pt.len()];
    let tag = encrypt(&key, &iv, &aad, &pt, &mut ct).expect("encrypt");
    assert_eq!(ct, ct_expected);
    assert_eq!(tag.as_slice(), tag_expected.as_slice());

    // Roundtrip
    let mut pt_back = vec![0u8; ct.len()];
    let mut tag_arr = [0u8; TAG_LEN];
    tag_arr.copy_from_slice(&tag_expected);
    decrypt(&key, &iv, &aad, &ct, &mut pt_back, &tag_arr).expect("decrypt");
    assert_eq!(pt_back, pt);
}

/// NIST GCM CAVS Test Case 16 (AES-256-GCM, 96-bit IV).
/// Key:   feffe9928665731c6d6a8f9467308308feffe9928665731c6d6a8f9467308308
/// IV:    cafebabefacedbaddecaf888
/// PT:    d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a721c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b39
/// AAD:   feedfacedeadbeeffeedfacedeadbeefabaddad2
/// CT:    522dc1f099567d07f47f37a32a84427d643a8cdcbfe5c0c97598a2bd2555d1aa8cb08e48590dbb3da7b08b1056828838c5f61e6393ba7a0abcc9f662
/// Tag:   76fc6ece0f4e1768cddf8853bb2d551b
#[test]
fn aes256_gcm_nist_case16() {
    let key =
        hex::decode("feffe9928665731c6d6a8f9467308308feffe9928665731c6d6a8f9467308308").unwrap();
    let iv = hex::decode("cafebabefacedbaddecaf888").unwrap();
    let aad = hex::decode("feedfacedeadbeeffeedfacedeadbeefabaddad2").unwrap();
    let pt = hex::decode(
        "d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a721c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b39",
    )
    .unwrap();
    let ct_expected = hex::decode(
        "522dc1f099567d07f47f37a32a84427d643a8cdcbfe5c0c97598a2bd2555d1aa8cb08e48590dbb3da7b08b1056828838c5f61e6393ba7a0abcc9f662",
    )
    .unwrap();
    let tag_expected = hex::decode("76fc6ece0f4e1768cddf8853bb2d551b").unwrap();

    let mut ct = vec![0u8; pt.len()];
    let tag = encrypt(&key, &iv, &aad, &pt, &mut ct).expect("encrypt");
    assert_eq!(ct, ct_expected);
    assert_eq!(tag.as_slice(), tag_expected.as_slice());

    let mut pt_back = vec![0u8; ct.len()];
    let mut tag_arr = [0u8; TAG_LEN];
    tag_arr.copy_from_slice(&tag_expected);
    decrypt(&key, &iv, &aad, &ct, &mut pt_back, &tag_arr).expect("decrypt");
    assert_eq!(pt_back, pt);
}

/// Empty plaintext + empty AAD case (NIST GCM Test Case 1, AES-128).
/// Key=00..., IV=00..., Tag=58e2fccefa7e3061367f1d57a4e7455a.
#[test]
fn aes128_gcm_nist_case1_empty() {
    let key = [0u8; 16];
    let iv = [0u8; 12];
    let aad: &[u8] = &[];
    let pt: &[u8] = &[];
    let mut ct = [0u8; 0];
    let tag = encrypt(&key, &iv, aad, pt, &mut ct).expect("encrypt");
    let expected = hex::decode("58e2fccefa7e3061367f1d57a4e7455a").unwrap();
    assert_eq!(tag.as_slice(), expected.as_slice());

    let mut pt_back = [0u8; 0];
    let mut tag_arr = [0u8; TAG_LEN];
    tag_arr.copy_from_slice(&expected);
    decrypt(&key, &iv, aad, &ct, &mut pt_back, &tag_arr).expect("decrypt");
}

/// Tampered ciphertext must fail authentication.
#[test]
fn aes128_gcm_tampered_ct_fails() {
    let key = [0x11u8; 16];
    let iv = [0x22u8; 12];
    let pt = b"hello world";
    let aad = b"context";

    let mut ct = vec![0u8; pt.len()];
    let tag = encrypt(&key, &iv, aad, pt, &mut ct).expect("encrypt");

    // Flip one bit in ciphertext.
    ct[0] ^= 0x01;
    let mut pt_back = vec![0u8; ct.len()];
    let err = decrypt(&key, &iv, aad, &ct, &mut pt_back, &tag).unwrap_err();
    assert!(matches!(err, AeadError::TagMismatch), "got {err:?}");
    // Speculatively-decrypted output must be wiped.
    assert!(pt_back.iter().all(|&b| b == 0));
}

/// Tampered tag must fail authentication.
#[test]
fn aes128_gcm_tampered_tag_fails() {
    let key = [0x11u8; 16];
    let iv = [0x22u8; 12];
    let pt = b"hello world";
    let aad: &[u8] = &[];

    let mut ct = vec![0u8; pt.len()];
    let mut tag = encrypt(&key, &iv, aad, pt, &mut ct).expect("encrypt");

    tag[0] ^= 0xff;
    let mut pt_back = vec![0u8; ct.len()];
    let err = decrypt(&key, &iv, aad, &ct, &mut pt_back, &tag).unwrap_err();
    assert!(matches!(err, AeadError::TagMismatch), "got {err:?}");
}

/// Tampered AAD must fail authentication.
#[test]
fn aes128_gcm_tampered_aad_fails() {
    let key = [0x11u8; 16];
    let iv = [0x22u8; 12];
    let pt = b"hello world";

    let mut ct = vec![0u8; pt.len()];
    let tag = encrypt(&key, &iv, b"correct-aad", pt, &mut ct).expect("encrypt");

    let mut pt_back = vec![0u8; ct.len()];
    let err = decrypt(&key, &iv, b"wrong-aad", &ct, &mut pt_back, &tag).unwrap_err();
    assert!(matches!(err, AeadError::TagMismatch), "got {err:?}");
}

/// Invalid key length must error before touching CommonCrypto.
#[test]
fn aes_gcm_invalid_key_len() {
    let key = [0u8; 24]; // AES-192 not supported (TLS doesn't use it)
    let iv = [0u8; 12];
    let mut ct = [0u8; 0];
    let err = encrypt(&key, &iv, &[], &[], &mut ct).unwrap_err();
    assert!(matches!(err, AeadError::InvalidKeyLen(24)), "got {err:?}");
}

/// Decryption with the wrong key must fail tag verification (not return
/// garbled plaintext). This is the core AEAD authenticity guarantee.
#[test]
fn aes128_gcm_wrong_key_fails_auth() {
    let key_enc = [0x11u8; 16];
    let key_dec = [0x22u8; 16];
    let iv = [0x33u8; 12];
    let pt = b"hello world";

    let mut ct = vec![0u8; pt.len()];
    let tag = encrypt(&key_enc, &iv, &[], pt, &mut ct).expect("encrypt");

    let mut pt_back = vec![0u8; ct.len()];
    let err = decrypt(&key_dec, &iv, &[], &ct, &mut pt_back, &tag).unwrap_err();
    assert!(matches!(err, AeadError::TagMismatch));
    // Speculative output wiped.
    assert!(pt_back.iter().all(|&b| b == 0));
}

/// Decryption with the wrong IV must fail tag verification.
#[test]
fn aes128_gcm_wrong_iv_fails_auth() {
    let key = [0x11u8; 16];
    let iv_enc = [0x33u8; 12];
    let iv_dec = [0x44u8; 12];
    let pt = b"hello world";

    let mut ct = vec![0u8; pt.len()];
    let tag = encrypt(&key, &iv_enc, &[], pt, &mut ct).expect("encrypt");

    let mut pt_back = vec![0u8; ct.len()];
    let err = decrypt(&key, &iv_dec, &[], &ct, &mut pt_back, &tag).unwrap_err();
    assert!(matches!(err, AeadError::TagMismatch));
}

/// `output_too_small` is enforced *before* any FFI call. Catches a bug
/// where the size check is removed and CommonCrypto writes out of bounds.
#[test]
fn aes_gcm_output_too_small_errors_early() {
    let mut ct = vec![0u8; 5];
    let err = encrypt(&[0u8; 16], &[0u8; 12], &[], b"hello world", &mut ct)
        .expect_err("must reject undersized output");
    match err {
        AeadError::OutputTooSmall { needed, have } => {
            assert_eq!(needed, 11);
            assert_eq!(have, 5);
        }
        other => panic!("expected OutputTooSmall, got {other:?}"),
    }
    // The undersized buffer must NOT be touched by CommonCrypto.
    assert!(ct.iter().all(|&b| b == 0));
}

/// AAD-only authenticate-only-no-encrypt: empty plaintext, non-empty AAD.
/// Tag must depend on AAD; modifying AAD on decrypt must fail.
#[test]
fn aes128_gcm_aad_only_roundtrip_and_aad_dependency() {
    let key = [0x55u8; 16];
    let iv = [0x66u8; 12];
    let aad = b"authenticated-only-data";
    let mut ct = [0u8; 0];

    let tag = encrypt(&key, &iv, aad, &[], &mut ct).expect("encrypt");

    // Verify with correct AAD.
    let mut pt_back = [0u8; 0];
    decrypt(&key, &iv, aad, &ct, &mut pt_back, &tag).expect("decrypt");

    // Tampering AAD must break auth.
    let err = decrypt(&key, &iv, b"different-aad", &ct, &mut pt_back, &tag).unwrap_err();
    assert!(matches!(err, AeadError::TagMismatch));
}

/// Multi-block plaintext (1 KiB across 64 AES blocks) must roundtrip
/// intact. Exercises CCCryptorUpdate's internal chunking and confirms
/// that ciphertext is genuinely different from plaintext.
#[test]
fn aes256_gcm_long_plaintext_roundtrip() {
    let key = [0u8; 32];
    let iv = [0u8; 12];
    let pt: Vec<u8> = (0..=255u8).cycle().take(1024).collect();
    let aad = b"context";

    let mut ct = vec![0u8; pt.len()];
    let tag = encrypt(&key, &iv, aad, &pt, &mut ct).expect("encrypt");
    assert_ne!(ct, pt, "ciphertext must differ from plaintext");

    let mut pt_back = vec![0u8; ct.len()];
    decrypt(&key, &iv, aad, &ct, &mut pt_back, &tag).expect("decrypt");
    assert_eq!(pt, pt_back);
}

/// Confidentiality with empty AAD: same plaintext under two different
/// keys must produce different ciphertexts.
#[test]
fn aes128_gcm_different_keys_give_different_ciphertexts() {
    let iv = [0u8; 12];
    let pt = b"shared plaintext";
    let mut ct_a = vec![0u8; pt.len()];
    let mut ct_b = vec![0u8; pt.len()];
    encrypt(&[0x11u8; 16], &iv, &[], pt, &mut ct_a).expect("a");
    encrypt(&[0x22u8; 16], &iv, &[], pt, &mut ct_b).expect("b");
    assert_ne!(ct_a, ct_b);
}

/// Invalid nonce length must error.
#[test]
fn aes_gcm_invalid_nonce_len() {
    let key = [0u8; 16];
    let iv = [0u8; 13];
    let mut ct = [0u8; 0];
    let err = encrypt(&key, &iv, &[], &[], &mut ct).unwrap_err();
    assert!(matches!(err, AeadError::InvalidNonceLen(13)), "got {err:?}");
}
