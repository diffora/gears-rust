//! The gear's **one** canonical rendering rule, and the `SHA-256` digest
//! taken over it (`design/01-foundation.md` §4.3 "Engine-canonical
//! serialization is pinned here", §3.2 `inst-fd-idem-hash`; **P-D-29**,
//! **P-D-33**, **P-D-34**, **P-D-35**).
//!
//! §4.3 states the rule for a frozen `products_entity_version` row's content
//! and §3.2 reuses it for a parsed request, *"so the gear pins one
//! canonicalization rule rather than two"*. This module is that one place:
//! the rendering lives here, not beside either of its callers, so a later
//! door cannot quietly grow a second answer.
//!
//! # The rule, and which mode each clause belongs to
//!
//! Shared by both modes, verbatim from §4.3:
//!
//! - `JSON`, object keys **sorted lexicographically by field name**, `UTF-8`
//!   without `BOM`, and no insignificant whitespace at all.
//! - Integers and decimals as bare decimal strings, no locale and **no
//!   trailing zeroes** — so `1` and `1.0` render identically and hash equal.
//! - Timestamps `RFC 3339` in `UTC` at microsecond precision. Nothing here
//!   converts one: a caller renders its instant into a string field and that
//!   string is carried verbatim, which keeps the precision decision at the
//!   caller that owns the column rather than in a renderer that would have
//!   to guess a column's type from a `JSON` shape.
//! - Computed **application-side**, so both engines store identical bytes.
//!
//! Split by mode, because §4.3 and **P-D-34** disagree about absence on
//! purpose:
//!
//! - [`Absence::Null`] — §4.3's *complete* named field set, a version row's
//!   content. **An absent value is written `null` rather than omitted**, so
//!   absence and the empty string cannot collide. A complete set is only
//!   complete against something, so this mode carries the roster of field
//!   names that defines it; see the variant's own doc.
//! - [`Absence::Omit`] — §3.2's *parsed request* (**P-D-34**: *"A parsed
//!   request's named field set is the fields the request carries"*). An
//!   omitted field is omitted from the rendering, so a `PATCH` that omits a
//!   field and one that sends it `null` hash **differently**, which is what
//!   they mean at the head door.
//!
//! **An array is rendered in the order received**, in both modes. §4.3 sorts
//! a *row collection* — the category-assignment set, the attribute-value set
//! — by the collection's own identifier, and no payload on this surface
//! carries a collection today: no field on `CreateProductRequest` or
//! `CreateSkuRequest` is an array, and no version row is written yet. The
//! first door whose payload carries a collection owes that sort **here**,
//! rather than at its own call site. It is named as owed rather than
//! pre-built because the sort key is the collection's own identifier and no
//! collection exists yet to name one.
//!
//! # The digest
//!
//! [`content_digest`] is `SHA-256` through `aws-lc-rs`, the platform's
//! `FIPS`-validated provider, reached with the same call the donor gear uses
//! (`gears/bss/pricing`'s `IdempotencyGate::payload_hash`). `sha2`, `sha1`
//! and `md5` are refused outright by architecture lint `DE0708`
//! (`docs/security/SECURITY.md`), which allow-lists direct imports of those
//! crates and does not reach `aws-lc-rs`; §4.3's own text names `SHA-256` as
//! the primitive, so the choice is the design set's rather than this
//! module's.
//!
//! [`DIGEST_VERSION`] is the version a stored row records beside its digest,
//! and it is a **code constant** rather than config (**P-D-33**) — see its
//! own doc for why an edit to it is a migration.

use aws_lc_rs::digest::{SHA256, digest as sha256};
use chrono::{DateTime, Utc};
use serde_json::{Number, Value as JsonValue};

