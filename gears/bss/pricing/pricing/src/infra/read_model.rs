//! The `ReadModelProjector` — what turns a committed `CatalogVersion` into
//! something a consumer can pin.
//!
//! G5's publish commit deliberately stops one step short: it writes the flipped
//! revision, the published rows, a **pending** `pricing_catalog_version_ref`
//! carrying its subject (D-157), an undrained `PlanPublished` and an audit row,
//! and **nothing** in `pricing_read_model` or `pricing_pin_frontier`. It cannot
//! do otherwise — that table is keyed `catalog_version NOT NULL` and the commit
//! holds a handle, not a number. This module begins there: resolve the handle,
//! project the subject it published, write the delta warm, and advance the
//! frontier if that completed the version.
//!
//! It lives in `infra` rather than in `domain` because it holds an
//! `AccessScope` and repositories, which dylint DE0301 forbids inside
//! `src/domain/**` — the same reason [`crate::infra::publish`] is here, and the
//! sibling ledger puts `infra::posting::service` here. The projected payload's
//! **vocabulary and rendering** stay pure, in [`crate::domain::projection`].
//!
//! # Where the version comes from: a pull, not a push
//!
//! D-66 settled the emitter — `CatalogVersionPublished` is the **registry's**
//! event, tenant-wide, and deliberately not in this gear's frozen event set.
//! `gears/bss/products` is docs only, there is no event broker in this
//! repository at all (`config.events_enabled` defaults `false` and says why),
//! so an inbound port defined here would be a trait with neither producer nor
//! caller.
//!
//! The pull is already contracted. `CatalogVersionRegistryV1::committed_version`
//! resolves a pending ref to its committed version, and its own doc hands the
//! overdue decision to the caller "since only it knows how long the ref has
//! been outstanding" — which is this gear's sweep over
//! `pricing_catalog_version_ref.requested_at`. That port was written for a
//! reconciler and had never been called.
//!
//! **When the registry gear lands, its event handler hooks to
//! [`ReadModelProjector::project_version`]** — this entry point, unchanged —
//! and the sweep stays as the backstop §3.6's own remediation posture already
//! asks for ("a `CatalogVersionPublished` batch that omits an expected pending
//! ref is treated the same; remediation is a registry re-request, never a
//! silent re-emit").
//!
//! # Three premises, each stated as a premise
//!
//! The [`crate::infra::publish`] posture, three times. Each is a place where
//! what is built and what the design set says are not the same thing, and each
//! is written here rather than reconciled by editing a document.
//!
//! ## 1. D-121's horizon is absent, and what holds it is that the set it
//! filters is empty
//!
//! D-121 projects "every price row of the plan whose window set intersects
//! `[projection_time - H, ...)`", `H = 2 x` the longest billing cycle sold on
//! the plan. There is **no window store in this gear** — Slice 7's, unbuilt —
//! so `pricing_price` carries no interval columns and the predicate cannot be
//! evaluated at all. Taken literally it would project *nothing*: no row has a
//! window set, so no row intersects the horizon.
//!
//! What the bound buys is stated in D-121's own words: "a delta is O(live keys
//! x windows within `H`), never O(the plan's accumulated history)". A plan's
//! accumulated history is its `superseded` price rows — and
//! `published -> superseded` has exactly two sanctioned producers, the D-88
//! supersession unit and the D-100 cutover, **neither of which exists**.
//! [`crate::infra::publish`] already carries that premise in its CONTRACT
//! paragraph and names the group that deletes it. So the set D-121 filters is,
//! today, exactly the plan's `published` rows, capped at 500 by §14.
//!
//! **The group that builds the supersession unit or the cutover deletes this
//! premise and owes the horizon** — and, with it, `H`'s own input, "the longest
//! billing cycle sold on the plan". This paragraph is where they will find out
//! that they do.
//!
//! The cost of projecting anyway is real and is declared per fact in
//! [`crate::domain::projection`]: a version produced before Slice 7 cannot
//! answer sellability predicate (1) or the D-80 coverage horizon. What makes it
//! bearable rather than reckless is that **nothing reads a delta payload
//! today and nothing can** — `bss_pricing_sdk::api` says resolution "arrives
//! with the slices that own those payloads", there is no resolution route, no
//! in-process resolve method and no consumer gear here.
//!
//! ## 2. D-136's advance strands a version that completes out of order
//!
//! Read literally, the frontier advances only in the transaction completing
//! "the frontier's **next** version in order". §4.4's own D-114 worked example
//! is `V5` degraded and `V6` fully warm: when `V5` finally completes the
//! frontier moves to `V5` — and `V6`, already complete, will never see another
//! completion to advance on. The frontier would stand at `V5` forever with a
//! complete `V6` behind it.
//!
//! So [`advance_frontier`] **walks forward** after an advance, moving the
//! watermark through every immediately-following version that is already fully
//! warm. That is strictly within "only ever forward" and "never past a gap" —
//! what it is not is the literal sentence. **Reported.**
//!
//! ## 3. §4.4's "current revision" is the wrong source, and the ref row carries
//! what is right
//!
//! If revision 3 publishes into `V5` and revision 4 publishes into `V6` before
//! `V5` warms, projecting `V5` from the plan's *current* revision freezes
//! revision 4's content into `V5`'s delta. §4.4 defends the re-drive against
//! **draft** leakage and says nothing at all about a later **published**
//! revision — and the window is up to D-47's five-minute batching maximum
//! rather than the 5s warm SLO, so this is reachable rather than theoretical.
//! Permanently, too: a delta is INSERT-only on the seven-year horizon, in a
//! store whose contract is that a completed version never changes.
//!
//! So the commit pins **the revision it judged and the lifecycle state that
//! judgement produced** on the ref row, and the projector reads those. An
//! earlier version of this paragraph argued the opposite — that putting the
//! revision on the ref row "contradicts D-86/D-91's subject typing and invents
//! a column" — and that was wrong rather than merely outweighed: D-86/D-91 key
//! `pricing_read_model`, which is untouched, and a revision is a property of
//! the **publish unit**, exactly as D-157's subject columns are.
//!
//! D-128's substance is untouched and is what makes a pinned revision safe:
//! `(plan_id, revision)` is the same row whether it stands `published` or
//! `retired`, and retirement opens no revision — it is a publish unit of its
//! own, which pins that same number with its own state and its own version.
//! That is also why the **state** is pinned and not read live: read live it
//! admits `superseded`, a third value D-128 does not contemplate for a
//! projected subject and that `load_current` could never return, which a
//! consumer coding sellability predicate (4) as "is published" reads as
//! unsellable.
//!
//! **Both columns are reported as divergences.** §3.7 lists neither, and
//! §4.4's current-revision rule is what they contradict.
//!
//! # What is deliberately not here
//!
//! **No audit row.** No document requires one for a projection, and
//! `pricing_audit_log` is the actor/before-after trail of a **mutation**
//! (`inst-au-complete`): a projection has no actor and changes no truth. The
//! absence is stated so it is not read as an oversight.
//!
//! **No producer for `DomainError::ReadModelUnavailable`.** It is the **read**
//! path's fail-closed answer, and the read path does not exist.
//!
//! **No wire code for anything refused here.** The projector has no caller a
//! client can reach — it is driven by a background sweep — so a refusal has
//! nobody to report to, which is D-146's own line about the pin frontier.

