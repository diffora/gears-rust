//! D-125's collection contract, decided once for every list surface this gear
//! serves: `limit` (server default 100, hard cap 1,000) plus an **opaque**
//! `cursor`, over a stable key order, with `next_cursor` on every page until the
//! result is exhausted.
//!
//! # Why a keyset walk and not an offset
//!
//! D-125 forbids offset pagination outright, and the reason is the store rather
//! than taste: these tables are append-only over a >= 7-year retention, so
//! `OFFSET n` names a different row every time somebody inserts ahead of it. A
//! keyset walk names a **row**, so an insert ahead of the cursor cannot shift
//! what the next page starts at.
//!
//! # The stability guarantee, and exactly where it stops
//!
//! D-125 promises a walk that "never skips or duplicates a row at or before the
//! cursor". A keyset walk on `price_id ASC` gives that for **inserts**: a row
//! inserted ahead of the cursor is simply not visited (it sorts before the
//! walk's position), and one inserted behind is visited once.
//!
//! It does **not** give it for a **delete**. Draft price rows are deletable
//! (D-141), and a row deleted behind the cursor was already returned to the
//! caller, while one deleted ahead of it is never returned at all. Neither is a
//! skip or a duplicate of a row *at or before* the cursor — the letter of D-125
//! holds — but a caller that walks a page set and reassembles it does not get a
//! snapshot, and no cursor over a mutable draft plane can give one. This is a
//! **deliberately absent** guarantee, stated so a reader does not assume it, not
//! a defect to paper over: providing it needs an MVCC snapshot held across
//! pages, which is a transaction spanning HTTP requests.
//!
//! # `prev_cursor` is always `None`
//!
//! [`toolkit_odata::PageInfo`] carries the field because other surfaces on the
//! platform use it. D-125 specifies a **forward** walk only, and a backward
//! cursor over an append-only store is a guarantee nothing here can keep: the
//! rows behind a cursor are exactly the ones a concurrent delete may have
//! removed, so a backward page would be answering about a set that no longer
//! exists. Serving `null` says so; serving a token that sometimes works would
//! not.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use toolkit_odata::PageInfo;
use uuid::Uuid;

use crate::domain::error::DomainError;

/// Rows per page when the caller names no `limit` (D-125).
pub const DEFAULT_LIMIT: u64 = 100;

/// The hard cap on `limit` (D-125) — the unit the export SLO is expressed in.
pub const MAX_LIMIT: u64 = 1_000;

/// One page's worth of request: how many rows, and where to resume.
///
/// `after` is the **decoded** cursor, so nothing below this module ever sees the
/// encoding. That is what "opaque" has to mean to be worth anything: a client
/// that decoded the token and constructed its own would be depending on a
/// representation this module is free to change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageRequest {
    /// How many rows the page may carry, already clamped to [`MAX_LIMIT`].
    pub limit: u64,
    /// The key the previous page ended at; the walk resumes strictly after it.
    pub after: Option<Uuid>,
}

impl PageRequest {
    /// Read a page request from the two query parameters.
    ///
    /// **A `limit` above the cap is clamped, not refused.** The cap is a
    /// *server* limit — the page size the SLO is stated per — and a caller
    /// asking for more has made no mistake it could correct: refusing would
    /// force every client to know a number the server owns. A `limit` of
    /// **zero** is refused, because it is not a smaller page but a request for
    /// no rows at all, which no caller means and which would walk forever.
    ///
    /// # Errors
    /// [`DomainError::InvalidRequest`] for `limit = 0`, and for a cursor that
    /// does not decode. An undecodable cursor mints no code of its own — it is a
    /// malformed request under the Foundation validation envelope, the same
    /// reading D-141 gives an absent `If-Match`.
    pub fn parse(limit: Option<u64>, cursor: Option<&str>) -> Result<Self, DomainError> {
        let limit = match limit {
            None => DEFAULT_LIMIT,
            Some(0) => {
                return Err(DomainError::InvalidRequest(
                    "limit must be at least 1; a page of zero rows never advances".to_owned(),
                ));
            }
            Some(asked) => asked.min(MAX_LIMIT),
        };
        let after = cursor.map(decode).transpose()?;
        Ok(Self { limit, after })
    }
}

