//! Cloning a plan into a new draft (`design/12-operator-efficiency.md` §3
//! `algo-clone`, `inst-cl-copy`, `inst-cl-resets`, `inst-cl-discount`,
//! `inst-cl-draft`, `inst-cl-windows`; D-19, D-264).
//!
//! The clone copies **configuration** and nothing else. What separates the two
//! is not a list to memorize: configuration is what an author wrote, and
//! everything left behind is *lifecycle state* — where the plan got to, not what
//! it is. Windows, grandfathered generations and superseded history are all the
//! second kind, and `inst-cl-resets` says so in the same breath for all three.
//!
//! # The phase remap is the whole difficulty, and it has three sites
//!
//! Phase rows are copied under **new** `phase_id`s (D-19), so every reference to
//! the old ids has to move with them. There are exactly three:
//!
//! 1. the phase rows' own `converts_to_phase_id` chain;
//! 2. the `phase` axis of every copied price row's canonical scope key;
//! 3. **the keys of the D-41 `entitlement_grants.perPhase` map.**
//!
//! The third is the one the 2026-08-01 review found missing (C-7): the map is
//! keyed *by* `phase_id`, so an unremapped clone published a grant set pointing
//! at phases that existed only in the source, and its first publish failed
//! `GRANT_SET_PHASE_UNKNOWN` on dangling keys. `AddonRule` carries no phase and
//! the descriptor set carries no phase, so those two are copies rather than
//! remaps — checked rather than assumed.
//!
//! # Three clauses of §3 have no operand in this gear, and are named rather than
//! written
//!
//! Writing a guard for any of them would be a rule that can never fire, which
//! this program has spent a day removing:
//!
//! - **`pricing_plan_grant` is not copied because it does not exist.** The copy
//!   set names it, and it is *Slice 10's* credit-grant table (D-52) — a different
//!   object from Slice 6's `entitlement_grants` column, carrying `category`,
//!   `applicability`, `drawdownPriority` and the `source ∈ {authored,
//!   compiled_allowance}` lineage. It is unbuilt, and it sits inside the
//!   `inst-ac-*` chunk D-177 blocks. So **D-130's rule — a `compiled_allowance`
//!   grant is recompiled at the clone's own publish rather than copied — has
//!   nothing to range over**, and the arm that would implement it is not here.
//! - **Contract locks are not copied because this gear stores none.** They live
//!   in the Contracts registry, which D-251 records as absent; nothing on a plan
//!   or a price row is a lock for this code to skip.
//! - **`discountRef` copies unconditionally.** `inst-cl-discount` drops it unless
//!   it "still resolves to a registered instrument", and `m20260802_000056`
//!   already records `inst-dr-referential` as **not buildable**: there is no
//!   instrument registry in this workspace to resolve against. A
//!   drop-if-unresolved arm could never fire, so the ref rides along with the
//!   rest of the row's content.
//!
//! # The clone is an ordinary draft
//!
//! `inst-cl-draft`: no rule reads `cloned_from`, the content pin does not frame
//! it, and the clone's first publish takes the full pipeline and an approval like
//! any other first publish (G1). This module therefore performs **no validation
//! of its own** — it authors a draft, and the publish path judges it. In
//! particular the clone is expected to be *unpublishable* on arrival, because
//! `inst-cl-windows` leaves its billable rows without coverage; that is reported
//! rather than prevented.

use chrono::{DateTime, Utc};
use toolkit_db::secure::AccessScope;
use toolkit_db::{DBProvider, DbError};
use uuid::Uuid;

use crate::domain::audit::AuditStamp;
use crate::domain::contracts::{EntitlementGrants, GrantSet};
use crate::domain::error::DomainError;
use crate::domain::lifecycle::LifecycleState;
use crate::domain::plan::PlanShapePatch;
use crate::domain::plan_shape::PlanPhase;
use crate::domain::price_record::PriceRecord;
use crate::domain::scope_key::{PhaseId, PlanId, PriceEligibility, ScopeKey};
use crate::infra::storage::repo::{
    BundleRepo, NewPlanDraft, NewPriceDraft, PlanRepo, PlanShapeRepo, PriceRepo, plan_repo,
    plan_shape_repo, price_repo,
};
use crate::infra::storage::repo_failure;
use std::collections::BTreeMap;