use bss_pricing_sdk::CatalogVersion;
use chrono::{DateTime, Utc};
use toolkit_db::secure::{AccessScope, DBRunner};
use toolkit_db::{DBProvider, DbError};
use uuid::Uuid;

use crate::domain::error::DomainError;
use crate::domain::lifecycle::LifecycleState;
use crate::domain::projection::{PROJECTED_ROW_STATES, PlanSubjectDelta};
use crate::domain::read_model::{SubjectKind, SubjectRef};
use crate::domain::scope_key::PlanId;
use crate::infra::storage::repo::{
    NewDelta, PendingVersionRow, catalog_version_ref_repo, pin_frontier_repo, plan_repo,
    plan_shape_repo, price_repo, read_model_repo,
};
use crate::infra::storage::repo_failure;

/// Whether the pass calling the projector **could have seen** the whole of one
/// tenant's pending set (D-163 clause 2).
///
/// §4.4's pin-eligibility counts "**every** subject row that version projects",
/// which is decidable only if the version's subject set is closed. Clause (1)
/// closes it — a batch commits atomically into one version, so any one committed
/// ref of `V` closes `V`'s set — but *this gear* learns each ref's version by
/// asking the registry **per ref**, so its knowledge of the set is only as good
/// as the pass that gathered it. Two ordinary conditions leave a pass holding
/// part of a set, and in both the version reads complete, the frontier advances,
/// and the straggler then arrives at an already-pinnable version.
///
/// A pass that could not have seen the whole set resolves and warms exactly as
/// usual and decides **no completion**. The frontier lags, which is the safe
/// direction, and it is not silent: `pricing.readmodel.pin_eligibility_overdue`
/// (D-166 clause 5) fires on exactly that state.
///
/// **`Ok(None)` from the registry is not one of the two conditions**, and the
/// distinction is load-bearing rather than pedantic. "Not committed yet" is a
/// successful answer: under clause (1) a ref the registry has not committed
/// cannot belong to a version that already has a committed ref, so it says
/// nothing about that version's set. A registry **error** says the pass does not
/// know.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PassCoverage {
    /// Every pending ref of the tenant was read, and the registry answered for
    /// each of them.
    Whole,
    /// The per-tenant scan filled its bound, so refs beyond it were not read and
    /// a version's refs may straddle the page.
    ScanSaturated,
    /// The registry **failed** to answer for at least one ref of this tenant, so
    /// a sibling of some version is of unknown version.
    RefUnresolved,
}

