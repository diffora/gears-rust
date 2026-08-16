//! `BundleService` — `inst-ba-validate` and the publish half of
//! `inst-ba-material` / `inst-ba-return` (`design/08-bundles.md` §2).
//!
//! Three things the domain cannot do for itself, and nothing else:
//!
//! **Assemble.** [`crate::domain::bundle_rules`] is pure over a snapshot it is
//! handed, deliberately — §4.2 requires it, because the same rule set runs at
//! submit and again inside the publish commit where the world has moved. Somebody
//! has to do the reads, and this is that somebody. [`BundleService::assemble`]
//! resolves, per referenced component: has its plan published, is that plan
//! itself a bundle, does it carry a phase schedule beyond the D-19 implicit
//! terminal phase, and which coverage-eligible rows does it have.
//!
//! **Declare the act.** A bundle composition change is an
//! [`Trigger::BundleComposition`] and a rev-share change is a
//! [`Trigger::RevenueShareChange`] (D-104), and whether a call *is* one of those
//! is knowable only at the surface performing it —
//! `domain::materiality::triggers`' module doc makes exactly that argument.
//! [`composition_change_set`] and [`rev_share_change_set`] are where this slice
//! builds the declaration, and `evaluate` answers `alwaysMaterialTrigger` from it
//! **whatever a threshold policy says**, which is the whole of what D-104 buys.
//!
//! **A constructor is not a declaration until something calls it**, and only one of
//! the two is called. `api::rest::bundles::bundle_publish_materiality` evaluates
//! over [`composition_change_set`] on the mounted publish route; nothing in `src/`
//! calls [`rev_share_change_set`], so `Trigger::RevenueShareChange` answers
//! `subject_exists_in_this_crate() == false` (D-232, D-321) even though this file,
//! `pricing_bundle_revshare` and the D-07 reconciler are all here.
//!
//! **Normalise at publish.** D-07's residual lands on the group's absorber and
//! the effective shares are written back summing to exactly 10000 bp. That is a
//! write, so it is not the reconciler's; the reconciler is the arithmetic and
//! [`BundleService::publish_composition`] is the transaction.
//!
//! # The coverage narrowing happens here, once
//!
//! `inst-bc-coverage` ranges over `priceEligibility = all_subscriptions`
//! (`cohort = none`) **published** rows only: grandfathered generations are never
//! coverage candidates (ADR-0002) and `new_subscriptions_only` rows are not
//! either — bundle composition demands the durable base, and a new-only promo row
//! expires with its intent. The filter is a `WHERE` clause in [`component_rows`]
//! and nowhere else, which is what keeps it from drifting away from the rules
//! written over it.
//!
//! # A phase schedule is "more than one phase row"
//!
//! D-19 auto-creates an **implicit terminal phase** (kind `evergreen`) on every
//! plan at creation, so "carries no authored schedule" is *exactly one* phase row
//! rather than zero. `COMPONENT_PHASED` fires on two or more. Reading it as
//! `> 0` would refuse every component in the catalogue.

use std::collections::BTreeSet;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use toolkit_db::secure::{AccessScope, DBRunner, SecureEntityExt};
use toolkit_db::{DBProvider, DbError};
use uuid::Uuid;

use crate::domain::bundle::{PriceBasis, reconcile};
use crate::domain::bundle_rules::{
    BundleComposition, ComponentDefect, ComponentSnapshot, CoverageRow, validate,
};
use crate::domain::lifecycle::LifecycleState;
use crate::domain::materiality::ChangeSet;
use crate::domain::materiality::triggers::Trigger;
use crate::domain::money::CurrencyCode;
use crate::domain::plan_shape::Frequency;
use crate::domain::publish::rules::ReferencingMarket;
use crate::domain::scope_key::{Cohort, PlanId, PriceEligibility, Region};
use crate::domain::validation::ValidationReport;
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::{
    bundle, bundle_component, bundle_revshare, plan, plan_phase, price,
};
use crate::infra::storage::repo::outbox_repo::BundleUpdatedPayload;
use crate::infra::storage::repo::plan_repo::{self, read_frequency};
use crate::infra::storage::repo::{BundleRepo, NewOutboxEvent, outbox_repo};

