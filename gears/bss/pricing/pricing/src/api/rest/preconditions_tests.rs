//! Each case names the defect it closes rather than the function it calls.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use axum::http::{HeaderMap, HeaderValue};
use serde::Serialize;
use std::collections::BTreeMap;

use super::{IDEMPOTENCY_KEY, etag, idempotency_key, if_match, request_digest};
use crate::domain::concurrency::RowVersion;
use crate::domain::error::DomainError;

fn headers_with(name: &'static str, value: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(name, HeaderValue::from_str(value).expect("header value"));
    headers
}

fn refusal(err: &DomainError) -> &str {
    match err {
        DomainError::InvalidRequest(detail) => detail,
        other => panic!("expected a malformed-request refusal, got {other:?}"),
    }
}

#[test]
fn a_tag_round_trips_through_the_header_it_is_carried_in() {
    // The whole point of emitting an `ETag`: what a caller reads back has to be
    // submittable verbatim on the next mutating verb. A rendering and a parse
    // that disagreed would make every precondition unsatisfiable.
    let version = RowVersion::new(7);
    let headers = headers_with("if-match", &etag(version));

    assert_eq!(if_match(&headers).expect("the tag parses"), version);
}

#[test]
fn an_absent_if_match_is_a_malformed_request_and_mints_no_code() {
    // D-141: "an absent precondition is a malformed request under the Foundation
    // validation envelope, so no new code is minted." The refusal has to be
    // InvalidRequest, and it has to name the header so the caller knows what to
    // add.
    let err = if_match(&HeaderMap::new()).expect_err("an absent precondition is refused");

    assert!(refusal(&err).contains("If-Match"), "{err:?}");
}

#[test]
fn the_wildcard_is_refused_because_it_is_an_unconditional_write() {
    // `*` means "if the resource exists at all", i.e. overwrite whichever
    // version is current — exactly what D-141 and D-145 exist to prevent.
    let err = if_match(&headers_with("if-match", "*")).expect_err("the wildcard is refused");

    assert!(refusal(&err).contains("wildcard"), "{err:?}");
}

#[test]
fn a_weak_validator_and_a_list_are_both_refused() {
    let weak = if_match(&headers_with("if-match", "W/\"7\"")).expect_err("weak is refused");
    let list = if_match(&headers_with("if-match", "\"7\", \"8\"")).expect_err("a list is refused");

    assert!(matches!(weak, DomainError::InvalidRequest(_)), "{weak:?}");
    assert!(matches!(list, DomainError::InvalidRequest(_)), "{list:?}");
}

#[test]
fn garbage_in_the_precondition_is_refused_rather_than_coerced() {
    for raw in ["7", "\"\"", "\"-1\"", "\"seven\"", ""] {
        let err = if_match(&headers_with("if-match", raw))
            .expect_err("only one strong quoted decimal is a tag");
        assert!(
            matches!(err, DomainError::InvalidRequest(_)),
            "{raw:?} -> {err:?}"
        );
    }
}

#[test]
fn an_absent_idempotency_key_is_refused_on_a_guarded_create() {
    // The decision made once here rather than per route: S2/S3 §5 name the cell
    // *client idempotency key* and §4.2 makes the gate the first step, so an
    // unguarded create is not an option the surface offers.
    let err = idempotency_key(&HeaderMap::new()).expect_err("the guarded create requires a key");

    assert!(refusal(&err).contains("Idempotency-Key"), "{err:?}");
}

#[test]
fn a_key_is_bounded_and_printable() {
    let long = "k".repeat(256);
    for raw in ["", long.as_str(), "with\tcontrol"] {
        let err = idempotency_key(&headers_with(IDEMPOTENCY_KEY, raw))
            .expect_err("the key is stored and echoed, so its shape is bounded");
        assert!(
            matches!(err, DomainError::InvalidRequest(_)),
            "{raw:?} -> {err:?}"
        );
    }
    // A byte a header may carry (RFC 9110 obs-text) and UTF-8 may not: the
    // refusal comes from the decode rather than from the charset check, and
    // both arms have to answer, or a caller meets a panic instead of a 400.
    let mut binary = HeaderMap::new();
    binary.insert(
        IDEMPOTENCY_KEY,
        HeaderValue::from_bytes(&[0xffu8]).expect("obs-text is a legal header byte"),
    );
    assert!(
        matches!(
            idempotency_key(&binary).expect_err("a non-UTF-8 key is refused"),
            DomainError::InvalidRequest(_)
        ),
        "a header this gear cannot read back is a malformed request, not a panic"
    );
    assert_eq!(
        idempotency_key(&headers_with(
            IDEMPOTENCY_KEY,
            "3f2504e0-4f89-11d3-9a0c-0305e82c3301"
        ))
        .expect("an ordinary client key is accepted"),
        "3f2504e0-4f89-11d3-9a0c-0305e82c3301"
    );
}

#[derive(Serialize)]
struct Sample {
    name: String,
    tags: BTreeMap<String, String>,
}

#[test]
fn two_requests_that_differ_only_in_member_order_digest_identically() {
    // The reason the digest is over the PARSED request rather than the raw body:
    // a retry through a different client serializes the same intent in a
    // different order, and hashing bytes would answer it
    // IDEMPOTENCY_PAYLOAD_MISMATCH — the refusal spent on a caller who did
    // nothing wrong.
    let first = Sample {
        name: "starter".to_owned(),
        tags: BTreeMap::from([
            ("b".to_owned(), "2".to_owned()),
            ("a".to_owned(), "1".to_owned()),
        ]),
    };
    let second = Sample {
        name: "starter".to_owned(),
        tags: BTreeMap::from([
            ("a".to_owned(), "1".to_owned()),
            ("b".to_owned(), "2".to_owned()),
        ]),
    };

    assert_eq!(
        request_digest(&first).expect("digest"),
        request_digest(&second).expect("digest"),
        "a BTreeMap serializes in key order, so insertion order cannot move the digest"
    );
}

#[test]
fn two_requests_that_differ_in_a_value_digest_differently() {
    let first = Sample {
        name: "starter".to_owned(),
        tags: BTreeMap::new(),
    };
    let second = Sample {
        name: "pro".to_owned(),
        tags: BTreeMap::new(),
    };

    assert_ne!(
        request_digest(&first).expect("digest"),
        request_digest(&second).expect("digest"),
        "a different intent under one key must be refused, so it must digest differently"
    );
}
