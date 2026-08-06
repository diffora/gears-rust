//! Repository for a bundle and its **composition** — `pricing_bundle` plus the
//! three child tables that version with the plan revision
//! (`design/08-bundles.md` §6, D-92 + D-105).
//!
//! # The composition carries no entity tag of its own, and that is a decision
//!
//! A composition edit advances the **plan revision's** `row_version`, which is
//! [`plan_shape_repo`](super::plan_shape_repo)'s argument for a plan's child sets
//! applied to a bundle's: if editing the component set left the revision's tag
//! alone, two authors editing one draft would both satisfy `If-Match` and
//! silently interleave — the lost update `fr-concurrent-edit` exists to refuse.
//! [`BundleRepo::replace_composition`] therefore takes the revision's
//! [`RowVersion`] and bumps it in the same statement that matches on it, through
//! `plan_shape_repo::plan_revision_bump` rather than a second spelling of the
//! same swap.
//!
//! # The composition is replaced wholesale, never merged
//!
//! Not a convenience. Every Slice-8 rule is a property of the **set** — "every
//! referenced component" (`inst-bc-coverage`), the per-market coverage walk, the
//! frequency comparison across components, "sum to 100% **per** included vendor
//! SKU" — and a partial update leaves no set the rules could evaluate. It is also
//! `PriceRepo`'s band precedent and `PlanShapeRepo`'s phase precedent, for the
//! same reason both give.
//!
//! # The delete precedes the insert, and the read precedes the delete
//!
//! All three child tables refuse INSERT, UPDATE *and* DELETE against a non-draft
//! parent revision, and their refusal is a raw driver error carrying no state.
//! The read is there so a caller aiming at a frozen revision is told which
//! conjunct failed before any statement reaches a child table. It is **not** the
//! guard — the compare-and-swap is — and losing the revision to a concurrent
//! publish between them rolls the whole transaction back, deleted rows and all.
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
use sea_orm::{ColumnTrait, Condition, EntityTrait, Order};
use toolkit_db::secure::{
    AccessScope, DBRunner, SecureDeleteExt, SecureEntityExt, SecureInsertExt,
};
use toolkit_db::{DBProvider, DbError};
use uuid::Uuid;

use crate::domain::audit::{AuditAction, AuditStamp};
use crate::domain::bundle::{
    Absorber, InvoiceItemization, Party, PartyShare, PriceBasis, RevShareGroup,
};
use crate::domain::concurrency::RowVersion;
use crate::domain::plan::PlanRevision;
use crate::domain::scope_key::PlanId;
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::{
    bundle, bundle_component, bundle_revshare, bundle_revshare_group,
};
use crate::infra::storage::repo::plan_repo::{
    load_revision, mutable_draft, not_found, record_revision_mutation, refuse, swap_guard,
    tx_failure,
};
use crate::infra::storage::repo::plan_shape_repo::plan_revision_bump;

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

// ---------------------------------------------------------------------------
// The authoring surface.
// ---------------------------------------------------------------------------

/// A bundle to create on a plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewBundle {
    /// The bundle's own identity, minted by the caller.
    pub bundle_id: Uuid,
    /// The tenant, checked against the caller's scope on the way in.
    pub tenant_id: Uuid,
    /// The `bundle`-type plan whose revisions the composition rides.
    pub plan_id: PlanId,
    /// `inst-bb-declared`'s declared basis.
    pub price_basis: PriceBasis,
    /// B3's invoice layout.
    pub invoice_itemization: InvoiceItemization,
}

/// One authored component of a composition.
///
/// No `bundle_id`, no `plan_revision` and no `tenant_id`: all three are the
/// repository's to fill from the parent it resolved, never the request's
/// (Global Constraint 9). That is why this type cannot carry them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleComponentDraft {
    /// The component's plan (B1).
    pub component_plan_id: Uuid,
    /// The registry SKU it publishes under.
    pub included_sku_id: Uuid,
    /// Selection-time lower bound.
    pub min_qty: Option<i32>,
    /// Selection-time upper bound.
    pub max_qty: Option<i32>,
}

