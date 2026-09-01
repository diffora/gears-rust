//! Tests for the gear's one canonical rendering rule and its `SHA-256`
//! digest.
//!
//! Every case here asserts a **string or a byte vector**, never a predicate
//! over two of them: a suite that only checked "these two renderings agree"
//! would pass just as happily against a function that returned a constant,
//! and the property the design set actually pins is that a *named* set of
//! bytes comes out — §5's golden vector compares them across two engines and
//! slice 10's restore drill re-verifies stored digests against them, so a
//! rendering that drifted while staying self-consistent is exactly the
//! failure neither of those would survive.
//!
//! The two absence modes are tested against **one** input, because the
//! difference between them is not a difference of input: §4.3 reads a
//! complete field set and §3.2 reads a parsed request, and the same `JSON`
//! object means different things under the two readings (**P-D-34**).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fmt::Write as _;

use serde_json::json;

use super::{Absence, DIGEST_VERSION, canonical_rendering, content_digest, decode_rendering};

/// The roster the complete-set cases render against — a stand-in for a
/// frozen version row's content columns, wide enough that one of its names
/// is genuinely absent from the value under test.
const ROSTER: &[&str] = &[
    "brand_id",
    "name",
    "product_code",
    "published_at",
    "weight_kg",
];

/// Lowercase hex, so a golden vector can be read and re-typed by a human and
/// compared against any `sha256sum` on any machine.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, byte| {
        write!(out, "{byte:02x}").expect("writing to a String cannot fail");
        out
    })
}

/// The digest version is `1`, and it is a constant in this crate.
///
/// §4.3 pins the starting value and **P-D-33** pins where it lives. The
/// assertion is worth its line because the number is what a stored row
/// records: a bump made without the migration that can still recompute the
/// old rendering turns slice 10's restore drill into a whole-table false
/// alarm, and this case is the first thing that goes red when someone edits
/// the constant on its own.
#[test]
fn the_digest_version_starts_at_one_and_is_pinned_in_code() {
    assert_eq!(
        DIGEST_VERSION, 1,
        "the digest version starts at 1 and a bump is a migration, not an edit"
    );
}

/// The two absence modes render the **same** value differently: the complete
/// set writes an absent field `null`, the parsed request omits it.
///
/// This is the whole of **P-D-34**. §4.3's "absent values written `null`"
/// clause addresses a version row's columns, where absence and the empty
/// string must not collide; a request's named field set is the fields it
/// carries, where an omitted field and a cleared one are two different acts.
/// A single mode would have to get one of the two wrong.
#[test]
fn the_two_absence_modes_disagree_about_a_field_the_value_does_not_carry() {
    let value = json!({ "name": "Fibre 500", "product_code": "FIBRE-500" });
    let roster: &[&str] = &["name", "product_code", "brand_id"];

    assert_eq!(
        canonical_rendering(&value, Absence::Null { roster }),
        r#"{"brand_id":null,"name":"Fibre 500","product_code":"FIBRE-500"}"#,
        "a complete set writes the field the value does not carry as null"
    );
    assert_eq!(
        canonical_rendering(&value, Absence::Omit),
        r#"{"name":"Fibre 500","product_code":"FIBRE-500"}"#,
        "a parsed request's field set is the fields it carries"
    );
    assert_ne!(
        content_digest(&canonical_rendering(&value, Absence::Null { roster })),
        content_digest(&canonical_rendering(&value, Absence::Omit)),
        "the two readings are not interchangeable and their digests must not be either"
    );
}

/// Under the complete-set mode an omitted field and an explicit `null` are
/// the **same** content; under the request mode they are not.
///
/// The mirror of the case above, and the one that shows the clause doing its
/// job in the direction §4.3 cares about: a frozen row whose column is
/// absent and one whose column is `null` are the same row, and a digest that
/// forked them would report an entity version as corrupt on a re-render that
/// merely built its operand a different way. In the request mode the very
/// same pair must fork, because a `PATCH` that omits a field and one that
/// clears it are different requests.
#[test]
fn a_complete_set_collapses_absence_and_null_while_a_request_forks_them() {
    let omitted = json!({ "name": "Fibre 500" });
    let explicit_null = json!({ "name": "Fibre 500", "product_code": null });
    let roster: &[&str] = &["name", "product_code"];

    assert_eq!(
        canonical_rendering(&omitted, Absence::Null { roster }),
        canonical_rendering(&explicit_null, Absence::Null { roster }),
        "in a complete set an absent value is written null, so the two are one content"
    );
    assert_eq!(
        canonical_rendering(&omitted, Absence::Null { roster }),
        r#"{"name":"Fibre 500","product_code":null}"#
    );
    assert_ne!(
        canonical_rendering(&omitted, Absence::Omit),
        canonical_rendering(&explicit_null, Absence::Omit),
        "a request that omits a field and one that clears it are different acts"
    );
}