impl PassCoverage {
    /// May a pass with this coverage decide a version complete?
    #[must_use]
    pub const fn may_decide_completion(self) -> bool {
        matches!(self, Self::Whole)
    }

    /// Why not, for a log line. `None` when it may.
    #[must_use]
    pub const fn deferral_reason(self) -> Option<&'static str> {
        match self {
            Self::Whole => None,
            Self::ScanSaturated => Some("the per-tenant pending scan filled its bound"),
            Self::RefUnresolved => Some("a ref of this tenant could not be resolved"),
        }
    }
}

/// What one pass over one version did — the sweep's input for the degraded
/// observation, and what the suite asserts against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionReport {
    /// Subjects this version publishes, per its ref rows (D-157).
    pub subjects: usize,
    /// Subjects this call projected. Zero on a re-drive that had nothing left.
    pub projected: usize,
    /// Subjects whose own transaction refused. Their refs stay pending, so the
    /// next tick re-drives them and their age eventually alarms.
    pub failed: usize,
    /// Whether every subject of the version is warm at the end of the call.
    pub complete: bool,
    /// Where the frontier stands if this call moved it. `None` means it did
    /// not, which on an incomplete version is the normal answer and on a
    /// complete one means an earlier version is still outstanding — the D-114
    /// prefix, holding.
    pub frontier_advanced_to: Option<CatalogVersion>,
}

/// The projector: one entry point, over one database provider.
#[derive(Clone)]
pub struct ReadModelProjector {
    db: DBProvider<DbError>,
}

impl ReadModelProjector {
    /// Build over one database provider.
    #[must_use]
    pub fn new(db: DBProvider<DbError>) -> Self {
        Self { db }
    }

