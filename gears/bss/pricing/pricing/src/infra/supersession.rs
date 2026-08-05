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
//! # Rows before windows, and a probe is what decided it
//!
//! The order **between** the two pairs is free for correctness: `window_repo` resolves a
//! window's canonical scope key through `pricing_price` without regard to the row's
//! `lifecycle_state` (verified against every predicate it applies, not from its docs), so
//! a `superseded` predecessor changes nothing either window write sees. What the order
//! decides is **what the loser of a race is told**, and that is measurable.
//!
//! This function wrote the windows first, on the stated ground that their preconditions
//! were the volatile ones while the row's was "a frozen version that only this unit's own
//! siblings move". **That ground was false** (review, 2026-08-05): the presented version
//! is the *successor draft's*, and `PriceRepo::update_draft` bumps it on every mounted
//! `PATCH …/prices/{priceId}`. D-141 freezes *published* rows.
//!
//! With the reason gone, `tests/postgres_supersession_race.rs` was written to measure the
//! difference, and it is not cosmetic. Two commits of one unit — a retry after a timeout,
//! a double approval — serialize on whichever plane is written first:
//!
//! - **windows first**: the loser blocks on the predecessor window's row, finds
//!   `mutation_seq` moved, and is answered [`RepoError::StaleRowVersion`] — which sends an
//!   operator to re-read a **window entity tag** that is not what changed.
//! - **rows first**: the loser blocks on the predecessor *price row*, finds it
//!   `superseded`, and is answered [`RepoError::NotSupersedable`] — whose message names the
//!   actionable remedy, *recompose against the key's new current row*.
//!
//! The probe is the evidence: reordering the pairs flips the refusal, with the same
//! choreography and the same winner. So rows go first, and the argument recorded here is
//! **diagnosis** rather than correctness — which is also why it can be stated plainly
//! instead of defended.
//!
//! The orchestrator does not make this moot, and an earlier note claiming it would was
//! wrong: re-running `plan_supersession` at commit checks the instant, the plane and the
//! content, and **not** whether the predecessor is still its key's current row —
//! `plan_supersession` takes two `PriceRow`s and a window plane, neither of which carries
//! a lifecycle state.
//!
//! It is also a **lock order**, and on Postgres a lock order must be consistent across
//! writers or two transactions deadlock. Nothing violates it: `infra::window`'s mutation
//! unit only *reads* `pricing_price` and writes only `pricing_price_window`, and
//! `publish_rows` touches only price rows, so no writer takes the two in the opposite
//! order. "Free" is about correctness, never a licence to reverse this elsewhere.
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
use crate::domain::supersession::{ComposedWindows, SupersessionPlan, WindowShorten};
use crate::infra::storage::RepoError;
use crate::infra::storage::repo::{NewWindow, WindowRecord, price_repo, window_repo};

/// Everything the supersession unit's commit writes, built **from** the composed
/// plan rather than beside it.
///
/// # The window instants are not two fields, and that is the whole point
///
/// An earlier shape of this struct carried `shorten.effective_to` and
/// `successor_from` as two independently-settable public fields, with a paragraph
/// here defending it: deriving both from one instant at write time would re-decide
/// what [`compose_windows`](crate::domain::supersession::compose_windows) already
/// decided. **That argument was against re-deriving and it does not license
/// declining to assert** — and the gap it permitted commits *silently*, which is
/// worse than every failure this module was built to order.
///
/// Concretely: a shorten to `T1` with a schedule from `T2 > T1` leaves `[T1, T2)`
/// uncovered. Nothing in the transaction notices, because window collision is an
/// **intersection** test (`window_repo`'s `intersects`) and a gap is not an
/// intersection. The result is exactly the `WINDOW_TRAILING_VOID` that
/// `inst-su-compose` promises cannot arise from a committed unit, and exactly the
/// *"no interim state exists in which the key is shortened without its scheduled
/// successor"* that `inst-su-commit` promises — both violated by a committed unit,
/// with no refusal anywhere.
///
/// So the fields are **private** and [`ComposedWindows`] — the value that has been
/// proved adjacent — is the only way to supply them. The disagreement is not
/// refused; it is unspellable. A module whose stated reason to exist is that "no
/// caller is in a position to satisfy one constraint and miss the other" cannot then
/// hand the caller a struct that drops the unit's defining relation.
///
/// # What is still checked at run time, and why it cannot be structural
///
/// Two relations are about rows the store holds rather than values the plan carries,
/// so no constructor can enforce them: that the predecessor and the successor stand
/// on the **same canonical scope key**, and that the successor's
/// `supersedes_price_id` names this predecessor. Both are checked inside
/// [`price_repo::commit_supersession_rows`], where the rows are in hand — see its
/// doc for why the key equality is the load-bearing half of the two.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SupersessionCommit {
    /// The plan whose rows are moving — [`price_repo::publish_rows`]' own filter.
    plan_id: PlanId,
    /// The row leaving the published plane.
    predecessor: Uuid,
    /// Both window operations, as `compose_windows` proved them adjacent.
    windows: ComposedWindows,
    /// The window's **act counter** as the unit was composed against it (D-190/D-191).
    ///
    /// Presented rather than re-read inside the commit, and that is the precondition
    /// doing its job: a commit that read the counter itself would apply the shorten
    /// over an adjustment somebody made between compose and approval, which is the
    /// exact window D-191's precondition exists to close.
    shorten_expected_seq: u64,
    /// The successor row, at the version the rule set judged it at.
    successor: (Uuid, RowVersion),
    /// The successor window's durable name, minted by the surface.
    successor_window_id: Uuid,
    /// The operator-supplied reason the **scheduled** window is recorded under.
    ///
    /// The shorten carries no reason and cannot be given one: `adjust_effective_to`
    /// takes no `reason_code`, and the column is frozen by the append-only trigger on
    /// both backends. That is a **design-set gap rather than a code choice** — D-99
    /// makes the shorten a publish unit in its own right and §6 calls `reason_code`
    /// "the operator-supplied change reason", so the shorten's reason has nowhere in
    /// this schema to live. The only place it can go is the unit's own approval or
    /// audit record, which this module deliberately does not write.
    reason_code: String,
}