/// A name the value carries that the roster does not name is rendered, not
/// dropped.
///
/// The alternative — silently discarding it — would let a caller freeze
/// content that the digest does not cover, which is the one failure a
/// content digest exists to make impossible. This module does not judge a
/// schema; the caller that owns the column set does.
#[test]
fn a_value_field_outside_the_roster_is_rendered_rather_than_dropped() {
    let value = json!({ "name": "Fibre 500", "internal_revision": 4 });
    let roster: &[&str] = &["name"];

    assert_eq!(
        canonical_rendering(&value, Absence::Null { roster }),
        r#"{"internal_revision":4,"name":"Fibre 500"}"#,
        "the rendering is the union of the roster and the value's own names, still sorted"
    );
}

/// Keys are sorted lexicographically at **every** object level and no
/// insignificant whitespace survives.
///
/// A canonicalizer that sorted the outermost keys and then handed each value
/// to `serde_json`'s own `Display` would pass a flat case and fail this one,
/// which is exactly the shortcut this test refuses. Whitespace is asserted
/// by the same literal: the expected string carries none, so a renderer that
/// pretty-printed anything at all fails here rather than at a golden vector
/// three slices later.
#[test]
fn keys_sort_at_every_level_and_no_whitespace_survives() {
    let as_first_sent: serde_json::Value =
        serde_json::from_str("{ \"outer\" : { \"z\" : 1 ,\n \"a\" : 2 } , \"first\" : true }")
            .expect("the case's own input must be valid JSON");

    assert_eq!(
        canonical_rendering(&as_first_sent, Absence::Omit),
        r#"{"first":true,"outer":{"a":2,"z":1}}"#
    );
}

/// Numbers render as bare decimal strings with no trailing zeroes, and a
/// timestamp string is carried through verbatim at microsecond precision.
///
/// §4.3 states both clauses. The number half is what keeps a client library
/// that renders `1` on one attempt and `1.0` on the next from forking a
/// digest. The timestamp half pins that nothing here reformats an instant:
/// the caller renders `RFC 3339` in `UTC` and this module carries the string
/// it was given, so the precision decision stays with the column's owner.
#[test]
fn numbers_carry_no_trailing_zeroes_and_a_timestamp_string_passes_through() {
    let value = json!({
        "count": 1.0,
        "negative": -7,
        "published_at": "2026-08-27T11:04:05.123456Z",
        "weight_kg": 1.5,
    });

    assert_eq!(
        canonical_rendering(&value, Absence::Omit),
        "{\"count\":1,\"negative\":-7,\"published_at\":\"2026-08-27T11:04:05.123456Z\",\
         \"weight_kg\":1.5}",
        "1.0 renders as 1, a fraction keeps its digits, and the instant is untouched"
    );
}

