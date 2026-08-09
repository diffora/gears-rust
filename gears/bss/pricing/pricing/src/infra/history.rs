//! The price-history read (`inst-he-read`), and the projection an export
//! streams (`inst-he-export`) — Slice 12 §3 `algo-history-export`.
//!
//! # There is no history store, and that is the design
//!
//! `inst-he-nostore` is normative: this slice adds **no** new table. The
//! Foundation's `pricing_price` is append-only — a published row is never
//! updated in content, a repricing writes a successor row and a cutover writes a
//! grandfathered one — so the sequence of rows on a scope key already *is* its
//! price history. Everything below is therefore a read, and the only thing that
//! had to be built is the walk order, the page contract and the projection.
//!
//! # The actor is the row's own column, never the audit log
//!
//! D-12 split two surfaces that had been conflated. Price **history** is
//! plan/price data and is read under `plan x read`, which Finance holds by
//! construction; the Slice-5 audit **trail** is `audit x read`, Auditor-only,
//! because its rows carry actor PII discipline the history read is not entitled
//! to. The 2026-07-28 amendment to that decision closed the hole the split left:
//! the history record needs an actor, and the only lawful source is
//! `pricing_price.created_by` — the pseudonymous principal stamped on the row by
//! whichever authoring path wrote it. [`HistoryEntry::actor`] is that column and
//! nothing else, and this module never reads `pricing_audit_log`.
//!
//! It is worth saying what makes that a real constraint rather than a
//! preference: a join to the audit table would be *invisible* in the answers for
//! every row an ordinary authoring path wrote, because those paths stamp the
//! same principal in both places. What it would change is who may read the
//! surface, and that is not something the read gets to decide for itself.
//!
//! # What "chronological" costs, and where the cursor contract stops
//!
//! D-125 asks history for **commit order** specifically — it gives catalog lists
//! deterministic key order and history append order, and they are different
//! promises. `pricing_price` has no commit sequence, so the order here is
//! `(created_at_utc, price_id)` and `created_at_utc` is the *authoring* instant
//! the request carried. [`crate::infra::storage::repo::price_repo::PriceRepo::list_history_page`]
//! states the residue in full: a row whose transaction commits late becomes
//! visible behind a walk that has already passed its instant, and is not
//! returned. Closing that needs a monotonic sequence column assigned in the
//! writing transaction — `pricing_price_window.mutation_seq` is the shape — and
//! therefore a migration. **Reported, not papered over.**
//!
//! The index the walk wants — `(tenant_id, created_at_utc, price_id)` — does not
//! exist either; `pricing_price` is indexed on the plan and on the supersession
//! link. Over a >= 7-year history that is a sort of the tenant's whole row set
//! per page. Also a migration, also reported.
//!
//! # The SLO is not enforced here, and no guard pretends otherwise
//!
//! `inst-he-export` budgets p95 <= 5s per 100-record chunk, scaling linearly to
//! D-125's cap, which is why a 1,000-row page is an export shape and never an
//! interactive read. A latency percentile is measured against a populated store
//! under load; nothing in this module can decide it, and a check that asserted
//! it against an in-memory fixture would be asserting about the test machine.
//! What **is** enforceable is the shape that follows from it, and that is what
//! [`HistoryPageRequest::parse`] does: a caller naming no page size gets the
//! server default of 100 — the interactive read C-6 preserved — and one naming
//! more than the cap is clamped to it.
//!
//! # Effective dates
//!
//! `inst-he-read` asks for records "with actor and effective dates", and a price
//! row's effective dating is its `pricing_price_window` intervals (D-03 puts
//! that store in this gear). A row may hold **several** — a shortened
//! predecessor interval beside its successor's, a cancelled schedule beside the
//! one that replaced it — and §3 does not say which of them a single "effective
//! date" would be. So the entry carries them all, in `effective_from` order,
//! each labelled with its state: that is what the store holds, and picking one
//! would be inventing a rule the design set does not draw. Cancelled and expired
//! intervals are included for `window_repo::list_for_plan`'s reason — this is the
//! store's read and not the read-model projection, and an operator asking what a
//! price was effective for is exactly the reader who needs to see the interval
//! that was withdrawn.

use std::collections::HashMap;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use sea_orm::{ColumnTrait, Condition, EntityTrait, Order};
use toolkit_db::secure::{AccessScope, DBRunner, SecureEntityExt};
use toolkit_db::{DBProvider, DbError};
use uuid::Uuid;

use crate::api::rest::cursor::{DEFAULT_LIMIT, MAX_LIMIT};
use crate::domain::error::DomainError;
use crate::domain::price_record::PriceRecord;
use crate::domain::window::WindowState;
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::price_window;
use crate::infra::storage::repo::PriceRepo;
use crate::infra::storage::repo::price_repo::HistoryPosition;

/// One page's worth of request against the history read.
///
/// The twin of [`crate::api::rest::cursor::PageRequest`], and it exists because
/// that one cannot express this walk's position: its `after` is a `Uuid`, and a
/// commit-ordered walk resumes at an *instant and* a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistoryPageRequest {
    /// How many entries the page may carry, already clamped to D-125's cap.
    pub limit: u64,
    /// The position the previous page ended at; the walk resumes strictly after
    /// it.
    pub after: Option<HistoryPosition>,
}