/// The eligibility class a component contributes coverage from: the durable base.
///
/// Rendered by the enum that owns the axis rather than spelled again. Both halves
/// of this narrowing used to be bare literals — `"all_subscriptions"` and
/// `"none"` — in a module that imports [`LifecycleState`] and compares *that* axis
/// through `as_str` three lines away (Z9-7, Z11-9). The reader on the same column
/// goes through the enum (`price_repo::read_eligibility` / `read_cohort`) and so
/// does every other filter on it, so a change to either rendering moved the writer
/// and the readers and left this query behind.
const COVERAGE_ELIGIBILITY: &str = PriceEligibility::AllSubscriptions.as_str();

/// The change set that says *"this call is a bundle composition change"*
/// (D-104, `inst-ba-material`).
///
/// No rows: a `sum_of_parts` recomposition carries **no price-row delta at all**,
/// which is precisely why D-104 had to register it — with a threshold configured
/// the `MaterialityEvaluator` saw nothing to trip on and a component swap reached
/// consumers with no approver, while a $1 price-row change above threshold took
/// two people.
#[must_use]
pub fn composition_change_set() -> ChangeSet {
    ChangeSet::of_act(Trigger::BundleComposition, [])
}

/// The change set for a rev-share, `price_basis` or `invoiceItemization` change
/// (D-104).
///
/// Separate from [`composition_change_set`] because D-104 registers two triggers
/// and a rev-share re-split *is* vendor payout, which an operator reading the
/// approval record should not have to infer from a trigger called "composition".
///
/// # It has no caller, and both halves of that are worth saying
///
/// Nothing in `src/` calls this (D-232, still owed; D-321 for what was done about
/// the attestation). `publish_bundle` declares [`composition_change_set`]'s act
/// unconditionally, so a rev-share-only publish **is** judged always-material —
/// D-104's rule is enforced. What is not built is the part the two triggers exist
/// for: telling the two acts apart in the record.
///
/// **And that needs more than a caller here.** Picking between the two wants a
/// composition diff against the published revision, which no operand on the publish
/// route holds; and the verdict has nowhere to put the answer —
/// `MaterialityVerdict` carries a `MaterialityReason` and no trigger, so both acts
/// render byte-identically today. Wiring this function without those two would move
/// no observable byte and would exist only to satisfy the census that reads it,
/// which is why the census's `false` is the honest answer meanwhile.
#[must_use]
pub fn rev_share_change_set() -> ChangeSet {
    ChangeSet::of_act(Trigger::RevenueShareChange, [])
}

/// The composition service.
#[derive(Clone)]
pub struct BundleService {
    db: DBProvider<DbError>,
    bundles: BundleRepo,
    metrics: Arc<dyn crate::domain::ports::metrics::PricingMetricsPort>,
}

impl BundleService {
    /// Build over one database provider.
    #[must_use]
    pub fn new(db: DBProvider<DbError>) -> Self {
        Self {
            db: db.clone(),
            bundles: BundleRepo::new(db),
            // The safe default, for `PublishService::new`'s reason: a service
            // built without one reports nothing rather than failing to build.
            metrics: Arc::new(crate::domain::ports::metrics::NoopPricingMetrics),
        }
    }

    /// Assemble one revision's composition into the snapshot the rules read.
    ///
    /// `markets` is the set of `(currency, region)` the bundle sells in, which
    /// for a `sum_of_parts` bundle cannot be read off its own rows — it has none
    /// (`inst-bb-rowless`) — so the caller supplies it. For `own_price` the
    /// bundle's own rows are read here and are in the tax-basis set.
    ///
    /// # Errors
    /// [`RepoError::NotFound`] when the plan carries no bundle;
    /// [`RepoError::Db`] on a scope or storage failure;
    /// [`RepoError::CorruptRow`] for a stored token no `CHECK` should have
    /// admitted.
    pub async fn assemble(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        plan_id: PlanId,
        revision: u64,
        markets: Vec<(CurrencyCode, Region)>,
    ) -> Result<BundleComposition, RepoError> {
        let Some(record) = self.bundles.find_by_plan(scope, tenant_id, plan_id).await? else {
            return Err(RepoError::NotFound {
                subject: "bundle".to_owned(),
                id: plan_id.get().to_string(),
            });
        };
        let draft = self
            .bundles
            .load_composition(scope, tenant_id, plan_id, revision)
            .await?;
        let conn = self
            .db
            .conn()
            .map_err(|e| RepoError::Db(format!("bundle service conn: {e}")))?;

        let mut components = Vec::with_capacity(draft.components.len());
        for component in &draft.components {
            let component_plan = PlanId::new(component.component_plan_id);
            components.push(ComponentSnapshot {
                component_plan_id: component.component_plan_id,
                included_sku_id: component.included_sku_id,
                defects: component_defects(&conn, scope, tenant_id, component_plan).await?,
                frequency: component_frequency(&conn, scope, tenant_id, component_plan).await?,
                rows: component_rows(&conn, scope, tenant_id, component_plan).await?,
            });
        }

        // `sum_of_parts` carries no own rows at all; reading them anyway would
        // put an empty set where the rule expects one and cost a query per
        // publish for a value that cannot exist.
        let own_rows = if record.price_basis == PriceBasis::OwnPrice {
            component_rows(&conn, scope, tenant_id, plan_id).await?
        } else {
            Vec::new()
        };

        Ok(BundleComposition {
            bundle_id: record.bundle_id,
            basis: record.price_basis,
            markets,
            components,
            own_rows,
            rev_share_groups: draft.rev_share_groups,
        })
    }