    /// Make every subject of one committed version resolvable, and advance the
    /// frontier if that completes it.
    ///
    /// `newly_committed` are the refs the sweep has just resolved to this
    /// version and which are therefore still `pending` in storage; they are
    /// finalized here, inside the transaction that projects them, so a crash
    /// between the two cannot leave a committed ref nothing ever projected.
    /// On a pure re-drive it is empty and the subject set comes entirely from
    /// the refs an earlier pass finalized.
    ///
    /// # The sequence, and why each move sits where it does
    ///
    /// 1. **The subject set** is the version's ref rows — the already-finalized
    ///    ones plus `newly_committed`. D-157 is why that is the only durable
    ///    answer: with no subject on the ref row there is no path at all from a
    ///    handle back to what it published.
    /// 2. **Subtract what is already warm.** On a first warm the difference is
    ///    everything; on a re-drive it is exactly what failed last time. **The
    ///    re-drive is this subtraction**, not a second mechanism, and §4.4's
    ///    unbounded degraded re-drive is the sweep simply arriving again.
    /// 3. **One transaction per outstanding subject**: finalize its ref,
    ///    project it from truth, write the delta warm, then re-read the
    ///    version's warm set — and if it is now complete **and** this version
    ///    is the frontier's next in order, advance, in that same transaction.
    ///    That is D-136 executed literally: the transaction setting the last
    ///    outstanding marker is the one that advances.
    ///
    /// Per subject rather than per version because D-91 projects a version's
    /// subjects independently: one subject whose truth rows are unreadable must
    /// not roll back the subjects that projected cleanly, or a version with one
    /// bad plan could never complete any of its others.
    ///
    /// `coverage` gates **only** the completeness decision, in both the places
    /// one is made — the report below and the advance inside `project_one`. A
    /// pass that could not have seen the whole subject set still resolves and
    /// warms everything it holds; what it does not do is claim a count over a
    /// set it knows is partial. See [`PassCoverage`].
    ///
    /// # Errors
    /// [`DomainError::Internal`] when a ref names a subject kind this gear
    /// cannot have written, when its reference is not the identifier that kind
    /// is keyed by, when a plan subject has no current revision, or on any
    /// storage failure. There is no other class: every input here comes from
    /// this gear's own tables or from the registry, so a refusal is an
    /// invariant breach and never a caller mistake.
    pub async fn project_version(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        catalog_version: CatalogVersion,
        newly_committed: &[PendingVersionRow],
        coverage: PassCoverage,
        now: DateTime<Utc>,
    ) -> Result<VersionReport, DomainError> {
        let conn = self
            .db
            .conn()
            .map_err(|e| DomainError::Internal(format!("bss-pricing: projector: {e}")))?;
        let subjects =
            version_subjects(&conn, scope, tenant_id, catalog_version, newly_committed).await?;
        let warm = read_model_repo::warm_subjects_at(&conn, scope, tenant_id, catalog_version)
            .await
            .map_err(|e| repo_failure(&e))?;

        let outstanding: Vec<PendingVersionRow> = outstanding_subjects(&subjects, &warm);
        let mut projected = 0_usize;
        let mut failed = 0_usize;
        let mut advanced_to = None;
        for subject in outstanding {
            // Per subject, and a fault here does **not** abandon the version.
            // D-91 projects a version's subjects independently, so one plan
            // whose truth rows are unreadable must not stop the others - and a
            // return here would additionally discard an advance an earlier
            // subject's transaction has already committed, which the report
            // would then deny having made.
            match self
                .project_one(
                    scope,
                    tenant_id,
                    &VersionPass {
                        catalog_version,
                        subjects: &subjects,
                        coverage,
                        now,
                    },
                    &subject,
                )
                .await
            {
                Ok(moved) => {
                    projected += 1;
                    if moved.is_some() {
                        advanced_to = moved;
                    }
                }
                Err(e) => {
                    failed += 1;
                    tracing::error!(
                        error = %e,
                        tenant_id = %tenant_id,
                        catalog_version = catalog_version.get(),
                        pending_ref = %subject.pending_ref,
                        "bss-pricing: projecting one subject failed; its ref stays pending"
                    );
                }
            }
        }

        let warm = read_model_repo::warm_subjects_at(&conn, scope, tenant_id, catalog_version)
            .await
            .map_err(|e| repo_failure(&e))?;
        // The completeness bound. A warm set with nothing outstanding is
        // evidence of completeness only over a subject set the pass could have
        // seen whole; over a partial one it is a count taken while the set was
        // still being added to (D-163 clause 2).
        let warm_and_whole =
            !subjects.is_empty() && outstanding_subjects(&subjects, &warm).is_empty();
        log_deferred_completion(tenant_id, catalog_version, coverage, warm_and_whole);
        Ok(VersionReport {
            subjects: subjects.len(),
            projected,
            failed,
            complete: warm_and_whole && coverage.may_decide_completion(),
            frontier_advanced_to: advanced_to,
        })
    }

