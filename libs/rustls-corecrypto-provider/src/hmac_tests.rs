use super::*;

/// RFC 4231 test case 1 — HMAC-SHA-256 with 20×0x0b key, "Hi There".
#[test]
fn hmac_sha256_rfc4231_case1() {
    let key = vec![0x0bu8; 20];
    let data = b"Hi There";
    let tag = HMAC_SHA256.with_key(&key).sign(&[data]);
    let expected =
        hex::decode("b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7").unwrap();
    assert_eq!(tag.as_ref(), expected.as_slice());
}

/// RFC 4231 test case 2 — "Jefe" key, "what do ya want for nothing?".
#[test]
fn hmac_sha256_rfc4231_case2() {
    let key = b"Jefe";
    let data = b"what do ya want for nothing?";
    let tag = HMAC_SHA256.with_key(key).sign(&[data]);
    let expected =
        hex::decode("5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843").unwrap();
    assert_eq!(tag.as_ref(), expected.as_slice());
}

/// RFC 4231 test case 1 — HMAC-SHA-384, 20×0x0b key, "Hi There".
#[test]
fn hmac_sha384_rfc4231_case1() {
    let key = vec![0x0bu8; 20];
    let data = b"Hi There";
    let tag = HMAC_SHA384.with_key(&key).sign(&[data]);
    let expected = hex::decode(
        "afd03944d84895626b0825f4ab46907f15f9dadbe4101ec682aa034c7cebc59cfaea9ea9076ede7f4af152e8b2fa9cb6",
    )
    .unwrap();
    assert_eq!(tag.as_ref(), expected.as_slice());
}

/// Multi-chunk update should match single-shot.
#[test]
fn hmac_sha256_multichunk_matches_oneshot() {
    let key = vec![0x42u8; 32];
    let single = HMAC_SHA256.with_key(&key).sign(&[b"hello world"]);
    let chunked = HMAC_SHA256.with_key(&key).sign(&[b"hel", b"lo ", b"world"]);
    assert_eq!(single.as_ref(), chunked.as_ref());
}

/// `sign_concat(first, middle, last)` should match `sign([first || middle || last])`.
#[test]
fn hmac_sha256_sign_concat() {
    let key = vec![0x42u8; 32];
    let direct = HMAC_SHA256.with_key(&key).sign(&[b"prefix-body-suffix"]);
    let concat = HMAC_SHA256
        .with_key(&key)
        .sign_concat(b"prefix-", &[b"body"], b"-suffix");
    assert_eq!(direct.as_ref(), concat.as_ref());
}

/// `hash_output_len()` and `tag_len()` are part of rustls's `Hmac` /
/// `Key` trait contract. Validate against actual tag length.
#[test]
fn hmac_sha256_contract_accessors_match_actual_tag() {
    let key = HMAC_SHA256.with_key(b"k");
    let tag = key.sign(&[b"data"]);
    assert_eq!(tag.as_ref().len(), key.tag_len());
    assert_eq!(key.tag_len(), 32);
    assert_eq!(HMAC_SHA256.hash_output_len(), 32);
}

#[test]
fn hmac_sha384_contract_accessors_match_actual_tag() {
    let key = HMAC_SHA384.with_key(b"k");
    let tag = key.sign(&[b"data"]);
    assert_eq!(tag.as_ref().len(), key.tag_len());
    assert_eq!(key.tag_len(), 48);
    assert_eq!(HMAC_SHA384.hash_output_len(), 48);
}

/// Different keys must produce different MACs over the same message.
/// A bug that ignored the key would silently pass other vector tests.
#[test]
fn hmac_sha256_different_keys_diverge() {
    let a = HMAC_SHA256.with_key(b"key-a").sign(&[b"message"]);
    let b = HMAC_SHA256.with_key(b"key-b").sign(&[b"message"]);
    assert_ne!(a.as_ref(), b.as_ref());
}

/// HMAC of empty data must not panic and must be deterministic.
/// Verified against the value produced by the same key on a different
/// invocation — catches accidental nondeterminism (e.g. uninit memory).
#[test]
fn hmac_sha256_empty_data_is_deterministic() {
    let key = HMAC_SHA256.with_key(b"some-key");
    let a = key.sign(&[]);
    let b = key.sign(&[b""]);
    assert_eq!(a.as_ref(), b.as_ref());
    assert_eq!(a.as_ref().len(), 32);
}

/// FIPS-claim contract.
#[test]
fn hmac_advertises_fips() {
    assert!(HMAC_SHA256.fips());
    assert!(HMAC_SHA384.fips());
}

/// Reusing the same `Key` for two signs must produce identical results.
#[test]
fn hmac_sha256_reuse_key() {
    let key = vec![0x42u8; 32];
    let mac = HMAC_SHA256.with_key(&key);
    let a = mac.sign(&[b"data"]);
    let b = mac.sign(&[b"data"]);
    assert_eq!(a.as_ref(), b.as_ref());
}

/// **Test-gap #1.** Concurrent signing with one `Arc<dyn Key>`: 16
/// threads × 64 independent `sign()` calls each, all producing the
/// same tag as a single-thread reference (since the message bytes
/// and key are identical). Pins the auto-`Send + Sync` shape of
/// `HmacKey` — `sign_concat` takes `&self` and clones
/// `self.template` into a local before any C-side mutation, so
/// concurrent readers of `self.template` must be sound.
#[test]
fn hmac_concurrent_sign_with_shared_key() {
    use std::sync::Arc;
    use std::thread;

    let key_bytes = vec![0x77u8; 32];
    let key: Arc<dyn Key> = HMAC_SHA256.with_key(&key_bytes).into();

    let msg: &[u8] = b"concurrent-hmac-message";
    let reference = HMAC_SHA256.with_key(&key_bytes).sign(&[msg]);
    let reference_bytes = reference.as_ref().to_vec();

    let mut handles = Vec::new();
    for _ in 0..16 {
        let key = Arc::clone(&key);
        let expected = reference_bytes.clone();
        handles.push(thread::spawn(move || {
            for _ in 0..64 {
                let tag = key.sign(&[msg]);
                assert_eq!(tag.as_ref(), expected.as_slice());
            }
        }));
    }
    for h in handles {
        h.join().expect("hmac thread did not panic");
    }
}
