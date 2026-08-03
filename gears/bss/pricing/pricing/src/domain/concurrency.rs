//! Optimistic concurrency: the row version, and the entity tag a caller submits
//! it back as.
//!
//! `cpt-cf-bss-pricing-fr-concurrent-edit` (`design/01-foundation.md` §1.2)
//! forbids one outcome above every other: a bulk import and an interactive edit
//! both writing the same draft, the loser's change gone and nobody told. The
//! guard is a version the caller must have **read** before it may write — a
//! submit carrying a version that is no longer current is refused as
//! `STALE_VERSION` (409, §3.3) instead of applied.
//!
//! The type lives in the **domain** even though `ETag` and `If-Match` are HTTP
//! spellings. Optimistic concurrency is a Foundation rule rather than a
//! transport convenience: the repositories and the REST layer have to agree, to
//! the byte, on what tag denotes what version. Were the rendering left to the
//! REST layer it would be re-derived there — a second definition of a valid tag,
//! free to drift from the one that parses tags back, and a drift between those
//! two is a submit that looks fresh and is not.
//!
//! There is deliberately **no increment** here. The bump belongs in SQL —
//! `row_version = row_version + 1` in the same atomic UPDATE that matches on
//! the version the caller read — so the successor is computed by the database
//! under the row lock and never by a caller. A `next()` on this type would let
//! two writers holding the same current version compute the same successor and
//! both write it: the silent overwrite the whole construct exists to prevent,
//! reintroduced by the helper meant to serve it.

use std::fmt;

use toolkit_macros::domain_model;

use crate::domain::error::DomainError;

/// The optimistic-concurrency version of an authoring row.
///
/// Carried as `u64` because a version is a count that only ever goes up, while
/// the storage column is `bigint`. Both directions of that mismatch are
/// **checked** rather than cast ([`RowVersion::from_stored`],
/// [`RowVersion::to_stored`]): a cast would turn a corrupt column into a
/// plausible version number, and a plausible version number is what the whole
/// comparison trusts.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RowVersion(u64);

impl RowVersion {
    /// Wrap a version number.
    #[must_use]
    pub const fn new(version: u64) -> Self {
        Self(version)
    }

    /// The underlying version number.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Rehydrate a version from its `bigint` storage column.
    ///
    /// # Errors
    ///
    /// [`DomainError::Internal`] — **not** [`DomainError::InvalidRequest`] — on
    /// a negative value, naming the column and the value. The column is
    /// `NOT NULL DEFAULT 0` and is only ever incremented, so a negative reading
    /// is a broken invariant: no caller could have caused it and no caller can
    /// reshape a request to avoid it, which is exactly the line between a bad
    /// request and an internal fault.
    pub fn from_stored(stored: i64) -> Result<Self, DomainError> {
        u64::try_from(stored).map(Self).map_err(|_| {
            DomainError::Internal(format!(
                "row_version column holds a negative value: {stored}"
            ))
        })
    }

    /// Render a version for its `bigint` storage column.
    ///
    /// # Errors
    ///
    /// [`DomainError::Internal`] past [`i64::MAX`], naming the column and the
    /// value — the column cannot hold it, and as with
    /// [`RowVersion::from_stored`] there is no request the caller could reshape
    /// to make it fit.
    pub fn to_stored(self) -> Result<i64, DomainError> {
        i64::try_from(self.0).map_err(|_| {
            DomainError::Internal(format!(
                "row_version {} exceeds the bigint column range",
                self.0
            ))
        })
    }

    /// The RFC 9110 **strong** entity tag for this version: the decimal version
    /// in double quotes, e.g. `"12"`.
    ///
    /// Strong on purpose. A weak tag asserts only that two representations are
    /// semantically equivalent, which is a statement about rendering; a write
    /// guard needs a statement about identity.
    #[must_use]
    pub fn to_etag(self) -> String {
        format!("\"{}\"", self.0)
    }

