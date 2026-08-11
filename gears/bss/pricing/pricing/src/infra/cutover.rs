//! The grandfathering cutover's commit, across both planes (`inst-gc-commit`, D-100).
//!
//! `infra::supersession`'s shape with one more window and one more row, and the two
//! ordering constraints it records hold here for the same reasons and through the
//! same two mechanisms. What this unit adds is a **second arrival**: the
//! grandfathered copy, on a new generation of the predecessor's key, born in the
//! same transaction as the successor that replaces it.

use std::sync::Arc;

use aws_lc_rs::digest::{SHA256, digest as sha256};
use chrono::{DateTime, Utc};
use toolkit_db::secure::{AccessScope, DBRunner, DbTx};
use toolkit_db::{DBProvider, DbError};
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::domain::audit::{AuditAction, AuditStamp, AuditSubjectKind, hex_bytes};
use crate::domain::concurrency::RowVersion;
use crate::domain::cutover::{
    ComposedCutover, check_cutover_instant, compose_cutover_windows, grandfathered_copy_key,
};
use crate::domain::error::DomainError;
use crate::domain::events::CatalogEvent;
use crate::domain::lifecycle::LifecycleState;
use crate::domain::materiality::triggers::Trigger;
use crate::domain::materiality::{self, ChangeSet, MaterialityVerdict};
use crate::domain::plan_shape::PlanShape;
use crate::domain::ports::{CatalogVersionRegistryV1, registry_failure};
use crate::domain::price_record::{PriceContent, PriceRecord};
use crate::domain::read_model::SubjectRef;
use crate::domain::scope_key::PlanId;
use crate::domain::scope_key::{Cohort, PriceEligibility, ScopeKey};
use crate::domain::supersession::{ChangeoverMoment, NamedWindow};
use crate::domain::window::WindowInterval;
use crate::infra::publish::assemble_from;
use crate::infra::storage::RepoError;
use crate::infra::storage::repo::approval_repo::{self, ApprovalRecord};
use crate::infra::storage::repo::audit_repo::{self, NewAuditEntry};
use crate::infra::storage::repo::window_repo::{NewWindow, WindowRecord};
use crate::infra::storage::repo::{
    NewOutboxEvent, NewPriceDraft, PendingVersionRow, PriceUpdatedPayload,
    PriceWindowTransitionPayload, catalog_version_ref_repo, outbox_repo, plan_repo, price_repo,
    window_repo,
};
use crate::infra::storage::repo_failure;
use crate::infra::window::VerdictJson;

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
    // D-154's readiness, resolved in **this** transaction so the successor's
    // frozen category is the one the world held when it published — the same
    // reason `infra::publish::rule_params` resolves it there rather than letting
    // the projector re-derive it later (`T-13`).
    let readiness = crate::domain::tax_display::RegionTaxReadiness::new(
        crate::infra::storage::repo::taxonomy_repo::region_readiness_map(txn, scope, tenant_id)
            .await?
            .into_iter()
            .map(|(region, markers)| {
                (
                    region,
                    crate::domain::tax_display::RegionReadiness {
                        tax_category: markers.tax_category,
                        tax_rate_present: markers.tax_rate_present,
                    },
                )
            })
            .collect(),
    );
    price_repo::commit_cutover_rows(
        txn,
        scope,
        tenant_id,
        plan.plan_id,
        plan.predecessor,
        plan.successor,
        plan.copy,
        plan.cutover_at(),
        &readiness,
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

// ---------------------------------------------------------------------------
// The unit: `inst-gc-compose`'s composer and `inst-gc-commit`'s owner.
// ---------------------------------------------------------------------------

/// One cutover request, as the surface hands it over (`inst-gc-api`).
///
/// `infra::supersession::SupersessionRequest`'s shape with the cutover's second
/// arrival: two more minted ids, for the grandfathered copy's row and its window.
/// The same note applies to all four — they are the **surface's**, minted per
/// request, and the ones the act really uses are the staged rows' when an earlier
/// attempt staged them. That is why the act's subject
/// ([`cutover_unit_ref`]) names none of them.
///
/// **One key, and D-28's selector is wider.** The register decided a cutover may
/// select several keys at one instant; this request carries one, and
/// [`ApprovalService::submit_cutover_on`](crate::infra::approval::ApprovalService::submit_cutover_on)
/// already takes the set. The multi-key payload is the surface's to build, and the
/// unit below composes per key when it arrives — recorded here rather than left as
/// a silent narrowing.
#[derive(Clone, Debug)]
pub struct CutoverRequest {
    /// The key being cut over: the `all_subscriptions` row's own.
    pub predecessor_key: ScopeKey,
    /// The instant all three window operations pivot on.
    pub cutover_at: DateTime<Utc>,
    /// The successor's authored content, landing on the predecessor's own key.
    pub successor: PriceContent,
    /// The id the successor draft is authored under, if this call stages it.
    pub successor_price_id: Uuid,
    /// The id the successor's window is scheduled under, if this call commits.
    pub successor_window_id: Uuid,
    /// The id the grandfathered copy is authored under, if this call stages it.
    pub copy_price_id: Uuid,
    /// The id the copy's window is scheduled under, if this call commits.
    pub copy_window_id: Uuid,
    /// The operator-supplied change reason both scheduled windows are recorded under.
    pub reason_code: String,
}

/// What a cutover did: it committed, or it opened a unit and stood *almost* still.
///
/// [`crate::infra::supersession::SupersessionOutcome`]'s two arms and the same
/// reason for them: a materially-refused cutover has written its **two drafts**,
/// because they are the reviewer's subject and there is nothing to review until
/// they exist. What has not happened is the flip, both publishes and all three
/// window operations.
#[derive(Clone, Debug)]
pub enum CutoverOutcome {
    /// The unit committed: five writes, one version handle, four events owed.
    Committed(Box<CutoverReceipt>),
    /// A second principal is required. Both drafts stand; nothing else moved.
    SubmittedForApproval(Box<CutoverPending>),
}

/// What a committed cutover leaves behind.
#[derive(Clone, Debug)]
pub struct CutoverReceipt {
    /// The plan whose subject re-projects.
    pub plan_id: PlanId,
    /// The revision that subject stands at.
    pub revision: u64,
    /// The row that left the key.
    pub predecessor_price_id: Uuid,
    /// The row that took it.
    pub successor_price_id: Uuid,
    /// The retained copy, on its own generation.
    pub copy_price_id: Uuid,
    /// The generation the copy was minted on.
    pub copy_key: ScopeKey,
    /// The instant coverage handed over.
    pub cutover_at: DateTime<Utc>,
    /// The predecessor's window, now ending at the cutover.
    pub shortened_window_id: Uuid,
    /// The registry's **pending** handle: a 202 in a value.
    pub pending_version_ref: String,
    /// The approval unit that authorized this commit, as `SupersessionReceipt`
    /// carries its own.
    ///
    /// `None` only on the fail-open arm the gate itself decides; a committed
    /// cutover under governance names its unit. It is here because `approval_ref`
    /// is the sole join between the approved unit and the act it authorized, and
    /// until 2026-08-11 this path wrote it in neither place — no audit record and
    /// no receipt field — so a caller holding a cutover could not name the second
    /// principal who signed it.
    pub authorization: Option<Uuid>,
}

/// What an opened cutover unit leaves behind: two drafts and a record.
#[derive(Clone, Debug)]
pub struct CutoverPending {
    /// The plan the unit's subject belongs to.
    pub plan_id: PlanId,
    /// The revision the content pin covers.
    pub revision: u64,
    /// The staged successor's id — the **stored** one when an earlier attempt
    /// staged it, never the id this call minted.
    pub successor_price_id: Uuid,
    /// The staged copy's id, on the same rule.
    pub copy_price_id: Uuid,
    /// The generation the copy stands on.
    pub copy_key: ScopeKey,
    /// The unit a second principal has to decide.
    pub approval: ApprovalRecord,
}

/// The plan-scoped facts one cutover is judged against, read once inside the
/// writing transaction (D-176).
struct CutoverContext {
    plan_id: PlanId,
    revision: u64,
    lifecycle_state: LifecycleState,
    shape: PlanShape,
    predecessor: PriceRecord,
    staged_successor: Option<PriceRecord>,
    staged_copy: Option<PriceRecord>,
    plane: Vec<NamedWindow>,
    stored_plane: Vec<WindowRecord>,
    /// Every generation the plan already carries on this market — `inst-co-copy`'s
    /// duplicate refusal needs them, and it is a read rather than a guess.
    existing_generations: Vec<Cohort>,
}

impl CutoverContext {
    /// The stored window the compose decided to shorten, with its act counter.
    fn shorten_target(&self, window_id: Uuid) -> Result<&WindowRecord, DomainError> {
        self.stored_plane
            .iter()
            .find(|record| record.window_id == window_id)
            .ok_or_else(|| DomainError::NotFound {
                subject: "price window".to_owned(),
                id: window_id.to_string(),
            })
    }
}

/// The cutover workflow (`inst-gc-api`).
#[derive(Clone)]
pub struct CutoverService {
    db: DBProvider<DbError>,
    registry: Arc<dyn CatalogVersionRegistryV1>,
}

impl CutoverService {
    /// Build the workflow over one provider and the resolved registry.
    ///
    /// The **same** registry `Arc` every other requester holds; what keeps their
    /// handles apart is `unit_request_id`'s distinct first segment.
    #[must_use]
    pub const fn new(db: DBProvider<DbError>, registry: Arc<dyn CatalogVersionRegistryV1>) -> Self {
        Self { db, registry }
    }

    /// Compose and, if it may, commit one grandfathering cutover.
    ///
    /// The whole act in **one** transaction of this service's own. See
    /// [`cutover_in`] for the order and why every step is where it is.
    ///
    /// # Errors
    /// [`cutover_in`]'s, exactly.
    pub async fn cut_over(
        &self,
        ctx: &SecurityContext,
        scope: &AccessScope,
        tenant_id: Uuid,
        request: CutoverRequest,
        verdict_json: VerdictJson,
        stamp: AuditStamp,
    ) -> Result<CutoverOutcome, DomainError> {
        let ctx = ctx.clone();
        let scope = scope.clone();
        let registry = Arc::clone(&self.registry);
        let (_, outcome) = self
            .db
            .db()
            .in_transaction::<CutoverOutcome, DomainError, _>(move |txn| {
                Box::pin(async move {
                    // Boxed a second time on purpose: the orchestrator's own future
                    // is large — eleven steps, a whole `PlanShape` and two staged
                    // records live across its awaits — and `clippy::large_futures`
                    // is what says so. The cost is one allocation per act; the
                    // alternative is that size on every task's stack.
                    Box::pin(cutover_in(
                        txn,
                        &registry,
                        &ctx,
                        &scope,
                        tenant_id,
                        &request,
                        verdict_json,
                        stamp,
                    ))
                    .await
                })
            })
            .await;
        outcome.map_err(|err| {
            err.into_domain(|infra| {
                DomainError::Internal(format!("bss-pricing: cutover transaction: {infra}"))
            })
        })
    }
}

/// One cutover, composed and committed or composed and submitted, in the caller's
/// transaction.
///
/// **`infra::supersession::supersede_in`'s eleven steps, side by side, with three
/// differences** — and they are written here in that order so the two can be read
/// against each other:
///
/// 1. **Two keys through `inst-co-single-pending` instead of one.** The
///    `all_subscriptions` key and the generation this act mints.
/// 2. **A second staged row.** The grandfathered copy, on the generation's key,
///    beside the successor on the predecessor's own.
/// 3. **The verdict is fixed, not asked.** `inst-mat-registered` registers a
///    grandfathering cutover, so `evaluate` answers `alwaysMaterialTrigger` from
///    the **act** half and no threshold policy is read. This is also the function
///    that makes `Trigger::GrandfatheringCutover`'s subject exist in this crate:
///    it is the surface that declares the act, which is what
///    `domain::materiality::triggers`' module doc requires before that predicate
///    may answer `true`.
///
/// # Errors
///
/// Whatever the compose refuses (`CUTOVER_INSTANT_PASSED`, `CUTOVER_GAP`,
/// `WINDOW_OVERLAP`, a duplicate generation), whatever the approval plane refuses
/// (`PENDING_CHANGE_UNIT_EXISTS`), whatever the row and window doors refuse, and
/// [`DomainError::Internal`] on a storage or registry failure.
#[allow(
    clippy::too_many_arguments,
    reason = "`supersede_in`'s argument list exactly: the transaction, the registry, the security \
              context, the scope, the tenant, the request, the verdict renderer and the stamp. \
              Every one is a different authority and none is derivable from the others"
)]
pub async fn cutover_in(
    txn: &DbTx<'_>,
    registry: &Arc<dyn CatalogVersionRegistryV1>,
    ctx: &SecurityContext,
    scope: &AccessScope,
    tenant_id: Uuid,
    request: &CutoverRequest,
    verdict_json: VerdictJson,
    stamp: AuditStamp,
) -> Result<CutoverOutcome, DomainError> {
    let now = stamp.recorded_at;

    // 0. The refusal that needs no world at all, answered from the key alone and
    //    ahead of the registry request because it is **permanent** (D-156). A
    //    grandfathered generation may not be superseded (Foundation §4.3), and a
    //    cutover's successor supersedes the key it lands on — so a generation is not
    //    cuttable-over either, for the same reason and through the same spelling.
    price_repo::refuse_unsupersedable_class(&request.predecessor_key)
        .map_err(|e| repo_failure(&e))?;

    // 1. Every fact the judgement needs, read inside the transaction that writes.
    let context = read_cutover_context(
        txn,
        scope,
        tenant_id,
        &request.predecessor_key,
        request.cutover_at,
        now,
    )
    .await?;

    // 2. `inst-gc-compose` at the floor **every** call must clear. The commit floor is
    //    stricter and is applied at step 7; this run is what refuses a request nobody
    //    should be asked to review.
    check_cutover_instant(request.cutover_at, now, ChangeoverMoment::Submit)?;
    let composed = compose_cutover_windows(&context.plane, request.cutover_at)?;
    let copy_key = grandfathered_copy_key(
        &request.predecessor_key,
        request.cutover_at,
        &context.existing_generations,
    )?;

    let selected = [request.predecessor_key.clone()];
    let subject_ref = cutover_unit_ref(context.plan_id, &selected, request.cutover_at);

    // 3. **An approved unit decides before the evaluator is asked**, exactly as the
    //    supersession does and for the reason its step 3 records: consulting the
    //    evaluator first let a threshold raised between the two calls make an act
    //    auto-publishable at the second one, committing on a single principal while
    //    the unit opened over it stayed `submitted` forever.
    let authorization = crate::infra::approval::authorizing_unit(
        txn,
        scope,
        tenant_id,
        &context.shape,
        &subject_ref,
    )
    .await?;

    // 4. The verdict, and it is **fixed**. `ChangeSet::of_act` is the declaration
    //    `domain::materiality::triggers` requires of a surface, and `evaluate`
    //    answers the act half before it reads anything else — so no threshold policy
    //    is fetched and none could change the answer. The rows go in so the stored
    //    verdict carries what moved, not so the delta decides anything.
    let verdict = materiality::evaluate(
        &ChangeSet::of_act(
            Trigger::GrandfatheringCutover,
            [context.predecessor.clone()],
        ),
        None,
        None,
    );

    if authorization.is_none() {
        // 4. This act's **own** pending unit — `inst-gc-api`'s idempotency, answered
        //    before anything is staged, because the answer is that nothing more
        //    should be. `authorizing_unit` looks for an *approved* record; a
        //    `submitted` one means the act is under review, and a retry of it must
        //    find that unit rather than try to open a second one over the same
        //    subject. Without this arm the retry reaches `submit_cutover_on` and is
        //    refused `PENDING_CHANGE_UNIT_EXISTS` by its own act — measured, by the
        //    replay case, which failed exactly that way.
        if let Some(held) =
            approval_repo::find_pending_for_subject(txn, scope, tenant_id, &subject_ref)
                .await
                .map_err(|e| repo_failure(&e))?
        {
            // The **stored** rows, never the ids this call minted: the drafts belong
            // to the act, and `SupersessionRequest`'s doc makes the same point about
            // the id a second call brings.
            let successor = context
                .staged_successor
                .as_ref()
                .map_or(request.successor_price_id, |row| row.price_id);
            let copy = context
                .staged_copy
                .as_ref()
                .map_or(request.copy_price_id, |row| row.price_id);
            return Ok(CutoverOutcome::SubmittedForApproval(Box::new(
                CutoverPending {
                    plan_id: context.plan_id,
                    revision: context.revision,
                    successor_price_id: successor,
                    copy_price_id: copy,
                    copy_key,
                    approval: held,
                },
            )));
        }

        // 5. and 6. The controlled arm: stage both rows, open the unit over both
        //    keys, write nothing else.
        return submitted_cutover(
            txn,
            scope,
            tenant_id,
            &context,
            request,
            &copy_key,
            &selected,
            verdict,
            verdict_json,
            stamp,
        )
        .await;
    }

    // 6. `inst-co-single-pending` on the **committing** arm too, which is the gap the
    //    2026-08-06 review found on the supersession: asking only inside the submit
    //    means an act that arrives already approved walks over a key another unit is
    //    reviewing. This unit's own approved record does not hold its keys — the
    //    register row follows its parent out of `submitted` through a trigger.
    let mut held = std::collections::BTreeSet::new();
    held.insert(request.predecessor_key.to_string());
    held.insert(copy_key.to_string());
    crate::infra::approval::refuse_held_key(txn, scope, tenant_id, &held).await?;

    // 7. `inst-gc-compose` re-run at the **commit** floor, which is the stricter one.
    check_cutover_instant(request.cutover_at, now, ChangeoverMoment::Commit)?;
    let shorten = context.shorten_target(composed.shorten().window_id)?;
    let shorten_seq = shorten.mutation_seq;
    let shortened_window_id = shorten.window_id;

    // 8. Both drafts, staged now if no earlier attempt did — and **before** the
    //    registry request, for `supersede_in` step 8's reason: this insert is the
    //    compose's own write, and keeping content validation on the far side of the
    //    handle strands a refusal no retry can pass.
    let (successor, copy) =
        stage_both(txn, scope, tenant_id, &context, request, &copy_key, stamp).await?;

    // 9. Addressability, fail closed, after every refusal and before the writes
    //    (D-156).
    let pending = registry
        .request_version(ctx, &cutover_request_id(tenant_id, &subject_ref))
        .await
        .map_err(|e| registry_failure(&e))?;

    // 10. The five writes, in one order, in this transaction.
    let written = commit_cutover(
        txn,
        scope,
        tenant_id,
        CutoverCommit::of_composition(
            composed,
            context.plan_id,
            context.predecessor.price_id,
            shorten_seq,
            (successor.price_id, successor.row_version),
            request.successor_window_id,
            (copy.price_id, copy.row_version),
            request.copy_window_id,
            request.reason_code.clone(),
        ),
        stamp,
    )
    .await
    .map_err(|e| repo_failure(&e))?;

    // 11. The plan subject the projector re-freezes — one subject, a second reason to
    //     re-project it, exactly as the supersession's step 11 argues.
    catalog_version_ref_repo::record_pending(
        txn,
        scope,
        PendingVersionRow::for_subject(
            tenant_id,
            pending.pending_ref.clone(),
            &SubjectRef::Plan(context.plan_id.get()),
            Some(context.revision),
            Some(context.lifecycle_state),
            now,
        ),
    )
    .await
    .map_err(|e| repo_failure(&e))?;

    // 12. The announcement, and it is **two** events rather than `inst-gc-commit`'s
    //     four. That clause lists `PriceCreated` x2 as well, on the ground that this
    //     unit's two rows are *"born published and pass no authoring door"* — which
    //     is false, and unavoidably so: both are staged as **drafts** at compose,
    //     because clause (a) makes them the reviewer's subject and a content pin over
    //     a shape that does not contain them is a unit no approve can satisfy. So
    //     both pass the authoring door, both announce there, and a second emission
    //     here would be a duplicate its own `PriceCreated/<price_id>` dedup key
    //     refuses. D-203's "second producer" conclusion goes with the premise.
    //
    //     Both schedules go through `price_window_mutation`, whose **act segment** is
    //     what keeps them out of a collision with any other `PriceWindowScheduled` on
    //     the same window: the subject is the act, and two cutovers of one key at two
    //     instants are two acts.
    // `PriceUpdated` for the successor, which this act announced to nobody until
    // D-218 (2026-08-07). The transition being announced is the **same** transition
    // `infra::supersession` announces — a row landing on an occupied published
    // canonical scope key and flipping its predecessor `published → superseded` —
    // and D-127 already binds both producers of it to one guard, on the ground that
    // the guard follows the key rather than the mechanism. A consumer that learned
    // "stop resolving the row this replaces" from a supersession learned nothing
    // from a cutover, so its correctness depended on which act moved the row. That
    // is the coupling D-100 removed on the truth side and this removes on the wire.
    //
    // The predecessor is `context.predecessor.price_id` rather than a column read:
    // a cutover's successor arrives as a **client-authored row ref**, so this
    // transaction is the only place that holds both sides of the link without a
    // lookup. `pending.pending_ref` is borrowed here and moved into the receipt
    // below, which is why this sits before the `Ok`.
    outbox_repo::enqueue(
        txn,
        scope,
        NewOutboxEvent::price_updated(
            tenant_id,
            &PriceUpdatedPayload {
                plan_id: context.plan_id,
                price_id: successor.price_id,
                scope_key: request.predecessor_key.to_string(),
                supersedes_price_id: context.predecessor.price_id,
                changeover: request.cutover_at,
                pending_version_ref: pending.pending_ref.clone(),
                correlation_id: stamp.correlation_id,
            },
            now,
        ),
    )
    .await
    .map_err(|e| repo_failure(&e))?;

    let act = format!("cutover/{}", request.cutover_at.to_rfc3339());
    for (window, price_id) in [
        (&written.successor_window, successor.price_id),
        (&written.copy_window, copy.price_id),
    ] {
        outbox_repo::enqueue(
            txn,
            scope,
            NewOutboxEvent::price_window_mutation(
                tenant_id,
                CatalogEvent::PriceWindowScheduled,
                &PriceWindowTransitionPayload {
                    window_id: window.window_id,
                    plan_id: context.plan_id,
                    price_id,
                    // **As written, not as asked for** — `commit_cutover` returns the
                    // rows for exactly this reason.
                    effective_from: window.effective_from,
                    effective_to: window.effective_to,
                    correlation_id: stamp.correlation_id,
                },
                now,
                &act,
            ),
        )
        .await
        .map_err(|e| repo_failure(&e))?;
    }

    // 13. The record of who did it, at the **act's** level rather than one per row
    //     moved — `supersession.rs:995`'s step verbatim, and this is the step the
    //     cutover's copy of that eleven-step skeleton stopped one short of.
    //
    //     **It was absent entirely until 2026-08-11.** Seven infra services call
    //     `audit_repo::append`; `cutover.rs` contained no occurrence of
    //     `audit_repo`, `AuditAction` or `NewAuditEntry`, and neither `publish_rows`
    //     nor `supersede_row` compensates beneath it. The submit half *was* audited
    //     (`submit_cutover_on` → `approval_repo::open`) and so was the staging of
    //     the two drafts, so what had no record was precisely the act that moves
    //     money: the predecessor's supersession and the two publishes.
    //
    //     `approval_ref` is the point. It is the only join between the approved unit
    //     and the act it authorized, and on this path it was written nowhere — not
    //     on a record (there was none) and not on the receipt (no field) — so an
    //     auditor holding a cutover could not establish that a second principal had
    //     approved it, and `inst-tp-record` was unsatisfied for this act.
    //     §8's `dod-audit` is "every mutation MUST record".
    record_cutover(
        txn,
        scope,
        CutoverAudit {
            tenant_id,
            plan_id: context.plan_id,
            subject_ref: &subject_ref,
            predecessor: context.predecessor.price_id,
            successor: &successor,
            copy: &copy,
            approval_ref: authorization.as_ref().map(|record| record.approval_id),
            stamp,
            now,
        },
    )
    .await?;

    Ok(CutoverOutcome::Committed(Box::new(CutoverReceipt {
        plan_id: context.plan_id,
        revision: context.revision,
        predecessor_price_id: context.predecessor.price_id,
        successor_price_id: successor.price_id,
        copy_price_id: copy.price_id,
        copy_key,
        cutover_at: request.cutover_at,
        shortened_window_id,
        pending_version_ref: pending.pending_ref,
        authorization: authorization.as_ref().map(|record| record.approval_id),
    })))
}