    /// `inst-ba-validate`: run the whole rule set over an assembled composition.
    ///
    /// A thin forward, and deliberately so — the rules are the domain's and this
    /// is only where they are reached from. Kept as a method rather than left to
    /// callers because the pair *assemble then validate* is the contract, and a
    /// caller free to validate something it assembled differently is a caller
    /// free to publish an unvalidated composition.
    ///
    /// # Errors
    /// Whatever [`Self::assemble`] refuses with.
    pub async fn validate_publish(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        plan_id: PlanId,
        revision: u64,
        markets: Vec<(CurrencyCode, Region)>,
    ) -> Result<ValidationReport, RepoError> {
        let composition = self
            .assemble(scope, tenant_id, plan_id, revision, markets)
            .await?;
        let report = validate(&composition);
        // §10's counter, cases (ii) and (iii) (`T-17`). **One run, not two**, and
        // that is a real difference from the plan plane: `report_market_metrics`
        // is called from a pre-check *and* a commit because the publish route's
        // approved arm reaches the commit without pre-checking. The bundle
        // publish has no such arm — `bundles::publish_bundle` calls this and then
        // `publish_composition`, in that order, always — so counting here counts
        // every block exactly once.
        crate::infra::metrics::report_bundle_coverage_metrics(
            &*self.metrics,
            composition.basis,
            &report,
        );
        Ok(report)
    }

    /// Attach the metrics port (`T-17`).
    ///
    /// A **second call** rather than a parameter on [`Self::new`], for
    /// `PublishService::with_metrics`' reason and to the same effect: every
    /// existing caller has nothing to say to it, and the no-op [`Self::new`]
    /// installs is exactly what a test harness means.
    #[must_use]
    pub fn with_metrics(
        mut self,
        metrics: Arc<dyn crate::domain::ports::metrics::PricingMetricsPort>,
    ) -> Self {
        self.metrics = metrics;
        self
    }