    /// One subject, one transaction: finalize, project, warm, and advance if
    /// that was the last outstanding marker.
    async fn project_one(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        pass: &VersionPass<'_>,
        subject: &PendingVersionRow,
    ) -> Result<Option<CatalogVersion>, DomainError> {
        let VersionPass {
            catalog_version,
            subjects,
            coverage,
            now,
        } = *pass;
        let scope = scope.clone();
        let subject = subject.clone();
        let siblings = subjects.to_vec();
        let (_, outcome) = self
            .db
            .db()
            .in_transaction::<Option<CatalogVersion>, DomainError, _>(move |txn| {
                Box::pin(async move {
                    let (plan_id, revision, lifecycle_state) = plan_subject_of(&subject)?;
                    refuse_projection_below_frontier(
                        txn,
                        &scope,
                        tenant_id,
                        catalog_version,
                        &subject.pending_ref,
                    )
                    .await?;
                    catalog_version_ref_repo::finalize(
                        txn,
                        &scope,
                        tenant_id,
                        &subject.pending_ref,
                        catalog_version,
                        now,
                    )
                    .await
                    .map_err(|e| repo_failure(&e))?;

                    let delta = project_plan_subject(
                        txn,
                        &scope,
                        tenant_id,
                        plan_id,
                        revision,
                        lifecycle_state,
                    )
                    .await?;
                    read_model_repo::project_subject(
                        txn,
                        &scope,
                        NewDelta {
                            tenant_id,
                            catalog_version,
                            subject: SubjectRef::Plan(plan_id.get()),
                            payload: delta.to_value(),
                            projected_at: now,
                        },
                    )
                    .await
                    .map_err(|e| repo_failure(&e))?;

                    // The same bound as the report's, at the place where the
                    // decision has a consequence rather than a value: an
                    // advance is what makes a version pinnable, so a pass
                    // holding a partial subject set must not reach it.
                    if !coverage.may_decide_completion()
                        || !version_is_complete(txn, &scope, tenant_id, catalog_version, &siblings)
                            .await?
                    {
                        return Ok(None);
                    }
                    advance_frontier(txn, &scope, tenant_id, catalog_version, now).await
                })
            })
            .await;
        outcome.map_err(|err| {
            err.into_domain(|infra| {
                DomainError::Internal(format!("bss-pricing: projection transaction: {infra}"))
            })
        })
    }
}

/// What the pass holds about the version it is projecting.
///
/// One value rather than four parameters because the four are one fact — this
/// pass's whole knowledge of this version — and a per-subject transaction that
/// took the version without the coverage could advance a frontier over a subject
/// set the pass was still guessing at, which is exactly D-163 clause (2)'s defect.
#[derive(Clone, Copy)]
struct VersionPass<'a> {
    /// The version every subject here belongs to.
    catalog_version: CatalogVersion,
    /// Every subject the pass believes the version publishes — the already
    /// finalized refs plus the ones it is finalizing.
    subjects: &'a [PendingVersionRow],
    /// Whether the pass could have seen that set whole.
    coverage: PassCoverage,
    /// The pass's instant, for the finalize and the warm marker.
    now: DateTime<Utc>,
}

/// Say out loud that a version reads warm and was not judged complete.
///
/// Silent deferral is how a lagging frontier becomes indistinguishable from a
/// stuck one, and the two have different remedies: this one needs the next pass,
/// that one needs an operator.
fn log_deferred_completion(
    tenant_id: Uuid,
    catalog_version: CatalogVersion,
    coverage: PassCoverage,
    warm_and_whole: bool,
) {
    if !warm_and_whole {
        return;
    }
    if let Some(reason) = coverage.deferral_reason() {
        tracing::info!(
            tenant_id = %tenant_id,
            catalog_version = catalog_version.get(),
            reason,
            "bss-pricing: a version reads warm and this pass may not judge it complete; the \
             frontier lags until a pass that could have seen the whole subject set arrives"
        );
    }
}

/// The subject set of one version: the refs already finalized, plus the ones
/// this pass has just resolved and is about to finalize.
///
/// Deduplicated on `pending_ref`, because a re-drive re-offers a ref the
/// previous pass finalized and the two halves would otherwise both name it.
async fn version_subjects(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    catalog_version: CatalogVersion,
    newly_committed: &[PendingVersionRow],
) -> Result<Vec<PendingVersionRow>, DomainError> {
    let mut subjects =
        catalog_version_ref_repo::list_at_version(runner, scope, tenant_id, catalog_version)
            .await
            .map_err(|e| repo_failure(&e))?;
    for candidate in newly_committed {
        if !subjects
            .iter()
            .any(|known| known.pending_ref == candidate.pending_ref)
        {
            subjects.push(candidate.clone());
        }
    }
    Ok(subjects)
}

/// The subjects of a version that are not yet warm.
///
/// Pure, and extracted for it: this subtraction **is** the re-drive, so it is
/// the one piece of the projector worth asserting without a database.
fn outstanding_subjects(
    subjects: &[PendingVersionRow],
    warm: &[(SubjectKind, String)],
) -> Vec<PendingVersionRow> {
    subjects
        .iter()
        .filter(|subject| {
            !warm.iter().any(|(kind, reference)| {
                *kind == subject.subject_kind && *reference == subject.subject_ref
            })
        })
        .cloned()
        .collect()
}