/// The `limit` query parameter, read out of its raw text.
///
/// # Why the query types carry `Option<String>` and this exists
///
/// A `Query<T>` member typed `Option<u64>` is parsed by **axum's** extractor, and
/// its rejection is a bare `400` with *no problem document at all* — against a
/// registration whose declared 400 has `Problem` as its schema. That is the same
/// defect class as a handler taking the `Json` extractor, one axis over:
/// `?limit=abc` on any of the nine paginated reads answered outside the canonical
/// envelope, so a client keying on the document got nothing to key on.
///
/// `windows.rs`'s `SellabilityQuery` recorded the lesson and applied it to one
/// struct of twelve. This is the shared half, so the nine paginated reads cannot
/// each phrase the refusal differently.
///
/// # Errors
/// [`DomainError::InvalidRequest`] naming the parameter, for a value that is not a
/// non-negative integer. No new wire code — a parameter this gear cannot interpret
/// is the Foundation validation envelope's own case, the same reading D-141 gives
/// an absent `If-Match` and [`decode`] gives an undecodable cursor.
pub fn parse_limit(raw: Option<&str>) -> Result<Option<u64>, DomainError> {
    raw.map(|value| {
        value.trim().parse::<u64>().map_err(|_| {
            DomainError::InvalidRequest(format!(
                "limit: `{value}` is not a whole number of rows; send a positive integer, or omit \
                 it for the server default of {DEFAULT_LIMIT}"
            ))
        })
    })
    .transpose()
}

/// A `Uuid`-valued query parameter, read out of its raw text.
///
/// [`parse_limit`]'s reason on the other type: `Option<Uuid>` at the query struct
/// hands the refusal to axum's extractor. `subject` names the parameter in the
/// refusal, because a caller told only "invalid argument" over a request carrying
/// two ids cannot act.
///
/// # Errors
/// [`DomainError::InvalidRequest`] naming the parameter and the value.
pub fn parse_uuid_param(subject: &str, raw: Option<&str>) -> Result<Option<Uuid>, DomainError> {
    raw.map(|value| {
        value
            .trim()
            .parse::<Uuid>()
            .map_err(|_| DomainError::InvalidRequest(format!("{subject}: `{value}` is not a UUID")))
    })
    .transpose()
}

/// The opaque token naming the last row of a page.
///
/// The encoding — URL-safe base64 without padding, over the key's 16 raw bytes —
/// is declared **here and nowhere else**. It is deliberately not the key's own
/// text form: a token a caller can read is a token a caller will construct, and
/// then the walk's contract is whatever that caller assumed rather than what
/// this module promises.
#[must_use]
pub fn encode(after: Uuid) -> String {
    URL_SAFE_NO_PAD.encode(after.as_bytes())
}

/// The key an opaque cursor names.
///
/// # Errors
/// [`DomainError::InvalidRequest`] when the token is not URL-safe base64, or
/// does not decode to exactly the 16 bytes of a key. No new wire code: an
/// undecodable cursor is a request this gear cannot interpret, which is the
/// Foundation validation envelope's own case.
pub fn decode(raw: &str) -> Result<Uuid, DomainError> {
    let refuse = || {
        DomainError::InvalidRequest(
            "cursor: the token is not one this surface issued; \
             pass back a `next_cursor` verbatim, or omit it to start from the beginning"
                .to_owned(),
        )
    };
    let bytes = URL_SAFE_NO_PAD.decode(raw).map_err(|_| refuse())?;
    Uuid::from_slice(&bytes).map_err(|_| refuse())
}

/// The page envelope's cursor block.
///
/// `next` is `Some` only while the walk can continue: a page that exhausted the
/// result carries `null`, which is what lets a client stop **without** issuing
/// the extra request that returns an empty page. See the module doc for why
/// `prev_cursor` is always `null`.
#[must_use]
pub fn page_info(next: Option<Uuid>, limit: u64) -> PageInfo {
    PageInfo {
        next_cursor: next.map(encode),
        prev_cursor: None,
        limit,
    }
}

#[cfg(test)]
#[path = "cursor_tests.rs"]
mod cursor_tests;