/// A whole composition, as an author submits it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompositionDraft {
    /// The referenced components.
    pub components: Vec<BundleComponentDraft>,
    /// The rev-share groups, one per included vendor SKU.
    pub rev_share_groups: Vec<RevShareGroup>,
}

/// A bundle as the store holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleRecord {
    /// The bundle's identity.
    pub bundle_id: Uuid,
    /// The plan it rides.
    pub plan_id: PlanId,
    /// Its declared basis.
    pub price_basis: PriceBasis,
    /// Its invoice layout.
    pub invoice_itemization: InvoiceItemization,
}

/// Repository over `pricing_bundle` and its three composition children.
pub struct BundleRepo {
    db: DBProvider<DbError>,
}

impl BundleRepo {
    /// Build over one database provider.
    #[must_use]
    pub fn new(db: DBProvider<DbError>) -> Self {
        Self { db }
    }

    /// Create the bundle row on its plan.
    ///
    /// The row is the bundle's **identity** and is not revision-scoped, so this
    /// is not a composition edit and does not move the revision's tag. The two
    /// content columns it carries are the subject of owed-register entry B-1 and
    /// of `m20260802_000024`'s module doc.
    ///
    /// # Errors
    /// [`RepoError::DuplicateBundleOnPlan`] when the plan already carries one
    /// (`uq_pricing_bundle_plan`), surfaced as a typed refusal rather than as a
    /// driver error; [`RepoError::Db`] on a scope or storage failure.
    pub async fn create(
        &self,
        scope: &AccessScope,
        new: NewBundle,
        stamp: AuditStamp,
    ) -> Result<Uuid, RepoError> {
        let _ = stamp;
        let conn = self
            .db
            .conn()
            .map_err(|e| RepoError::Db(format!("pricing_bundle conn: {e}")))?;
        // The read is the explanatory path and the index is the guarantee — the
        // read-then-index arrangement D-148 describes, here for a uniqueness the
        // caller can be told about in its own words.
        if bundle_of_plan(&conn, scope, new.tenant_id, new.plan_id)
            .await?
            .is_some()
        {
            return Err(RepoError::DuplicateBundleOnPlan {
                plan_id: new.plan_id.get().to_string(),
            });
        }
        let row = bundle::ActiveModel {
            bundle_id: Set(new.bundle_id),
            tenant_id: Set(new.tenant_id),
            plan_id: Set(new.plan_id.get()),
            price_basis: Set(new.price_basis.as_str().to_owned()),
            invoice_itemization: Set(new.invoice_itemization.as_str().to_owned()),
        };
        bundle::Entity::insert(row.clone())
            .secure()
            .scope_with_model(scope, &row)
            .map_err(|e| RepoError::Db(format!("pricing_bundle scope: {e}")))?
            .exec(&conn)
            .await
            .map_err(|e| {
                // The index is the guarantee; a racing creator loses here rather
                // than on the read above.
                if e.to_string().contains("uq_pricing_bundle_plan")
                    || e.to_string().contains("pricing_bundle.plan_id")
                {
                    RepoError::DuplicateBundleOnPlan {
                        plan_id: new.plan_id.get().to_string(),
                    }
                } else {
                    RepoError::Db(format!("insert pricing_bundle: {e}"))
                }
            })?;
        Ok(new.bundle_id)
    }

    /// The bundle on this plan, if it is one.
    ///
    /// # Errors
    /// [`RepoError::Db`] on a scope or storage failure;
    /// [`RepoError::CorruptRow`] for a stored basis or layout token no `CHECK`
    /// should have admitted.
    pub async fn find_by_plan(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        plan_id: PlanId,
    ) -> Result<Option<BundleRecord>, RepoError> {
        let conn = self
            .db
            .conn()
            .map_err(|e| RepoError::Db(format!("pricing_bundle conn: {e}")))?;
        let Some(row) = bundle_row_of_plan(&conn, scope, tenant_id, plan_id).await? else {
            return Ok(None);
        };
        record_of(&row).map(Some)
    }