    /// Normalise the revision's rev-share onto its absorbers and emit
    /// `BundleUpdated`, in one transaction (`inst-rs-residual`,
    /// `inst-ba-return`).
    ///
    /// The two are one transaction because the event says the composition
    /// changed and the normalisation is what changed it: an event that could
    /// commit separately from the write it describes is evidence of something
    /// that may not have happened — the outbox's own contract.
    ///
    /// # `inst-ba-return`'s read-model half is **not** built here, and nothing
    /// else builds it either
    ///
    /// That instruction's third clause is *"composition frozen into the read
    /// model / snapshot"*. This method records **no**
    /// [`PendingVersionRow`](crate::infra::storage::repo::PendingVersionRow), so
    /// no `CatalogVersion` advances and no subject re-projects — and there would
    /// be nothing to re-project into, because
    /// [`PlanSubjectDelta`](crate::domain::projection::PlanSubjectDelta) carries
    /// no bundle member. Every other publish unit in this gear records one
    /// (`publish`, `window`, `supersession`, `cutover`, `overlay_publish`,
    /// `retirement`, `membership_publish`); this act is the only one that does
    /// not, and `bundle_sellability_tests`'
    /// `a_composition_publish_advances_no_catalog_version` is what holds the
    /// absence rather than leaving it to be rediscovered.
    ///
    /// **It is reported here because of what it costs one surface over.**
    /// `inst-sg-bundle` and `dod-sellability` both require the sellability
    /// surface to expose the *frozen* component key set and walk it; with no
    /// version carrying a composition there is nothing to freeze it into, which
    /// is why [`crate::domain::bundle_sellability`] is built, tested and
    /// uncalled.
    ///
    /// Callers reach this only with a publishable report;
    /// [`Self::validate_publish`] is what produces one, and a group that would
    /// refuse here has already refused there. The refusal is re-derived rather
    /// than trusted, because §4.2 has the rule set run twice for exactly this
    /// reason.
    ///
    /// # One revision may publish **more than once**
    ///
    /// Unlike a plan publish, and it is the premise the announcement's dedup key
    /// used to contradict (Z8-3): nothing here is a compare-and-swap. The writes
    /// are idempotent — the same composition re-normalises to the same effective
    /// shares — and a composition edit moves the plan revision's `row_version`,
    /// which voids the content pin and sends the operator back through a second
    /// approval **over the same revision**. So the second call is a legal act,
    /// and it is answered rather than refused; see the dedup key below for what
    /// keys the announcement instead.
    ///
    /// # Errors
    /// [`RepoError::NotFound`] when the plan carries no bundle, and — Z9-6 —
    /// when a reconciled share addresses no stored party row, which rolls the
    /// whole transaction back so no `BundleUpdated` announces a normalisation
    /// that did not land; [`RepoError::Db`] on a scope or storage failure;
    /// [`RepoError::CorruptRow`] when a group refuses reconciliation at commit
    /// time — which is a state the pre-check should have caught, so it is
    /// reported as a corrupt read rather than as a caller error.
    pub async fn publish_composition(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        plan_id: PlanId,
        revision: u64,
        correlation_id: Uuid,
        at: DateTime<Utc>,
    ) -> Result<(), RepoError> {
        let Some(record) = self.bundles.find_by_plan(scope, tenant_id, plan_id).await? else {
            return Err(RepoError::NotFound {
                subject: "bundle".to_owned(),
                id: plan_id.get().to_string(),
            });
        };
        let draft = self
            .bundles
            .load_composition(scope, tenant_id, plan_id, revision)
            .await?;

        let mut normalised = Vec::new();
        for group in &draft.rev_share_groups {
            let reconciled = reconcile(group).map_err(|refusal| {
                RepoError::CorruptRow(format!(
                    "bundle {} vendor {} does not reconcile at commit: {}",
                    record.bundle_id,
                    group.vendor_sku_id,
                    refusal.code()
                ))
            })?;
            for (party, effective) in reconciled.effective_shares {
                normalised.push((group.vendor_sku_id, party.get().to_owned(), effective));
            }
        }

        let bundle_id = record.bundle_id;
        let scope = scope.clone();
        let (_, outcome) = self
            .db
            .db()
            .in_transaction::<(), RepoError, _>(move |txn| {
                Box::pin(async move {
                    let Ok(number) = i64::try_from(revision) else {
                        return Err(RepoError::CorruptRow(format!(
                            "bundle {bundle_id} revision {revision} exceeds the storable range"
                        )));
                    };
                    for (vendor_sku_id, party, effective) in normalised {
                        write_effective_share(
                            txn,
                            &scope,
                            bundle_id,
                            tenant_id,
                            number,
                            vendor_sku_id,
                            &party,
                            effective,
                        )
                        .await?;
                    }
                    // The name, the aggregate and the key are the module's, never
                    // this call site's — Z8-4. What that buys is
                    // `bundle_updated_dedup_key`, whose doc carries Z8-3's whole
                    // argument: the key is **the act's**, because a revision may
                    // publish more than once and the key used to say it could not.
                    outbox_repo::enqueue(
                        txn,
                        &scope,
                        NewOutboxEvent::bundle_updated(
                            tenant_id,
                            &BundleUpdatedPayload {
                                bundle_id,
                                plan_id,
                                plan_revision: revision,
                                price_basis: record.price_basis,
                                invoice_itemization: record.invoice_itemization,
                                correlation_id,
                            },
                            at,
                        ),
                    )
                    .await?;
                    Ok(())
                })
            })
            .await;
        outcome.map_err(|e| {
            e.into_domain(|infra| RepoError::Db(format!("bundle publish transaction: {infra}")))
        })
    }
}