/// The states a source row is copied from.
///
/// `published` only. A `draft` row of the source belongs to an edit its author
/// has not finished and is not configuration the plan *has*; `superseded` and
/// `retired` rows are history, which `inst-cl-resets` leaves behind for the same
/// reason it leaves windows behind.
const COPIED_ROW_STATES: &[LifecycleState] = &[LifecycleState::Published];

/// What the clone left behind, so the operator learns it from the response
/// rather than from a refused publish.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloneNotice {
    /// `inst-cl-windows`: `PriceWindow` schedules are Slice 7-owned runtime
    /// state (D-03) and are never cloned, so the clone's billable rows have no
    /// coverage and its publish is blocked until the operator schedules some.
    /// Expected, and the reason this is a notice rather than a failure.
    NoCoverageScheduled { rows: usize },
    /// `inst-cl-resets`: `existing_grandfathered` rows are lifecycle state, not
    /// configuration. Copying them under a reset eligibility would collapse two
    /// rows onto one canonical scope key and guarantee a duplicate-scope failure
    /// at the clone's first publish.
    GrandfatheredRowsNotCopied { rows: usize },
    /// **The source is a bundle and its composition did not come across.**
    ///
    /// §3's copy set predates Slice 8 and names `pricing_bundle` nowhere, while
    /// `plan_repo::open_revision` treats the composition as one of the plan's
    /// child tables and copies it — so the two paths that reproduce a plan
    /// disagree, and this one produces a plan that holds the bundle's price rows
    /// and none of its composition. Reported rather than copied *or* refused,
    /// because both of those are edges the design set does not draw; see D-266.
    BundleCompositionNotCopied,
}

/// What a clone produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloneReceipt {
    /// The new plan, in `draft` at revision 0.
    pub plan_id: PlanId,
    /// The plan it was copied from, as recorded in `cloned_from`.
    pub cloned_from: PlanId,
    pub phases_copied: usize,
    pub prices_copied: usize,
    pub composites_copied: usize,
    /// What was deliberately left behind; see [`CloneNotice`].
    pub notices: Vec<CloneNotice>,
}

/// Clones a plan's current revision into a new draft plan.
#[derive(Clone)]
pub struct PlanCloner {
    db: DBProvider<DbError>,
    plans: PlanRepo,
    shapes: PlanShapeRepo,
    prices: PriceRepo,
    /// Read-only here, and only to notice that the source **is** a bundle. This
    /// path copies no composition; see [`CloneNotice::BundleCompositionNotCopied`].
    bundles: BundleRepo,
}

impl PlanCloner {
    #[must_use]
    pub const fn new(
        db: DBProvider<DbError>,
        plans: PlanRepo,
        shapes: PlanShapeRepo,
        prices: PriceRepo,
        bundles: BundleRepo,
    ) -> Self {
        Self {
            db,
            plans,
            shapes,
            prices,
            bundles,
        }
    }