    /// Replace an open draft revision's whole composition, under the caller's
    /// row version, in one transaction.
    ///
    /// The order is: resolve the revision, drop its composition, insert the
    /// submitted set, then compare-and-swap `row_version + 1` under `expected`.
    /// The module doc says why each step sits where it does.
    ///
    /// # Errors
    /// [`RepoError::StaleRowVersion`] / [`RepoError::NotDraft`] /
    /// [`RepoError::NotFound`] as [`refuse`] resolves them;
    /// [`RepoError::NotFound`] when the plan carries no bundle at all;
    /// [`RepoError::Db`] on a scope or storage failure.
    #[allow(
        clippy::too_many_arguments,
        reason = "the compare-and-swap needs the whole coordinate — scope, tenant, \
                  plan, revision and the version read — beside the payload and the \
                  audit stamp; bundling them into a struct would put five values \
                  that are never carried together anywhere else into a type that \
                  exists only to satisfy a count. `PlanShapeRepo`'s three writers \
                  take the same eight for the same reason."
    )]
    pub async fn replace_composition(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        plan_id: PlanId,
        revision: u64,
        expected: RowVersion,
        draft: CompositionDraft,
        stamp: AuditStamp,
    ) -> Result<PlanRevision, RepoError> {
        let scope = scope.clone();
        let (_, outcome) = self
            .db
            .db()
            .in_transaction::<PlanRevision, RepoError, _>(move |txn| {
                Box::pin(async move {
                    let Some(guard) = swap_guard(tenant_id, plan_id, revision, expected) else {
                        return Err(
                            refuse(txn, &scope, tenant_id, plan_id, revision, expected).await
                        );
                    };
                    if mutable_draft(txn, &scope, tenant_id, plan_id, revision, expected)
                        .await?
                        .is_none()
                    {
                        return Err(
                            refuse(txn, &scope, tenant_id, plan_id, revision, expected).await
                        );
                    }
                    let Some(bundle_id) = bundle_of_plan(txn, &scope, tenant_id, plan_id).await?
                    else {
                        return Err(RepoError::NotFound {
                            subject: "bundle".to_owned(),
                            id: plan_id.get().to_string(),
                        });
                    };
                    let Some(number) = stored_revision(revision) else {
                        return Err(RepoError::CorruptRow(format!(
                            "bundle {bundle_id} revision {revision} exceeds the storable range"
                        )));
                    };

                    drop_composition(txn, &scope, bundle_id, tenant_id, number).await?;
                    write_composition(txn, &scope, bundle_id, tenant_id, number, &draft).await?;

                    let moved = plan_revision_bump(txn, &scope, guard).await?;
                    // The read above is not the guard — a concurrent publish can
                    // land between it and this statement — so a swap that matched
                    // nothing is still resolved, and the composition it has
                    // already replaced is restored by the rollback.
                    if moved == 0 {
                        return Err(
                            refuse(txn, &scope, tenant_id, plan_id, revision, expected).await
                        );
                    }
                    let updated = load_revision(txn, &scope, tenant_id, plan_id, revision)
                        .await?
                        .ok_or_else(|| not_found(plan_id, revision))?;
                    record_revision_mutation(
                        txn,
                        &scope,
                        tenant_id,
                        &updated,
                        AuditAction::Update,
                        expected,
                        stamp,
                    )
                    .await?;
                    Ok(updated)
                })
            })
            .await;
        outcome.map_err(tx_failure)
    }

    /// One revision's whole composition, in a stable order.
    ///
    /// # Errors
    /// [`RepoError::Db`] on a scope or storage failure;
    /// [`RepoError::CorruptRow`] for a stored absorber or party value no `CHECK`
    /// should have admitted.
    pub async fn load_composition(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        plan_id: PlanId,
        revision: u64,
    ) -> Result<CompositionDraft, RepoError> {
        let conn = self
            .db
            .conn()
            .map_err(|e| RepoError::Db(format!("pricing_bundle conn: {e}")))?;
        let Some(bundle_id) = bundle_of_plan(&conn, scope, tenant_id, plan_id).await? else {
            return Ok(CompositionDraft::default());
        };
        let Some(number) = stored_revision(revision) else {
            return Ok(CompositionDraft::default());
        };
        read_composition(&conn, scope, bundle_id, tenant_id, number).await
    }
}

