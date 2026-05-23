use super::*;

/// RFC 5869 test case 1 (HKDF-SHA-256).
/// IKM  = 0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b
/// salt = 000102030405060708090a0b0c
/// info = f0f1f2f3f4f5f6f7f8f9
/// L    = 42
/// OKM  = 3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf
///        34007208d5b887185865
#[test]
fn hkdf_sha256_rfc5869_case1() {
    let ikm = hex::decode("0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b").unwrap();
    let salt = hex::decode("000102030405060708090a0b0c").unwrap();
    let info = hex::decode("f0f1f2f3f4f5f6f7f8f9").unwrap();

    let exp = HKDF_SHA256.extract_from_secret(Some(&salt), &ikm);
    let mut okm = [0u8; 42];
    exp.expand_slice(&[&info], &mut okm).expect("expand");

    let expected = hex::decode(
        "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865",
    )
    .unwrap();
    assert_eq!(okm.as_slice(), expected.as_slice());
}

/// RFC 5869 test case 3 (HKDF-SHA-256, empty salt = treated as zeros, empty info).
/// IKM  = 0b × 22, L = 42
/// OKM  = 8da4e775a563c18f715f802a063c5a31b8a11f5c5ee1879ec3454e5f3c738d2d
///        9d201395faa4b61a96c8
#[test]
fn hkdf_sha256_rfc5869_case3() {
    let ikm = hex::decode("0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b").unwrap();
    let exp = HKDF_SHA256.extract_from_secret(None, &ikm);
    let mut okm = [0u8; 42];
    exp.expand_slice(&[], &mut okm).expect("expand");

    let expected = hex::decode(
        "8da4e775a563c18f715f802a063c5a31b8a11f5c5ee1879ec3454e5f3c738d2d9d201395faa4b61a96c8",
    )
    .unwrap();
    assert_eq!(okm.as_slice(), expected.as_slice());
}

/// HKDF-SHA-384 sanity vector (computed via OpenSSL).
/// IKM=20×0b, salt=13 bytes 00..0c, info=10 bytes f0..f9, L=42.
#[test]
fn hkdf_sha384_basic() {
    let ikm = hex::decode("0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b").unwrap();
    let salt = hex::decode("000102030405060708090a0b0c").unwrap();
    let info = hex::decode("f0f1f2f3f4f5f6f7f8f9").unwrap();

    let exp = HKDF_SHA384.extract_from_secret(Some(&salt), &ikm);
    let mut okm = [0u8; 42];
    exp.expand_slice(&[&info], &mut okm).expect("expand");

    // Reference computed with `openssl kdf -keylen 42 ...` / Python hkdf.
    let expected = hex::decode(
        "9b5097a86038b805309076a44b3a9f38063e25b516dcbf369f394cfab43685f748b6457763e4f0204fc5",
    )
    .unwrap();
    assert_eq!(okm.as_slice(), expected.as_slice());
}

/// Output longer than one hash block must use multiple T(i) iterations.
#[test]
fn hkdf_sha256_multiblock() {
    let exp = HKDF_SHA256.extract_from_secret(Some(b"salt"), b"ikm");
    let mut okm = [0u8; 80]; // ~2.5 blocks
    exp.expand_slice(&[b"info"], &mut okm).expect("expand");

    // The first 32 bytes should equal expand_block().
    let block = HKDF_SHA256
        .extract_from_secret(Some(b"salt"), b"ikm")
        .expand_block(&[b"info"]);
    assert_eq!(&okm[..32], block.as_ref());
}

/// `extract_from_zero_ikm` must equal `extract_from_secret(salt, &zeros)`
/// where zeros has length = hash_len. Catches a bug where the wrong
/// "zero" is passed.
#[test]
fn hkdf_sha256_zero_ikm_matches_explicit_zeros() {
    let exp_zero = HKDF_SHA256.extract_from_zero_ikm(Some(b"salt"));
    let exp_explicit = HKDF_SHA256.extract_from_secret(Some(b"salt"), &[0u8; 32]);
    let mut a = [0u8; 40];
    let mut b = [0u8; 40];
    exp_zero.expand_slice(&[b"info"], &mut a).unwrap();
    exp_explicit.expand_slice(&[b"info"], &mut b).unwrap();
    assert_eq!(a, b);
}

