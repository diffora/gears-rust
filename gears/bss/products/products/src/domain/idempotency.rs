//! The payload digest an idempotency claim is taken against
//! (`design/01-foundation.md` §3.2 `inst-fd-idem-hash`, §4.3; **P-D-34**).
//!
//! `cpt-cf-bss-products-dod-idempotency-store` states the operand in one
//! sentence: *"The payload hash MUST be taken over the canonical rendering of
//! the **parsed** request, excluding the precondition header"*. Both halves
//! are load-bearing and this module exists so neither is re-decided per door.
//!
//! # Parsed, not the received bytes
//!
//! A client that re-serialises its `JSON` on retry — a different key order, a
//! reflowed body, `1` where it first sent `1.0` — is making **the same
//! request**. A digest over the received bytes would answer that retry
//! `IDEMPOTENCY_CONFLICT` instead of replaying its outcome, breaking
//! idempotency exactly where a client needs it (§3.2: *"a byte hash would
//! answer it `IDEMPOTENCY_CONFLICT` instead of replaying the outcome"*).
//! [`payload_digest`] therefore takes a [`JsonValue`] the door has already
//! parsed, never a body slice.
//!
//! # The precondition is not part of what the request *is*
//!
//! `If-Match` is a header, and a client refused `STALE_REVISION` that re-read
//! the head and retried is making the same request with a fresher tag
//! (**P-D-34**). Nothing in this module can see a header: its whole operand
//! is the value its caller builds out of the parsed body's own fields, which
//! is what makes the exclusion structural rather than a rule a later edit
//! could forget. The door-level measurement of that property is
//! `crate::api::rest::products::products_tests
//! ::an_answered_key_replays_its_stored_response_even_though_the_retry_carries_a_precondition`.
//!
//! # The canonical rendering is §4.3's, and the design set fixed it
//!
//! This gear pins **one** canonicalization rule rather than two: §4.3
//! ("Engine-canonical serialization is pinned here") states it for a frozen
//! version row's content, and §3.2 reuses it for a parsed request precisely
//! so a later reader has one rule to learn. As it applies here:
//!
//! - `JSON`, object keys **sorted lexicographically by field name**, `UTF-8`
//!   without `BOM`, and no insignificant whitespace at all.
//! - Integers as bare decimal strings, no locale and no trailing zeroes — so
//!   `1` and `1.0` render identically and hash equal.
//! - Timestamps `RFC 3339` in `UTC`: a caller rendering one into a string
//!   field gets that string verbatim, since no request field on this surface
//!   is typed as an instant.
//!
//! **What §4.3's "absent values written `null`" clause does *not* mean here.**
//! That clause addresses a *complete* named field set — a version row's
//! columns. **P-D-34** narrows it for a request: *"A parsed request's named
//! field set is the fields the request carries"*, so an omitted field is
//! omitted from the rendering rather than rendered `null`, and a `PATCH` that
//! omits a field and one that sends it `null` hash **differently**, which is
//! what they mean at the head door. Callers build the operand accordingly:
//! see [`payload_digest`]'s own contract.
//!
//! **An array is rendered in the order received.** §4.3 sorts a *row
//! collection* by the collection's own identifier, and neither create door's
//! payload carries a collection today — no field on `CreateProductRequest` or
//! `CreateSkuRequest` is an array. The first door whose payload does carry
//! one owes that sort here rather than at its own call site; it is named as
//! owed rather than pre-built, because the sort key is the collection's
//! identifier and no collection exists yet to name one.
//!
//! # The digest primitive, and what it costs
//!
//! [`payload_digest`] is a `UUID` v5 — the `RFC 4122` namespaced `SHA-1`
//! construction — over the canonical rendering, stored as its 16 bytes in
//! `payload_hash` (`bytea` on Postgres, `blob` on `SQLite`; the migration
//! fixes no length, so 16 bytes is admitted).
//!
//! It is **not** the primitive the donor uses: `gears/bss/pricing`'s
//! `IdempotencyGate::payload_hash` takes a full `SHA-256` through
//! `aws-lc-rs`, the platform's own `FIPS`-validated provider. This gear
//! cannot: `aws-lc-rs` is not a dependency of this crate and this slice may
//! not touch `Cargo.toml`, while `sha2`/`sha1`/`md5` are refused outright by
//! architecture lint `DE0708` (`docs/security/SECURITY.md`). The `uuid` crate
//! is already a direct dependency with its `v5` feature on, so v5 is the one
//! specified, stable, cross-language digest reachable from here without a
//! manifest change.
//!
//! **The cost, stated plainly**: 128 bits truncated out of `SHA-1` rather
//! than 256 bits of `SHA-256`. This digest is not a security primitive — it
//! never crosses the wire, and it is compared only against the other digests
//! stored under the *same* `(tenant_id, endpoint, client_key)`, so the whole
//! consequence of a collision is that one caller's own retry replays its own
//! earlier answer instead of being refused `IDEMPOTENCY_CONFLICT`. Moving to
//! the donor's `aws-lc-rs` `SHA-256` is owed to whichever slice may add a
//! dependency to this gear's manifest; nothing else about this module changes
//! when it does, because the rendering — the part the design set actually
//! pins — is independent of which digest consumes it.
//!
//! **A later reader can reproduce a digest** without this crate:
//! `python3 -c "import uuid;
//! print(uuid.uuid5(uuid.UUID('8a1f4d2c-7b93-4e51-9c6a-2f08d3b715e4'),
//! '<the canonical rendering>'))"`. `idempotency_tests
//! ::the_digest_is_stable_across_runs_and_reproducible_outside_this_crate`
//! pins one such vector byte for byte.
//!
//! @cpt-cf-bss-products-dod-idempotency-store