// ---------------------------------------------------------------------------
// Statements.
// ---------------------------------------------------------------------------

async fn bundle_row_of_plan(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    plan_id: PlanId,
) -> Result<Option<bundle::Model>, RepoError> {
    bundle::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(bundle::Column::TenantId.eq(tenant_id))
                .add(bundle::Column::PlanId.eq(plan_id.get())),
        )
        .one(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read pricing_bundle by plan: {e}")))
}

/// A stored bundle row as the domain reads it.
///
/// The two token columns are parsed rather than carried as strings, so a value
/// no `CHECK` should have admitted is a [`RepoError::CorruptRow`] here instead of
/// an unmatched arm three layers up.
fn record_of(row: &bundle::Model) -> Result<BundleRecord, RepoError> {
    let price_basis = PriceBasis::parse(&row.price_basis).ok_or_else(|| {
        RepoError::CorruptRow(format!(
            "pricing_bundle {} carries an unknown price_basis `{}`",
            row.bundle_id, row.price_basis
        ))
    })?;
    let invoice_itemization =
        InvoiceItemization::parse(&row.invoice_itemization).ok_or_else(|| {
            RepoError::CorruptRow(format!(
                "pricing_bundle {} carries an unknown invoice_itemization `{}`",
                row.bundle_id, row.invoice_itemization
            ))
        })?;
    Ok(BundleRecord {
        bundle_id: row.bundle_id,
        plan_id: PlanId::new(row.plan_id),
        price_basis,
        invoice_itemization,
    })
}

/// Drop one revision's composition — parties before groups, per the foreign key.
async fn drop_composition(
    runner: &impl DBRunner,
    scope: &AccessScope,
    bundle_id: Uuid,
    tenant_id: Uuid,
    revision: i64,
) -> Result<(), RepoError> {
    bundle_revshare::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(revision_of(bundle_id, tenant_id, revision, &PartyCols))
        .exec(runner)
        .await
        .map_err(|e| RepoError::Db(format!("clear bundle rev-share parties: {e}")))?;
    bundle_revshare_group::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(revision_of(bundle_id, tenant_id, revision, &GroupCols))
        .exec(runner)
        .await
        .map_err(|e| RepoError::Db(format!("clear bundle rev-share groups: {e}")))?;
    bundle_component::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(revision_of(bundle_id, tenant_id, revision, &ComponentCols))
        .exec(runner)
        .await
        .map_err(|e| RepoError::Db(format!("clear bundle components: {e}")))?;
    Ok(())
}

