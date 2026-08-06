//! The grandfathering cutover's commit, across both planes (`inst-gc-commit`, D-100).
//!
//! `infra::supersession`'s shape with one more window and one more row, and the two
//! ordering constraints it records hold here for the same reasons and through the
//! same two mechanisms. What this unit adds is a **second arrival**: the
//! grandfathered copy, on a new generation of the predecessor's key, born in the
//! same transaction as the successor that replaces it.

use chrono::{DateTime, Utc};
use toolkit_db::secure::{AccessScope, DbTx};
use uuid::Uuid;

use crate::domain::audit::AuditStamp;
use crate::domain::concurrency::RowVersion;
use crate::domain::cutover::ComposedCutover;
use crate::domain::scope_key::PlanId;
use crate::domain::window::WindowInterval;
use crate::infra::storage::RepoError;
use crate::infra::storage::repo::window_repo::{NewWindow, WindowRecord};
use crate::infra::storage::repo::{price_repo, window_repo};

/// Everything the cutover's commit writes, named by the orchestrator and **not
/// separable**.
///
/// The fields are private and the three instants come out of one
/// [`ComposedCutover`], which is D-88's review lesson applied before it can cost
/// anything: there, two independent public instants let a caller compose a shorten to
/// `T1` and a schedule from `T2 > T1`, leaving `[T1, T2)` uncovered and committing
/// cleanly, because collision is an intersection test and a gap is not an
/// intersection. This unit schedules **two** windows, so the same hazard would be
/// here twice.
///
/// The identities are separate arguments because none of them is the domain's to
/// know: it reasons about intervals and content, while the row ids, the act counter
/// and the minted window ids are the orchestrator's.
#[derive(Clone, Debug)]
pub struct CutoverCommit {
    plan_id: PlanId,
    predecessor: Uuid,
    windows: ComposedCutover,
    shorten_expected_seq: u64,
    successor: (Uuid, RowVersion),
    successor_window_id: Uuid,
    copy: (Uuid, RowVersion),
    copy_window_id: Uuid,
    reason_code: String,
}

impl CutoverCommit {
    /// Build the commit from a composition the domain has already proven adjacent.
    #[must_use]
    #[allow(
        clippy::too_many_arguments,
        reason = "the composition plus four identities the domain cannot know: two row ids with \
                  their versions, two minted window ids, the act counter and the reason code. \
                  Collapsing them into a struct would move the argument list rather than shorten \
                  it, and the composition is already the one thing that must not be assembled \
                  field by field"
    )]
    pub const fn of_composition(
        windows: ComposedCutover,
        plan_id: PlanId,
        predecessor: Uuid,
        shorten_expected_seq: u64,
        successor: (Uuid, RowVersion),
        successor_window_id: Uuid,
        copy: (Uuid, RowVersion),
        copy_window_id: Uuid,
        reason_code: String,
    ) -> Self {
        Self {
            plan_id,
            predecessor,
            windows,
            shorten_expected_seq,
            successor,
            successor_window_id,
            copy,
            copy_window_id,
            reason_code,
        }
    }

    /// The successor's interval, as the composition proved it.
    #[must_use]
    pub const fn successor_window(&self) -> WindowInterval {
        self.windows.successor()
    }

    /// The copy's interval, on the new generation's key.
    #[must_use]
    pub const fn copy_window(&self) -> WindowInterval {
        self.windows.copy()
    }

    /// The instant all three operations pivot on.
    #[must_use]
    pub const fn cutover_at(&self) -> DateTime<Utc> {
        self.windows.shorten().effective_to
    }
}

/// What the commit wrote, for the caller to announce.
#[derive(Clone, Debug)]
pub struct CutoverWritten {
    /// The predecessor's window, now ending at the cutover.
    pub shortened: WindowRecord,
    /// The successor's window.
    pub successor_window: WindowRecord,
    /// The grandfathered copy's window.
    pub copy_window: WindowRecord,
}

/// Commit the cutover: five writes, one transaction (`inst-gc-commit`, D-03).
///
/// **The rows go first**, for `commit_supersession_rows`' diagnosis reason: a
/// replayed commit then blocks on the predecessor *row* and is refused by name —
/// recompose against the key's new current row — instead of blocking on the
/// predecessor *window* and being told an entity tag is stale, which is not what
/// changed.
///
/// **The shorten precedes both schedules**, and that is correctness rather than
/// preference: `window_repo::schedule` refuses an interval intersecting one already
/// on the key, and the successor's open-ended `[cutover, …)` sits inside the
/// predecessor's still-open interval until it is shortened. The copy is on a
/// different key and could go in any order; it follows the successor so that the two
/// arrivals stay adjacent in one reading.
///
/// **`shorten_expected_seq` is presented, never re-read.** A commit that read the
/// window's act counter itself would apply the shorten over an adjustment made
/// between compose and approval — exactly the gap D-191's precondition closes.
///
/// # Errors
///
/// Whatever the row plane refuses (`price_repo::commit_cutover_rows`), and whatever
/// the window plane refuses on the shorten or either schedule.
pub async fn commit_cutover(
    txn: &DbTx<'_>,
    scope: &AccessScope,
    tenant_id: Uuid,
    plan: CutoverCommit,
    stamp: AuditStamp,
) -> Result<CutoverWritten, RepoError> {
    price_repo::commit_cutover_rows(
        txn,
        scope,
        tenant_id,
        plan.plan_id,
        plan.predecessor,
        plan.successor,
        plan.copy,
    )
    .await?;

    let shortened = window_repo::adjust_effective_to(
        txn,
        scope,
        tenant_id,
        plan.windows.shorten().window_id,
        Some(plan.windows.shorten().effective_to),
        plan.shorten_expected_seq,
        stamp,
    )
    .await?;

    let successor_window = window_repo::schedule(
        txn,
        scope,
        NewWindow {
            window_id: plan.successor_window_id,
            tenant_id,
            price_id: plan.successor.0,
            effective_from: plan.successor_window().effective_from,
            effective_to: plan.successor_window().effective_to,
            reason_code: plan.reason_code.clone(),
        },
        stamp,
    )
    .await?;

    let copy_window = window_repo::schedule(
        txn,
        scope,
        NewWindow {
            window_id: plan.copy_window_id,
            tenant_id,
            price_id: plan.copy.0,
            effective_from: plan.copy_window().effective_from,
            effective_to: plan.copy_window().effective_to,
            reason_code: plan.reason_code.clone(),
        },
        stamp,
    )
    .await?;

    Ok(CutoverWritten {
        shortened,
        successor_window,
        copy_window,
    })
}