    /// Copy `source`'s **current** revision into `target` as a fresh draft.
    ///
    /// The target id is the caller's, for `NewPlanDraft`'s stated reason: an
    /// authoring surface has to be able to name what it created before the row is
    /// durable, and a store that minted the id would make an idempotent retry
    /// create a second plan. The *child* ids — phases, prices, composites — are
    /// minted here, because no caller can name objects it has not seen.
    ///
    /// # Errors
    /// [`DomainError::NotFound`] when `source` has no current revision — §5's
    /// `CLONE_SOURCE_NOT_FOUND`, which is **not** a declared code in this crate,
    /// so it renders through the canonical not-found family with the source named
    /// in the sentence. The same posture `RepoError::NotSupersedable` took
    /// (D-146): a gear may mint its own error variants, but a **wire code** is
    /// the design set's to declare, and minting one ahead of the route that
    /// returns it is how a code ends up in two spellings.
    ///
    /// Otherwise whatever the repositories refuse with.
    pub async fn clone_plan(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        source: PlanId,
        target: PlanId,
        now: DateTime<Utc>,
        stamp: AuditStamp,
    ) -> Result<CloneReceipt, DomainError> {
        let conn = self
            .db
            .conn()
            .map_err(|e| DomainError::Internal(format!("bss-pricing: clone: {e}")))?;
        let current = plan_repo::load_current(&conn, scope, tenant_id, source)
            .await
            .map_err(|e| repo_failure(&e))?
            .ok_or_else(|| DomainError::NotFound {
                subject: "clonable plan".to_owned(),
                id: source.to_string(),
            })?;

        let source_revision = current.revision;
        // **Destructured, so a field added to `PlanRevision` and forgotten here
        // is a compile error** — D-259's remedy, applied to the second place in
        // this crate that rebuilds a revision from another one. It is here
        // because this path was written field by field and **lost
        // `change_contract` on its first draft**: the clone dropped the plan's
        // `allowedChangeTargets`, its `comparabilityRank` and D-113's carry flag,
        // and nothing refused it — with no edges K4 asks for no rank, so the
        // clone published clean and wrong. `open_revision` had already written
        // the comment explaining why an edge list that resets itself is a silent
        // drop; this path did not read it.
        //
        // The five ignored fields are ignored **by name**: the source's own
        // `plan_id` and `revision` (the clone is a different plan starting at 0),
        // its `cloned_from` (lineage is to the source, not through it), and the
        // three provenance fields the create stamps itself.
        let crate::domain::plan::PlanRevision {
            plan_id: _,
            revision: _,
            cloned_from: _,
            sku_id,
            plan_tier,
            billing_cycle,
            frequency,
            plan_tier_override,
            purchase_min_qty,
            purchase_max_qty,
            invoice_grouping_key,
            available_from,
            available_to,
            entitlement_grants,
            change_contract,
            lifecycle_state: _,
            created_by: _,
            created_at_utc: _,
            row_version: _,
        } = current;

        let source_phases =
            plan_shape_repo::load_phase_set(&conn, scope, tenant_id, source, source_revision)
                .await
                .map_err(|e| repo_failure(&e))?;
        let remap = phase_remap(&source_phases);

        let created = self
            .plans
            .create_draft(
                scope,
                NewPlanDraft {
                    plan_id: target,
                    tenant_id,
                    created_by: stamp.actor_principal_id,
                    created_at_utc: now,
                    sku_id,
                    plan_tier,
                    billing_cycle,
                    frequency,
                    plan_tier_override,
                    purchase_min_qty,
                    purchase_max_qty,
                    invoice_grouping_key,
                    available_from,
                    available_to,
                    cloned_from: Some(source),
                    correlation_id: stamp.correlation_id,
                },
            )
            .await
            .map_err(|e| repo_failure(&e))?;

        let mut version = created.row_version;
        let revision = created.revision;

        if !source_phases.is_empty() {
            let phases = source_phases
                .iter()
                .map(|phase| remapped_phase(phase, &remap))
                .collect();
            version = self
                .shapes
                .replace_phases(scope, tenant_id, target, revision, version, phases, stamp)
                .await
                .map_err(|e| repo_failure(&e))?
                .row_version;
        }

        let rules =
            plan_shape_repo::load_addon_rule_set(&conn, scope, tenant_id, source, source_revision)
                .await
                .map_err(|e| repo_failure(&e))?;
        if !rules.is_empty() {
            version = self
                .shapes
                .replace_addon_rules(scope, tenant_id, target, revision, version, rules, stamp)
                .await
                .map_err(|e| repo_failure(&e))?
                .row_version;
        }

        if let Some(descriptors) =
            plan_shape_repo::load_descriptor(&conn, scope, tenant_id, source, source_revision)
                .await
                .map_err(|e| repo_failure(&e))?
        {
            version = self
                .shapes
                .set_descriptor_set(
                    scope,
                    tenant_id,
                    target,
                    revision,
                    version,
                    descriptors,
                    stamp,
                )
                .await
                .map_err(|e| repo_failure(&e))?
                .row_version;
        }

        let source_composites =
            plan_shape_repo::load_composite_set(&conn, scope, tenant_id, source, source_revision)
                .await
                .map_err(|e| repo_failure(&e))?;
        let composites_copied = source_composites.len();
        if composites_copied > 0 {
            let composites = source_composites
                .into_iter()
                .map(|mut composite| {
                    // A new id: the definition is the same, the row is not, and
                    // `composite_id` is stable across *revisions of one plan*
                    // (D-106) rather than across plans.
                    composite.composite_id = Uuid::new_v4();
                    composite
                })
                .collect();
            version = self
                .shapes
                .replace_composites(
                    scope, tenant_id, target, revision, version, composites, stamp,
                )
                .await
                .map_err(|e| repo_failure(&e))?
                .row_version;
        }

        // **The two authored facts `NewPlanDraft` cannot express**, patched onto
        // the created draft: the grant set with the per-phase map's keys remapped
        // (C-7), and the plan-change contract, which the create path drops
        // because its struct has no field for it.
        self.plans
            .update_draft(
                scope,
                tenant_id,
                target,
                revision,
                version,
                PlanShapePatch {
                    entitlement_grants: Some(remapped_grants(&entitlement_grants, &remap)),
                    change_contract: Some(change_contract),
                    ..PlanShapePatch::default()
                },
                stamp,
            )
            .await
            .map_err(|e| repo_failure(&e))?;

        let (prices_copied, grandfathered) = self
            .copy_rows(scope, tenant_id, source, target, &remap, now, stamp)
            .await?;

        let mut notices = Vec::new();
        notices.extend(self.bundle_notice(scope, tenant_id, source).await?);
        if prices_copied > 0 {
            notices.push(CloneNotice::NoCoverageScheduled {
                rows: prices_copied,
            });
        }
        if grandfathered > 0 {
            notices.push(CloneNotice::GrandfatheredRowsNotCopied {
                rows: grandfathered,
            });
        }

        Ok(CloneReceipt {
            plan_id: target,
            cloned_from: source,
            phases_copied: source_phases.len(),
            prices_copied,
            composites_copied,
            notices,
        })
    }
}

