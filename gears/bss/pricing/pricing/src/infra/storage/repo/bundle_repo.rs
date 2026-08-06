//! Repository for a bundle's **composition** — the Slice-8 child tables that
//! version with the plan revision (`design/08-bundles.md` §6, D-92 + D-105).
//!
//! `pricing_bundle_component`, `pricing_bundle_revshare_group` and
//! `pricing_bundle_revshare` are all three of them. It is the Slice-8 analogue of
//! [`plan_shape_repo`](super::plan_shape_repo), and it exists for the same
//! reason: a revision-scoped child set owes two things nothing else in this gear
//! supplies — a **copy forward** when a successor revision opens, and a **drop**
//! when a draft is abandoned. Both are called from
//! [`PlanRepo`](super::PlanRepo), because the revision row is what they are
//! ordered against and it is that repository's.
//!
//! # The composition hangs off the plan by one indirection
//!
//! The Slice-2 child tables carry `plan_id` and key directly on the revision.
//! These three key on `bundle_id`, and the plan is reached through
//! `pricing_bundle`. So every entry point here begins by resolving *which
//! bundle, if any, this plan is* — and a plan that is not a bundle is a
//! **no-op**, not an error: the overwhelming majority of plans are not bundles,
//! and `PlanRepo` calls these functions unconditionally.
//!
//! # The order inside each function is forced by the foreign keys
//!
//! A party row references its group, so the copy writes **groups before
//! parties** and the drop deletes **parties before groups**. Getting either
//! backwards is a foreign-key violation and not a silent reordering, which is
//! why the two are written as one function each rather than left to callers.
//!
//! # `effective_share_bp` is deliberately **not** carried forward
//!
//! It is the publish-time normalization of D-07 — what the *previous* revision
//! published — and the successor has not published. Copying it forward would
//! hand a draft an answer it has not computed, and a draft whose typed shares
//! were then edited would carry effective shares reconciling the **old** split.
//! The typed `share_bp` is the authored content and travels; the effective share
//! is minted by the publish that normalizes it.

use sea_orm::ActiveValue::Set;
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use toolkit_db::secure::{
    AccessScope, DBRunner, SecureDeleteExt, SecureEntityExt, SecureInsertExt,
};
use uuid::Uuid;

use crate::domain::scope_key::PlanId;
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::{
    bundle, bundle_component, bundle_revshare, bundle_revshare_group,
};

/// A revision number as the store holds it, or `None` when it is unstorable.
///
/// [`plan_shape_repo`](super::plan_shape_repo)'s helper of the same name, spelled
/// again rather than imported across module boundaries for one `try_from`.
fn stored_revision(revision: u64) -> Option<i64> {
    i64::try_from(revision).ok()
}

/// Which bundle this plan is, if it is one.
///
/// `None` is the ordinary answer and never an error: most plans are not bundles.
///
/// # Errors
/// [`RepoError::Db`] on a scope or storage failure.
async fn bundle_of_plan(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    plan_id: PlanId,
) -> Result<Option<Uuid>, RepoError> {
    let row = bundle::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(bundle::Column::TenantId.eq(tenant_id))
                .add(bundle::Column::PlanId.eq(plan_id.get())),
        )
        .one(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read pricing_bundle by plan: {e}")))?;
    Ok(row.map(|row| row.bundle_id))
}