// ---------------------------------------------------------------------------
// The reads the rules cannot make for themselves.
// ---------------------------------------------------------------------------

/// Everything disqualifying about one component plan.
///
/// One function rather than three predicates, so the set the rules receive is
/// built in one place and a fourth disqualifier has one home.
async fn component_defects(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    component_plan_id: PlanId,
) -> Result<std::collections::BTreeSet<ComponentDefect>, RepoError> {
    let mut defects = std::collections::BTreeSet::new();

    // Published means a revision of this plan is `published`. `retired` is not
    // published for composition purposes — a retired component is one Slice 11
    // has already taken out of sale.
    let published = plan::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(plan::Column::TenantId.eq(tenant_id))
                .add(plan::Column::PlanId.eq(component_plan_id.get()))
                .add(plan::Column::LifecycleState.eq(LifecycleState::Published.as_str())),
        )
        .count(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read component plan state: {e}")))?;
    if published == 0 {
        defects.insert(ComponentDefect::Unpublished);
    }

    let is_bundle = bundle::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(bundle::Column::TenantId.eq(tenant_id))
                .add(bundle::Column::PlanId.eq(component_plan_id.get())),
        )
        .count(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read component bundle flag: {e}")))?;
    if is_bundle > 0 {
        defects.insert(ComponentDefect::IsBundlePlan);
    }

    // See the module doc: D-19's implicit terminal phase means "no authored
    // schedule" is exactly one row.
    let phases = plan_phase::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(plan_phase::Column::TenantId.eq(tenant_id))
                .add(plan_phase::Column::PlanId.eq(component_plan_id.get())),
        )
        .count(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read component phases: {e}")))?;
    if phases > 1 {
        defects.insert(ComponentDefect::Phased);
    }

    Ok(defects)
}

/// A component's recurring frequency, or `None` when it is usage-only.
///
/// Read off the plan's `frequency` column. A plan with none is usage-only for
/// `inst-bc-frequency`'s purposes (L-8), which is the same thing the column
/// says: a frequency is what a recurring cycle has.
async fn component_frequency(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    component_plan_id: PlanId,
) -> Result<Option<Frequency>, RepoError> {
    let row = plan::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(plan::Column::TenantId.eq(tenant_id))
                .add(plan::Column::PlanId.eq(component_plan_id.get()))
                .add(plan::Column::LifecycleState.eq(LifecycleState::Published.as_str())),
        )
        .one(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read component frequency: {e}")))?;
    let Some(row) = row else {
        return Ok(None);
    };
    // `plan_repo`'s reader, not a second one: it refuses the half-set interval
    // pairings the CHECK cannot see, and a component's frequency has to mean
    // exactly what its own plan's does — `inst-bc-frequency` compares the two.
    read_frequency(&row)
}

/// One plan's coverage-eligible published rows.
///
/// The narrowing is here and nowhere else; see the module doc.
async fn component_rows(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    plan_id: PlanId,
) -> Result<Vec<CoverageRow>, RepoError> {
    let rows = price::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(price::Column::TenantId.eq(tenant_id))
                .add(price::Column::PlanId.eq(plan_id.get()))
                .add(price::Column::LifecycleState.eq(LifecycleState::Published.as_str()))
                .add(price::Column::PriceEligibility.eq(COVERAGE_ELIGIBILITY))
                // The non-grandfathered generation (ADR-0002), rendered by
                // `Cohort` and not spelled: `Display` is the writer's
                // rendering (`price_repo`'s `cohort.to_string()`), and it is
                // not `const`, so it lives at the call site rather than in a
                // constant above.
                .add(price::Column::Cohort.eq(Cohort::None.to_string())),
        )
        .all(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read component coverage rows: {e}")))?;

    let mut coverage = Vec::with_capacity(rows.len());
    for row in rows {
        let currency = CurrencyCode::new(&row.currency).map_err(|e| {
            RepoError::CorruptRow(format!(
                "price {} carries an unusable currency: {e}",
                row.price_id
            ))
        })?;
        let region = Region::new(&row.region).map_err(|e| {
            RepoError::CorruptRow(format!(
                "price {} carries an unusable region: {e}",
                row.price_id
            ))
        })?;
        coverage.push(CoverageRow {
            currency,
            region,
            tax_inclusive: row.tax_inclusive,
        });
    }
    Ok(coverage)
}

