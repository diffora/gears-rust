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
//! # The canonical rendering is §4.3's, and it lives in one place
//!
//! This gear pins **one** canonicalization rule rather than two: §4.3
//! ("Engine-canonical serialization is pinned here") states it for a frozen
//! version row's content, and §3.2 reuses it for a parsed request precisely
//! so a later reader has one rule to learn. The rule itself therefore lives
//! in [`crate::domain::canonical`], not here, and this module binds it to the
//! one reading a request is taken under: [`Absence::Omit`].
//!
//! **What §4.3's "absent values written `null`" clause does *not* mean here.**
//! That clause addresses a *complete* named field set — a version row's
//! columns — and is [`Absence::Null`]. **P-D-34** narrows it for a request:
//! *"A parsed request's named field set is the fields the request carries"*,
//! so an omitted field is omitted from the rendering rather than rendered
//! `null`, and a `PATCH` that omits a field and one that sends it `null` hash
//! **differently**, which is what they mean at the head door. Callers build
//! the operand accordingly: see [`payload_digest`]'s own contract.
//!
//! **An array is rendered in the order received**, and the collection sort
//! §4.3 states is owed by the first door whose payload carries a collection.
//! Neither create door's payload does today — no field on
//! `CreateProductRequest` or `CreateSkuRequest` is an array — and the debt is
//! recorded in [`crate::domain::canonical`]'s own doc, beside the code that
//! will pay it.
//!
//! # The digest primitive
//!
//! [`payload_digest`] is `SHA-256` through `aws-lc-rs`, the platform's
//! `FIPS`-validated provider, stored as its 32 bytes in `payload_hash`
//! (`bytea` on Postgres, `blob` on `SQLite`; the migration fixes no length).
//! It is [`crate::domain::canonical::content_digest`] — the same function and
//! the same primitive §4.3 names for a version row's `content_digest`, so the
//! gear has one digest as well as one rendering.
//!
//! It is also the donor's: `gears/bss/pricing`'s
//! `IdempotencyGate::payload_hash` takes `aws-lc-rs` `SHA-256` through the
//! identical call. This module previously could not, `aws-lc-rs` not being a
//! dependency of this crate while `sha2`/`sha1`/`md5` are refused outright by
//! architecture lint `DE0708` (`docs/security/SECURITY.md`), and stood on a
//! `UUID` v5 instead. **That debt — recorded here as owed "to whichever slice
//! may add a dependency to this gear's manifest" — is paid**: the manifest
//! now carries `aws-lc-rs` and the truncated 128-bit `SHA-1` construction is
//! gone. Nothing else in this module moved, because the rendering — the part
//! the design set actually pins — is independent of which digest consumes it,
//! which is what that entry predicted.
//!
//! **A later reader can reproduce a digest** without this crate, the digest
//! being a plain `SHA-256` over the rendering with no namespace, no salt and
//! no length prefix:
//! `printf '%s' '<the canonical rendering>' | sha256sum`. `idempotency_tests
//! ::the_digest_is_stable_across_runs_and_reproducible_outside_this_crate`
//! pins one such vector byte for byte.
//!
//! # What the swap costs a stored row, and why it is free here
//!
//! The primitive changed under a table that stores its output. The namespace
//! constant removed by the swap carried the rule for exactly this class of
//! change in its own doc: *"A change here is a digest-version bump with a
//! retention window to wait out, never a refactor."* Paying the debt above
//! does not discharge that rule, so it is stated here rather than left to be
//! rediscovered.
//!
//! **The consequence.** `payload_hash` moved from 16 bytes to 32 and from one
//! construction to another, so any row written before this commit carries a
//! digest no arriving request can now match. `claim_idempotency_key` compares
//! the arriving digest against the stored one and answers
//! `IDEMPOTENCY_CONFLICT` on a mismatch, so an in-window retry against such a
//! row is refused rather than replayed - the precise failure the store exists
//! to prevent.
//!
//! **The disposition.** This gear is pre-production: it has never been
//! deployed, `products_idempotency` has no rows anywhere, and the migration
//! chain that creates it is itself unreleased. The swap is therefore free,
//! and it is free **because of that fact and no other** - not because the
//! rule is soft.
//!
//! **What a later swap owes.** Against live data the same change is a
//! `digest_version` bump plus a wait-out of the store's retention floor - C6
//! (`design/01-foundation.md` §1.6): at least 24 hours **and** at least the
//! maximum freeze timeout slice 06 exports. Every key claimed before the swap
//! must have expired before the new primitive may answer a claim, or a
//! caller's own retry meets a digest it cannot match.
//!
//! @cpt-cf-bss-products-dod-idempotency-store

use serde_json::Value as JsonValue;

use crate::domain::canonical::{Absence, content_digest};

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
    content_digest(&canonical_rendering(payload))
}

/// The canonical rendering [`payload_digest`] hashes — §4.3's rule read
/// under **P-D-34**'s request mode.
///
/// A named binding rather than a call site's argument: the mode a request is
/// rendered under is a decision of this module, not of each door, and
/// [`crate::domain::canonical::canonical_rendering`] takes the mode
/// explicitly precisely so no caller can pick it by accident.
///
/// Public because the rendering, not the digest, is the part the design set
/// pins: a test, and a later reader reproducing a stored digest by hand, both
/// need to see the exact string that went in.
#[must_use]
pub fn canonical_rendering(payload: &JsonValue) -> String {
    crate::domain::canonical::canonical_rendering(payload, Absence::Omit)
}

#[cfg(test)]
#[path = "idempotency_tests.rs"]
mod idempotency_tests;