    /// Parse one `If-Match` header value.
    ///
    /// Exactly one strong entity tag is accepted: optional surrounding
    /// whitespace, then a double quote, one or more ASCII digits, a closing
    /// double quote. Everything else is refused rather than coerced, and three
    /// of those refusals are refusals of *meaning* rather than of syntax:
    ///
    /// - a **weak** validator (`W/"12"`) — RFC 9110 §13.1.1 forbids weak
    ///   validators in `If-Match`, because a weak comparison cannot decide
    ///   whether a write is safe;
    /// - the **wildcard** `*` — it means "if the resource exists at all", i.e.
    ///   *overwrite whatever is there*, which is precisely the silent overwrite
    ///   `fr-concurrent-edit` forbids;
    /// - a comma-separated **list** — an authoring mutation targets one known
    ///   version, so picking a member of a list would be guessing which one the
    ///   caller actually read.
    ///
    /// # Errors
    ///
    /// [`DomainError::InvalidRequest`] naming what was wrong with the value, for
    /// every shape that is not one strong entity tag — the three refusals of
    /// meaning above (a weak validator, the wildcard `*`, a comma-separated
    /// list), and the five of syntax: an unquoted bare integer, an empty tag, a
    /// sign, a non-digit, and a version past `u64`.
    pub fn from_etag(raw: &str) -> Result<Self, DomainError> {
        let refuse = |why: &str| DomainError::InvalidRequest(format!("If-Match {raw}: {why}"));
        let digits = strong_tag_body(raw)?;
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err(refuse("the tag must quote one or more ASCII digits"));
        }
        digits
            .parse::<u64>()
            .map(Self)
            .map_err(|_| refuse("the version is past the representable range"))
    }
}

/// Unwrap **one strong entity tag** and hand back the text inside its quotes.
///
/// Extracted from [`RowVersion::from_etag`] rather than copied, because a second
/// reader of `If-Match` arrived: a plan revision's tag has to name the revision
/// it was minted against as well as the version (`api::rest::preconditions`),
/// and if that reader re-implemented the envelope it would be free to accept a
/// weak validator or a wildcard on the very verbs where those are the defect.
/// The three refusals of *meaning* live here once, and both readers get them.
///
/// # Errors
///
/// [`DomainError::InvalidRequest`] for the wildcard `*`, a weak validator
/// (`W/"…"`), a comma-separated list, and anything not wrapped in double quotes.
/// What is **inside** the quotes is the caller's to interpret.
pub fn strong_tag_body(raw: &str) -> Result<&str, DomainError> {
    let refuse = |why: &str| DomainError::InvalidRequest(format!("If-Match {raw}: {why}"));
    let tag = raw.trim();

    if tag == "*" {
        return Err(refuse(
            "the wildcard matches any version and would overwrite whichever one is current",
        ));
    }
    if tag.starts_with("W/") {
        return Err(refuse(
            "RFC 9110 forbids a weak validator here; a weak comparison cannot decide whether a write is safe",
        ));
    }
    if tag.contains(',') {
        return Err(refuse(
            "one entity tag is expected, and a list does not say which version was read",
        ));
    }
    tag.strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .ok_or_else(|| refuse("a strong entity tag is wrapped in double quotes"))
}

impl fmt::Display for RowVersion {
    /// The bare integer — the storage spelling and the body of the entity tag,
    /// so a log line and a wire value are read the same way.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Refuse a submit that is not working from the row's current version.
///
/// A free function rather than a method on a repository: the same comparison is
/// owed by the transport, which decides the conflict from a header, and by a
/// publish unit that has already read the row. One definition serves both, so
/// the surface can never end up laxer than the store.
///
/// # Errors
///
/// [`DomainError::StaleVersion`] naming **both** versions when they differ. An
/// operator reading the rejection needs to see which read the caller was working
/// from, not only that it was stale — that difference is what distinguishes a
/// caller that never refreshed from a genuine bulk-vs-interactive collision.
pub fn require_match(current: RowVersion, submitted: RowVersion) -> Result<(), DomainError> {
    if current == submitted {
        return Ok(());
    }
    Err(DomainError::StaleVersion(format!(
        "current {current}, submitted {submitted}"
    )))
}

#[cfg(test)]
#[path = "concurrency_tests.rs"]
mod concurrency_tests;