/// The cutover's before/after, `supersession::unit_state`'s shape with the third
/// row this act moves.
///
/// A cutover publishes **two** rows — the successor and the grandfathered copy —
/// where a supersession publishes one, so a state naming only the successor would
/// leave the copy's transition unrecorded on the act that created it.
/// Everything the cutover's audit record names, gathered so the append is one
/// call — `clippy::too_many_lines` bounds `cutover_in` at 200 and the inline
/// spelling put it at 213, which is `error_mapping`'s `precondition` situation and
/// takes its remedy.
struct CutoverAudit<'a> {
    tenant_id: Uuid,
    plan_id: PlanId,
    subject_ref: &'a str,
    predecessor: Uuid,
    successor: &'a PriceRecord,
    copy: &'a PriceRecord,
    approval_ref: Option<Uuid>,
    stamp: AuditStamp,
    now: DateTime<Utc>,
}

/// The record of who performed the cutover — `supersession.rs:995`'s step, and the
/// one the cutover's copy of that eleven-step skeleton stopped short of.
///
/// **It was absent entirely until 2026-08-11.** Seven infra services call
/// `audit_repo::append`; `cutover.rs` contained no occurrence of `audit_repo`,
/// `AuditAction` or `NewAuditEntry`, and neither `publish_rows` nor `supersede_row`
/// compensates beneath it. The submit half *was* audited (`submit_cutover_on` →
/// `approval_repo::open`) and so was the staging of the two drafts, so what had no
/// record was precisely the act that moves money: the predecessor's supersession
/// and the two publishes.
///
/// `approval_ref` is the point. It is the only join between the approved unit and
/// the act it authorized, and on this path it was written nowhere — not on a record
/// (there was none) and not on the receipt (no field) — so an auditor holding a
/// cutover could not establish that a second principal had approved it, and
/// `inst-tp-record` was unsatisfied for this act. §8's `dod-audit` is "every
/// mutation MUST record".
async fn record_cutover(
    txn: &DbTx<'_>,
    scope: &AccessScope,
    audit: CutoverAudit<'_>,
) -> Result<(), DomainError> {
    audit_repo::append(
        txn,
        scope,
        NewAuditEntry {
            tenant_id: audit.tenant_id,
            chain_id: audit_repo::plan_chain(audit.plan_id),
            recorded_at: audit.now,
            actor_principal_id: audit.stamp.actor_principal_id,
            action: AuditAction::Publish,
            subject_kind: AuditSubjectKind::PriceUnit,
            // The act, not either row: `price_unit_ref` names a *row*, which cannot
            // tell a cutover from a `PATCH` on the same id, and the unit's approval
            // record already stands under this exact string.
            subject_ref: audit.subject_ref.to_owned(),
            before_state: Some(cutover_state(
                audit.predecessor,
                LifecycleState::Published,
                audit.successor,
                audit.copy,
                LifecycleState::Draft,
            )),
            after_state: Some(cutover_state(
                audit.predecessor,
                LifecycleState::Superseded,
                audit.successor,
                audit.copy,
                LifecycleState::Published,
            )),
            approval_ref: audit.approval_ref,
            correlation_id: audit.stamp.correlation_id,
        },
    )
    .await
    .map(|_| ())
    .map_err(|e| repo_failure(&e))
}

