//! The grandfathering cutover's commit, across both planes (`inst-gc-commit`, D-100).
//!
//! `infra::supersession`'s shape with one more window and one more row, and the two
//! ordering constraints it records hold here for the same reasons and through the
//! same two mechanisms. What this unit adds is a **second arrival**: the
//! grandfathered copy, on a new generation of the predecessor's key, born in the
//! same transaction as the successor that replaces it.

use aws_lc_rs::digest::{SHA256, digest as sha256};
use chrono::{DateTime, Utc};
use toolkit_db::secure::{AccessScope, DbTx};
use uuid::Uuid;

use crate::domain::audit::{AuditStamp, hex_bytes};
use crate::domain::concurrency::RowVersion;
use crate::domain::cutover::{ComposedCutover, grandfathered_copy_key};
use crate::domain::error::DomainError;
use crate::domain::scope_key::PlanId;
use crate::domain::scope_key::{Cohort, ScopeKey};
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
        plan.cutover_at(),
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

/// Domain separation for the key-set hash, so a digest that names an **act** can
/// never be confused with one that pins **content**.
///
/// The crate's other two digests carry their own
/// ([`CONTENT_PIN_DOMAIN_SEP`](crate::domain::approval::content_pin::CONTENT_PIN_DOMAIN_SEP),
/// `THRESHOLD_PIN_DOMAIN_SEP`), and this one is read by a different comparison
/// than either: the subject register matches it for equality, while a pin is
/// compared against a re-derivation at approve.
const CUTOVER_KEY_SET_SEP: &[u8] = b"VHP-BSS-PRICING-CUTOVER-KEYSET-v1\x1f";

/// The hash of the **selected** keys, as D-28 names it.
///
/// **A set, so it is sorted and deduplicated first.** The selector is a set in the
/// payload and nothing downstream makes it a list, so two orderings of one
/// selection are one act and a key named twice is one member. Sorting is over the
/// canonical [`ScopeKey`] rendering, which is total and stable.
///
/// **Length-framed, so two selections cannot be re-split into each other.** The
/// axis values are operator-supplied strings and nothing forbids a separator
/// character inside one, so a plain join would let two distinct selections share a
/// preimage — the hazard `content_pin`'s framing exists for, met here at a
/// different layer.
///
/// It hashes [`ScopeKey`]'s own rendering rather than an encoding written out
/// here, and that is the point rather than a shortcut: `Display` is fixed at ten
/// segments by its own doc, so this hash discriminates on every axis the key has
/// and gains an eleventh without being edited. A hand-listed encoding is what left
/// `content_pin::put_scope_key` on eight.
fn key_set_hash(selected: &[ScopeKey]) -> String {
    let mut rendered: Vec<String> = selected.iter().map(ScopeKey::to_string).collect();
    rendered.sort_unstable();
    rendered.dedup();

    let mut preimage = CUTOVER_KEY_SET_SEP.to_vec();
    preimage.extend_from_slice(
        &u64::try_from(rendered.len())
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
    );
    for key in &rendered {
        preimage.extend_from_slice(&u64::try_from(key.len()).unwrap_or(u64::MAX).to_be_bytes());
        preimage.extend_from_slice(key.as_bytes());
    }
    hex_bytes(sha256(&SHA256, &preimage).as_ref())
}

/// The name of one cutover **act**, as its approval unit and any retry of the
/// request look it up by: `(planId, key-set hash, cutover instant)`.
///
/// **D-28 (decided 2026-07-10) and S7 §5's API row both spell it that way**, and
/// this function dropped the middle term between `cf6af5c3d` and 2026-08-06,
/// citing `inst-gc-api` — which states no idempotency rule at all. The argument
/// recorded then was that the selection is content and rendering it into the
/// subject would make a narrowed retry a different act. It is a different act:
/// the selection is *what is being cut over*, so a retry that drops a key is a
/// second request, and answering it out of the unit standing for the wider
/// selection is the failure rather than the protection. The caller is told
/// `submitted`, believes their narrower set is under review, and an approver
/// authorizes the key they removed.
///
/// **Nothing is lost by naming the selection, because a second unit over
/// overlapping keys does not open.** `inst-co-single-pending` pends every touched
/// key, so an overlapping second selection is refused at submit by name; a
/// *disjoint* second selection is two genuinely different acts on different keys,
/// and both may proceed. The subject was never what kept the two apart.
///
/// The difference from
/// [`supersession_unit_ref`](crate::infra::supersession::supersession_unit_ref) is
/// therefore only in arity: that act names its one key, this one names its set,
/// and both name what they change.
///
/// The instant is at the millisecond quantum for `supersession_unit_ref`'s reason:
/// it is matched for equality by a retry, and two renderings of one instant at
/// different resolutions are two subjects.
#[must_use]
pub fn cutover_unit_ref(
    plan_id: PlanId,
    selected: &[ScopeKey],
    cutover_at: DateTime<Utc>,
) -> String {
    format!(
        "{}/cutover/{}/{}",
        plan_id.get(),
        key_set_hash(selected),
        cutover_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    )
}

/// The canonical scope keys one cutover entry holds while its unit is pending
/// (`inst-co-single-pending`).
///
/// **Two, and the second is the cutover's own.** The supersession pends the one key
/// it reprices; a cutover also mints a generation, so it pends *"the
/// `all_subscriptions` key and the **new generation's** key — prior generations are
/// not pended"*. Both are needed and neither is spare: without the first, a window
/// mutation could be approved against the key mid-cutover; without the second, two
/// cutovers of one plan at the same instant would each believe the generation free,
/// and the loser would discover it as a partial-`UNIQUE` violation inside its commit
/// rather than as a refusal at submit.
///
/// Prior generations are excluded **by construction rather than by a filter**: this
/// function is handed the two keys the act touches, and a prior generation is not one
/// of them. Stated because "prior generations are not pended" reads like something a
/// filter enforces, and here there is nothing to filter.
///
/// # Errors
///
/// Whatever [`grandfathered_copy_key`] refuses — a generation that already carries
/// the instant, or an axis the constructor will not take.
pub fn cutover_held_keys(
    predecessor: &ScopeKey,
    cutover_at: DateTime<Utc>,
    existing_generations: &[Cohort],
) -> Result<[ScopeKey; 2], DomainError> {
    Ok([
        predecessor.clone(),
        grandfathered_copy_key(predecessor, cutover_at, existing_generations)?,
    ])
}
