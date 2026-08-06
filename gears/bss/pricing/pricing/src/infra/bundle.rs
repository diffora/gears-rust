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
//! makes the declaration, and `evaluate` answers `alwaysMaterialTrigger` from it
//! **whatever a threshold policy says**, which is the whole of what D-104 buys.
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

use chrono::{DateTime, Utc};
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use toolkit_db::secure::{AccessScope, DBRunner, SecureEntityExt};
use toolkit_db::{DBProvider, DbError};
use uuid::Uuid;

use crate::domain::bundle::{PriceBasis, reconcile};
use crate::domain::bundle_rules::{
    BundleComposition, ComponentDefect, ComponentSnapshot, CoverageRow, validate,
};
use crate::domain::events::CatalogEvent;
use crate::domain::lifecycle::LifecycleState;
use crate::domain::materiality::ChangeSet;
use crate::domain::materiality::triggers::Trigger;
use crate::domain::money::CurrencyCode;
use crate::domain::plan_shape::Frequency;
use crate::domain::scope_key::{PlanId, Region};
use crate::domain::validation::ValidationReport;
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::{bundle, bundle_revshare, plan, plan_phase, price};
use crate::infra::storage::repo::plan_repo::read_frequency;
use crate::infra::storage::repo::{BundleRepo, NewOutboxEvent, outbox_repo};

/// The rows a component contributes to coverage: published, on the durable base.
const COVERAGE_ELIGIBILITY: &str = "all_subscriptions";
/// The non-grandfathered generation (ADR-0002).
const COVERAGE_COHORT: &str = "none";

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
/// and the stored verdict names which act it was: a rev-share re-split *is*
/// vendor payout, and an operator reading the approval record should not have to
/// infer that from a trigger called "composition".
#[must_use]
pub fn rev_share_change_set() -> ChangeSet {
    ChangeSet::of_act(Trigger::RevenueShareChange, [])
}

/// The composition service.
#[derive(Clone)]
pub struct BundleService {
    db: DBProvider<DbError>,
    bundles: BundleRepo,
}

impl BundleService {
    /// Build over one database provider.
    #[must_use]
    pub fn new(db: DBProvider<DbError>) -> Self {
        Self {
            db: db.clone(),
            bundles: BundleRepo::new(db),
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
        Ok(validate(&composition))
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
    /// Callers reach this only with a publishable report;
    /// [`Self::validate_publish`] is what produces one, and a group that would
    /// refuse here has already refused there. The refusal is re-derived rather
    /// than trusted, because §4.2 has the rule set run twice for exactly this
    /// reason.
    ///
    /// # Errors
    /// [`RepoError::NotFound`] when the plan carries no bundle;
    /// [`RepoError::Db`] on a scope or storage failure;
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
                    outbox_repo::enqueue(
                        txn,
                        &scope,
                        NewOutboxEvent {
                            tenant_id,
                            // The **plan**, because that is the aggregate the
                            // outbox orders within and the bundle rides it.
                            aggregate_id: plan_id.get(),
                            event: CatalogEvent::BundleUpdated,
                            payload: serde_json::json!({
                                "bundleId": bundle_id,
                                "planId": plan_id.get(),
                                "planRevision": revision,
                                "priceBasis": record.price_basis.as_str(),
                                "invoiceItemization": record.invoice_itemization.as_str(),
                            }),
                            dedup_key: format!("BundleUpdated:{bundle_id}:{revision}"),
                            correlation_id,
                            enqueued_at: at,
                        },
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
                .add(price::Column::Cohort.eq(COVERAGE_COHORT)),
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

/// Write one party's normalised effective share.
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

    bundle_revshare::Entity::update_many()
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
        .map_err(|e| RepoError::Db(format!("write effective share: {e}")))?;
    Ok(())
}
