use super::*;

/// NIST CAVS empty input — SHA-256 known answer.
#[test]
fn sha256_empty_oneshot() {
    let h = SHA256.hash(&[]);
    let expected =
        hex::decode("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855").unwrap();
    assert_eq!(h.as_ref(), expected.as_slice());
}

/// NIST CAVS "abc" — SHA-256 known answer.
#[test]
fn sha256_abc_oneshot() {
    let h = SHA256.hash(b"abc");
    let expected =
        hex::decode("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad").unwrap();
    assert_eq!(h.as_ref(), expected.as_slice());
}

/// Same KAT via Update/Final path — confirms streaming agrees with oneshot.
#[test]
fn sha256_abc_streaming() {
    let mut ctx = SHA256.start();
    ctx.update(b"a");
    ctx.update(b"b");
    ctx.update(b"c");
    let h = ctx.finish();
    let expected =
        hex::decode("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad").unwrap();
    assert_eq!(h.as_ref(), expected.as_slice());
}

/// Fork: take a snapshot mid-stream, advance original, verify snapshot
/// finishes at the earlier state.
#[test]
fn sha256_fork_snapshot() {
    let mut ctx = SHA256.start();
    ctx.update(b"ab");
    let forked_digest = ctx.fork_finish();
    ctx.update(b"c");
    let full_digest = ctx.finish();

    let ab =
        hex::decode("fb8e20fc2e4c3f248c60c39bd652f3c1347298bb977b8b4d5903b85055620603").unwrap();
    let abc =
        hex::decode("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad").unwrap();
    assert_eq!(forked_digest.as_ref(), ab.as_slice());
    assert_eq!(full_digest.as_ref(), abc.as_slice());
}

/// NIST CAVS empty input — SHA-384 known answer.
#[test]
fn sha384_empty_oneshot() {
    let h = SHA384.hash(&[]);
    let expected = hex::decode(
        "38b060a751ac96384cd9327eb1b1e36a21fdb71114be07434c0cc7bf63f6e1da274edebfe76f65fbd51ad2f14898b95b",
    )
    .unwrap();
    assert_eq!(h.as_ref(), expected.as_slice());
}

/// NIST CAVS "abc" — SHA-384 known answer.
#[test]
fn sha384_abc_oneshot() {
    let h = SHA384.hash(b"abc");
    let expected = hex::decode(
        "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7",
    )
    .unwrap();
    assert_eq!(h.as_ref(), expected.as_slice());
}

/// `output_len()` and `algorithm()` are part of rustls's `Hash` trait
/// contract. Validate them by linking to actual digest length — catches
/// drift between accessor and implementation.
#[test]
fn sha256_contract_accessors_match_actual_digest() {
    let digest = SHA256.hash(b"abc");
    assert_eq!(digest.as_ref().len(), SHA256.output_len());
    assert_eq!(SHA256.output_len(), 32);
    assert_eq!(SHA256.algorithm(), HashAlgorithm::SHA256);
}

#[test]
fn sha384_contract_accessors_match_actual_digest() {
    let digest = SHA384.hash(b"abc");
    assert_eq!(digest.as_ref().len(), SHA384.output_len());
    assert_eq!(SHA384.output_len(), 48);
    assert_eq!(SHA384.algorithm(), HashAlgorithm::SHA384);
}

/// Streaming with arbitrary chunking across SHA block boundary (>64 bytes
/// for SHA-256, >128 for SHA-384) must equal one-shot. Catches bugs in
/// chunk handling (off-by-one, alignment, padding boundary).
#[test]
fn sha256_arbitrary_chunking_equals_oneshot() {
    let data: Vec<u8> = (0..=255u8).cycle().take(200).collect(); // > 3 blocks
    let oneshot = SHA256.hash(&data);

    let mut ctx = SHA256.start();
    for chunk in data.chunks(33) {
        // 33 is intentionally non-aligned to 64-byte block size
        ctx.update(chunk);
    }
    let streamed = ctx.finish();
    assert_eq!(oneshot.as_ref(), streamed.as_ref());
}

#[test]
fn sha384_arbitrary_chunking_equals_oneshot() {
    let data: Vec<u8> = (0..=255u8).cycle().take(400).collect(); // > 3 SHA-384 blocks
    let oneshot = SHA384.hash(&data);

    let mut ctx = SHA384.start();
    for chunk in data.chunks(57) {
        ctx.update(chunk);
    }
    let streamed = ctx.finish();
    assert_eq!(oneshot.as_ref(), streamed.as_ref());
}

/// `Context::fork` then advancing both copies independently must yield
/// digests matching their respective separately-built digests. Catches
/// shared-mutable-state bugs in fork.
#[test]
fn sha256_fork_then_diverge_remains_correct() {
    let mut a = SHA256.start();
    a.update(b"prefix");

    let mut b = a.fork();
    a.update(b"-branch-a");
    b.update(b"-branch-b");

    let digest_a = a.finish();
    let digest_b = b.finish();

    let expected_a = SHA256.hash(b"prefix-branch-a");
    let expected_b = SHA256.hash(b"prefix-branch-b");
    assert_eq!(digest_a.as_ref(), expected_a.as_ref());
    assert_eq!(digest_b.as_ref(), expected_b.as_ref());
    assert_ne!(digest_a.as_ref(), digest_b.as_ref());
}

/// C-1 regression: `sha256_update_all` chunks the input by
/// `u32::MAX` bytes. A single Rust slice cannot easily exceed 4 GiB
/// in a CI environment, but we can verify the chunking logic itself
/// by comparing one call with a manually-pre-chunked sequence — the
/// hash must match byte-for-byte. If a future refactor accidentally
/// re-introduced `data.len() as u32`, this would still pass (because
/// our chunks are small); the real protection against >4GiB truncation
/// is the *structural* shape of `sha256_update_all` (no `as u32` over
/// `data.len()`). We test that shape by running the streaming loop
/// against 200 + 333-byte chunks (forcing multiple `Update` calls)
/// and comparing to oneshot.
///
/// For the actual >4GiB case, see the manual-runbook test in
/// `libs/rustls-corecrypto-provider/README.md` (not run in CI).
#[test]
fn sha256_chunked_update_matches_single_call() {
    let data: Vec<u8> = (0..=255u8).cycle().take(2048).collect();
    let oneshot = SHA256.hash(&data);

    // Drive the chunked-update path through Context::update by feeding
    // misaligned chunks.
    let mut ctx = SHA256.start();
    for chunk in data.chunks(333) {
        ctx.update(chunk);
    }
    let streamed = ctx.finish();
    assert_eq!(oneshot.as_ref(), streamed.as_ref());
}

/// SHA-384 streaming agrees with oneshot.
#[test]
fn sha384_abc_streaming() {
    let mut ctx = SHA384.start();
    ctx.update(b"a");
    ctx.update(b"b");
    ctx.update(b"c");
    let h = ctx.finish();
    let expected = hex::decode(
        "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7",
    )
    .unwrap();
    assert_eq!(h.as_ref(), expected.as_slice());
}