use serde_json::{Number, Value as JsonValue};
use uuid::Uuid;

/// The `UUID` v5 namespace every payload digest on this surface is taken
/// under.
///
/// Arbitrary in the way any namespace is, and **pinned** in the way that
/// matters: changing it changes every digest, which would make every live
/// claim's stored `payload_hash` unmatchable and turn every in-window retry
/// into an `IDEMPOTENCY_CONFLICT`. A change here is a digest-version bump
/// with a retention window to wait out, never a refactor.
pub const PAYLOAD_NAMESPACE: Uuid = Uuid::from_u128(0x8a1f_4d2c_7b93_4e51_9c6a_2f08_d3b7_15e4);

/// The digest of one parsed request, as `products_idempotency.payload_hash`
/// stores it.
///
/// `payload` is the **fields the request carries**, already parsed: the
/// caller builds a [`JsonValue::Object`] out of its own DTO and omits the
/// fields the request did not send (**P-D-34**; this module's doc, "What
/// §4.3's absent-values clause does not mean here"). Two consequences the
/// caller owns rather than this function:
///
/// - Nothing about the transport may enter `payload` — not the precondition,
///   not a correlation id, not a retry counter. Anything that varies between
///   two attempts at the same act turns every honest retry into
///   `IDEMPOTENCY_CONFLICT`.
/// - A DTO whose `Option` field cannot tell an absent key from an explicit
///   `null` renders the two identically. Each door states which of its own
///   fields that applies to.
#[must_use]
pub fn payload_digest(payload: &JsonValue) -> Vec<u8> {
    Uuid::new_v5(&PAYLOAD_NAMESPACE, canonical_rendering(payload).as_bytes())
        .as_bytes()
        .to_vec()
}

/// The canonical rendering [`payload_digest`] hashes — §4.3's rule as this
/// module's doc restates it for a parsed request.
///
/// Public because the rendering, not the digest, is the part the design set
/// pins: a test, and a later reader reproducing a stored digest by hand, both
/// need to see the exact string that went in.
#[must_use]
pub fn canonical_rendering(value: &JsonValue) -> String {
    let mut rendered = String::new();
    render_into(value, &mut rendered);
    rendered
}

/// Append `value`'s canonical rendering to `out`.
///
/// Recursive rather than iterative for the reason the shape is recursive:
/// a value nests, and the sort applies at every object level, not only the
/// outermost one.
fn render_into(value: &JsonValue, out: &mut String) {
    match value {
        JsonValue::Null => out.push_str("null"),
        JsonValue::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        JsonValue::Number(number) => out.push_str(&render_number(number)),
        JsonValue::String(text) => out.push_str(&render_string(text)),
        JsonValue::Array(items) => {
            out.push('[');
            for (position, item) in items.iter().enumerate() {
                if position > 0 {
                    out.push(',');
                }
                render_into(item, out);
            }
            out.push(']');
        }
        JsonValue::Object(map) => {
            // Sorted here rather than trusted from the map: `serde_json`'s
            // own ordering depends on whether its `preserve_order` feature
            // is on anywhere in the graph, and a digest that changed with a
            // feature unification elsewhere in the workspace would be the
            // opposite of canonical.
            let mut entries: Vec<(&String, &JsonValue)> = map.iter().collect();
            entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
            out.push('{');
            for (position, (key, entry)) in entries.into_iter().enumerate() {
                if position > 0 {
                    out.push(',');
                }
                out.push_str(&render_string(key));
                out.push(':');
                render_into(entry, out);
            }
            out.push('}');
        }
    }
}

/// One string, escaped the way `JSON` escapes strings.
///
/// Reached through [`JsonValue`]'s own infallible `Display` rather than
/// `serde_json::to_string`'s `Result`: escaping a string cannot fail, and a
/// fallible call here would need a fallback branch that could only ever
/// render something *other* than the canonical form.
fn render_string(text: &str) -> String {
    JsonValue::String(text.to_owned()).to_string()
}

/// One number, as a bare decimal string with no trailing zeroes.
///
/// Integers render exactly. A fractional value renders through `f64`'s own
/// shortest-round-trip `Display`, which prints `1.0` as `1` — so a client
/// that sent `1` on its first attempt and `1.0` on its retry hashes the same,
/// which is the whole point of hashing a *parsed* request.
///
/// **The stated cost**: a magnitude large or small enough to make `f64`'s
/// `Display` reach exponent form renders in that form, and a value beyond
/// `f64`'s precision renders as the nearest representable one. No request
/// field on this surface is numeric today; the first door that adds one owes
/// a decimal-string operand rather than a float, which is §4.3's own rule
/// ("integers and decimals as bare decimal strings") read strictly.
fn render_number(number: &Number) -> String {
    if let Some(value) = number.as_i64() {
        return value.to_string();
    }
    if let Some(value) = number.as_u64() {
        return value.to_string();
    }
    match number.as_f64() {
        Some(value) => format!("{value}"),
        None => number.to_string(),
    }
}

#[cfg(test)]
#[path = "idempotency_tests.rs"]
mod idempotency_tests;