/// Write one revision's composition — groups before parties, per the foreign key.
async fn write_composition(
    runner: &impl DBRunner,
    scope: &AccessScope,
    bundle_id: Uuid,
    tenant_id: Uuid,
    revision: i64,
    draft: &CompositionDraft,
) -> Result<(), RepoError> {
    let components: Vec<_> = draft
        .components
        .iter()
        .map(|c| bundle_component::ActiveModel {
            bundle_id: Set(bundle_id),
            plan_revision: Set(revision),
            component_plan_id: Set(c.component_plan_id),
            tenant_id: Set(tenant_id),
            included_sku_id: Set(c.included_sku_id),
            min_qty: Set(c.min_qty),
            max_qty: Set(c.max_qty),
        })
        .collect();
    insert_bundle_component(runner, scope, components).await?;

    let groups: Vec<_> = draft
        .rev_share_groups
        .iter()
        .map(|g| bundle_revshare_group::ActiveModel {
            bundle_id: Set(bundle_id),
            plan_revision: Set(revision),
            vendor_sku_id: Set(g.vendor_sku_id),
            tenant_id: Set(tenant_id),
            platform_cut_bp: Set(g.platform_cut_bp),
            residual_absorber_party: Set(g.residual_absorber.as_str().to_owned()),
        })
        .collect();
    insert_bundle_revshare_group(runner, scope, groups).await?;

    let parties: Vec<_> = draft
        .rev_share_groups
        .iter()
        .flat_map(|g| {
            g.parties.iter().map(move |p| bundle_revshare::ActiveModel {
                bundle_id: Set(bundle_id),
                plan_revision: Set(revision),
                vendor_sku_id: Set(g.vendor_sku_id),
                party: Set(p.party.get().to_owned()),
                tenant_id: Set(tenant_id),
                share_bp: Set(p.share_bp),
                // Minted by the publish that normalizes it (D-07).
                effective_share_bp: Set(None),
            })
        })
        .collect();
    insert_bundle_revshare(runner, scope, parties).await
}

/// Read one revision's composition back into the domain's shape.
async fn read_composition(
    runner: &impl DBRunner,
    scope: &AccessScope,
    bundle_id: Uuid,
    tenant_id: Uuid,
    revision: i64,
) -> Result<CompositionDraft, RepoError> {
    let components = bundle_component::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(revision_of(bundle_id, tenant_id, revision, &ComponentCols))
        .order_by(bundle_component::Column::ComponentPlanId, Order::Asc)
        .all(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read bundle components: {e}")))?
        .into_iter()
        .map(|row| BundleComponentDraft {
            component_plan_id: row.component_plan_id,
            included_sku_id: row.included_sku_id,
            min_qty: row.min_qty,
            max_qty: row.max_qty,
        })
        .collect();

    let groups = bundle_revshare_group::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(revision_of(bundle_id, tenant_id, revision, &GroupCols))
        .order_by(bundle_revshare_group::Column::VendorSkuId, Order::Asc)
        .all(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read bundle rev-share groups: {e}")))?;
    let parties = bundle_revshare::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(revision_of(bundle_id, tenant_id, revision, &PartyCols))
        .order_by(bundle_revshare::Column::VendorSkuId, Order::Asc)
        .order_by(bundle_revshare::Column::Party, Order::Asc)
        .all(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read bundle rev-share parties: {e}")))?;

    let mut rev_share_groups = Vec::with_capacity(groups.len());
    for group in groups {
        let absorber = Absorber::parse(&group.residual_absorber_party).ok_or_else(|| {
            RepoError::CorruptRow(format!(
                "bundle {bundle_id} vendor {} carries an unusable absorber `{}`",
                group.vendor_sku_id, group.residual_absorber_party
            ))
        })?;
        let mut members = Vec::new();
        for party in parties
            .iter()
            .filter(|p| p.vendor_sku_id == group.vendor_sku_id)
        {
            let named = Party::new(&party.party).ok_or_else(|| {
                RepoError::CorruptRow(format!(
                    "bundle {bundle_id} carries an unusable party `{}`",
                    party.party
                ))
            })?;
            members.push(PartyShare {
                party: named,
                share_bp: party.share_bp,
            });
        }
        rev_share_groups.push(RevShareGroup {
            vendor_sku_id: group.vendor_sku_id,
            platform_cut_bp: group.platform_cut_bp,
            residual_absorber: absorber,
            parties: members,
        });
    }

    Ok(CompositionDraft {
        components,
        rev_share_groups,
    })
}
