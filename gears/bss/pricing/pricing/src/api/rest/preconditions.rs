//! The preconditions every mutating route on this surface speaks: the `ETag` it
//! hands back, the `If-Match` it demands, the `Idempotency-Key` two of them
//! demand, and the digest that decides whether two requests carrying one key are
//! the same request.
//!
//! It is one module rather than seven copies because each of these is a
//! **contract**, not a parse. D-141 says every mutating verb on a draft row
//! presents its `ETag`; D-145 says the abandon carries one for the same reason;
//! §5's idempotency column says two creates carry a client key. A route that
//! spelled any of those for itself would be free to spell it differently, and
//! the way that failure presents is a caller whose precondition is accepted on
//! one verb and refused on another.
//!
//! # What this module refuses, and what it deliberately does not
//!
//! **This module refuses a request it cannot understand. The store refuses one
//! whose premise has moved.** A well-formed tag that does not match the row is
//! none of this module's business: the repository's compare-and-swap is the
//! authority on that, and it answers [`RepoError::StaleRowVersion`] →
//! `STALE_VERSION` (409) naming both versions. Comparing here as well would put
//! a second, weaker guard in front of the real one — one that reads the row,
//! decides, and then hands the decision to a statement that races it.
//!
//! [`RepoError::StaleRowVersion`]: crate::infra::storage::RepoError::StaleRowVersion
//!
//! # No new wire code
//!
//! Every refusal here is [`DomainError::InvalidRequest`] → 400. That is not a
//! shortcut: `03-price-structure.md` §5 (D-141) settles it in as many words —
//! "an absent precondition is a malformed request under the Foundation
//! validation envelope, so **no new code** is minted". The same reading covers
//! an unparseable tag, a wildcard, a key of the wrong shape and an undecodable
//! cursor, because a caller cannot act differently on any of them than on the
//! others: the request as sent cannot be interpreted, and the remedy is to send
//! a different one.
//!
//! # The entity tag is the domain's, not the transport's
//!
//! [`RowVersion::to_etag`] and [`RowVersion::from_etag`] live in
//! `domain::concurrency` and are called from here rather than re-implemented,
//! for the reason that module's own doc gives: the repositories and the REST
//! layer have to agree, to the byte, on what tag denotes what version, and a
//! second rendering is free to drift from the one that parses tags back. What
//! this module adds is the **header** layer the domain must not know about —
//! which header carries the tag, and what an absent one means.

use axum::http::HeaderMap;
use axum::http::header::IF_MATCH;
use serde::Serialize;

use crate::domain::concurrency::{RowVersion, strong_tag_body};
use crate::domain::error::DomainError;
use crate::infra::storage::repo::IdempotencyGate;

/// The request header carrying the client idempotency key (§5's *client
/// idempotency key* cell, §4.2 step 1).
///
/// Lower-case because `HeaderMap` keys compare case-insensitively and this is
/// the spelling a `HeaderName` parses to; it is the one place the name is
/// written, so the router doc, the handler and the tests cannot disagree.
pub const IDEMPOTENCY_KEY: &str = "idempotency-key";

/// The longest client key this surface accepts.
///
/// A bound rather than none: the key is a primary-key column
/// (`pricing_idempotency_dedup.client_key`) and the digest it guards is
/// computed per request, so an unbounded key is an unbounded write a caller
/// chooses the size of. 255 is the conventional ceiling and is far above any
/// key a client generator produces.
const MAX_KEY_LEN: usize = 255;

/// The entity tag denoting `version`, ready for an `ETag` response header.
///
/// **Emitting it is not optional and every route asserts it.** A caller that
/// cannot obtain a tag cannot satisfy the precondition on the next verb, so a
/// read or a mutation that answered without one would leave the caller with no
/// way to edit what it just read — the precondition would be unsatisfiable
/// rather than merely undocumented.
#[must_use]
pub fn etag(version: RowVersion) -> String {
    version.to_etag()
}

/// The row version an `If-Match` header asserts.
///
/// # Errors
/// [`DomainError::InvalidRequest`] naming the header when it is **absent** —
/// D-141 and D-145 both exist to stop an unconditional mutation, and a verb
/// that ran without the header would be exactly that; when the value is not
/// valid UTF-8; and, through [`RowVersion::from_etag`], when it is a weak
/// validator, the wildcard `*`, a list, or not a quoted decimal at all. The
/// wildcard is worth naming separately: it means *overwrite whichever version
/// is current*, which is the silent overwrite `fr-concurrent-edit` forbids and
/// the one thing these preconditions exist to make unreachable.
pub fn if_match(headers: &HeaderMap) -> Result<RowVersion, DomainError> {
    let Some(raw) = headers.get(IF_MATCH) else {
        return Err(DomainError::InvalidRequest(
            "If-Match is required on this verb: a mutation of a draft row \
             asserts the version it was authored against (D-141), and an \
             unconditional write would overwrite a concurrent editor's work"
                .to_owned(),
        ));
    };
    let raw = raw.to_str().map_err(|_| {
        DomainError::InvalidRequest("If-Match: the header value is not valid UTF-8".to_owned())
    })?;
    RowVersion::from_etag(raw)
}