impl SupersessionCommit {
    /// Build the commit from a plan `plan_supersession` produced.
    ///
    /// Taking [`SupersessionPlan`] rather than its parts is what makes the adjacency
    /// structural: both window instants come out of the one `changeover` that
    /// [`compose_windows`](crate::domain::supersession::compose_windows) validated the
    /// plane against.
    ///
    /// The identities are separate arguments because none of them is the domain's to
    /// know: the plan reasons about intervals and content, while the row ids, the act
    /// counter and the minted window id are the orchestrator's.
    #[must_use]
    pub const fn of_plan(
        plan: &SupersessionPlan,
        plan_id: PlanId,
        predecessor: Uuid,
        shorten_expected_seq: u64,
        successor: (Uuid, RowVersion),
        successor_window_id: Uuid,
        reason_code: String,
    ) -> Self {
        Self {
            plan_id,
            predecessor,
            windows: plan.windows,
            shorten_expected_seq,
            successor,
            successor_window_id,
            reason_code,
        }
    }

    /// The predecessor window's operation.
    #[must_use]
    pub const fn shorten(&self) -> WindowShorten {
        self.windows.shorten
    }

    /// Where the successor's coverage opens — the same changeover the shorten ends at.
    ///
    /// There is no matching end: `inst-su-compose` schedules the successor
    /// open-ended, and an accessor for one would invite a caller to close a key's
    /// coverage as a side effect of repricing it.
    #[must_use]
    pub const fn successor_from(&self) -> DateTime<Utc> {
        self.windows.successor.effective_from
    }
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
/// [`RepoError::WindowHistorical`] from the window writes (`WindowHistoricalImmutable`
/// is the `DomainError` it maps to, not a variant a caller can match here);
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
    // 1. and 2. The row pair, in the one order that works (D-195). Their own ordering
    //    lives inside that function so it cannot be got wrong here either.
    //
    //    **First, so that a replayed commit is refused where the refusal is
    //    actionable** — see the module doc. This was the other way round until a probe
    //    measured what each order tells the loser of a race.
    price_repo::commit_supersession_rows(
        txn,
        scope,
        tenant_id,
        plan.plan_id,
        plan.predecessor,
        plan.successor,
    )
    .await?;

    // 3. The predecessor's coverage ends at the changeover. Before the schedule,
    //    because the successor's interval is inside this one until it does — the one
    //    ordering here that is about correctness rather than about diagnosis.
    let shortened = window_repo::adjust_effective_to(
        txn,
        scope,
        tenant_id,
        plan.shorten().window_id,
        Some(plan.shorten().effective_to),
        plan.shorten_expected_seq,
        stamp,
    )
    .await?;

    // 4. The successor's coverage opens there. Open-ended by decision, not by
    //    omission — see `successor_from`.
    let scheduled = window_repo::schedule(
        txn,
        scope,
        NewWindow {
            window_id: plan.successor_window_id,
            tenant_id,
            price_id: plan.successor.0,
            effective_from: plan.successor_from(),
            effective_to: None,
            reason_code: plan.reason_code.clone(),
        },
        stamp,
    )
    .await?;

    Ok(SupersessionWritten {
        shortened,
        scheduled,
    })
}

/// The name of one supersession **act**, as its approval unit and any retry of the
/// request look it up by.
///
/// `inst-su-api` makes the act idempotent per **`(planId, scope key, changeover
/// instant)`**, so those three are exactly what this renders — no more, because a
/// component the caller does not control would make a genuine retry a different act, and
/// no fewer, because each of the three is one an operator actually varies:
///
/// - drop the **changeover** and two reprices of one key on two dates share a subject, so
///   an approval of one authorizes the other. That is the defect
///   `infra::window`'s `unit_subject_ref` records for a schedule, one plane over.
/// - drop the **key** and a mass reprice — which names *one* changeover for every row it
///   touches (`inst-su-instant`'s bulk clause) — becomes one act with one approval.
/// - drop the **plan** and nothing else in the string is plan-scoped.
///
/// It carries no successor `price_id` and no window id: both are minted per request, so a
/// retry could not reproduce them, which is the same reason a window schedule's subject
/// names its interval rather than its id.
///
/// **The instant is rendered to the millisecond**, D-144's quantum. Anything coarser would
/// map two distinct legal changeovers onto one act; anything finer names a value
/// `plan_supersession` refuses.
///
/// The unit's `subject_kind` is `price_unit` — the token S5 §6 declares, since its
/// enumeration has no `supersession` member and the gear does not mint tokens the design
/// set has not declared. `supersession` therefore appears in the *subject* instead, which
/// is what tells this act's unit from a `PATCH`'s on the same row.
#[must_use]
pub fn supersession_unit_ref(
    plan_id: PlanId,
    key: &crate::domain::scope_key::ScopeKey,
    changeover: DateTime<Utc>,
) -> String {
    format!(
        "{}/supersession/{key}/{}",
        plan_id.get(),
        changeover.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    )
}

#[cfg(test)]
#[path = "supersession_tests.rs"]
mod supersession_tests;