/// The golden vector: one fixed input, its exact rendering, and its exact
/// `SHA-256`.
///
/// The rendering and the digest are pinned **separately and by literal**, so
/// the vector holds each of them independently: a change to the rendering
/// fails the first assertion, a change to the digest primitive fails only
/// the second, and a reader can tell the two apart without reading this
/// crate. The digest below was computed outside `Rust` and can be
/// reproduced by anyone:
///
/// ```text
/// printf '%s' '{"brand_id":"3f8f6a1e-0000-4000-8000-000000000001","name":"Fibre 500","product_code":null,"published_at":"2026-08-27T11:04:05.123456Z","weight_kg":1.5}' | sha256sum
/// # e252632893610a1207b4844a24a1aec1682c8a4b7b5242bd7a26b082b1e77c35
/// ```
///
/// The input deliberately exercises every clause at once: an absent roster
/// name written `null`, unsorted input keys, a fraction, and an `RFC 3339`
/// timestamp at microsecond precision. §4.3 calls for exactly such a vector,
/// *"a canonical-serialization golden vector committed with the first
/// migration"*, and §5 is where the cross-engine comparison lands.
#[test]
fn the_golden_vector_pins_the_rendering_and_the_digest_independently() {
    let content = json!({
        "name": "Fibre 500",
        "weight_kg": 1.5,
        "brand_id": "3f8f6a1e-0000-4000-8000-000000000001",
        "published_at": "2026-08-27T11:04:05.123456Z",
    });

    let rendered = canonical_rendering(&content, Absence::Null { roster: ROSTER });

    assert_eq!(
        rendered,
        "{\"brand_id\":\"3f8f6a1e-0000-4000-8000-000000000001\",\"name\":\"Fibre 500\",\
         \"product_code\":null,\"published_at\":\"2026-08-27T11:04:05.123456Z\",\
         \"weight_kg\":1.5}",
        "the rendering is the digest's whole input and is pinned first"
    );
    assert_eq!(
        hex(&content_digest(&rendered)),
        "e252632893610a1207b4844a24a1aec1682c8a4b7b5242bd7a26b082b1e77c35",
        "the digest must equal the independently computed vector, byte for byte"
    );
    assert_eq!(
        content_digest(&rendered).len(),
        32,
        "a full SHA-256, not a truncation of one"
    );
}

/// A string value is escaped the way `JSON` escapes strings, so a value
/// carrying a quote cannot forge the rendering's own punctuation.
///
/// Without escaping, a `name` of `a","brand_id":"b-2` would render as two
/// fields and let one content's digest be spelled by another's payload — the
/// injection a canonical rendering assembled by concatenation invites.
#[test]
fn a_string_value_cannot_forge_the_renderings_punctuation() {
    let hostile = json!({ "name": "a\",\"brand_id\":\"b-2", "brand_id": "b-1" });
    let honest = json!({ "name": "a", "brand_id": "b-2" });

    assert_eq!(
        canonical_rendering(&hostile, Absence::Omit),
        "{\"brand_id\":\"b-1\",\"name\":\"a\\\",\\\"brand_id\\\":\\\"b-2\"}",
        "the quotes inside the value are escaped, not passed through as structure"
    );
    assert_ne!(
        content_digest(&canonical_rendering(&hostile, Absence::Omit)),
        content_digest(&canonical_rendering(&honest, Absence::Omit))
    );
}

/// An array is rendered in the order received, in both modes.
///
/// Pinned as the **current** behaviour rather than as the final rule: §4.3
/// sorts a row collection by the collection's own identifier, and this case
/// is what will go red when the first door whose payload carries a
/// collection arrives — which is the point at which that sort is owed, and
/// the reason it is named as owed rather than pre-built here.
#[test]
fn an_array_is_rendered_in_the_order_received_today() {
    let value = json!({ "tags": ["z", "a", "m"] });

    assert_eq!(
        canonical_rendering(&value, Absence::Omit),
        r#"{"tags":["z","a","m"]}"#,
        "no collection sort is applied yet and none is owed until a collection exists"
    );
}

/// [`decode_rendering`] is [`canonical_rendering`]'s inverse (P-D-77): a
/// rendered object decodes back to a map whose re-rendering is
/// byte-identical, roster-written `null` members included — and the two
/// non-object shapes a corrupt store could hold are reported, not admitted.
#[test]
fn decode_rendering_round_trips_the_renderer() {
    let roster: &[&str] = &["absent_member", "name", "nested"];
    let value = serde_json::json!({
        "name": "Fibre 500",
        "nested": { "b": 2, "a": 1 },
    });
    let rendered = canonical_rendering(&value, Absence::Null { roster });

    let decoded = decode_rendering(&rendered).expect("a rendering decodes");
    assert_eq!(
        decoded.get("absent_member"),
        Some(&serde_json::Value::Null),
        "a roster name written null comes back as a null member, not as absence"
    );
    assert_eq!(
        canonical_rendering(
            &serde_json::Value::Object(decoded),
            Absence::Null { roster }
        ),
        rendered,
        "decode then render reproduces the stored bytes exactly"
    );

    assert!(
        decode_rendering("not json at all").is_err(),
        "a parse failure is reported"
    );
    assert!(
        decode_rendering("[1,2]").is_err(),
        "a non-object rendering is a store this gear wrote wrong"
    );
}