/// `extract_from_zero_ikm(None)` uses zero salt + zero ikm.
/// Catches a bug where None-salt path differs across the two extract
/// methods.
#[test]
fn hkdf_sha384_zero_ikm_no_salt_matches_explicit() {
    let exp_zero = HKDF_SHA384.extract_from_zero_ikm(None);
    let exp_explicit = HKDF_SHA384.extract_from_secret(None, &[0u8; 48]);
    let block_a = exp_zero.expand_block(&[b"info"]);
    let block_b = exp_explicit.expand_block(&[b"info"]);
    assert_eq!(block_a.as_ref(), block_b.as_ref());
}

/// `expander_for_okm` treats the OkmBlock bytes directly as PRK; the
/// result must match a hand-computed first HMAC iteration of HKDF-Expand.
#[test]
fn hkdf_sha256_expander_for_okm_matches_manual_first_iteration() {
    let okm_bytes = [0x42u8; 32];
    let okm = OkmBlock::new(&okm_bytes);
    let exp = HKDF_SHA256.expander_for_okm(&okm);

    let mut out = [0u8; 32];
    exp.expand_slice(&[b"info"], &mut out).unwrap();

    // RFC 5869 §2.3: T(1) = HMAC(PRK, "" || info || 0x01)
    let mut concat = Vec::from(&b"info"[..]);
    concat.push(0x01);
    let expected = HMAC_SHA256.with_key(&okm_bytes).sign(&[&concat]);
    assert_eq!(&out[..], expected.as_ref());
}

/// `hmac_sign(key, msg)` should be the same byte-for-byte as a direct
/// HMAC call with `key.as_ref()` as the key.
#[test]
fn hkdf_sha384_hmac_sign_matches_direct_hmac() {
    let key_bytes = [0xaau8; 48];
    let okm = OkmBlock::new(&key_bytes);
    let tag = HKDF_SHA384.hmac_sign(&okm, b"message");
    let expected = HMAC_SHA384.with_key(&key_bytes).sign(&[b"message"]);
    assert_eq!(tag.as_ref(), expected.as_ref());
}

/// `expand_block` must yield exactly `hash_len` bytes equal to the
/// first hash_len bytes of an `expand_slice` with the same info.
#[test]
fn hkdf_sha384_expand_block_equals_truncated_slice() {
    let block = HKDF_SHA384
        .extract_from_secret(Some(b"salt"), b"ikm")
        .expand_block(&[b"info"]);
    let mut slice_out = [0u8; 48];
    HKDF_SHA384
        .extract_from_secret(Some(b"salt"), b"ikm")
        .expand_slice(&[b"info"], &mut slice_out)
        .unwrap();
    assert_eq!(block.as_ref(), &slice_out[..]);
    assert_eq!(block.as_ref().len(), 48);
}

/// `hash_len()` accessor must equal the actual block length produced by
/// `expand_block` — contract verification.
#[test]
fn hkdf_hash_len_accessors_match_block_size() {
    let exp_256 = HKDF_SHA256.extract_from_zero_ikm(None);
    assert_eq!(
        exp_256.hash_len(),
        exp_256.expand_block(&[b"x"]).as_ref().len()
    );
    assert_eq!(exp_256.hash_len(), 32);

    let exp_384 = HKDF_SHA384.extract_from_zero_ikm(None);
    assert_eq!(
        exp_384.hash_len(),
        exp_384.expand_block(&[b"x"]).as_ref().len()
    );
    assert_eq!(exp_384.hash_len(), 48);
}

/// rustls passes multi-chunk info as `&[&[u8]]`; semantically this must
/// equal calling expand_slice with a single concatenated chunk.
#[test]
fn hkdf_sha256_multi_chunk_info_matches_concatenated() {
    let mut multi = [0u8; 40];
    HKDF_SHA256
        .extract_from_secret(Some(b"salt"), b"ikm")
        .expand_slice(&[b"part-a", b"part-b", b"part-c"], &mut multi)
        .unwrap();
    let mut concat = [0u8; 40];
    HKDF_SHA256
        .extract_from_secret(Some(b"salt"), b"ikm")
        .expand_slice(&[b"part-apart-bpart-c"], &mut concat)
        .unwrap();
    assert_eq!(multi, concat);
}

/// FIPS-claim contract.
#[test]
fn hkdf_advertises_fips() {
    assert!(HKDF_SHA256.fips());
    assert!(HKDF_SHA384.fips());
}

/// L > 255 · HashLen → error.
#[test]
fn hkdf_sha256_too_long() {
    let exp = HKDF_SHA256.extract_from_secret(Some(b"salt"), b"ikm");
    let mut okm = vec![0u8; 255 * 32 + 1];
    assert!(exp.expand_slice(&[b"info"], &mut okm).is_err());
}
