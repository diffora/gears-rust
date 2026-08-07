//! What refers to a plan, read for the retirement guard (`inst-re-references`).
//!
//! `domain::retirement::ReferenceReport` decides whether a retirement may
//! proceed; this module is where the report's contents come from. The split is
//! the usual one — the judgement is a pure function with cases of its own, and
//! the reads are here because they are queries against a world the domain layer
//! may not know about.
//!
//! # Two blocking classes, and the narrowing that makes them true
//!
//! **Bundle components** ride `idx_pricing_bundle_component_plan`
//! (`(tenant_id, component_plan_id)`), the index `m20260802_000025` added for
//! exactly this shape of question — `infra::bundle::referencing_markets` asks its
//! forward half. Only a bundle's **current published revision** counts, for
//! D-212's reason restated one direction over: `pricing_bundle_component` is
//! revision-scoped, so one plan may appear in several revisions of one bundle,
//! and a draft revision's component set is nobody's truth yet. Blocking a
//! retirement on a composition no consumer can resolve would refuse an operator
//! an act nothing depends on.
//!
//! **Add-on price-override targets** are the `pricing_plan_addon_rule` rows whose
//! `price_override_ref` names a **price row of the retiring plan**. The column
//! holds a price id rather than a plan id, so the question is asked in two steps:
//! the retiring plan's price ids, then the rules pointing at any of them. Same
//! revision narrowing, same reason.
//!
//! # What this module deliberately cannot see, and it is not an omission
//!
//! **`allowedChangeTargets` (D-24) has no store in this gear.** Nothing persists
//! it — no column, no table, no projection — so the warning class
//! `WarningReferenceKind::AllowedChangeTarget` exists in the domain vocabulary
//! and has no producer here. That is reported rather than smoothed over: the
//! alternative is a dry-run that silently claims no plan lists the retiree as a
//! change target when the truth is that nobody ever wrote it down. The class
//! stays in the domain type because the refusal it belongs to is decided, and a
//! producer landing later needs no change to the judgement.
//!
//! **Overlay targets (D-31) are not read here either.** `pricing_price_overlay`
//! carries them inside a `jsonb` `target_ref` of shape `{"plans": [...]}`, which
//! is answerable only by scanning the tenant's published overlays and matching in
//! Rust — a different cost profile from the two indexed probes above, and one
//! that belongs with the surface that decides how much of it to pay. Also
//! reported.
//!
//! Both absences are **warnings**, never blocks, so nothing this module cannot
//! see can make a retirement wrongly succeed: the two classes it does read are
//! exactly the two that refuse.

use std::collections::BTreeSet;

use sea_orm::{ColumnTrait, Condition, EntityTrait};
use toolkit_db::secure::{AccessScope, DBRunner, SecureEntityExt};
use uuid::Uuid;

use crate::domain::retirement::{BlockingReferenceKind, PlanReference, ReferenceReport};
use crate::domain::scope_key::PlanId;
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::{bundle, bundle_component, plan_addon_rule};
use crate::infra::storage::repo::{plan_repo, price_repo};

/// Everything that refers to `plan_id`, in the two weights
/// `inst-re-references` gives them.
///
/// The report is built even when it is empty — a retirement of a plan nothing
/// refers to costs two index probes returning no rows, which is the common case
/// by a wide margin, and the dry-run needs the empty report to say so.
///
/// # Errors
/// [`RepoError::Db`] on a scope or storage failure; [`RepoError::CorruptRow`]
/// when a revision number has no column representation.
pub async fn references(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    plan_id: PlanId,
) -> Result<ReferenceReport, RepoError> {
    let mut blocking = Vec::new();
    for reference in referencing_bundles(runner, scope, tenant_id, plan_id).await? {
        blocking.push((BlockingReferenceKind::BundleComponent, reference));
    }
    for reference in referencing_addon_overrides(runner, scope, tenant_id, plan_id).await? {
        blocking.push((BlockingReferenceKind::AddOnPriceOverrideTarget, reference));
    }
    Ok(ReferenceReport {
        blocking,
        // See the module doc: neither warning class has a producer in this gear.
        warnings: Vec::new(),
    })
}

/// The bundles whose **current published revision** lists this plan as a
/// component.
async fn referencing_bundles(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    component_plan_id: PlanId,
) -> Result<Vec<PlanReference>, RepoError> {
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
        .map_err(|e| RepoError::Db(format!("read bundles referencing a retiring plan: {e}")))?;

    let bundle_ids: BTreeSet<Uuid> = memberships.iter().map(|row| row.bundle_id).collect();
    let mut found = Vec::new();
    for bundle_id in bundle_ids {
        let Some(record) = bundle_row(runner, scope, tenant_id, bundle_id).await? else {
            continue;
        };
        let Some(revision) = current_revision(runner, scope, tenant_id, record.plan_id).await?
        else {
            continue;
        };
        if !memberships
            .iter()
            .any(|row| row.bundle_id == bundle_id && row.plan_revision == revision)
        {
            // A component of some *other* revision of that bundle, not of the
            // one consumers resolve against.
            continue;
        }
        found.push(PlanReference {
            referrer_id: bundle_id,
            // The bundle's own plan, because that is the thing an operator goes
            // and edits: `pricing_bundle` carries no display name, and a bare
            // bundle id names nothing they can open.
            referrer_label: format!("bundle on plan {}", record.plan_id),
        });
    }
    Ok(found)
}

/// The plans whose current published revision carries an add-on rule overriding
/// a **price row of the retiring plan**.
async fn referencing_addon_overrides(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    plan_id: PlanId,
) -> Result<Vec<PlanReference>, RepoError> {
    let price_ids: Vec<Uuid> =
        price_repo::load_scope_keys_for_plan(runner, scope, tenant_id, plan_id)
            .await?
            .into_iter()
            .map(|(price_id, _key)| price_id)
            .collect();
    if price_ids.is_empty() {
        return Ok(Vec::new());
    }

    let rules = plan_addon_rule::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(plan_addon_rule::Column::TenantId.eq(tenant_id))
                .add(plan_addon_rule::Column::PriceOverrideRef.is_in(price_ids)),
        )
        .all(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read add-on overrides of a retiring plan: {e}")))?;

    let mut found = Vec::new();
    let mut seen: BTreeSet<Uuid> = BTreeSet::new();
    for rule in rules {
        // The rule's **own** plan is the referrer. A rule on the retiring plan
        // itself is not a reference to anything: retiring a plan whose add-on
        // overrides one of its own rows refuses nothing.
        if rule.plan_id == plan_id.get() || !seen.insert(rule.plan_id) {
            continue;
        }
        let Some(revision) = current_revision(runner, scope, tenant_id, rule.plan_id).await? else {
            continue;
        };
        if rule.plan_revision != revision {
            continue;
        }
        found.push(PlanReference {
            referrer_id: rule.plan_id,
            referrer_label: format!("plan {}", rule.plan_id),
        });
    }
    Ok(found)
}

/// The referring plan's current revision, as the child tables store it.
async fn current_revision(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    plan_id: Uuid,
) -> Result<Option<i64>, RepoError> {
    let Some(current) =
        plan_repo::load_current(runner, scope, tenant_id, PlanId::new(plan_id)).await?
    else {
        return Ok(None);
    };
    i64::try_from(current.revision).map(Some).map_err(|_| {
        RepoError::CorruptRow(format!(
            "plan {plan_id} stands at revision {}, which no column can address",
            current.revision
        ))
    })
}

/// Read one bundle header.
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
        .map_err(|e| RepoError::Db(format!("read a referencing bundle: {e}")))
}