impl PlanCloner {
    /// Copy the source's published price rows onto the clone, resetting each.
    ///
    /// Returns `(copied, left behind)`. Separate from [`PlanCloner::clone_plan`]
    /// because it is the *rows* rather than the shape, and because the exclusion
    /// branch is the one place this path decides not to copy something.
    ///
    /// # Errors
    /// Whatever the price repository refuses with, and
    /// [`DomainError::ValidationFailed`] if a reset key is not constructible.
    /// Whether the source is a bundle, as the one notice this path owes about a
    /// table it does not copy.
    ///
    /// Its own method rather than a branch inside `clone_plan`, because it is a
    /// different question from anything else there: not *what did the copy do*
    /// but *what is the source that this copy set cannot express*.
    ///
    /// # Errors
    /// Whatever the bundle repository refuses with.
    async fn bundle_notice(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        source: PlanId,
    ) -> Result<Option<CloneNotice>, DomainError> {
        Ok(self
            .bundles
            .find_by_plan(scope, tenant_id, source)
            .await
            .map_err(|e| repo_failure(&e))?
            .map(|_| CloneNotice::BundleCompositionNotCopied))
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "every argument is a fact only the caller holds: the scope, the tenant, the two \
                  plans, the phase remap the shape copy already built, the clone instant and the \
                  D-135 audit stamp. `plan_repo::update_draft` carries the same allow for the \
                  same reason"
    )]
    async fn copy_rows(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        source: PlanId,
        target: PlanId,
        remap: &BTreeMap<Uuid, PhaseId>,
        now: DateTime<Utc>,
        stamp: AuditStamp,
    ) -> Result<(usize, usize), DomainError> {
        let conn = self
            .db
            .conn()
            .map_err(|e| DomainError::Internal(format!("bss-pricing: clone rows: {e}")))?;
        let source_rows =
            price_repo::load_for_plan(&conn, scope, tenant_id, source, COPIED_ROW_STATES)
                .await
                .map_err(|e| repo_failure(&e))?;
        let mut copied = 0_usize;
        let mut grandfathered = 0_usize;
        for row in source_rows {
            if row.scope_key.price_eligibility() == PriceEligibility::ExistingGrandfathered {
                grandfathered += 1;
                continue;
            }
            self.prices
                .create_draft(
                    scope,
                    tenant_id,
                    NewPriceDraft {
                        price_id: Uuid::new_v4(),
                        scope_key: reset_key(&row.scope_key, target, remap)?,
                        content: reset_content(&row),
                        created_by: stamp.actor_principal_id,
                        created_at_utc: now,
                        correlation_id: stamp.correlation_id,
                    },
                )
                .await
                .map_err(|e| repo_failure(&e))?;
            copied += 1;
        }
        Ok((copied, grandfathered))
    }
}