impl HistoryPageRequest {
    /// Read a page request from the two query parameters D-125 defines.
    ///
    /// # The limit rule is duplicated in shape and single in substance, and the
    /// layer boundary is why
    ///
    /// Delegating to [`crate::api::rest::cursor::PageRequest::parse`] would be
    /// the obvious move and is **not available**: `DE0202` refuses an `api` DTO
    /// *type* named in `infra`, and that rule is right — a request struct is the
    /// transport's shape, and a store-facing reader that took one would make the
    /// transport's vocabulary load-bearing below it. What crosses instead is the
    /// pair of **numbers**, [`DEFAULT_LIMIT`] and [`MAX_LIMIT`], which are D-125's
    /// own and stay declared in exactly one place. The three branches below are
    /// restated; the values they compare against are not, and it is the values
    /// the export SLO is stated per.
    ///
    /// A page size above the cap is **clamped, not refused** — the cap is a
    /// server limit and a caller asking for more has made no mistake it could
    /// correct. A size of **zero** is refused, being a request for no rows at all
    /// rather than a smaller page, and a walk that never advances.
    ///
    /// # Errors
    /// [`DomainError::InvalidRequest`] for `limit = 0` and for a cursor that does
    /// not decode. Neither mints a wire code of its own: an undecodable cursor is
    /// a malformed request under the Foundation validation envelope, the reading
    /// [`crate::api::rest::cursor`] argues once for every list surface.
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

/// The opaque token naming the position a page left the walk at.
///
/// Opaque for [`crate::api::rest::cursor::encode`]'s reason, which is the whole
/// value of the contract: a token a caller can read is a token a caller will
/// construct, and then the walk's guarantee is whatever that caller assumed.
///
/// # The instant is two fields, and that is not padding
///
/// 28 bytes: `i64` seconds, `u32` sub-second nanoseconds, the row's 16-byte id.
/// A **single** count of either unit would have been shorter and wrong. Counting
/// nanoseconds from the epoch overflows an `i64` in 2262, which a >= 7-year
/// retention has no business depending on; counting milliseconds or microseconds
/// silently **truncates** an instant the store holds at finer resolution, and a
/// truncated cursor sorts *before* the row it names — so the next page would
/// open by serving that row a second time. The pair round-trips every value a
/// [`DateTime<Utc>`] can hold, exactly.
#[must_use]
pub fn encode(position: HistoryPosition) -> String {
    let raw: Vec<u8> = position
        .authored_at
        .timestamp()
        .to_be_bytes()
        .into_iter()
        .chain(position.authored_at.timestamp_subsec_nanos().to_be_bytes())
        .chain(*position.price_id.as_bytes())
        .collect();
    URL_SAFE_NO_PAD.encode(raw)
}

/// The position an opaque cursor names.
///
/// # Errors
/// [`DomainError::InvalidRequest`] when the token is not URL-safe base64, is not
/// exactly the 28 bytes [`encode`] writes, or names an instant outside the range
/// a [`DateTime<Utc>`] holds. All three are one refusal, because a caller can act
/// no differently on any of them: the token did not come from this surface.
pub fn decode(raw: &str) -> Result<HistoryPosition, DomainError> {
    let refuse = || {
        DomainError::InvalidRequest(
            "cursor: the token is not one this surface issued; \
             pass back a `next_cursor` verbatim, or omit it to start from the beginning"
                .to_owned(),
        )
    };
    let bytes = URL_SAFE_NO_PAD.decode(raw).map_err(|_| refuse())?;
    let (seconds, rest) = bytes.split_first_chunk::<8>().ok_or_else(refuse)?;
    let (nanos, id) = rest.split_first_chunk::<4>().ok_or_else(refuse)?;
    // Fixes the total length as well as the last field: a longer token leaves
    // more than sixteen bytes here and is refused.
    let id: [u8; 16] = id.try_into().map_err(|_| refuse())?;
    let authored_at =
        DateTime::from_timestamp(i64::from_be_bytes(*seconds), u32::from_be_bytes(*nanos))
            .ok_or_else(refuse)?;
    Ok(HistoryPosition {
        authored_at,
        price_id: Uuid::from_bytes(id),
    })
}

/// One interval a price row was, is, or was to have been effective over.
///
/// Half-open `[from, to)`, exactly as `pricing_price_window` stores it; `to`
/// absent is open-ended and not a missing value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EffectiveInterval {
    /// Inclusive start, UTC.
    pub from: DateTime<Utc>,
    /// Exclusive end, UTC; `None` is open-ended.
    pub to: Option<DateTime<Utc>>,
    /// Where the interval stands in Slice 7's window machine — carried because
    /// a `cancelled` interval never took effect and a reader told only its dates
    /// could not tell it from one that did.
    pub state: WindowState,
}

