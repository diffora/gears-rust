//! D-125's contract, asserted where it is decided.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use uuid::Uuid;

use super::{DEFAULT_LIMIT, MAX_LIMIT, PageRequest, decode, encode, page_info};
use crate::domain::error::DomainError;

/// **D-125's two numbers, pinned to their numbers.**
///
/// Every assertion in this file compared against `DEFAULT_LIMIT` and `MAX_LIMIT`
/// imported from the module under test, so changing either constant kept the whole
/// file green while breaking the decision it implements. The one place in the
/// eleven module suites that pins a spec constant to a literal is
/// `threshold_policy_tests`' `10001`, and it is right; this is the same discipline.
#[test]
fn the_page_numbers_are_the_ones_the_decision_names() {
    assert_eq!(DEFAULT_LIMIT, 100, "D-125's server default");
    assert_eq!(
        MAX_LIMIT, 1_000,
        "D-125's hard cap, the unit the export SLO is in"
    );
}

#[test]
fn an_absent_limit_takes_the_server_default() {
    let page = PageRequest::parse(None, None).expect("no parameters is a valid first page");

    assert_eq!(
        page.limit, 100,
        "the literal, not the constant it came from"
    );
    assert_eq!(page.after, None);
}

#[test]
fn a_limit_above_the_cap_is_clamped_rather_than_refused() {
    // The cap is a server limit, not a caller mistake: refusing would make every
    // client responsible for knowing a number the server owns.
    let page = PageRequest::parse(Some(5_000), None).expect("an oversized limit is served, capped");

    assert_eq!(
        page.limit, 1_000,
        "the literal, not the constant it came from"
    );
}

#[test]
fn a_zero_limit_is_refused_because_it_never_advances() {
    let err = PageRequest::parse(Some(0), None).expect_err("a page of no rows is not a page");

    assert!(matches!(err, DomainError::InvalidRequest(_)), "{err:?}");
}

#[test]
fn a_cursor_round_trips_and_stays_opaque() {
    let key = Uuid::now_v7();
    let token = encode(key);

    // **Opacity, asserted so it can fail.** This read `!token.contains(&key.to_string())`
    // until 2026-08-18, which no implementation can violate: the token is URL-safe
    // base64 of 16 raw bytes, 22 characters, and a uuid's text form is 36 — a
    // shorter string cannot contain a longer one, for any encoding including a
    // broken one. What opacity actually means here is that the token is *not* the
    // key's own text form, so both halves are asserted: the length the encoding
    // fixes, and the refusal of the text form a caller would otherwise construct.
    assert_eq!(token.len(), 22, "URL-safe base64 of 16 raw bytes: {token}");
    assert!(
        decode(&key.to_string()).is_err(),
        "the key's own text form is not a token this surface issued, or a caller who \
         skipped the cursor entirely would page successfully"
    );
    assert_eq!(decode(&token).expect("the token this surface issued"), key);
    assert_eq!(
        PageRequest::parse(None, Some(&token))
            .expect("a page resumes at its cursor")
            .after,
        Some(key)
    );
}

#[test]
fn an_undecodable_cursor_is_a_malformed_request_and_mints_no_code() {
    for raw in ["not-base64!!", "", "aGVsbG8"] {
        let err = decode(raw).expect_err("only a token this surface issued decodes");
        assert!(
            matches!(err, DomainError::InvalidRequest(_)),
            "{raw:?} -> {err:?}"
        );
    }
}

#[test]
fn the_envelope_says_null_forward_when_the_result_is_exhausted() {
    // A client must be able to stop WITHOUT issuing the extra request that
    // returns an empty page - which is what `next_cursor` on every page until
    // exhaustion means (D-125).
    let last = page_info(None, DEFAULT_LIMIT);
    assert!(last.next_cursor.is_none());
    assert_eq!(last.limit, DEFAULT_LIMIT);

    let more = page_info(Some(Uuid::now_v7()), 10);
    assert!(more.next_cursor.is_some());

    // Backwards is not offered, and the field says so rather than carrying a
    // token that sometimes works.
    assert!(last.prev_cursor.is_none());
    assert!(more.prev_cursor.is_none());
}