/// The client idempotency key this request carries.
///
/// # An absent key is refused, and the argument is here rather than per route
///
/// §5 gives these two creates the cell *client idempotency key*, and §4.2 makes
/// the gate the **first step** of a mutation. Executing unguarded when the
/// header is missing would mean the at-most-once promise held only for callers
/// who happened to opt in — and the caller who most needs it is precisely the
/// one whose retry logic sent the request twice without thinking about it. So
/// an absent key is refused on the routes that declare one, which is also the
/// only reading under which "the gate is the first step" is true of every
/// request rather than of some.
///
/// # Errors
/// [`DomainError::InvalidRequest`] when the header is absent, is not valid
/// UTF-8, is empty, is longer than 255 characters, or carries a byte outside
/// printable ASCII — the key is a stored primary-key component and a control
/// character in it is a value no operator can read back in a refusal.
pub fn idempotency_key(headers: &HeaderMap) -> Result<String, DomainError> {
    let refuse = |why: &str| DomainError::InvalidRequest(format!("Idempotency-Key: {why}"));
    let Some(raw) = headers.get(IDEMPOTENCY_KEY) else {
        return Err(refuse(
            "this operation is guarded at-most-once and requires the header; \
             an unguarded create on a governed authoring plane is the retry \
             hazard the gate exists for",
        ));
    };
    let key = raw
        .to_str()
        .map_err(|_| refuse("the header value is not valid UTF-8"))?;
    if key.is_empty() {
        return Err(refuse("an empty key names nothing"));
    }
    if key.len() > MAX_KEY_LEN {
        return Err(refuse(
            "the key is longer than the 255 characters this surface stores",
        ));
    }
    if !key.bytes().all(|b| b.is_ascii_graphic() || b == b' ') {
        return Err(refuse(
            "the key must be printable ASCII; it is stored and echoed in refusals",
        ));
    }
    Ok(key.to_owned())
}

/// A plan-plane precondition: **which revision** the caller read, and at which
/// version.
///
/// # Why a plan's tag names its revision and a price row's does not
///
/// `/bss-pricing/v1/plans/{planId}` does not name a revision. It answers "the
/// open draft, else the current revision", and a `PATCH` resolves the same way
/// **independently** — so without the revision in the tag, a version read
/// against revision *N* is submitted against whatever revision the `PATCH`
/// happens to resolve, and the two counters are unrelated: a freshly opened
/// successor starts at `RowVersion::new(0)`, which is exactly the value the
/// plan's first draft carries.
///
/// That is the lost update D-145 spends two paragraphs eliminating, arriving
/// through the **revision** instead of the version. D-145 closed it on the
/// storage side by keeping the number consumed, so `(plan_id, revision)` never
/// names two rows; a transport that addresses a plan without naming a revision
/// reopens it, because the caller's tag no longer says which row it was read
/// from. The fix is to make the precondition name its subject.
///
/// A **price** route addresses a row by `priceId` and has no such ambiguity:
/// the tag and the row are one to one, so those tags stay a bare version
/// ([`etag`]) and nothing about them changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RevisionTag {
    /// The revision the caller read.
    pub revision: u64,
    /// The version that revision stood at.
    pub version: RowVersion,
}

/// The entity tag denoting `version` **of `revision`**: `"3-7"`, quoted.
///
/// Two decimals joined by a hyphen rather than a structured token, because the
/// tag is opaque to a client and has to survive being copied verbatim out of a
/// response header and back into a request header — which is the only thing a
/// caller ever does with it.
#[must_use]
pub fn revision_etag(revision: u64, version: RowVersion) -> String {
    format!("\"{revision}-{version}\"")
}

