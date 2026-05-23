use super::*;

/// `start()` produces an uncompressed-point public key of the right size:
/// 0x04 || X(32) || Y(32) for P-256.
#[test]
fn p256_start_yields_uncompressed_point() {
    let kx = SECP256R1.start().expect("start");
    assert_eq!(kx.group(), NamedGroup::secp256r1);
    let pk = kx.pub_key();
    assert_eq!(pk.len(), 65);
    assert_eq!(pk[0], 0x04, "uncompressed point prefix");
}

#[test]
fn p384_start_yields_uncompressed_point() {
    let kx = SECP384R1.start().expect("start");
    assert_eq!(kx.group(), NamedGroup::secp384r1);
    let pk = kx.pub_key();
    assert_eq!(pk.len(), 97);
    assert_eq!(pk[0], 0x04);
}

/// ECDH commutativity: each side computing complete() with the other's
/// public key MUST produce identical shared secrets. This is the
/// behavioural contract that makes ECDHE work.
fn ecdh_commutativity_for(group: &dyn SupportedKxGroup, expected_len: usize) {
    let a = group.start().expect("a start");
    let b = group.start().expect("b start");
    let a_pub = a.pub_key().to_vec();
    let b_pub = b.pub_key().to_vec();

    let secret_a = a.complete(&b_pub).expect("a.complete(b_pub)");
    let secret_b = b.complete(&a_pub).expect("b.complete(a_pub)");
    assert_eq!(secret_a.secret_bytes(), secret_b.secret_bytes());
    assert_eq!(secret_a.secret_bytes().len(), expected_len);
    // Two independent ECDH runs must not produce all-zero shared
    // secrets (probability ~2^-256).
    assert!(secret_a.secret_bytes().iter().any(|&b| b != 0));
}

#[test]
fn p256_ecdh_is_commutative() {
    ecdh_commutativity_for(&SECP256R1, 32);
}

#[test]
fn p384_ecdh_is_commutative() {
    ecdh_commutativity_for(&SECP384R1, 48);
}

/// Two fresh `start()` calls must produce DIFFERENT key pairs — catches
/// a deterministic-RNG bug that would make ephemeral keys reusable.
#[test]
fn p256_start_produces_unique_keys() {
    let a = SECP256R1.start().unwrap();
    let b = SECP256R1.start().unwrap();
    assert_ne!(a.pub_key(), b.pub_key());
}

/// Independent ECDH sessions must produce DIFFERENT shared secrets.
/// Catches a bug where private state leaks across sessions.
#[test]
fn p256_independent_sessions_diverge() {
    let alice = SECP256R1.start().unwrap();
    let bob = SECP256R1.start().unwrap();
    let bob_pub = bob.pub_key().to_vec();
    let secret_ab = alice.complete(&bob_pub).unwrap();

    let alice2 = SECP256R1.start().unwrap();
    let bob2 = SECP256R1.start().unwrap();
    let bob2_pub = bob2.pub_key().to_vec();
    let secret_ab2 = alice2.complete(&bob2_pub).unwrap();

    assert_ne!(secret_ab.secret_bytes(), secret_ab2.secret_bytes());
}

/// Malformed peer pub key (wrong length) must be rejected before the
/// FFI import is attempted.
#[test]
fn p256_rejects_wrong_length_peer_key() {
    let kx = SECP256R1.start().unwrap();
    let bad = vec![0u8; 64]; // missing the 0x04 prefix byte
    match kx.complete(&bad) {
        Err(Error::General(_)) => {}
        Err(other) => panic!("expected General error, got {other:?}"),
        Ok(_) => panic!("expected an error, got Ok(_)"),
    }
}

/// Malformed peer pub key (wrong prefix byte = compressed format) must
/// be rejected — we only accept uncompressed.
#[test]
fn p256_rejects_compressed_peer_key() {
    let kx = SECP256R1.start().unwrap();
    let mut bad = vec![0u8; 65];
    bad[0] = 0x02; // compressed-point prefix, unsupported here
    match kx.complete(&bad) {
        Err(Error::General(_)) => {}
        Err(other) => panic!("expected General error, got {other:?}"),
        Ok(_) => panic!("expected an error, got Ok(_)"),
    }
}

/// Peer key that has the right shape but is mathematically invalid
/// (not on the curve) must be rejected by Apple's import path.
#[test]
fn p256_rejects_point_not_on_curve() {
    let kx = SECP256R1.start().unwrap();
    // 0x04 || 0..0 || 0..0 is not on P-256.
    let mut bad = vec![0u8; 65];
    bad[0] = 0x04;
    match kx.complete(&bad) {
        Err(Error::General(_)) => {}
        Err(other) => panic!("expected General error, got {other:?}"),
        Ok(_) => panic!("expected an error, got Ok(_)"),
    }
}

/// `name()` and `fips()` constants are part of rustls's
/// `SupportedKxGroup` trait contract.
///
/// The two `fips()` assertions delegate to
/// [`crate::oe::fips_witness_ok`] under the witness rework, so they
/// pass **only on a host whose macOS major is inside**
/// [`crate::oe::SUPPORTED_OE_MACOS_MAJOR`]. If a failure here
/// surprises you, check `crate::oe::tests::fips_witness_ok_on_supported_host`
/// first — it is the canonical attribution test for OE drift.
#[test]
fn accessor_contracts() {
    assert_eq!(SECP256R1.name(), NamedGroup::secp256r1);
    assert_eq!(SECP384R1.name(), NamedGroup::secp384r1);
    // Shape-of-config the kx layer actually controls.
    // The `fips()` calls below merely re-assert the runtime witness;
    // they are tested in `crate::oe::tests` and are not the
    // responsibility of the kx layer per se.
    assert!(SECP256R1.fips());
    assert!(SECP384R1.fips());
}