/// The bundle markets a **component plan's** publish is judged against
/// (`inst-bc-taxbasis`'s reverse half; D-119, homed by D-212).
///
/// The read the pure publish walk cannot do and must not do. It answers *"which
/// bundles sell this plan, and on what tax display basis does each of their
/// markets already stand"*, so that
/// `domain::publish::rules::BundleMarketBasisUnmixed` can compare this publish's
/// candidate rows against it. The index it rides is
/// `idx_pricing_bundle_component_plan` on `(tenant_id, component_plan_id)`, added
/// by `m20260802_000025` for exactly this shape of question — S11's
/// `inst-re-references` asks the mirror one for retirement.
///
/// # Three narrowings, each of them a decision rather than an optimisation
///
/// **Only the referencing bundle's *current published* revision counts** (D-212).
/// `pricing_bundle_component` is revision-scoped, so the same plan may appear in
/// several revisions of one bundle; a draft revision's component set is not yet
/// anybody's truth, and guarding against it would fail a publish over a
/// composition no consumer can resolve.
///
/// **The basis is the *other* members'**, never the publishing plan's own rows —
/// those are the thing being judged, and folding them in would make the
/// comparison a tautology on every market where this plan is the only priced
/// member.
///
/// **Where the other members already disagree among themselves, the first is
/// taken and nothing more is said.** That bundle fails its own publish on the
/// forward half; reporting it again here would send an operator to repair a
/// market whose fault is not the one they are publishing.
///
/// A plan that is a component of nothing costs **one index probe returning no
/// rows**, which is the common case by a wide margin.
///
/// # Errors
///
/// [`RepoError::Db`] on a scope or storage failure; [`RepoError::CorruptRow`] for
/// a stored token no `CHECK` should have admitted.
pub async fn referencing_markets(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    component_plan_id: PlanId,
) -> Result<Vec<ReferencingMarket>, RepoError> {
    let memberships = bundle_component::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(bundle_component::Column::TenantId.eq(tenant_id))
                .add(bundle_component::Column::ComponentPlanId.eq(component_plan_id.get())),
        )
        .all(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read bundles referencing a component: {e}")))?;

    let mut bundle_ids: BTreeSet<Uuid> = BTreeSet::new();
    for row in &memberships {
        bundle_ids.insert(row.bundle_id);
    }

    let mut markets = Vec::new();
    for bundle_id in bundle_ids {
        let Some(record) = bundle_row(runner, scope, tenant_id, bundle_id).await? else {
            // The row went while we read; the composition it named is not
            // resolvable, so there is no market to be judged against.
            continue;
        };
        let bundle_plan = PlanId::new(record.plan_id);
        let Some(current) = plan_repo::load_current(runner, scope, tenant_id, bundle_plan).await?
        else {
            continue;
        };
        let revision = i64::try_from(current.revision).unwrap_or(i64::MAX);
        if !memberships
            .iter()
            .any(|row| row.bundle_id == bundle_id && row.plan_revision == revision)
        {
            // This plan is a component of some *other* revision of that bundle,
            // not of the one consumers resolve against.
            continue;
        }

        let basis = PriceBasis::parse(&record.price_basis).ok_or_else(|| {
            RepoError::CorruptRow(format!(
                "pricing_bundle {bundle_id} carries an unknown price_basis `{}`",
                record.price_basis
            ))
        })?;
        let mut rows: Vec<CoverageRow> = Vec::new();
        if basis == PriceBasis::OwnPrice {
            rows.extend(component_rows(runner, scope, tenant_id, bundle_plan).await?);
        }
        // Every component of that revision *except* the one publishing.
        for other in siblings_of(runner, scope, tenant_id, bundle_id, revision).await? {
            if other == component_plan_id {
                continue;
            }
            rows.extend(component_rows(runner, scope, tenant_id, other).await?);
        }

        let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
        for row in rows {
            let market = (
                row.currency.as_str().to_owned(),
                row.region.as_str().to_owned(),
            );
            if seen.insert(market) {
                markets.push(ReferencingMarket::new(
                    bundle_id,
                    row.currency,
                    row.region,
                    row.tax_inclusive,
                ));
            }
        }
    }
    Ok(markets)
}