/// The revision and version an `If-Match` header asserts, on a plan route.
///
/// The envelope — no wildcard, no weak validator, no list, quotes required — is
/// [`strong_tag_body`]'s, shared with [`RowVersion::from_etag`] rather than
/// re-implemented, so this reader cannot end up laxer than that one on the very
/// verbs where a wildcard is the defect.
///
/// # Errors
/// [`DomainError::InvalidRequest`] when the header is absent, is not valid
/// UTF-8, is not one strong entity tag, or does not quote exactly two decimal
/// numbers joined by `-`.
pub fn if_match_revision(headers: &HeaderMap) -> Result<RevisionTag, DomainError> {
    let Some(raw) = headers.get(IF_MATCH) else {
        return Err(DomainError::InvalidRequest(
            "If-Match is required on this verb: a mutation of a plan revision asserts both \
             the revision it was read from and the version that revision stood at (D-141, \
             D-145), and an unconditional write would overwrite a concurrent editor's work"
                .to_owned(),
        ));
    };
    let raw = raw.to_str().map_err(|_| {
        DomainError::InvalidRequest("If-Match: the header value is not valid UTF-8".to_owned())
    })?;
    let refuse = || {
        DomainError::InvalidRequest(format!(
            "If-Match {raw}: a plan revision's tag quotes its revision and its version, \
             joined by a hyphen (for example \"3-7\"); pass back the `ETag` this surface \
             issued, verbatim"
        ))
    };
    let body = strong_tag_body(raw)?;
    let (revision, version) = body.split_once('-').ok_or_else(refuse)?;
    if revision.is_empty()
        || version.is_empty()
        || !revision.bytes().all(|b| b.is_ascii_digit())
        || !version.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(refuse());
    }
    Ok(RevisionTag {
        revision: revision.parse::<u64>().map_err(|_| refuse())?,
        version: RowVersion::new(version.parse::<u64>().map_err(|_| refuse())?),
    })
}

/// Parse a JSON request body.
///
/// # Why the handlers do not use axum's `Json` extractor
///
/// Its rejection for a body that parses as JSON but not as the target type is
/// **422 Unprocessable Entity**, and `01-foundation.md` §3.3 is explicit that no
/// path in this gear can produce a 422: the platform's canonical family has no
/// such category, every architectural 422 in the design set reaches the wire as
/// a 400 carrying its code, and an endpoint must not declare one. A handler that
/// let the extractor answer would emit a status its own `OpenAPI` registration
/// denies it can, discovered by a consumer rather than by a gate.
///
/// # Errors
/// [`DomainError::InvalidRequest`] naming what could not be read, for an empty
/// body, a body that is not JSON, and a body whose members do not fit the
/// request type.
pub fn parse_body<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, DomainError> {
    if body.is_empty() {
        return Err(DomainError::InvalidRequest(
            "the request body is empty; send a JSON object, `{}` for an empty one".to_owned(),
        ));
    }
    serde_json::from_slice(body)
        .map_err(|e| DomainError::InvalidRequest(format!("the request body is not readable: {e}")))
}

/// The digest of a request, for [`IdempotencyGate::claim`]'s `request_hash`.
///
/// # The canonical rendering is the parsed request re-serialized
///
/// Not the raw body. Two bodies differing only in member order, in whitespace or
/// in a trailing newline are the **same request**, and hashing the bytes would
/// make an honest retry through a different HTTP client answer
/// `IDEMPOTENCY_PAYLOAD_MISMATCH` — the refusal that exists for a caller who
/// reused a key for a different intent, spent on a caller who did nothing wrong.
/// Re-serializing the parsed value normalizes all three by construction.
///
/// Two consequences are stated rather than hidden:
///
/// - **An unknown member the request type ignores does not change the digest.**
///   A retry that adds a field the gear does not model is the same request as
///   far as this gate is concerned. That is the correct reading — the gear
///   cannot promise at-most-once over a field it does not act on — but it means
///   the digest is a digest of *what was understood*, not of what was sent.
/// - **Any map inside a request type must be a `BTreeMap`.** `serde_json`
///   preserves a struct's declaration order and a `BTreeMap`'s key order, and
///   neither of those varies between two runs; a `HashMap` would serialize in an
///   order that varies per process, so one caller's retry would digest
///   differently from its own first attempt.
///
/// # Errors
/// [`DomainError::Internal`] when the request value cannot be serialized. It is
/// an internal fault rather than a bad request: the value has already been
/// parsed out of the caller's body, so a failure here is this gear's rendering
/// and nothing the caller can reshape.
pub fn request_digest<T: Serialize>(request: &T) -> Result<Vec<u8>, DomainError> {
    let canonical = serde_json::to_string(request).map_err(|e| {
        DomainError::Internal(format!("cannot render the request for its digest: {e}"))
    })?;
    Ok(IdempotencyGate::payload_hash(&canonical))
}

#[cfg(test)]
#[path = "preconditions_tests.rs"]
mod preconditions_tests;
