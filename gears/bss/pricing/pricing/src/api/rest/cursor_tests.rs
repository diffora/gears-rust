//! D-125's contract, asserted where it is decided.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use uuid::Uuid;

use super::{DEFAULT_LIMIT, MAX_LIMIT, PageRequest, decode, encode, page_info};
use crate::domain::error::DomainError;

#[test]
fn an_absent_limit_takes_the_server_default() {
    let page = PageRequest::parse(None, None).expect("no parameters is a valid first page");

    assert_eq!(page.limit, DEFAULT_LIMIT);
    assert_eq!(page.after, None);
}

#[test]
fn a_limit_above_the_cap_is_clamped_rather_than_refused() {
    // The cap is a server limit, not a caller mistake: refusing would make every
    // client responsible for knowing a number the server owns.
    let page = PageRequest::parse(Some(5_000), None).expect("an oversized limit is served, capped");

    assert_eq!(page.limit, MAX_LIMIT);
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

    assert!(
        !token.contains(&key.to_string()),
        "a token carrying the key verbatim is a token a caller will construct: {token}"
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