fn cutover_state(
    predecessor: Uuid,
    predecessor_state: LifecycleState,
    successor: &PriceRecord,
    copy: &PriceRecord,
    published_state: LifecycleState,
) -> serde_json::Value {
    serde_json::json!({
        "predecessorPriceId": predecessor,
        "predecessorState": predecessor_state.as_str(),
        "successorPriceId": successor.price_id,
        "successorState": published_state.as_str(),
        "successorScopeKey": successor.scope_key.to_string(),
        "copyPriceId": copy.price_id,
        "copyState": published_state.as_str(),
        "copyScopeKey": copy.scope_key.to_string(),
    })
}

/// The controlled arm: stage both rows, open the unit over both keys, write nothing
/// else.
#[allow(
    clippy::too_many_arguments,
    reason = "the context, the request, the minted copy key, the selection, the verdict and its \
              renderer, plus the runner, the scope, the tenant and the stamp. Splitting it would \
              hand the halves to two functions that would both need all of it"
)]
async fn submitted_cutover(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    context: &CutoverContext,
    request: &CutoverRequest,
    copy_key: &ScopeKey,
    selected: &[ScopeKey],
    verdict: MaterialityVerdict,
    verdict_json: VerdictJson,
    stamp: AuditStamp,
) -> Result<CutoverOutcome, DomainError> {
    let (successor, copy) =
        stage_both(runner, scope, tenant_id, context, request, copy_key, stamp).await?;
    let record = crate::infra::approval::ApprovalService::submit_cutover_on(
        runner,
        scope,
        tenant_id,
        selected,
        request.cutover_at,
        Uuid::now_v7(),
        verdict_json(&verdict)?,
        stamp,
    )
    .await?;
    Ok(CutoverOutcome::SubmittedForApproval(Box::new(
        CutoverPending {
            plan_id: context.plan_id,
            revision: context.revision,
            successor_price_id: successor.price_id,
            copy_price_id: copy.price_id,
            copy_key: copy_key.clone(),
            approval: record,
        },
    )))
}