/// The plan a ref row names, refusing every other subject kind.
///
/// `PriceOverlay`, `OverlayIndex` and `GroupMembership` have **no store in this
/// gear**, so a ref carrying one is a subject this gear cannot have written.
/// It is refused naming the kind rather than skipped: a subject silently
/// unprojected holds the version incomplete, and therefore holds the frontier,
/// forever with nothing saying why.
///
/// # Errors
/// [`DomainError::Internal`] naming the kind, or naming the reference that is
/// not the `plan_id` a `plan` subject is keyed by.
fn plan_subject_of(
    subject: &PendingVersionRow,
) -> Result<(PlanId, u64, LifecycleState), DomainError> {
    if subject.subject_kind != SubjectKind::Plan {
        return Err(DomainError::Internal(format!(
            "bss-pricing: catalog version ref {} names subject kind {}, which has no store in \
             this gear and therefore no publish unit that could have written it",
            subject.pending_ref, subject.subject_kind
        )));
    }
    let plan_id = Uuid::parse_str(&subject.subject_ref)
        .map(PlanId::new)
        .map_err(|e| {
            DomainError::Internal(format!(
                "bss-pricing: catalog version ref {} names plan subject {}, which is not a plan \
                 id: {e}",
                subject.pending_ref, subject.subject_ref
            ))
        })?;
    // Refused rather than defaulted to the current revision. A default here is
    // a guess about which content a frozen version froze, and the guess is
    // wrong exactly when a second publish beat the warm - the case the column
    // was added for.
    let revision = subject.subject_revision.ok_or_else(|| {
        DomainError::Internal(format!(
            "bss-pricing: catalog version ref {} names plan subject {plan_id} and no revision; \
             the revision the publish judged is what a frozen version freezes",
            subject.pending_ref
        ))
    })?;
    // Likewise the state that revision was in when its publish judged it. The
    // row's own state keeps moving after the commit and before the warm, and
    // `superseded` is a value D-128 does not contemplate for a projected
    // subject at all.
    let lifecycle_state = subject.subject_lifecycle_state.ok_or_else(|| {
        DomainError::Internal(format!(
            "bss-pricing: catalog version ref {} names plan subject {plan_id} revision \
             {revision} and no lifecycle state; sellability predicate (4) reads that field at \
             the pin, and the row's own state has moved on since",
            subject.pending_ref
        ))
    })?;
    Ok((plan_id, revision, lifecycle_state))
}

/// Build one plan subject's delta from the truth rows, through the caller's
/// transaction.
///
/// **The source is the revision the publish judged**, pinned on the ref row at
/// commit time and read back here. §4.4 says "the plan's **current** revision",
/// and that is a defect rather than a subtlety: the projector arrives up to the
/// D-47 maximum batching delay — five minutes — after the commit, so a second
/// publish of the same plan inside that window makes its own revision current
/// and "current" then freezes content this version's publish never judged.
/// Permanently, since a delta is INSERT-only on the seven-year horizon in a
/// store whose contract is that a completed version never changes. **Reported.**
///
/// D-128's substance is untouched and is what makes reading a pinned revision
/// safe: the row `(plan_id, revision)` is the same row whether it stands
/// `published` or `retired`, so a retired plan keeps a resolvable delta for the
/// in-flight subscribers D-51 preserves coverage for — and a retirement publish
/// unit, which opens no revision, pins that same number and re-projects it with
/// its new state.
///
/// **The state is pinned too**, and for a sharper reason than the content. The
/// row's own `lifecycle_state` keeps moving after the commit and before the
/// warm — `published -> superseded` at the next revision's commit,
/// `published -> retired` at retirement — and `superseded` is a value D-128
/// does not contemplate for a projected subject and that
/// [`plan_repo::load_current`] could never return. Frozen into an INSERT-only
/// delta it makes the version read **unsellable** to any consumer coding
/// sellability predicate (4) as "is published", which is how D-90 names that
/// predicate's input. The content needs no such pinning — a published revision
/// row and its revision-scoped children are physically immutable — which is why
/// this function reads the **row** for content and the **ref** for state.
///
/// The projector **never reads the open draft revision**: a pinned revision
/// number is a `draft` row's number only if the publish never happened, and the
/// publish is what wrote the ref.
///
/// A pinned revision that does not exist is an **internal fault**, not an empty
/// delta. An empty plan subject is exactly what D-128 was raised to prevent.
///
/// # Errors
/// [`DomainError::Internal`] when the pinned revision is not there, or on a
/// storage failure.
async fn project_plan_subject(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    plan_id: PlanId,
    revision: u64,
    lifecycle_state: LifecycleState,
) -> Result<PlanSubjectDelta, DomainError> {
    let current = plan_repo::load_revision(runner, scope, tenant_id, plan_id, revision)
        .await
        .map_err(|e| repo_failure(&e))?
        .ok_or_else(|| {
            DomainError::Internal(format!(
                "bss-pricing: plan {plan_id} revision {revision} is a subject of a committed \
                 catalog version and is not there; projecting an empty plan subject is what \
                 D-128 exists to prevent"
            ))
        })?;

    Ok(PlanSubjectDelta {
        plan_id,
        revision,
        lifecycle_state,
        sku_id: current.sku_id,
        plan_tier: current.plan_tier,
        plan_tier_override: current.plan_tier_override,
        billing_cycle: current.billing_cycle,
        frequency: current.frequency,
        available_from: current.available_from,
        available_to: current.available_to,
        purchase_min_qty: current.purchase_min_qty,
        purchase_max_qty: current.purchase_max_qty,
        invoice_grouping_key: current.invoice_grouping_key,
        phases: plan_shape_repo::load_phase_set(runner, scope, tenant_id, plan_id, revision)
            .await
            .map_err(|e| repo_failure(&e))?,
        addon_rules: plan_shape_repo::load_addon_rule_set(
            runner, scope, tenant_id, plan_id, revision,
        )
        .await
        .map_err(|e| repo_failure(&e))?,
        descriptor_set: plan_shape_repo::load_descriptor(
            runner, scope, tenant_id, plan_id, revision,
        )
        .await
        .map_err(|e| repo_failure(&e))?,
        // D-121's projected row set, minus its horizon; the module doc carries
        // the premise and names the group that deletes it.
        prices: price_repo::load_for_plan(runner, scope, tenant_id, plan_id, PROJECTED_ROW_STATES)
            .await
            .map_err(|e| repo_failure(&e))?,
    })
}