/// The digest version every rendering in this module is computed under, as
/// `products_entity_version.digest_version` stores it (§4.3, **P-D-33**).
///
/// **A code constant, not config.** §4.3 pins it *"as a code constant rather
/// than by config"* for a reason config cannot serve: the rule it makes
/// checkable is *"adding a column to a frozen row's content is a
/// digest-version bump, not a silent change"*, and a value an operator could
/// turn would let one deployment's rows disagree with another's while both
/// claimed version `1`. Storing it on the row is what lets slice 10's
/// restore drill re-verify a sampled entity version against the rule it was
/// actually computed under; without it, version-history corruption is
/// invisible to every checksum.
///
/// **Bumping this is a migration, not an edit.** Every row already written
/// carries the old value, so a bump must arrive with the code that can still
/// recompute the old rendering for those rows — otherwise slice 10's drill
/// re-verifies every historical row against a rule it was never computed
/// under and reports the whole table corrupt. Changing the number alone
/// changes nothing about the bytes and is strictly a lie told to the drill.
///
/// # Why the `composition_pending` addition did not bump it
///
/// The rule §4.3 states is real: *"adding a column to a frozen row's content
/// is a digest-version bump, not a silent change"*. `composition_pending`
/// joined `skus::SKU_VERSION_CONTENT_ROSTER` and this constant stayed at `1`
/// anyway, and the reason is the sentence above about rows, not a reading of
/// the rule.
///
/// **No row has ever been written under version `1`.** The gear has never
/// been deployed; `products_entity_version` is created by an unreleased
/// migration; and the content shape is still being assembled inside the very
/// phase that introduced it. A bump here would mint a version `1` that no row
/// ever used and that no restore drill could ever encounter — a phantom in
/// the one place whose whole value is being an accurate record of what a
/// stored row was computed under. Leaving `1` alone keeps the number
/// truthful: every row that will ever carry it was computed under the shape
/// this code holds now.
///
/// It is the identical disposition, on the identical ground, as this phase's
/// swap of the idempotency digest primitive: free **because** the gear is
/// pre-production, and free for no other reason. Nothing about the rule was
/// weakened.
///
/// **The first content change after deployment must bump, and from that point
/// the rule is unconditional.** Once one row exists carrying version `1`, a
/// roster edit changes the bytes a re-rendering produces for rows the drill
/// will re-verify, and the bump — plus the code that can still reproduce the
/// old rendering for those rows — is what keeps the drill's answer meaningful.
/// The condition that makes today's non-bump correct is exactly *"no stored
/// row"*, and it expires the first time this gear writes one.
pub const DIGEST_VERSION: i32 = 1;

/// Which of §4.3's two readings of *absence* a rendering is taken under.
///
/// The design set states one rendering rule and applies it to two operand
/// kinds whose notion of a named field set genuinely differs, so the mode is
/// an explicit argument rather than a default: a caller that picked the
/// wrong one would produce a plausible string and a wrong digest, and
/// nothing downstream could tell.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Absence<'roster> {
    /// §4.3's **complete** named field set — a frozen version row's content.
    ///
    /// `roster` names every field of the set. A name the value does not
    /// carry is rendered `null`, which is the clause *"absent values written
    /// `null` rather than omitted, so absence and the empty string cannot
    /// collide"*. A set is only *complete* against a declared roster, so the
    /// roster travels with the mode rather than being inferred from the
    /// value: inferring it would make the rule a no-op precisely in the case
    /// it exists for — a field the caller forgot to include.
    ///
    /// Names the value carries that the roster does not name are rendered
    /// too: the rendering is the union, still sorted. Dropping them silently
    /// would hide a mismatch between a caller and the column set it claims
    /// to be freezing, and this module is not the place that judges a
    /// schema.
    ///
    /// The roster applies to the **outermost** object only. §4.3's complete
    /// set is a version row's columns, which are flat; a nested complete set
    /// arrives with the first row collection, and is owed with the
    /// collection sort this module's doc already names as owed.
    Null {
        /// Every field name the complete set contains.
        roster: &'roster [&'roster str],
    },
    /// §3.2's **parsed request** (**P-D-34**).
    ///
    /// The named field set is the fields the request carries, so an absent
    /// field is simply absent from the rendering. An explicit `null` is
    /// carried as `null`, which is what makes an omitted field and a cleared
    /// one two different acts with two different digests.
    Omit,
}