/// Stage the successor and the grandfathered copy, reusing whatever an earlier
/// attempt staged.
///
/// **Two doors, not one, and that is `inst-co-copy` rather than a shortcut.** The
/// successor lands on a key a published row occupies, which is
/// `insert_successor_draft_on`'s precondition; the copy lands on a **new
/// generation**, a key nothing occupies at all, which is the authoring door's. A
/// single door could not admit both — each refuses the occupant the other requires
/// (D-195).
async fn stage_both(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    context: &CutoverContext,
    request: &CutoverRequest,
    copy_key: &ScopeKey,
    stamp: AuditStamp,
) -> Result<(PriceRecord, PriceRecord), DomainError> {
    let successor = if let Some(staged) = context.staged_successor.as_ref() {
        staged.clone()
    } else {
        let (record, _predecessor) = price_repo::insert_successor_draft_on(
            runner,
            scope,
            tenant_id,
            NewPriceDraft {
                price_id: request.successor_price_id,
                scope_key: request.predecessor_key.clone(),
                content: request.successor.clone(),
                created_by: stamp.actor_principal_id,
                created_at_utc: stamp.recorded_at,
                correlation_id: stamp.correlation_id,
            },
        )
        .await
        .map_err(|e| repo_failure(&e))?;
        record
    };
    let copy = match context.staged_copy.as_ref() {
        Some(staged) => staged.clone(),
        None => price_repo::create_draft_on(
            runner,
            scope,
            tenant_id,
            NewPriceDraft {
                price_id: request.copy_price_id,
                scope_key: copy_key.clone(),
                // **The predecessor's content, not the successor's.** The copy is what
                // retained subscribers keep paying, so it is the row being closed,
                // carried onto a generation of its own key — a copy of the *new* price
                // would grandfather nobody.
                content: context.predecessor.content(),
                created_by: stamp.actor_principal_id,
                created_at_utc: stamp.recorded_at,
                correlation_id: stamp.correlation_id,
            },
        )
        .await
        .map_err(|e| repo_failure(&e))?,
    };
    Ok((successor, copy))
}