/// Is every subject of `catalog_version` warm?
///
/// The subject set is the version's **committed** refs **union what the caller
/// already knows** — the refs this pass has resolved to this version and is in
/// the middle of finalizing. Both halves are load-bearing, and the second is
/// the one an earlier version of this function did without: a ref the pass has
/// resolved but not yet finalized carries no version in storage, so
/// [`catalog_version_ref_repo::list_at_version`] cannot see it, and a version
/// with two subjects read as complete after the first was warmed. Advancing
/// there is the D-101 divergence with the frontier asserting the opposite.
///
/// A version with no refs at all answers `false`. It cannot arise — a version
/// exists in this store because a ref points at it — and answering `true` would
/// let the frontier advance over a version whose subject set nobody knows.
async fn version_is_complete(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    catalog_version: CatalogVersion,
    known: &[PendingVersionRow],
) -> Result<bool, DomainError> {
    let subjects = version_subjects(runner, scope, tenant_id, catalog_version, known).await?;
    if subjects.is_empty() {
        return Ok(false);
    }
    let warm = read_model_repo::warm_subjects_at(runner, scope, tenant_id, catalog_version)
        .await
        .map_err(|e| repo_failure(&e))?;
    Ok(outstanding_subjects(&subjects, &warm).is_empty())
}