/// The `SHA-256` digest of a canonical rendering, as the 32 raw bytes a
/// `bytea`/`blob` column stores.
///
/// Takes the rendering rather than the value, so that a caller — and a test,
/// and a later reader reproducing a stored digest by hand — can see and pin
/// the exact string that went in. The rendering is the part the design set
/// pins; the digest is a function of it and of nothing else.
///
/// **Only the digest is stored, never the payload** (the donor gear states
/// this the same way): keeping request bodies or frozen content beside their
/// digests would put a second, unmanaged copy of what callers sent next to
/// the audit trail that is supposed to be the one place it lives.
#[must_use]
pub fn content_digest(canonical: &str) -> Vec<u8> {
    sha256(&SHA256, canonical.as_bytes()).as_ref().to_vec()
}

/// Render `value` canonically under `absence`.
///
/// Public because the rendering, not the digest, is what the design set
/// pins: §5's golden vector compares these bytes across engines, and slice
/// 10's restore drill re-verifies stored digests against a re-rendering.
#[must_use]
pub fn canonical_rendering(value: &JsonValue, absence: Absence<'_>) -> String {
    let mut rendered = String::new();
    match (value, absence) {
        (JsonValue::Object(map), Absence::Null { roster }) => {
            render_complete_object(map, roster, &mut rendered);
        }
        _ => render_into(value, &mut rendered),
    }
    rendered
}

/// Decode one stored canonical rendering back into its object — the inverse
/// of [`canonical_rendering`], and deliberately beside it (**P-D-77**,
/// `features/clone.md` §7 row 23): a parse written at a consumer would be
/// the second serialization rule this module exists to prevent.
///
/// The rendering is JSON, so the decode is a JSON parse plus the one check
/// the contract adds: a version row's content is always the rendering of an
/// **object** (the roster describes exactly one object — the outermost one),
/// so anything else is a store this gear wrote wrong, reported in the
/// message rather than admitted. Roster names written `null` by
/// [`Absence::Null`] come back as JSON `null` members — the decoder does
/// not drop them, because "absent" and "absent from the map" are the very
/// distinction the rendering mode exists to keep.
///
/// # Errors
///
/// The parse failure's own text, or the non-object's JSON kind — the caller
/// (a frozen-content reader) wraps either into its `CorruptRow` alarm.
pub fn decode_rendering(canonical: &str) -> Result<serde_json::Map<String, JsonValue>, String> {
    let value: JsonValue =
        serde_json::from_str(canonical).map_err(|e| format!("not valid JSON: {e}"))?;
    match value {
        JsonValue::Object(map) => Ok(map),
        other => Err(format!(
            "a canonical rendering is always an object; found {}",
            match other {
                JsonValue::Null => "null",
                JsonValue::Bool(_) => "a boolean",
                JsonValue::Number(_) => "a number",
                JsonValue::String(_) => "a string",
                JsonValue::Array(_) => "an array",
                JsonValue::Object(_) => "an object",
            }
        )),
    }
}