/// Old `phase_id` -> new `phase_id`, one entry per source phase.
fn phase_remap(phases: &[PlanPhase]) -> BTreeMap<Uuid, PhaseId> {
    phases
        .iter()
        .map(|phase| (phase.phase_id.get(), PhaseId::new(Uuid::new_v4())))
        .collect()
}

/// A phase under its new id, with its conversion target remapped too.
///
/// A `converts_to_phase_id` the map does not know is left as it stands rather
/// than dropped: the chain is `PhaseGraph`'s to judge, and a cloner that silently
/// repaired a broken source chain would hide the fault instead of copying it.
fn remapped_phase(phase: &PlanPhase, remap: &BTreeMap<Uuid, PhaseId>) -> PlanPhase {
    let mut copy = *phase;
    if let Some(new_id) = remap.get(&phase.phase_id.get()) {
        copy.phase_id = *new_id;
    }
    copy.converts_to_phase_id = phase
        .converts_to_phase_id
        .map(|target| remap.get(&target.get()).copied().unwrap_or(target));
    copy
}

/// The grant set with its per-phase keys moved onto the clone's phases (C-7).
///
/// A key the map does not know is **kept as it stands**, for `remapped_phase`'s
/// reason: `GrantSetPhasesKnown` is what judges a dangling key, and dropping it
/// here would silence the very refusal the clone's first publish owes the author.
fn remapped_grants(
    grants: &EntitlementGrants,
    remap: &BTreeMap<Uuid, PhaseId>,
) -> EntitlementGrants {
    EntitlementGrants {
        plan_tier_ref: grants.plan_tier_ref.clone(),
        plan_level: grants.plan_level.clone(),
        per_phase: grants
            .per_phase
            .iter()
            .map(|(phase_id, set)| {
                let key = remap.get(phase_id).map_or(*phase_id, |id| id.get());
                (key, set.clone())
            })
            .collect::<BTreeMap<Uuid, GrantSet>>(),
    }
}

/// The copied row's key: the clone's plan, the clone's phase, eligibility reset.
///
/// `inst-cl-resets` (O1): `priceEligibility` goes to `all_subscriptions` because
/// eligibility must be re-decided, and the cohort follows it to `none` — the two
/// are one fact, and `ScopeKey::new` refuses the pair that disagrees.
fn reset_key(
    key: &ScopeKey,
    target: PlanId,
    remap: &BTreeMap<Uuid, PhaseId>,
) -> Result<ScopeKey, DomainError> {
    let phase = remap
        .get(&key.phase().get())
        .copied()
        .unwrap_or(key.phase());
    let reset = ScopeKey::new(
        target,
        key.currency().clone(),
        key.region().clone(),
        phase,
        PriceEligibility::AllSubscriptions,
        key.charge_kind(),
        crate::domain::scope_key::Cohort::None,
    )?;
    reset.with_usage_line(key.meter().cloned(), key.dimension_key().clone())
}

/// The copied row's content, with the two lifecycle fields cleared.
///
/// `grandfather_until` is a cutover's tombstone on the source row and says
/// nothing about the clone; `supersedes_price_id` names a row in the *source's*
/// chain, and a clone's first row supersedes nothing.
fn reset_content(row: &PriceRecord) -> crate::domain::price_record::PriceContent {
    let mut content = row.content();
    content.grandfather_until = None;
    content.supersedes_price_id = None;
    content
}
