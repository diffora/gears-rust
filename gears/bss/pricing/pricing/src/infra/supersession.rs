//! The supersession unit's commit, across both planes (`inst-su-commit`, D-88).
//!
//! [`crate::domain::supersession`] decides — the changeover's two floors, the two
//! window operations, and whether the successor's content may land on a continued
//! counter. This module *writes* what that decided, and it exists for one property
//! neither the row repository nor the window repository can hold on its own:
//! **four writes on two planes, in one transaction, in one order**.
//!
//! # The order is the whole of it, and it is two constraints rather than one
//!
//! `inst-su-commit` says "or everything rolls back" and D-195 makes the *row* order
//! normative. Building it turned up a second ordering constraint of the same shape on
//! the window plane, enforced by a different mechanism:
//!
//! 1. **The predecessor's window is shortened before the successor's is scheduled.**
//!    `window_repo::schedule` refuses an interval intersecting one already on the key,
//!    so an open-ended successor scheduled while the predecessor's window is still
//!    open-ended is `WINDOW_OVERLAP` — the refusal `inst-su-compose` promises a
//!    committed unit can never produce, arriving from the store instead.
//! 2. **The predecessor's row leaves the published plane before the successor's
//!    arrives** ([`price_repo::commit_supersession_rows`], D-195): §3.7 admits one
//!    published row per key, and the wrong order is a raw driver error rather than a
//!    refusal.
//!
//! Two constraints, two mechanisms, one function — so no caller is in a position to
//! satisfy one and miss the other. That is the same argument
//! `commit_supersession_rows` makes about its own pair, applied one level up, and it
//! is why this composition is not a convenience.
//!
//! # Windows before rows, and it is a choice with a reason
//!
//! The four writes are ordered windows-then-rows. Nothing forces it — the two pairs
//! are independent, since `window_repo` resolves a window's key through
//! `pricing_price` without asking the row's lifecycle state — so the reason is about
//! which failure is cheapest to reach: the window operations carry the two
//! preconditions most likely to have gone stale between compose and approval (an
//! `effectiveTo` somebody adjusted, an interval somebody scheduled), while the row
//! preconditions are a state and a frozen version that only this unit's own siblings
//! move. Refusing on the volatile half first means the common refusal does the least
//! work before it answers. Both orders are correct; this one is faster to be wrong in.
//!
//! # What this module does **not** do
//!
//! It requests no `CatalogVersion`, enqueues no event, writes no approval record and
//! no audit entry. `inst-su-commit` names all of those, and they belong to the service
//! that owns the unit — the same division `price_repo::publish_rows` keeps against
//! `PublishService::commit`, whose step 6 writes one record for the whole act rather
//! than one per row moved. What is here is the part that has to be one transaction
//! whatever the layer above decides.

use chrono::{DateTime, Utc};
use toolkit_db::secure::{AccessScope, DbTx};
use uuid::Uuid;

use crate::domain::audit::AuditStamp;
use crate::domain::concurrency::RowVersion;
use crate::domain::scope_key::PlanId;
use crate::domain::supersession::WindowShorten;
use crate::infra::storage::RepoError;
use crate::infra::storage::repo::{NewWindow, WindowRecord, price_repo, window_repo};

/// Everything the supersession unit's commit writes, as the composed plan decided
/// it.
///
/// A struct rather than nine parameters, and the fields are deliberately the
/// *decided* values rather than the inputs they were derived from: the changeover
/// appears twice — once as the predecessor window's new end and once as the
/// successor window's start — because those are two writes, and a commit that took
/// one instant and derived both would be re-deciding at write time what
/// [`compose_windows`](crate::domain::supersession::compose_windows) already decided
/// and proved adjacent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SupersessionCommit {
    /// The plan whose rows are moving — [`price_repo::publish_rows`]' own filter.
    pub plan_id: PlanId,
    /// The row leaving the published plane.
    pub predecessor: Uuid,
    /// Its window, and the end the changeover moves it to.
    pub shorten: WindowShorten,
    /// The window's **act counter** as the unit was composed against it (D-190/D-191).
    ///
    /// Presented rather than re-read inside the commit, and that is the precondition
    /// doing its job: a commit that read the counter itself would apply the shorten
    /// over an adjustment somebody made between compose and approval, which is the
    /// exact window D-191's precondition exists to close.
    pub shorten_expected_seq: u64,
    /// The successor row, at the version the rule set judged it at.
    pub successor: (Uuid, RowVersion),
    /// The successor window's durable name, minted by the surface.
    pub successor_window_id: Uuid,
    /// Where the successor's coverage opens — the changeover.
    ///
    /// There is no matching end: `inst-su-compose` schedules the successor
    /// open-ended, and a field for it would invite a caller to close a key's coverage
    /// as a side effect of repricing it.
    pub successor_from: DateTime<Utc>,
    /// The operator-supplied reason both window writes are recorded under.
    pub reason_code: String,
}

/// The two window rows as they stand after the commit.
///
/// Returned so the layer above can build `inst-su-return`'s events from what was
/// **written** rather than from what was asked for — the distinction
/// `infra::window`'s `pending_approval` makes for the same reason one plane over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SupersessionWritten {
    /// The predecessor's window, shortened.
    pub shortened: WindowRecord,
    /// The successor's window, scheduled.
    pub scheduled: WindowRecord,
}

/// Apply the supersession unit's four writes, in the caller's transaction.
///
/// See the module doc for the two ordering constraints and why they are one function.
///
/// # Errors
/// [`RepoError::StaleRowVersion`] from either precondition — the window's act counter
/// or the successor's row version; [`RepoError::WindowOverlap`] or
/// [`RepoError::WindowHistoricalImmutable`] from the window writes;
/// [`RepoError::NotSupersedable`] when the predecessor is no longer the key's current
/// row; [`RepoError::NotFound`] when anything named is invisible to `scope`;
/// [`RepoError::Db`] on a storage failure. Every one of them rolls the whole unit
/// back, which is the point.
pub async fn commit_supersession(
    txn: &DbTx<'_>,
    scope: &AccessScope,
    tenant_id: Uuid,
    plan: SupersessionCommit,
    stamp: AuditStamp,
) -> Result<SupersessionWritten, RepoError> {
    // 1. The predecessor's coverage ends at the changeover. First, because the
    //    successor's interval is inside this one until it does.
    let shortened = window_repo::adjust_effective_to(
        txn,
        scope,
        tenant_id,
        plan.shorten.window_id,
        Some(plan.shorten.effective_to),
        plan.shorten_expected_seq,
        stamp,
    )
    .await?;

    // 2. The successor's coverage opens there. Open-ended by decision, not by
    //    omission — see `successor_from`.
    let scheduled = window_repo::schedule(
        txn,
        scope,
        NewWindow {
            window_id: plan.successor_window_id,
            tenant_id,
            price_id: plan.successor.0,
            effective_from: plan.successor_from,
            effective_to: None,
            reason_code: plan.reason_code,
        },
        stamp,
    )
    .await?;

    // 3. and 4. The row pair, in the one order that works (D-195). Their own
    //    ordering lives inside that function so it cannot be got wrong here either.
    price_repo::commit_supersession_rows(
        txn,
        scope,
        tenant_id,
        plan.plan_id,
        plan.predecessor,
        plan.successor,
    )
    .await?;

    Ok(SupersessionWritten {
        shortened,
        scheduled,
    })
}
