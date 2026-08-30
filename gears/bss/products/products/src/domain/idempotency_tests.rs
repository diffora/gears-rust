//! Tests for the payload digest an idempotency claim is taken against.
//!
//! The digest is a **cross-attempt** contract: the same act, sent twice by a
//! client that re-serialised its `JSON` in between, must produce the same
//! bytes, and two different acts must not. Every case below asserts the
//! bytes or the rendering that produces them, never a predicate over them —
//! a test that only checked "two digests are equal" would pass just as
//! happily against a function that returned a constant.
//!
//! Header exclusion is measured at the door, not here: nothing in
//! `super` can see a header, which is the point (`super`'s own doc, "The
//! precondition is not part of what the request *is*"). What this file can
//! and does pin is the other half of that argument — that a rendering which
//! *had* folded a precondition in would be a different rendering, so the
//! door's choice is observable rather than cosmetic.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use serde_json::json;

use super::{canonical_rendering, payload_digest};

/// Parse `text` the way a door's `Json` extractor would, so a case can state
/// its input as the wire text a client actually sent rather than as an
/// already-canonical value.
fn parsed(text: &str) -> serde_json::Value {
    serde_json::from_str(text).expect("the case's own input must be valid JSON")
}

/// Two renderings of one logical payload that differ only in key order and
/// whitespace hash **equal**.
///
/// This is the case the whole "parsed, not the received bytes" rule exists
/// for: a client that re-serialises its body on retry — a different key
/// order out of a hash map, a pretty-printer's newlines — is making the same
/// request, and a digest over the received bytes would answer that retry
/// `IDEMPOTENCY_CONFLICT` instead of replaying its outcome.
#[test]
fn two_renderings_of_one_payload_differing_only_in_key_order_hash_equal() {
    let as_first_sent = parsed(r#"{"brand_id":"b-1","name":"Fibre 500","product_code":"F-500"}"#);
    let as_retried = parsed(
        "{ \"product_code\" : \"F-500\" ,\n \"name\":\"Fibre 500\", \"brand_id\" : \"b-1\" }",
    );

    assert_eq!(
        canonical_rendering(&as_first_sent),
        r#"{"brand_id":"b-1","name":"Fibre 500","product_code":"F-500"}"#,
        "the rendering sorts keys lexicographically and carries no insignificant whitespace"
    );
    assert_eq!(
        payload_digest(&as_first_sent),
        payload_digest(&as_retried),
        "a re-serialised retry is the same request and must claim the same key"
    );
}

/// Key order is normalized at **every** object level, not only the outermost
/// one.
///
/// A canonicalizer that sorted the top-level keys and then handed each value
/// to `serde_json`'s own `Display` would pass the case above and fail this
/// one, which is exactly the shortcut this test exists to refuse.
#[test]
fn nested_objects_are_sorted_too() {
    let one = parsed(r#"{"outer":{"z":1,"a":2},"first":true}"#);
    let other = parsed(r#"{"first":true,"outer":{"a":2,"z":1}}"#);

    assert_eq!(
        canonical_rendering(&one),
        r#"{"first":true,"outer":{"a":2,"z":1}}"#
    );
    assert_eq!(payload_digest(&one), payload_digest(&other));
}

/// A differing field value hashes differently — the other half of the
/// contract, and the one that makes `IDEMPOTENCY_CONFLICT` reachable at all.
///
/// Without this the store would replay one caller's answer to a different
/// act sent under the same key, which is a silent no-op in place of a
/// refusal (§3.2 `inst-fd-idem-conflict`: "never a silent no-op").
#[test]
fn a_differing_field_hashes_differently() {
    let ordered = parsed(r#"{"brand_id":"b-1","name":"Fibre 500"}"#);
    let a_different_name = parsed(r#"{"brand_id":"b-1","name":"Fibre 900"}"#);
    let a_different_brand = parsed(r#"{"brand_id":"b-2","name":"Fibre 500"}"#);

    assert_ne!(
        payload_digest(&ordered),
        payload_digest(&a_different_name),
        "a different name is a different act"
    );
    assert_ne!(
        payload_digest(&ordered),
        payload_digest(&a_different_brand),
        "a different brand is a different act"
    );
}

/// An **omitted** field and one sent explicitly as `null` hash differently
/// (**P-D-34**).
///
/// §4.3's "absent values written `null`" clause addresses a complete field
/// set — a version row's columns — and P-D-34 narrows it for a request: a
/// parsed request's named field set is the fields the request *carries*. The
/// distinction is what a `PATCH` means by omitting a field versus clearing
/// it, and a digest that collapsed the two would let one of those replay the
/// other's answer.
#[test]
fn an_omitted_field_and_an_explicit_null_hash_differently() {
    let omitted = parsed(r#"{"name":"Fibre 500"}"#);
    let explicit_null = parsed(r#"{"name":"Fibre 500","product_code":null}"#);

    assert_eq!(canonical_rendering(&omitted), r#"{"name":"Fibre 500"}"#);
    assert_eq!(
        canonical_rendering(&explicit_null),
        r#"{"name":"Fibre 500","product_code":null}"#
    );
    assert_ne!(
        payload_digest(&omitted),
        payload_digest(&explicit_null),
        "omitting a field and clearing it are different requests"
    );
}

/// Number formatting does not fork a digest: `1` and `1.0` are the same
/// value and render identically, with no trailing zero (§4.3, "no trailing
/// zeroes").
///
/// A client library that renders an integer-valued number as a decimal on
/// one attempt and as an integer on the next is a real shape, and it must
/// not turn a retry into a conflict.
#[test]
fn a_number_renders_with_no_trailing_zeroes_so_one_and_one_point_zero_agree() {
    let as_integer = parsed(r#"{"quantity":1}"#);
    let as_decimal = parsed(r#"{"quantity":1.0}"#);

    assert_eq!(canonical_rendering(&as_integer), r#"{"quantity":1}"#);
    assert_eq!(canonical_rendering(&as_decimal), r#"{"quantity":1}"#);
    assert_eq!(payload_digest(&as_integer), payload_digest(&as_decimal));
}

/// Folding a precondition into the operand **would** change the digest —
/// which is why no door on this surface does (**P-D-34**).
///
/// The exclusion itself is structural: [`super::payload_digest`] is handed a
/// value built from the parsed body's own fields and can see no header at
/// all. What this case pins is that the exclusion is *observable* rather
/// than cosmetic: a door that hashed its `If-Match` in would produce a
/// different digest for the same act, so a client refused `STALE_REVISION`
/// that re-read the head and retried with a fresher tag would be answered
/// `IDEMPOTENCY_CONFLICT` instead of having its request run. The door-level
/// measurement of the live behaviour is
/// `crate::api::rest::products`'s
/// `an_answered_key_replays_its_stored_response_even_though_the_retry_carries_a_precondition`.
#[test]
fn folding_a_precondition_into_the_operand_would_change_the_digest() {
    let body_only = json!({ "brand_id": "b-1", "name": "Fibre 500" });
    let body_plus_precondition =
        json!({ "brand_id": "b-1", "name": "Fibre 500", "if_match": "\"7\"" });

    assert_ne!(
        payload_digest(&body_only),
        payload_digest(&body_plus_precondition),
        "a precondition folded into the operand is not free: it forks the digest of one act"
    );
}

/// The digest is stable across runs and reproducible outside this crate.
///
/// The vector below was computed independently of this code, from the
/// canonical rendering asserted beside it. It is a plain `SHA-256` over the
/// rendering — no namespace, no salt, no length prefix — so any tool
/// reproduces it:
///
/// ```text
/// printf '%s' '{"brand_id":"3f8f6a1e-0000-4000-8000-000000000001","name":"Fibre 500","product_code":"FIBRE-500"}' | sha256sum
/// # f116d4e24d6e8f5d078b202390f70f3386bbf31595071b0814bc31bcc3802365
/// ```
///
/// A digest that drifted — a changed rendering, a changed primitive, a
/// hasher swapped for a `Rust`-version-dependent one such as
/// `DefaultHasher` — would make every claim stored before the change
/// unmatchable and turn every in-window retry into an
/// `IDEMPOTENCY_CONFLICT`. This case is what makes such a change loud, and
/// it is the reason the move off the earlier `UUID` v5 construction to
/// `aws-lc-rs` `SHA-256` had to re-pin the vector rather than keep it.
#[test]
fn the_digest_is_stable_across_runs_and_reproducible_outside_this_crate() {
    let payload = json!({
        "name": "Fibre 500",
        "brand_id": "3f8f6a1e-0000-4000-8000-000000000001",
        "product_code": "FIBRE-500",
    });

    assert_eq!(
        canonical_rendering(&payload),
        "{\"brand_id\":\"3f8f6a1e-0000-4000-8000-000000000001\",\"name\":\"Fibre 500\",\
         \"product_code\":\"FIBRE-500\"}",
        "the rendering is the digest's whole input and is pinned first"
    );
    assert_eq!(
        payload_digest(&payload),
        vec![
            0xf1, 0x16, 0xd4, 0xe2, 0x4d, 0x6e, 0x8f, 0x5d, 0x07, 0x8b, 0x20, 0x23, 0x90, 0xf7,
            0x0f, 0x33, 0x86, 0xbb, 0xf3, 0x15, 0x95, 0x07, 0x1b, 0x08, 0x14, 0xbc, 0x31, 0xbc,
            0xc3, 0x80, 0x23, 0x65,
        ],
        "the stored digest must equal the independently computed vector, byte for byte"
    );
    assert_eq!(
        payload_digest(&payload),
        payload_digest(&payload),
        "two calls in one process agree, which is the weaker half the vector above subsumes"
    );
}

/// A string is escaped the way `JSON` escapes strings, so a value carrying a
/// quote or a backslash cannot forge the rendering's own punctuation.
///
/// Without escaping, a `name` of `a","brand_id":"b-2` would render as two
/// fields and let one act's digest be spelled by another's payload — the
/// injection a canonical rendering assembled by concatenation invites.
#[test]
fn a_string_value_is_escaped_so_it_cannot_forge_the_renderings_punctuation() {
    let hostile = json!({ "name": "a\",\"brand_id\":\"b-2", "brand_id": "b-1" });
    let honest = json!({ "name": "a", "brand_id": "b-2" });

    assert_eq!(
        canonical_rendering(&hostile),
        "{\"brand_id\":\"b-1\",\"name\":\"a\\\",\\\"brand_id\\\":\\\"b-2\"}",
        "the quotes inside the value are escaped, not passed through as structure"
    );
    assert_ne!(payload_digest(&hostile), payload_digest(&honest));
}