/// Append the complete-set rendering of one object to `out`: every roster
/// name the map does not carry is written `null`.
///
/// Kept separate from [`render_into`] rather than threading the mode through
/// the recursion, because the roster describes exactly one object — the
/// outermost one — and a mode carried down would silently apply a version
/// row's column names to a nested value that shares none of them.
fn render_complete_object(
    map: &serde_json::Map<String, JsonValue>,
    roster: &[&str],
    out: &mut String,
) {
    let mut entries: Vec<(&str, Option<&JsonValue>)> = map
        .iter()
        .map(|(key, entry)| (key.as_str(), Some(entry)))
        .collect();
    for name in roster {
        if !map.contains_key(*name) {
            entries.push((*name, None));
        }
    }
    entries.sort_unstable_by(|left, right| left.0.cmp(right.0));

    out.push('{');
    for (position, (key, entry)) in entries.into_iter().enumerate() {
        if position > 0 {
            out.push(',');
        }
        out.push_str(&render_string(key));
        out.push(':');
        match entry {
            Some(present) => render_into(present, out),
            // The clause this mode exists for: an absent value is written
            // `null`, not omitted, so absence and the empty string — and
            // absence and any other value a later reader might assume — can
            // never render to the same bytes.
            None => out.push_str("null"),
        }
    }
    out.push('}');
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
/// # The two costs, measured
///
/// An earlier revision of this doc named a third that does not exist —
/// *"a magnitude large or small enough to make `f64`'s `Display` reach
/// exponent form renders in that form"*. Rust's `Display` for floats **never**
/// emits exponent notation; only `{:e}` does. `format!("{}", 1e300f64)` prints
/// a 301-digit integer string, and `format!("{}", 1e-9f64)` prints
/// `0.000000001`. The claim was withdrawn rather than repaired, because the
/// two real costs are both stronger than it was:
///
/// - **A large magnitude renders as a several-hundred-digit integer string.**
///   `1e300` is 301 characters of `JSON` inside the rendering this module
///   hashes, and inside anything a reader reproduces the digest from by hand.
///   Nothing here bounds that length.
/// - **Two distinct `JSON` literals differing below `f64` precision render
///   identically, and therefore hash equal.** `0.1 + 0.2` renders
///   `0.30000000000000004`, and every literal that parses to the same `f64`
///   renders to that same string. At the idempotency door this means two
///   requests that a client considers different are one request, and the
///   second is answered by replaying the first's outcome.
///
/// The second is the one that matters: the first is ugly, the second is a
/// wrong answer. No request field on this surface is numeric today; the first
/// door that adds one owes a decimal-string operand rather than a float, which
/// is §4.3's own rule ("integers and decimals as bare decimal strings") read
/// strictly, and which neither cost can reach.
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

/// One instant, as §4.3's timestamp clause renders it: `RFC 3339` in `UTC` at
/// **microsecond** precision, with the offset spelled `Z`.
///
/// # Why it lives here
///
/// This is the only renderer in the gear that converts an operand rather than
/// carrying it verbatim, which is why the module doc's timestamp clause says
/// the precision decision stays with the caller that owns the column. What
/// that clause does not license is *two* callers each keeping their own copy
/// of the conversion. `api::rest::products` and `api::rest::skus` held one
/// verbatim duplicate each, both recording it as owed a home here; both have
/// been deleted and both doors now call this. A third copy must not be
/// written.
///
/// # It truncates, and rendering alone cannot fix what that costs
///
/// `%.6f` **truncates** the sub-microsecond digits; it does not round them.
/// That is deliberate as a rendering — a renderer that rounded would move a
/// value the caller supplied — but it does not discharge the hazard, and the
/// hazard is a real one against §4.3's *"computed application-side, so both
/// engines store identical bytes"*:
///
/// `created_at` originates from `Utc::now()`, which carries nanoseconds.
/// `SQLite` stores all nine digits, as text. Postgres `timestamptz`
/// **rounds** to microseconds on write. So a head created at
/// `...:00.123456789Z` is read back as `.123456789` from `SQLite` and
/// `.123457` from Postgres; this function renders the first as `.123456` and
/// the second as `.123457`, and the same logical entity is frozen under two
/// different `content` strings and two different `content_digest` values on
/// the two engines. §5's golden vector compares exactly those bytes across
/// engines.
///
/// **Rendering cannot close this.** Truncating here only makes the renderer
/// agree with itself; the two engines disagreed before it was called, because
/// they stored different values. The fix is to truncate the instant to
/// microseconds **where it is written** — at the head-row insert, so that both
/// engines store a value with nothing below the microsecond for either of them
/// to disagree about. That is **still owed**, and this doc is no longer the
/// only place it is recorded: the note now sits at both create doors' own
/// `created_at: now` line, which is the write in question, so the debt is
/// beside the code that would pay it rather than only beside the renderer that
/// cannot. Until it is paid a Postgres-stored `created_at` can disagree with a
/// `SQLite`-stored one in the sixth digit.
#[must_use]
pub fn render_instant(instant: DateTime<Utc>) -> String {
    instant.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
}

#[cfg(test)]
#[path = "canonical_tests.rs"]
mod canonical_tests;