/// The registry request id of one cutover unit.
///
/// Its own first segment, for `unit_request_id`'s reason: three requesters share one
/// registry and what keeps their handles apart is this prefix.
fn cutover_request_id(tenant_id: Uuid, subject_ref: &str) -> String {
    format!("cutover/{tenant_id}/{subject_ref}")
}

/// Read every fact the judgement needs, inside the writing transaction.
async fn read_cutover_context(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    key: &ScopeKey,
    cutover: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<CutoverContext, DomainError> {
    let plan_id = key.plan_id();
    let revision = plan_repo::load_current(runner, scope, tenant_id, plan_id)
        .await
        .map_err(|e| repo_failure(&e))?
        .ok_or_else(|| DomainError::NotFound {
            subject: "current plan revision".to_owned(),
            id: plan_id.to_string(),
        })?;
    let lifecycle_state = revision.lifecycle_state;
    let shape = assemble_from(runner, scope, tenant_id, plan_id, revision, now).await?;

    let on_key = |wanted: &ScopeKey, state: LifecycleState| {
        shape
            .rows
            .iter()
            .find(|row| row.scope_key == *wanted && row.lifecycle_state == state)
            .cloned()
    };
    let predecessor =
        on_key(key, LifecycleState::Published).ok_or_else(|| DomainError::NotFound {
            subject: "current price row on canonical scope key".to_owned(),
            id: key.to_string(),
        })?;
    let staged_successor = on_key(key, LifecycleState::Draft);

    // Every generation already minted on this market, whatever its state: a cutover
    // onto an instant one of them carries is refused at compose, and a *superseded*
    // generation still holds its instant — the key is immutable history, not a slot
    // that frees.
    //
    // **The whole key, and nothing weaker** (D-296, narrowed by D-300). This
    // predicate first spelled `currency` and `region` by hand — two of ten axes —
    // so on a plan carrying a second key in one market a cutover adopted the draft
    // staged for a *different* key and never minted its own. D-296 moved it to
    // `is_sibling_of`, which excludes `price_eligibility` and `cohort` by design;
    // with the class re-imposed beside it that still left **any generation** on the
    // market key matching, and a grandfathered draft is authorable directly through
    // `POST …/prices`. So a stranger's draft at the same instant would have been
    // adopted as this act's copy — retained subscribers billed from operator-typed
    // content instead of the predecessor's — and one at a different instant made
    // the act permanently un-committable behind `refuse_ungenerational`.
    //
    // What this site wants is the key this act *mints*, which is the predecessor's
    // with two axes moved. [`generation_key`] is that construction and the only
    // place that knows it.
    let minted = crate::domain::cutover::generation_key(key, cutover).ok();
    let staged_copy = shape
        .rows
        .iter()
        .find(|row| {
            row.lifecycle_state == LifecycleState::Draft
                && minted
                    .as_ref()
                    .is_some_and(|target| row.scope_key == *target)
        })
        .cloned();

    // **The copy this act already staged is not a generation standing in its way.**
    // `inst-co-copy` refuses a cutover onto an instant some generation already
    // carries, and after the first attempt the plan carries exactly one such
    // generation: the draft copy that attempt staged. Counting it would make the
    // act's own idempotent replay refuse itself — measured, by the replay case,
    // which failed `DUPLICATE_SCOPE_KEY` before this filter existed. Every *other*
    // generation counts whatever its state: a superseded one still holds its
    // instant, because the key is immutable history rather than a slot that frees.
    let staged_copy_id = staged_copy.as_ref().map(|row| row.price_id);
    let existing_generations: Vec<Cohort> = shape
        .rows
        .iter()
        .filter(|row| Some(row.price_id) != staged_copy_id)
        // Six of ten by hand until D-296: `meter` and `dimension_key` were absent,
        // so on D-103's confirmed plan — several usage lines, one `charge_kind`,
        // one market — the generations of *every* line counted as this line's. A
        // cutover onto an instant that only a neighbouring meter's generation held
        // was refused `DUPLICATE_SCOPE_KEY` on a key that was genuinely free: the
        // fail-closed twin of the site above, and the same class as D-283.
        .filter(|row| {
            key.is_sibling_of(&row.scope_key)
                && row.scope_key.price_eligibility() == PriceEligibility::ExistingGrandfathered
        })
        .map(|row| row.scope_key.cohort())
        .collect();
    let stored_plane: Vec<WindowRecord> =
        window_repo::list_for_plan(runner, scope, tenant_id, plan_id)
            .await
            .map_err(|e| repo_failure(&e))?
            .into_iter()
            .filter(|record| record.scope_key == *key)
            .collect();
    let plane = stored_plane
        .iter()
        .map(|record| NamedWindow {
            window_id: record.window_id,
            interval: WindowInterval::new(record.effective_from, record.effective_to, record.state),
        })
        .collect();

    Ok(CutoverContext {
        plan_id,
        revision: shape.revision,
        lifecycle_state,
        shape,
        predecessor,
        staged_successor,
        staged_copy,
        plane,
        stored_plane,
        existing_generations,
    })
}