/// Copy one revision's whole composition onto the revision `to`.
///
/// Called from [`PlanRepo::open_revision`](super::PlanRepo::open_revision),
/// inside that method's transaction and **after** the destination revision row
/// is inserted: each of these tables refuses an INSERT whose new parent revision
/// is not `draft`, so the ordering is forced by the triggers rather than chosen.
///
/// A plan with no bundle copies nothing.
///
/// # Errors
/// [`RepoError::Db`] on a scope or storage failure — which includes the
/// append-only triggers' refusal when the destination revision is not a `draft`,
/// and the destination revision not existing at all.
pub(super) async fn copy_composition(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    plan_id: PlanId,
    from: u64,
    to: u64,
) -> Result<(), RepoError> {
    let Some(bundle_id) = bundle_of_plan(runner, scope, tenant_id, plan_id).await? else {
        return Ok(());
    };
    let (Some(source), Some(target)) = (stored_revision(from), stored_revision(to)) else {
        return Err(RepoError::CorruptRow(format!(
            "bundle {bundle_id} revision {from} or {to} exceeds the storable range"
        )));
    };

    let components = bundle_component::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(revision_of(bundle_id, tenant_id, source, &ComponentCols))
        .all(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read bundle components: {e}")))?;
    if !components.is_empty() {
        let copies: Vec<_> = components
            .into_iter()
            .map(|row| bundle_component::ActiveModel {
                bundle_id: Set(row.bundle_id),
                plan_revision: Set(target),
                component_plan_id: Set(row.component_plan_id),
                tenant_id: Set(row.tenant_id),
                included_sku_id: Set(row.included_sku_id),
                min_qty: Set(row.min_qty),
                max_qty: Set(row.max_qty),
            })
            .collect();
        insert_bundle_component(runner, scope, copies).await?;
    }

    // Groups before parties: a party row references its group.
    let groups = bundle_revshare_group::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(revision_of(bundle_id, tenant_id, source, &GroupCols))
        .all(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read bundle rev-share groups: {e}")))?;
    if !groups.is_empty() {
        let copies: Vec<_> = groups
            .into_iter()
            .map(|row| bundle_revshare_group::ActiveModel {
                bundle_id: Set(row.bundle_id),
                plan_revision: Set(target),
                vendor_sku_id: Set(row.vendor_sku_id),
                tenant_id: Set(row.tenant_id),
                platform_cut_bp: Set(row.platform_cut_bp),
                residual_absorber_party: Set(row.residual_absorber_party),
            })
            .collect();
        insert_bundle_revshare_group(runner, scope, copies).await?;
    }

    let parties = bundle_revshare::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(revision_of(bundle_id, tenant_id, source, &PartyCols))
        .all(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read bundle rev-share parties: {e}")))?;
    if !parties.is_empty() {
        let copies: Vec<_> = parties
            .into_iter()
            .map(|row| bundle_revshare::ActiveModel {
                bundle_id: Set(row.bundle_id),
                plan_revision: Set(target),
                vendor_sku_id: Set(row.vendor_sku_id),
                party: Set(row.party),
                tenant_id: Set(row.tenant_id),
                share_bp: Set(row.share_bp),
                // Minted by the publish that normalizes it; see the module doc.
                effective_share_bp: Set(None),
            })
            .collect();
        insert_bundle_revshare(runner, scope, copies).await?;
    }
    Ok(())
}

/// Drop one revision's whole composition.
///
/// Called from [`PlanRepo::abandon_draft`](super::PlanRepo::abandon_draft)
/// **before** it flips the revision row: `abandoned` is not `draft` and these
/// tables' DELETE triggers refuse everything afterwards. The ordering is forced
/// by the triggers, exactly as the copy's is.
///
/// A plan with no bundle drops nothing.
///
/// # Errors
/// [`RepoError::Db`] on a scope or storage failure — which includes the
/// append-only triggers' refusal when the revision is not a `draft`.
pub(super) async fn delete_composition(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    plan_id: PlanId,
    revision: u64,
) -> Result<(), RepoError> {
    let Some(bundle_id) = bundle_of_plan(runner, scope, tenant_id, plan_id).await? else {
        return Ok(());
    };
    let Some(number) = stored_revision(revision) else {
        return Ok(());
    };

    // Parties before groups: the foreign key points that way.
    bundle_revshare::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(revision_of(bundle_id, tenant_id, number, &PartyCols))
        .exec(runner)
        .await
        .map_err(|e| RepoError::Db(format!("delete bundle rev-share parties: {e}")))?;
    bundle_revshare_group::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(revision_of(bundle_id, tenant_id, number, &GroupCols))
        .exec(runner)
        .await
        .map_err(|e| RepoError::Db(format!("delete bundle rev-share groups: {e}")))?;
    bundle_component::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(revision_of(bundle_id, tenant_id, number, &ComponentCols))
        .exec(runner)
        .await
        .map_err(|e| RepoError::Db(format!("delete bundle components: {e}")))?;
    Ok(())
}

struct ComponentCols;
struct GroupCols;
struct PartyCols;

/// *This bundle, this tenant, this revision* — the one predicate all three
/// tables are ranged over by.
///
/// Written once rather than three times: three copies are three places for a
/// forgotten `tenant_id` to hide behind `SecureORM`'s own scoping.
fn revision_of<C>(bundle_id: Uuid, tenant_id: Uuid, revision: i64, cols: &C) -> Condition
where
    C: RevisionColumns,
{
    Condition::all()
        .add(cols.bundle_eq(bundle_id))
        .add(cols.tenant_eq(tenant_id))
        .add(cols.revision_eq(revision))
}

/// The column triple of one composition table.
trait RevisionColumns {
    fn bundle_eq(&self, value: Uuid) -> sea_orm::sea_query::SimpleExpr;
    fn tenant_eq(&self, value: Uuid) -> sea_orm::sea_query::SimpleExpr;
    fn revision_eq(&self, value: i64) -> sea_orm::sea_query::SimpleExpr;
}

impl RevisionColumns for ComponentCols {
    fn bundle_eq(&self, value: Uuid) -> sea_orm::sea_query::SimpleExpr {
        bundle_component::Column::BundleId.eq(value)
    }
    fn tenant_eq(&self, value: Uuid) -> sea_orm::sea_query::SimpleExpr {
        bundle_component::Column::TenantId.eq(value)
    }
    fn revision_eq(&self, value: i64) -> sea_orm::sea_query::SimpleExpr {
        bundle_component::Column::PlanRevision.eq(value)
    }
}

impl RevisionColumns for GroupCols {
    fn bundle_eq(&self, value: Uuid) -> sea_orm::sea_query::SimpleExpr {
        bundle_revshare_group::Column::BundleId.eq(value)
    }
    fn tenant_eq(&self, value: Uuid) -> sea_orm::sea_query::SimpleExpr {
        bundle_revshare_group::Column::TenantId.eq(value)
    }
    fn revision_eq(&self, value: i64) -> sea_orm::sea_query::SimpleExpr {
        bundle_revshare_group::Column::PlanRevision.eq(value)
    }
}

impl RevisionColumns for PartyCols {
    fn bundle_eq(&self, value: Uuid) -> sea_orm::sea_query::SimpleExpr {
        bundle_revshare::Column::BundleId.eq(value)
    }
    fn tenant_eq(&self, value: Uuid) -> sea_orm::sea_query::SimpleExpr {
        bundle_revshare::Column::TenantId.eq(value)
    }
    fn revision_eq(&self, value: i64) -> sea_orm::sea_query::SimpleExpr {
        bundle_revshare::Column::PlanRevision.eq(value)
    }
}

// ---------------------------------------------------------------------------
// Writers - one row at a time, under `scope_with_model`.
// ---------------------------------------------------------------------------
//
// `insert_many` cannot carry a per-row scope check, and the check is the point:
// `scope_with_model` validates the tenant of the `ActiveModel` it is given, which
// is the second half of the rule the module doc states — the value is copied from
// the parent bundle, and then checked against the caller's scope. This is
// `plan_shape_repo::insert_addon_rules`' shape, for its reason.

async fn insert_bundle_component(
    runner: &impl DBRunner,
    scope: &AccessScope,
    rows: Vec<bundle_component::ActiveModel>,
) -> Result<(), RepoError> {
    for row in rows {
        bundle_component::Entity::insert(row.clone())
            .secure()
            .scope_with_model(scope, &row)
            .map_err(|e| RepoError::Db(format!("pricing_bundle_component scope: {e}")))?
            .exec(runner)
            .await
            .map_err(|e| RepoError::Db(format!("insert pricing_bundle_component: {e}")))?;
    }
    Ok(())
}

async fn insert_bundle_revshare_group(
    runner: &impl DBRunner,
    scope: &AccessScope,
    rows: Vec<bundle_revshare_group::ActiveModel>,
) -> Result<(), RepoError> {
    for row in rows {
        bundle_revshare_group::Entity::insert(row.clone())
            .secure()
            .scope_with_model(scope, &row)
            .map_err(|e| RepoError::Db(format!("pricing_bundle_revshare_group scope: {e}")))?
            .exec(runner)
            .await
            .map_err(|e| RepoError::Db(format!("insert pricing_bundle_revshare_group: {e}")))?;
    }
    Ok(())
}

async fn insert_bundle_revshare(
    runner: &impl DBRunner,
    scope: &AccessScope,
    rows: Vec<bundle_revshare::ActiveModel>,
) -> Result<(), RepoError> {
    for row in rows {
        bundle_revshare::Entity::insert(row.clone())
            .secure()
            .scope_with_model(scope, &row)
            .map_err(|e| RepoError::Db(format!("pricing_bundle_revshare scope: {e}")))?
            .exec(runner)
            .await
            .map_err(|e| RepoError::Db(format!("insert pricing_bundle_revshare: {e}")))?;
    }
    Ok(())
}