/// One history record: the append-only row, plus the dates it is effective over.
///
/// The row is carried whole rather than reduced to a summary. Every column of it
/// is something an operator asking "what did this price say" is asking about,
/// and a projection that picked a subset would be a second, quieter statement of
/// what a price row is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryEntry {
    /// The append-only `pricing_price` row, as the store holds it.
    pub record: PriceRecord,
    /// Its `pricing_price_window` intervals, in `from` order; empty when the row
    /// has never been scheduled — a draft, or a clone whose operator has not
    /// scheduled coverage yet.
    pub effective: Vec<EffectiveInterval>,
}

impl HistoryEntry {
    /// Who authored the row — the pseudonymous principal on its **own**
    /// `created_by` column.
    ///
    /// Named rather than left as a field access so the one sentence D-12's
    /// 2026-07-28 amendment turns on has a place to live: the actor of a history
    /// record is the row's column and never `pricing_audit_log`, which D-12
    /// confines to `audit x read`, Auditor-only, while this surface is
    /// `plan x read` and Finance-readable.
    #[must_use]
    pub const fn actor(&self) -> Uuid {
        self.record.created_by
    }
}

/// One page of history, and where the next one resumes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryPage {
    /// The page's entries, oldest first.
    pub entries: Vec<HistoryEntry>,
    /// The position to resume at, or `None` when the walk is exhausted.
    ///
    /// `None` on the last page is what lets a client stop **without** issuing the
    /// extra request that returns nothing, and it is why the read asks the store
    /// for one row more than the page.
    pub next: Option<HistoryPosition>,
}

/// §1.7's `HistoryExporter`: the chronological read both the interactive
/// history surface and the export stream are pages of.
///
/// One type for both, because `inst-he-export` describes the same order in
/// bounded chunks and differs only in the page size a caller asks for. Two types
/// would be two walk orders free to disagree, over a store whose whole claim is
/// that the sequence of rows is the truth.
#[derive(Clone)]
pub struct HistoryExporter {
    prices: PriceRepo,
    db: DBProvider<DbError>,
}

impl HistoryExporter {
    /// Build the reader over one database provider.
    #[must_use]
    pub fn new(db: DBProvider<DbError>) -> Self {
        Self {
            prices: PriceRepo::new(db.clone()),
            db,
        }
    }

    /// One page of the tenant's price history, oldest first.
    ///
    /// # Errors
    /// [`RepoError::Db`] on a scope or storage failure;
    /// [`RepoError::CorruptRow`] when a stored row or window token cannot be read
    /// as the domain value its column is `CHECK`-constrained to hold.
    pub async fn read_page(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        request: HistoryPageRequest,
    ) -> Result<HistoryPage, RepoError> {
        // One row more than the page, so "is there another page" is answered
        // without a second query and without a page whose `next` points at
        // nothing. The repository does not decide this: only the surface knows
        // the page size it promised.
        let probe = request.limit.saturating_add(1);
        let mut records = self
            .prices
            .list_history_page(scope, tenant_id, request.after, probe)
            .await?;
        let has_more = u64::try_from(records.len()).unwrap_or(u64::MAX) > request.limit;
        if has_more {
            records.pop();
        }
        let next = has_more
            .then(|| records.last().map(HistoryPosition::of))
            .flatten();

        let conn = self
            .db
            .conn()
            .map_err(|e| RepoError::Db(format!("conn: {e}")))?;
        let mut intervals = effective_intervals(&conn, scope, tenant_id, &records).await?;
        let entries = records
            .into_iter()
            .map(|record| {
                let effective = intervals.remove(&record.price_id).unwrap_or_default();
                HistoryEntry { record, effective }
            })
            .collect();
        Ok(HistoryPage { entries, next })
    }
}

/// The page's rows' effective dating, in one query rather than one per row.
///
/// Read here rather than through `window_repo::list_for_plan` because that read
/// is per **plan** and rebuilds every window's canonical scope key on the way
/// out; a history page is a set of rows from arbitrary plans, and the key it
/// would resolve is already on the record beside it.
async fn effective_intervals(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    records: &[PriceRecord],
) -> Result<HashMap<Uuid, Vec<EffectiveInterval>>, RepoError> {
    let mut grouped: HashMap<Uuid, Vec<EffectiveInterval>> = HashMap::new();
    if records.is_empty() {
        return Ok(grouped);
    }
    let ids: Vec<Uuid> = records.iter().map(|record| record.price_id).collect();
    let rows = price_window::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(price_window::Column::TenantId.eq(tenant_id))
                .add(price_window::Column::PriceId.is_in(ids)),
        )
        .order_by(price_window::Column::EffectiveFrom, Order::Asc)
        .order_by(price_window::Column::WindowId, Order::Asc)
        .all(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read price history windows: {e}")))?;

    for row in rows {
        let state = WindowState::from_token(&row.state).ok_or_else(|| {
            RepoError::CorruptRow(format!("pricing_price_window.state: {}", row.state))
        })?;
        grouped
            .entry(row.price_id)
            .or_default()
            .push(EffectiveInterval {
                from: row.effective_from,
                to: row.effective_to,
                state,
            });
    }
    Ok(grouped)
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod history_tests;