/// One bundle row by id.
async fn bundle_row(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    bundle_id: Uuid,
) -> Result<Option<bundle::Model>, RepoError> {
    bundle::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(bundle::Column::TenantId.eq(tenant_id))
                .add(bundle::Column::BundleId.eq(bundle_id)),
        )
        .one(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read pricing_bundle by id: {e}")))
}

/// Every component plan of one bundle revision, **in a fixed order**.
///
/// Sorted by plan id, and that is correctness rather than tidiness: the caller
/// takes the *first* basis it sees per market, so an unsorted read would make the
/// resolved basis depend on the order the store happened to return rows in — two
/// engines, or one engine after a vacuum, could answer differently about the same
/// composition. A probe found this: excluding the publishing plan from the set
/// reddened nothing, because the sibling happened to be read first, and the guard
/// would have compared this plan against **itself** whenever the order flipped.
async fn siblings_of(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    bundle_id: Uuid,
    revision: i64,
) -> Result<Vec<PlanId>, RepoError> {
    let rows = bundle_component::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(bundle_component::Column::TenantId.eq(tenant_id))
                .add(bundle_component::Column::BundleId.eq(bundle_id))
                .add(bundle_component::Column::PlanRevision.eq(revision)),
        )
        .all(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read a bundle revision's components: {e}")))?;
    let mut plans: Vec<PlanId> = rows
        .into_iter()
        .map(|row| PlanId::new(row.component_plan_id))
        .collect();
    plans.sort_unstable_by_key(|plan| plan.get());
    Ok(plans)
}

/// Write one party's normalised effective share.
///
/// **The zero-row outcome is a refusal, not a success** (Z9-6). `update_many`
/// answers `Ok` for a filter that matched nothing, and dropping its
/// `rows_affected` made that outcome indistinguishable from a write: the stored
/// `effective_share_bp` stayed at whatever it was, `publish_composition` returned
/// `Ok(())`, and the `BundleUpdated` in the same transaction announced a
/// composition whose reconciled shares were never stored. Downstream parties are
/// paid on this column, so a normalisation that addressed no row has to be told
/// rather than announced — the same reading `repricing_journal_repo::mark_applied`
/// and `mark_failed` take of their own zero-row case.
///
/// The read behind the write is `bundle_repo::load_composition`, on a **separate**
/// connection and before the transaction opens, so the state it described is not
/// the state this statement runs against: a concurrent composition replace or a
/// `plan_repo::abandon_draft` between the two leaves exactly this filter matching
/// nothing. `Party::new` also trims, and `chk_pricing_bundle_revshare_party` does
/// not, so a row written around this repository reads back under a name no
/// statement here can address.
///
/// # Errors
/// [`RepoError::NotFound`] naming the four-column key when no row answers to it;
/// [`RepoError::Db`] on a scope or storage failure.
#[allow(
    clippy::too_many_arguments,
    reason = "the row's whole four-column primary key plus the tenant, the scope \
              and the value; every one of them addresses the row and none is \
              derivable from the others"
)]
async fn write_effective_share(
    runner: &impl DBRunner,
    scope: &AccessScope,
    bundle_id: Uuid,
    tenant_id: Uuid,
    revision: i64,
    vendor_sku_id: Uuid,
    party: &str,
    effective_share_bp: i32,
) -> Result<(), RepoError> {
    use sea_orm::sea_query::Expr;
    use toolkit_db::secure::SecureUpdateExt;

    let affected = bundle_revshare::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            bundle_revshare::Column::EffectiveShareBp,
            Expr::value(effective_share_bp),
        )
        .filter(
            Condition::all()
                .add(bundle_revshare::Column::BundleId.eq(bundle_id))
                .add(bundle_revshare::Column::TenantId.eq(tenant_id))
                .add(bundle_revshare::Column::PlanRevision.eq(revision))
                .add(bundle_revshare::Column::VendorSkuId.eq(vendor_sku_id))
                .add(bundle_revshare::Column::Party.eq(party)),
        )
        .exec(runner)
        .await
        .map_err(|e| RepoError::Db(format!("write effective share: {e}")))?
        .rows_affected;
    if affected == 0 {
        return Err(RepoError::NotFound {
            subject: "bundle rev-share party row".to_owned(),
            // The whole filter, because any one of the four could be the axis
            // that missed and an operator cannot tell which from a bundle id.
            id: format!("{bundle_id}/{revision}/{vendor_sku_id}/{party}"),
        });
    }
    Ok(())
}