/// Refuse to project into a version the frontier has already declared
/// pin-eligible.
///
/// **This is the enforcement the premise above cannot provide.** A finalize
/// targeting a version at or below the frontier is an already-pin-eligible
/// version acquiring a **new subject** — which is exactly one pin resolving two
/// different contents over time, the divergence D-101 and D-114 exist to close.
/// It is detected at the moment it would corrupt rather than predicted, so a
/// registry that violates batch atomicity produces a loud, operator-visible
/// fault instead of a silently wrong frozen version.
///
/// It cannot fire on any correct path: the frontier standing at `V` asserts
/// every version at or below `V` is complete, so no subject of such a version
/// can still be outstanding. If one is, the frontier already lied.
///
/// # What it costs when it fires, and what keeps that cost attached to a
/// contract violation
///
/// The price is total for that publish and D-163 clause (3) states it beside the
/// guarantee: the publish becomes **unresolvable at any version**, its subject is
/// never projected, its ref never finalizes, both Critical alarms fire every
/// pass, and **nothing self-heals it** — the remedy is out of band, §3.6's own
/// "a registry re-request, never a silent re-emit".
///
/// A price like that may only be charged for a **broken contract**, and as first
/// built this guard could reach it through an ordinary **per-ref registry
/// outage**: a version whose sibling ref the registry could not answer read
/// complete from the warm set alone, the frontier advanced, and the sibling
/// arrived below it a tick later. [`PassCoverage`] is what removed that path —
/// a pass that could not have seen the whole subject set decides no completion,
/// so the frontier never passes a version whose set the pass was still guessing
/// at. What is left reaching this refusal is the registry contradicting clause
/// (1): a batch that did not commit atomically into one version. That is the
/// case it was written for, and it is the case it should be loud about.
///
/// # Errors
/// [`DomainError::Internal`] naming the version, the frontier and the handle.
async fn refuse_projection_below_frontier(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    catalog_version: CatalogVersion,
    pending_ref: &str,
) -> Result<(), DomainError> {
    let Some(frontier) = pin_frontier_repo::read_at(runner, scope, tenant_id)
        .await
        .map_err(|e| repo_failure(&e))?
    else {
        return Ok(());
    };
    if catalog_version > frontier.catalog_version {
        return Ok(());
    }
    Err(DomainError::Internal(format!(
        "bss-pricing: pending ref {pending_ref} of tenant {tenant_id} resolves to catalog \
         version {}, at or below the pin-eligibility frontier standing at {}; that version is \
         already pinnable and acquiring a subject now would make one pin resolve two contents",
        catalog_version.get(),
        frontier.catalog_version.get()
    )))
}

/// Advance the frontier to `completed`, then walk it forward over every
/// following version that is already complete.
///
/// The first half is D-136 as written: the advance happens only when
/// `completed` is the frontier's **next** version in order, which is what makes
/// the D-114 prefix true by construction rather than re-derived. When it is
/// not, nothing moves and that is the prefix holding — an earlier version is
/// still outstanding.
///
/// # What makes "complete" trustworthy, and what happens when it is not
///
/// Completeness is judged from the version's **committed** refs plus the ones
/// this pass is finalizing, so it is only as good as the premise that a
/// **batch commits atomically into one version** — which is what "the registry
/// batches approved publishes and emits `CatalogVersionPublished`" (§4.2 step 5,
/// §3.6) means on its face: one batch, one version, one event. Under it, any
/// committed ref of `V` implies `V`'s subject set is **closed**, and the only
/// open question is warmth.
///
/// The premise is **not stated by any document in the set**, and it is not
/// predicted here either. What guards it is
/// [`refuse_projection_below_frontier`], which detects the violation **at the
/// moment it would corrupt**: a ref resolving into a version the frontier has
/// already passed is refused loudly rather than quietly added to a pinnable
/// version. **Reported.**
///
async fn advance_frontier(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    completed: CatalogVersion,
    now: DateTime<Utc>,
) -> Result<Option<CatalogVersion>, DomainError> {
    let standing = pin_frontier_repo::read_at(runner, scope, tenant_id)
        .await
        .map_err(|e| repo_failure(&e))?
        .map(|frontier| frontier.catalog_version);
    let next =
        catalog_version_ref_repo::next_committed_version_after(runner, scope, tenant_id, standing)
            .await
            .map_err(|e| repo_failure(&e))?;
    if next != Some(completed) {
        return Ok(None);
    }

    pin_frontier_repo::advance(runner, scope, tenant_id, completed, now)
        .await
        .map_err(|e| repo_failure(&e))?;
    let mut reached = completed;

    // The walk. Every step is still "the frontier's next version in order" and
    // still complete, so it is strictly forward and never past a gap.
    loop {
        let following = catalog_version_ref_repo::next_committed_version_after(
            runner,
            scope,
            tenant_id,
            Some(reached),
        )
        .await
        .map_err(|e| repo_failure(&e))?;
        let Some(following) = following else {
            break;
        };
        // `&[]`: this pass knows nothing extra about a version it is not
        // projecting, so the walk sees only committed refs. Under batch
        // atomicity that is the whole subject set; when it is not,
        // `refuse_projection_below_frontier` catches the straggler loudly
        // instead of letting it join a pinnable version.
        if !version_is_complete(runner, scope, tenant_id, following, &[]).await? {
            break;
        }
        pin_frontier_repo::advance(runner, scope, tenant_id, following, now)
            .await
            .map_err(|e| repo_failure(&e))?;
        reached = following;
    }
    Ok(Some(reached))
}

#[cfg(test)]
#[path = "read_model_tests.rs"]
mod read_model_tests;
