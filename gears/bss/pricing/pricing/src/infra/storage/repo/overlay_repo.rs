//! Repository for a `PriceOverlay` and its revision chain — `pricing_price_overlay`
//! plus the two child tables that version with it (`design/09-price-overlays.md`
//! §6, D-42, D-92, D-107).
//!
//! # The overlay carries its own revision chain, unlike a bundle
//!
//! [`bundle_repo`](super::bundle_repo)'s composition rides the **plan's**
//! revisions and takes the plan revision's entity tag. An overlay has no plan:
//! it is a separate object evaluated downstream, and D-92 gives it a chain of
//! its own keyed `(price_overlay_id, revision)`. So this repository owns the
//! whole discipline rather than borrowing one — the open, the copy forward, the
//! compare-and-swap and the publish flip are all here.
//!
//! # The line set is replaced wholesale, never merged
//!
//! Every D-42 rule is a property of the **set** — "≥ 1 line", the duplicate-key
//! rule, the most-specific resolution, the per-currency coverage walk — and a
//! partial update leaves no set the rules could evaluate. It is `PriceRepo`'s
//! band precedent and `BundleRepo`'s composition precedent, for the reason both
//! give.
//!
//! # The copy forward preserves `line_id`, which is the whole of D-92's clause
//!
//! [`OverlayRepo::open_revision`] copies each line onto the successor revision
//! **under its own id**. That is what *"copy-on-new-revision with stable line
//! identity where unchanged"* means, and it is only expressible because the
//! line's key is `(line_id, overlay_revision)` — see `m20260802_000033`'s module
//! doc for why §6's literal `PK line_id` is not buildable beside that clause.
//!
//! # The publish is one commit, and it has to be
//!
//! §6: *"the submit/commit publishes that revision and flips its predecessor
//! `published -> superseded` **in the same commit**"*. Two commits would leave a
//! window in which the overlay has two published revisions — which the partial
//! precedence index would refuse outright, so the intermediate state is not
//! merely wrong but unreachable. [`OverlayRepo::publish_revision`] does both
//! inside one transaction.
//!
//! **The predecessor is superseded first, and the order matters more than the
//! index argument suggests.** Measured by reversing it: eight cases redden, not
//! the one about superseding. The predecessor is found *by state* — the
//! published revision of this overlay — so a lookup made after the successor's
//! flip finds **the row just published** and supersedes it, leaving the overlay
//! with no published revision at all. The partial index is the second reason;
//! the first is that the query has no way to tell the two apart once both are
//! `published`.
//!
//! # No audit record is written here, and that is owed rather than omitted
//!
//! Every other mutating repository in this crate appends to `pricing_audit_log`
//! inside its own transaction (D-14: a record that could commit separately from
//! the mutation it describes is evidence of something that may not have
//! happened). This one does not, and the obstacle is precise rather than
//! philosophical.
//!
//! [`AuditSubjectKind`](crate::domain::audit::AuditSubjectKind) has no overlay
//! member, and an overlay may not borrow `PlanRevision`: an overlay is not a
//! plan and has no plan, so that token would put two aggregates on one chain and
//! make *"who changed this plan"* answer about an object the plan does not
//! contain. Adding the member is a two-line change — and it makes **two**
//! exhaustive `match`es non-exhaustive, one of them in `infra::approval`, which
//! this strand is forbidden to touch. `pricing_audit_log.subject_kind` carries
//! no `CHECK`, so nothing in the store is in the way; the whole obstacle is the
//! enum's second consumer.
//!
//! So the variant, the two arms and the four `append` calls this repository
//! would make are **owed to the controller**, and they are owed as one change
//! rather than four — exactly the arrangement `Trigger::PriceOverlayMutation` is
//! in, and for the same reason. `AuditStamp` is still threaded through every
//! mutating entry point here, so the change is an addition at four call sites
//! and not a signature sweep.
//!
//! # What the store answers and what the pipeline answers
//!
//! Two of §5's overlay codes are typed **409** rather than as architectural
//! 422s: `PRECEDENCE_DUPLICATE` and `OVERLAY_INTERVAL_OVERLAP`. Both have a
//! *checking* half in [`crate::domain::overlay_rules`], which puts them in the
//! one-pass report an author remediates from, and `PRECEDENCE_DUPLICATE` has a
//! *constraint* half here — `uq_pricing_price_overlay_precedence`, which is what
//! catches two submits that both read a free slot before either wrote. The two
//! halves answer the same code and the same remedy, which is
//! [`RepoError::PendingKeyHeld`]'s arrangement one plane over.

use std::collections::{BTreeMap, BTreeSet};

use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, EntityTrait, Order};
use toolkit_db::secure::{
    AccessScope, DBRunner, SecureDeleteExt, SecureEntityExt, SecureInsertExt, SecureUpdateExt,
};
use toolkit_db::{DBProvider, DbError};
use uuid::Uuid;

use crate::domain::audit::AuditStamp;
use crate::domain::money::CurrencyCode;
use crate::domain::overlay::{
    Adjustment, AmountSet, Disclosure, LineKey, Magnitude, OverlayInterval, OverlayLifecycle,
    OverlayLine, ScopeClass, ScopeSelector, ScopeValue, TargetRef, TargetSku, TaxBasis,
};
use crate::domain::overlay_rules::{OverlayWorld, PublishedLineInterval};
use crate::domain::scope_key::PlanId;
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::{
    brand_taxonomy, org_tier_taxonomy, partner_taxonomy, plan, price, price_overlay,
    price_overlay_line, price_overlay_line_amount, region_taxonomy,
};
use crate::infra::storage::repo::plan_repo::tx_failure;

// ---------------------------------------------------------------------------
// The authoring surface's types.
// ---------------------------------------------------------------------------

/// An overlay to create, with its first revision's lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewOverlay {
    /// The overlay's identity, minted by the caller.
    pub price_overlay_id: Uuid,
    /// The tenant, checked against the caller's scope on the way in.
    pub tenant_id: Uuid,
    /// Scope class and value.
    pub scope: ScopeSelector,
    /// L2's explicit precedence.
    pub precedence: i32,
    /// The overlay's own dating.
    pub interval: OverlayInterval,
    /// L5's declared basis — already past
    /// [`check_tax_basis_declared`](crate::domain::overlay_rules::check_tax_basis_declared).
    pub tax_basis: TaxBasis,
    /// L6's exposure flag.
    pub disclosure: Disclosure,
    /// The plans the lines may target.
    pub target_ref: TargetRef,
}

/// An overlay revision as the store holds it, lines and all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverlayRecord {
    /// The overlay's identity.
    pub price_overlay_id: Uuid,
    /// Which revision this is.
    pub revision: u64,
    /// Its state.
    pub lifecycle_state: OverlayLifecycle,
    /// Scope class and value.
    pub scope: ScopeSelector,
    /// Its precedence.
    pub precedence: i32,
    /// Its own dating.
    pub interval: OverlayInterval,
    /// Its declared basis.
    pub tax_basis: TaxBasis,
    /// Its exposure flag.
    pub disclosure: Disclosure,
    /// The plans its lines may target.
    pub target_ref: TargetRef,
    /// The concurrency token the next edit must present.
    pub row_version: i64,
    /// Its adjustment lines, in a stable order.
    pub lines: Vec<OverlayLine>,
}

/// Repository over `pricing_price_overlay` and its two child tables.
#[derive(Clone)]
pub struct OverlayRepo {
    db: DBProvider<DbError>,
}

impl OverlayRepo {
    /// Build over one database provider.
    #[must_use]
    pub const fn new(db: DBProvider<DbError>) -> Self {
        Self { db }
    }

    /// Create the overlay at revision `0`, in `draft`, with its lines.
    ///
    /// One transaction: the header, every line and every per-currency value. The
    /// audit record that belongs in it is owed — see the module doc.
    ///
    /// # Errors
    /// [`RepoError::Db`] on a scope or storage failure;
    /// [`RepoError::OverlayPrecedenceHeld`] when the precedence index refuses.
    pub async fn create(
        &self,
        scope: &AccessScope,
        new: NewOverlay,
        lines: Vec<OverlayLine>,
        stamp: AuditStamp,
    ) -> Result<u64, RepoError> {
        // Carried and not yet used: the audit append this would make is owed to
        // the controller with `AuditSubjectKind::PriceOverlay`, and keeping the
        // parameter is what makes that an addition at four call sites rather
        // than a signature sweep. `BundleRepo::create`'s precedent.
        let _ = stamp;
        let scope = scope.clone();
        let (_, outcome) = self
            .db
            .db()
            .in_transaction::<u64, RepoError, _>(move |txn| {
                Box::pin(async move {
                    let row = price_overlay::ActiveModel {
                        price_overlay_id: Set(new.price_overlay_id),
                        revision: Set(0),
                        tenant_id: Set(new.tenant_id),
                        lifecycle_state: Set(OverlayLifecycle::Draft.as_str().to_owned()),
                        scope_class: Set(new.scope.class().as_str().to_owned()),
                        scope_value: Set(new.scope.stored_value().to_owned()),
                        precedence: Set(new.precedence),
                        effective_from: Set(new.interval.from),
                        effective_to: Set(new.interval.to),
                        tax_basis: Set(new.tax_basis.as_str().to_owned()),
                        disclosure: Set(new.disclosure.as_str().to_owned()),
                        target_ref: Set(render_target_ref(&new.target_ref)),
                        row_version: Set(0),
                    };
                    insert_overlay(txn, &scope, row).await?;
                    write_lines(txn, &scope, new.price_overlay_id, new.tenant_id, 0, &lines)
                        .await?;
                    Ok(0)
                })
            })
            .await;
        outcome.map_err(tx_failure)
    }

    /// Open a successor **draft** revision, copying the published revision's
    /// header and its whole line set forward.
    ///
    /// The copy preserves each `line_id` — D-92's *"stable line identity where
    /// unchanged"*, and the reason the line table's key carries the revision.
    ///
    /// # Errors
    /// [`RepoError::NotFound`] when the overlay has no published revision;
    /// [`RepoError::OverlayOpenDraftExists`] when one is already open;
    /// [`RepoError::Db`] on a scope or storage failure.
    pub async fn open_revision(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        price_overlay_id: Uuid,
        stamp: AuditStamp,
    ) -> Result<u64, RepoError> {
        // Carried and not yet used: the audit append this would make is owed to
        // the controller with `AuditSubjectKind::PriceOverlay`, and keeping the
        // parameter is what makes that an addition at four call sites rather
        // than a signature sweep. `BundleRepo::create`'s precedent.
        let _ = stamp;
        let scope = scope.clone();
        let (_, outcome) = self
            .db
            .db()
            .in_transaction::<u64, RepoError, _>(move |txn| {
                Box::pin(async move {
                    // The read is the explanatory path and the partial index is
                    // the guarantee — D-148's read-then-index arrangement, here
                    // for a uniqueness the caller can be told about in words.
                    if let Some(open) = revision_in_state(
                        txn,
                        &scope,
                        tenant_id,
                        price_overlay_id,
                        OverlayLifecycle::Draft,
                    )
                    .await?
                    {
                        return Err(RepoError::OverlayOpenDraftExists {
                            price_overlay_id: price_overlay_id.to_string(),
                            revision: u64::try_from(open.revision).unwrap_or_default(),
                        });
                    }
                    let published = revision_in_state(
                        txn,
                        &scope,
                        tenant_id,
                        price_overlay_id,
                        OverlayLifecycle::Published,
                    )
                    .await?
                    .ok_or_else(|| RepoError::NotFound {
                        subject: "published price overlay".to_owned(),
                        id: price_overlay_id.to_string(),
                    })?;

                    let successor = published.revision + 1;
                    let copy = price_overlay::ActiveModel {
                        price_overlay_id: Set(published.price_overlay_id),
                        revision: Set(successor),
                        tenant_id: Set(published.tenant_id),
                        lifecycle_state: Set(OverlayLifecycle::Draft.as_str().to_owned()),
                        scope_class: Set(published.scope_class.clone()),
                        scope_value: Set(published.scope_value.clone()),
                        precedence: Set(published.precedence),
                        effective_from: Set(published.effective_from),
                        effective_to: Set(published.effective_to),
                        tax_basis: Set(published.tax_basis.clone()),
                        disclosure: Set(published.disclosure.clone()),
                        target_ref: Set(published.target_ref.clone()),
                        // The successor starts its own tag sequence: a caller
                        // holding the predecessor's tag is editing a revision
                        // that is now frozen, and must be told so rather than
                        // silently succeeding here.
                        row_version: Set(0),
                    };
                    insert_overlay(txn, &scope, copy).await?;
                    copy_lines(
                        txn,
                        &scope,
                        price_overlay_id,
                        tenant_id,
                        published.revision,
                        successor,
                    )
                    .await?;
                    Ok(u64::try_from(successor).unwrap_or_default())
                })
            })
            .await;
        outcome.map_err(tx_failure)
    }

    /// Replace an open draft revision's whole line set, under the caller's row
    /// version.
    ///
    /// The order is: resolve the revision under a compare-and-swap, drop its
    /// lines, insert the submitted set, record. The swap carries
    /// `lifecycle_state = 'draft'` as well as the version, so a caller aiming at
    /// a frozen revision is refused by the statement rather than by a trigger.
    ///
    /// # Errors
    /// [`RepoError::StaleRowVersion`] / [`RepoError::NotDraft`] /
    /// [`RepoError::NotFound`] as [`refuse_edit`] resolves them;
    /// [`RepoError::Db`] on a scope or storage failure.
    #[allow(
        clippy::too_many_arguments,
        reason = "the compare-and-swap needs the whole coordinate — scope, tenant, \
                  overlay, revision and the version read — beside the line set and \
                  the audit stamp. Bundling them into a struct would put six values \
                  that are never carried together anywhere else into a type that \
                  exists only to satisfy a count; `BundleRepo::replace_composition` \
                  takes the same eight for the same reason."
    )]
    pub async fn replace_lines(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        price_overlay_id: Uuid,
        revision: u64,
        expected: i64,
        lines: Vec<OverlayLine>,
        stamp: AuditStamp,
    ) -> Result<i64, RepoError> {
        // Carried and not yet used: the audit append this would make is owed to
        // the controller with `AuditSubjectKind::PriceOverlay`, and keeping the
        // parameter is what makes that an addition at four call sites rather
        // than a signature sweep. `BundleRepo::create`'s precedent.
        let _ = stamp;
        let scope = scope.clone();
        let (_, outcome) = self
            .db
            .db()
            .in_transaction::<i64, RepoError, _>(move |txn| {
                Box::pin(async move {
                    let Ok(number) = i64::try_from(revision) else {
                        return Err(RepoError::NotFound {
                            subject: "price overlay revision".to_owned(),
                            id: format!("{price_overlay_id}/{revision}"),
                        });
                    };
                    let moved = price_overlay::Entity::update_many()
                        .secure()
                        .scope_with(&scope)
                        .col_expr(
                            price_overlay::Column::RowVersion,
                            Expr::col(price_overlay::Column::RowVersion).add(1),
                        )
                        .filter(
                            Condition::all()
                                .add(price_overlay::Column::TenantId.eq(tenant_id))
                                .add(price_overlay::Column::PriceOverlayId.eq(price_overlay_id))
                                .add(price_overlay::Column::Revision.eq(number))
                                .add(price_overlay::Column::RowVersion.eq(expected))
                                .add(
                                    price_overlay::Column::LifecycleState
                                        .eq(OverlayLifecycle::Draft.as_str()),
                                ),
                        )
                        .exec(txn)
                        .await
                        .map_err(|e| RepoError::Db(format!("bump pricing_price_overlay: {e}")))?
                        .rows_affected;
                    if moved == 0 {
                        // The swap is the guard; this read only decides which of
                        // the three refusals the caller is owed.
                        return Err(refuse_edit(
                            txn,
                            &scope,
                            tenant_id,
                            price_overlay_id,
                            number,
                            expected,
                        )
                        .await);
                    }

                    drop_lines(txn, &scope, price_overlay_id, tenant_id, number).await?;
                    write_lines(txn, &scope, price_overlay_id, tenant_id, number, &lines).await?;
                    Ok(expected + 1)
                })
            })
            .await;
        outcome.map_err(tx_failure)
    }

    /// Publish a draft revision and supersede its predecessor, **in one commit**.
    ///
    /// §6 requires the pair to be atomic, and the partial precedence index makes
    /// the intermediate state unreachable rather than merely wrong: two
    /// published revisions of one overlay on one `(class, precedence)` is
    /// exactly what that index refuses.
    ///
    /// # Errors
    /// [`RepoError::NotFound`] when the revision is not an open draft;
    /// [`RepoError::OverlayPrecedenceHeld`] when the index refuses the flip;
    /// [`RepoError::Db`] on a scope or storage failure.
    pub async fn publish_revision(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        price_overlay_id: Uuid,
        revision: u64,
        stamp: AuditStamp,
    ) -> Result<(), RepoError> {
        // Carried and not yet used: the audit append this would make is owed to
        // the controller with `AuditSubjectKind::PriceOverlay`, and keeping the
        // parameter is what makes that an addition at four call sites rather
        // than a signature sweep. `BundleRepo::create`'s precedent.
        let _ = stamp;
        let scope = scope.clone();
        let (_, outcome) = self
            .db
            .db()
            .in_transaction::<(), RepoError, _>(move |txn| {
                Box::pin(async move {
                    let Ok(number) = i64::try_from(revision) else {
                        return Err(RepoError::NotFound {
                            subject: "price overlay revision".to_owned(),
                            id: format!("{price_overlay_id}/{revision}"),
                        });
                    };
                    // The predecessor first: publishing before superseding would
                    // put two published revisions on one `(class, precedence)`
                    // for the length of one statement, which the partial index
                    // refuses outright.
                    if let Some(predecessor) = revision_in_state(
                        txn,
                        &scope,
                        tenant_id,
                        price_overlay_id,
                        OverlayLifecycle::Published,
                    )
                    .await?
                    {
                        flip(
                            txn,
                            &scope,
                            tenant_id,
                            price_overlay_id,
                            predecessor.revision,
                            OverlayLifecycle::Superseded,
                        )
                        .await?;
                    }
                    let moved = flip(
                        txn,
                        &scope,
                        tenant_id,
                        price_overlay_id,
                        number,
                        OverlayLifecycle::Published,
                    )
                    .await?;
                    if moved == 0 {
                        return Err(RepoError::NotFound {
                            subject: "open draft price overlay revision".to_owned(),
                            id: format!("{price_overlay_id}/{revision}"),
                        });
                    }
                    Ok(())
                })
            })
            .await;
        outcome.map_err(tx_failure)
    }

    /// One revision of one overlay, lines and all.
    ///
    /// # Errors
    /// [`RepoError::Db`] on a scope or storage failure;
    /// [`RepoError::CorruptRow`] for a stored token no `CHECK` should have
    /// admitted.
    pub async fn load(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        price_overlay_id: Uuid,
        revision: u64,
    ) -> Result<Option<OverlayRecord>, RepoError> {
        let conn = self
            .db
            .conn()
            .map_err(|e| RepoError::Db(format!("pricing_price_overlay conn: {e}")))?;
        let Ok(number) = i64::try_from(revision) else {
            return Ok(None);
        };
        let Some(row) = price_overlay::Entity::find()
            .secure()
            .scope_with(scope)
            .filter(
                Condition::all()
                    .add(price_overlay::Column::TenantId.eq(tenant_id))
                    .add(price_overlay::Column::PriceOverlayId.eq(price_overlay_id))
                    .add(price_overlay::Column::Revision.eq(number)),
            )
            .one(&conn)
            .await
            .map_err(|e| RepoError::Db(format!("read pricing_price_overlay: {e}")))?
        else {
            return Ok(None);
        };
        let lines = read_lines(&conn, scope, price_overlay_id, tenant_id, number).await?;
        record_of(&row, lines).map(Some)
    }

    /// The **published** revision of one overlay, if it has one.
    ///
    /// # Errors
    /// As [`OverlayRepo::load`].
    pub async fn current(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        price_overlay_id: Uuid,
    ) -> Result<Option<OverlayRecord>, RepoError> {
        let conn = self
            .db
            .conn()
            .map_err(|e| RepoError::Db(format!("pricing_price_overlay conn: {e}")))?;
        let Some(row) = revision_in_state(
            &conn,
            scope,
            tenant_id,
            price_overlay_id,
            OverlayLifecycle::Published,
        )
        .await?
        else {
            return Ok(None);
        };
        let lines = read_lines(&conn, scope, price_overlay_id, tenant_id, row.revision).await?;
        record_of(&row, lines).map(Some)
    }

    /// Every overlay revision of one tenant, optionally narrowed to one scope
    /// class — the `GET /bss-pricing/v1/price-overlays` read.
    ///
    /// **Every revision, not only the published ones.** This is the admin and
    /// Tariffs read (`price_overlay x read`), and an operator listing overlays
    /// needs to see the draft they are editing. L6's `restricted` flag governs
    /// **consumer-facing** exposure and explicitly does not narrow an operator
    /// read (§3 step 7), so nothing is filtered here on it.
    ///
    /// # Errors
    /// As [`OverlayRepo::load`].
    pub async fn list(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        class: Option<ScopeClass>,
    ) -> Result<Vec<OverlayRecord>, RepoError> {
        let conn = self
            .db
            .conn()
            .map_err(|e| RepoError::Db(format!("pricing_price_overlay conn: {e}")))?;
        let mut condition = Condition::all().add(price_overlay::Column::TenantId.eq(tenant_id));
        if let Some(class) = class {
            condition = condition.add(price_overlay::Column::ScopeClass.eq(class.as_str()));
        }
        let rows = price_overlay::Entity::find()
            .secure()
            .scope_with(scope)
            .filter(condition)
            .order_by(price_overlay::Column::Precedence, Order::Asc)
            .order_by(price_overlay::Column::PriceOverlayId, Order::Asc)
            .order_by(price_overlay::Column::Revision, Order::Asc)
            .all(&conn)
            .await
            .map_err(|e| RepoError::Db(format!("list pricing_price_overlay: {e}")))?;

        let mut records = Vec::with_capacity(rows.len());
        for row in rows {
            let lines =
                read_lines(&conn, scope, row.price_overlay_id, tenant_id, row.revision).await?;
            records.push(record_of(&row, lines)?);
        }
        Ok(records)
    }

    /// Is `value` declared in the taxonomy `class` validates against, and active?
    ///
    /// `inst-plv-scope`'s lookup. The classless scope consults nothing and
    /// answers `true`: it has no value, so there is nothing to be undeclared.
    ///
    /// **`customer_group` answers `false` unconditionally**, and that is a
    /// refusal rather than a gap: `pricing_customer_group_taxonomy` belongs to
    /// the membership half, which is not built in this strand, so there is no
    /// universe to validate against. Answering `true` would let a
    /// `customerGroup` overlay publish against a universe that does not exist —
    /// exactly the D-211 shape these four tables were built to avoid — and
    /// answering `false` is the fail-closed reading: the class is refused with a
    /// refusal that names its missing universe.
    ///
    /// # Errors
    /// [`RepoError::Db`] on a scope or storage failure.
    pub async fn taxonomy_declares(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        selector: &ScopeSelector,
    ) -> Result<bool, RepoError> {
        let conn = self
            .db
            .conn()
            .map_err(|e| RepoError::Db(format!("taxonomy conn: {e}")))?;
        declares_selector(&conn, scope, tenant_id, selector).await
    }
}

// ---------------------------------------------------------------------------
// Taxonomy lookup — one function per table, because each is a distinct entity.
// ---------------------------------------------------------------------------

/// The `active` predicate every one of the four shares.
///
/// A **retired** value does not declare anything: §6 guards retirement while a
/// value is referenced, and a value that reached `retired` anyway must not
/// validate a new overlay against itself.
async fn declares(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    class: ScopeClass,
    value: &ScopeValue,
) -> Result<bool, RepoError> {
    const ACTIVE: &str = "active";
    let found = match class {
        ScopeClass::Global => return Ok(true),
        ScopeClass::Region => region_taxonomy::Entity::find()
            .secure()
            .scope_with(scope)
            .filter(
                Condition::all()
                    .add(region_taxonomy::Column::TenantId.eq(tenant_id))
                    .add(region_taxonomy::Column::Value.eq(value.as_str()))
                    .add(region_taxonomy::Column::State.eq(ACTIVE)),
            )
            .one(runner)
            .await
            .map_err(|e| RepoError::Db(format!("read pricing_region_taxonomy: {e}")))?
            .is_some(),
        ScopeClass::Brand => brand_taxonomy::Entity::find()
            .secure()
            .scope_with(scope)
            .filter(
                Condition::all()
                    .add(brand_taxonomy::Column::TenantId.eq(tenant_id))
                    .add(brand_taxonomy::Column::Value.eq(value.as_str()))
                    .add(brand_taxonomy::Column::State.eq(ACTIVE)),
            )
            .one(runner)
            .await
            .map_err(|e| RepoError::Db(format!("read pricing_brand_taxonomy: {e}")))?
            .is_some(),
        ScopeClass::Partner => partner_taxonomy::Entity::find()
            .secure()
            .scope_with(scope)
            .filter(
                Condition::all()
                    .add(partner_taxonomy::Column::TenantId.eq(tenant_id))
                    .add(partner_taxonomy::Column::Value.eq(value.as_str()))
                    .add(partner_taxonomy::Column::State.eq(ACTIVE)),
            )
            .one(runner)
            .await
            .map_err(|e| RepoError::Db(format!("read pricing_partner_taxonomy: {e}")))?
            .is_some(),
        ScopeClass::OrgTier => org_tier_taxonomy::Entity::find()
            .secure()
            .scope_with(scope)
            .filter(
                Condition::all()
                    .add(org_tier_taxonomy::Column::TenantId.eq(tenant_id))
                    .add(org_tier_taxonomy::Column::Value.eq(value.as_str()))
                    .add(org_tier_taxonomy::Column::State.eq(ACTIVE)),
            )
            .one(runner)
            .await
            .map_err(|e| RepoError::Db(format!("read pricing_org_tier_taxonomy: {e}")))?
            .is_some(),
        // The membership half is not built; see `taxonomy_declares`' own doc.
        ScopeClass::CustomerGroup => false,
    };
    Ok(found)
}

// ---------------------------------------------------------------------------
// Statements.
// ---------------------------------------------------------------------------

async fn insert_overlay(
    runner: &impl DBRunner,
    scope: &AccessScope,
    row: price_overlay::ActiveModel,
) -> Result<(), RepoError> {
    price_overlay::Entity::insert(row.clone())
        .secure()
        .scope_with_model(scope, &row)
        .map_err(|e| RepoError::Db(format!("pricing_price_overlay scope: {e}")))?
        .exec(runner)
        .await
        .map_err(|e| {
            // The index is the guarantee; a racing author loses here rather than
            // on the pipeline's check, and both answer `PRECEDENCE_DUPLICATE`.
            if e.to_string()
                .contains("uq_pricing_price_overlay_precedence")
                || e.to_string().contains("pricing_price_overlay.precedence")
            {
                RepoError::OverlayPrecedenceHeld
            } else {
                RepoError::Db(format!("insert pricing_price_overlay: {e}"))
            }
        })
        .map(|_| ())
}

/// One revision of one overlay in a given state, if it has one.
async fn revision_in_state(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    price_overlay_id: Uuid,
    state: OverlayLifecycle,
) -> Result<Option<price_overlay::Model>, RepoError> {
    price_overlay::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(price_overlay::Column::TenantId.eq(tenant_id))
                .add(price_overlay::Column::PriceOverlayId.eq(price_overlay_id))
                .add(price_overlay::Column::LifecycleState.eq(state.as_str())),
        )
        .one(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read pricing_price_overlay by state: {e}")))
}

/// Move one revision's `lifecycle_state`, answering how many rows it moved.
async fn flip(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    price_overlay_id: Uuid,
    revision: i64,
    to: OverlayLifecycle,
) -> Result<u64, RepoError> {
    let from = match to {
        OverlayLifecycle::Published => OverlayLifecycle::Draft,
        OverlayLifecycle::Superseded => OverlayLifecycle::Published,
        // A flip *to* draft is not a sanctioned edge; the trigger refuses it and
        // no caller here asks for one.
        OverlayLifecycle::Draft => return Ok(0),
    };
    price_overlay::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            price_overlay::Column::LifecycleState,
            Expr::value(to.as_str()),
        )
        .filter(
            Condition::all()
                .add(price_overlay::Column::TenantId.eq(tenant_id))
                .add(price_overlay::Column::PriceOverlayId.eq(price_overlay_id))
                .add(price_overlay::Column::Revision.eq(revision))
                // The source state is in the `WHERE`, so a second publisher's
                // flip matches zero rows rather than re-publishing.
                .add(price_overlay::Column::LifecycleState.eq(from.as_str())),
        )
        .exec(runner)
        .await
        .map(|r| r.rows_affected)
        .map_err(|e| {
            if e.to_string()
                .contains("uq_pricing_price_overlay_precedence")
                || e.to_string().contains("pricing_price_overlay.precedence")
            {
                RepoError::OverlayPrecedenceHeld
            } else {
                RepoError::Db(format!("flip pricing_price_overlay: {e}"))
            }
        })
}

/// Which of the three refusals a failed compare-and-swap owes the caller.
///
/// Not the guard — the swap is — so a concurrent publish landing between them
/// only changes which sentence the caller reads, never whether the write
/// happened.
async fn refuse_edit(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    price_overlay_id: Uuid,
    revision: i64,
    expected: i64,
) -> RepoError {
    let found = price_overlay::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(price_overlay::Column::TenantId.eq(tenant_id))
                .add(price_overlay::Column::PriceOverlayId.eq(price_overlay_id))
                .add(price_overlay::Column::Revision.eq(revision)),
        )
        .one(runner)
        .await;
    let subject = "price overlay revision".to_owned();
    let id = format!("{price_overlay_id}/{revision}");
    match found {
        Err(e) => RepoError::Db(format!("resolve pricing_price_overlay refusal: {e}")),
        Ok(None) => RepoError::NotFound { subject, id },
        Ok(Some(row)) if row.lifecycle_state != OverlayLifecycle::Draft.as_str() => {
            RepoError::NotDraft {
                subject,
                id,
                state: row.lifecycle_state,
            }
        }
        Ok(Some(row)) => RepoError::StaleRowVersion {
            subject,
            id,
            current: u64::try_from(row.row_version).unwrap_or_default(),
            submitted: u64::try_from(expected).unwrap_or_default(),
        },
    }
}

/// Write one revision's whole line set — lines before amounts, per the foreign
/// key.
async fn write_lines(
    runner: &impl DBRunner,
    scope: &AccessScope,
    price_overlay_id: Uuid,
    tenant_id: Uuid,
    revision: i64,
    lines: &[OverlayLine],
) -> Result<(), RepoError> {
    for line in lines {
        if line.key.plan_id() == Some(PlanId::new(Uuid::nil())) {
            // The line key's index coalesces an absent plan to the nil uuid, so
            // a request naming it would key as the list-default line and collide
            // with it. The sentinel is the store's; this is what keeps it
            // unforgeable. See `m20260802_000033`'s module doc.
            return Err(RepoError::ValueOutOfRange {
                field: "plan_id".to_owned(),
                value: Uuid::nil().to_string(),
            });
        }
        let row = price_overlay_line::ActiveModel {
            line_id: Set(line.line_id),
            overlay_revision: Set(revision),
            price_overlay_id: Set(price_overlay_id),
            tenant_id: Set(tenant_id),
            plan_id: Set(line.key.plan_id().map(PlanId::get)),
            target_sku: Set(line.key.target_sku().map(|s| s.as_str().to_owned())),
            cohort: Set(line.key.cohort()),
            adjustment_kind: Set(line.adjustment.kind().to_owned()),
            magnitude_kind: Set(line.adjustment.magnitude_kind().to_owned()),
            adjustment_value: Set(line.adjustment.percent_bp()),
        };
        price_overlay_line::Entity::insert(row.clone())
            .secure()
            .scope_with_model(scope, &row)
            .map_err(|e| RepoError::Db(format!("pricing_price_overlay_line scope: {e}")))?
            .exec(runner)
            .await
            .map_err(|e| RepoError::Db(format!("insert pricing_price_overlay_line: {e}")))?;

        let Some(amounts) = line.adjustment.amounts() else {
            continue;
        };
        for (currency, value) in amounts.iter() {
            let row = price_overlay_line_amount::ActiveModel {
                line_id: Set(line.line_id),
                overlay_revision: Set(revision),
                currency: Set(currency.as_str().to_owned()),
                tenant_id: Set(tenant_id),
                value_minor: Set(value),
            };
            price_overlay_line_amount::Entity::insert(row.clone())
                .secure()
                .scope_with_model(scope, &row)
                .map_err(|e| {
                    RepoError::Db(format!("pricing_price_overlay_line_amount scope: {e}"))
                })?
                .exec(runner)
                .await
                .map_err(|e| {
                    RepoError::Db(format!("insert pricing_price_overlay_line_amount: {e}"))
                })?;
        }
    }
    Ok(())
}

/// Drop one revision's lines — amounts before lines, per the foreign key.
async fn drop_lines(
    runner: &impl DBRunner,
    scope: &AccessScope,
    price_overlay_id: Uuid,
    tenant_id: Uuid,
    revision: i64,
) -> Result<(), RepoError> {
    let doomed = read_line_rows(runner, scope, price_overlay_id, tenant_id, revision).await?;
    for line in &doomed {
        price_overlay_line_amount::Entity::delete_many()
            .secure()
            .scope_with(scope)
            .filter(
                Condition::all()
                    .add(price_overlay_line_amount::Column::TenantId.eq(tenant_id))
                    .add(price_overlay_line_amount::Column::LineId.eq(line.line_id))
                    .add(price_overlay_line_amount::Column::OverlayRevision.eq(revision)),
            )
            .exec(runner)
            .await
            .map_err(|e| RepoError::Db(format!("clear overlay line amounts: {e}")))?;
    }
    price_overlay_line::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(line_revision(price_overlay_id, tenant_id, revision))
        .exec(runner)
        .await
        .map_err(|e| RepoError::Db(format!("clear overlay lines: {e}")))?;
    Ok(())
}

/// Copy one revision's whole line set onto `to`, **preserving `line_id`**.
async fn copy_lines(
    runner: &impl DBRunner,
    scope: &AccessScope,
    price_overlay_id: Uuid,
    tenant_id: Uuid,
    from: i64,
    to: i64,
) -> Result<(), RepoError> {
    let lines = read_lines(runner, scope, price_overlay_id, tenant_id, from).await?;
    write_lines(runner, scope, price_overlay_id, tenant_id, to, &lines).await
}

/// *This overlay, this tenant, this revision* — the predicate the line table is
/// ranged over by.
fn line_revision(price_overlay_id: Uuid, tenant_id: Uuid, revision: i64) -> Condition {
    Condition::all()
        .add(price_overlay_line::Column::TenantId.eq(tenant_id))
        .add(price_overlay_line::Column::PriceOverlayId.eq(price_overlay_id))
        .add(price_overlay_line::Column::OverlayRevision.eq(revision))
}

async fn read_line_rows(
    runner: &impl DBRunner,
    scope: &AccessScope,
    price_overlay_id: Uuid,
    tenant_id: Uuid,
    revision: i64,
) -> Result<Vec<price_overlay_line::Model>, RepoError> {
    price_overlay_line::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(line_revision(price_overlay_id, tenant_id, revision))
        .order_by(price_overlay_line::Column::LineId, Order::Asc)
        .all(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read overlay lines: {e}")))
}

/// Read one revision's lines back into the domain's shape.
async fn read_lines(
    runner: &impl DBRunner,
    scope: &AccessScope,
    price_overlay_id: Uuid,
    tenant_id: Uuid,
    revision: i64,
) -> Result<Vec<OverlayLine>, RepoError> {
    let rows = read_line_rows(runner, scope, price_overlay_id, tenant_id, revision).await?;
    let mut lines = Vec::with_capacity(rows.len());
    for row in rows {
        let amounts = price_overlay_line_amount::Entity::find()
            .secure()
            .scope_with(scope)
            .filter(
                Condition::all()
                    .add(price_overlay_line_amount::Column::TenantId.eq(tenant_id))
                    .add(price_overlay_line_amount::Column::LineId.eq(row.line_id))
                    .add(price_overlay_line_amount::Column::OverlayRevision.eq(revision)),
            )
            .order_by(price_overlay_line_amount::Column::Currency, Order::Asc)
            .all(runner)
            .await
            .map_err(|e| RepoError::Db(format!("read overlay line amounts: {e}")))?;
        lines.push(line_of(&row, &amounts)?);
    }
    Ok(lines)
}

/// A stored line and its values, as the domain reads them.
///
/// Every token is parsed rather than carried as a string, so a value no `CHECK`
/// should have admitted is a [`RepoError::CorruptRow`] here instead of an
/// unmatched arm three layers up.
fn line_of(
    row: &price_overlay_line::Model,
    amounts: &[price_overlay_line_amount::Model],
) -> Result<OverlayLine, RepoError> {
    let corrupt = |what: &str| {
        RepoError::CorruptRow(format!(
            "pricing_price_overlay_line {} carries an unusable {what}",
            row.line_id
        ))
    };

    let key = match (row.plan_id, row.target_sku.as_deref(), row.cohort) {
        (None, None, None) => LineKey::list_default(),
        (Some(plan), None, None) => LineKey::for_plan(PlanId::new(plan)),
        (Some(plan), Some(sku), None) => LineKey::for_sku(
            PlanId::new(plan),
            TargetSku::new(sku).ok_or_else(|| corrupt("target_sku"))?,
        ),
        (Some(plan), None, Some(cohort)) => LineKey::for_plan(PlanId::new(plan))
            .for_cohort(cohort)
            .ok_or_else(|| corrupt("cohort"))?,
        (Some(plan), Some(sku), Some(cohort)) => LineKey::for_sku(
            PlanId::new(plan),
            TargetSku::new(sku).ok_or_else(|| corrupt("target_sku"))?,
        )
        .for_cohort(cohort)
        .ok_or_else(|| corrupt("cohort"))?,
        // A SKU or a cohort with no plan: refused by two `CHECK`s and
        // unconstructible in the domain, so reaching here means the table was
        // written around.
        (None, _, _) => return Err(corrupt("plan-less line key")),
    };

    let mut set = Vec::with_capacity(amounts.len());
    for amount in amounts {
        let currency = CurrencyCode::new(&amount.currency).map_err(|_| corrupt("currency"))?;
        set.push((currency, amount.value_minor));
    }
    let amount_set = AmountSet::new(set);

    let magnitude = match row.magnitude_kind.as_str() {
        "percent_bp" => Magnitude::PercentBp(row.adjustment_value.ok_or_else(|| {
            // The biconditional `CHECK` forbids it, so this is a written-around
            // table rather than a caller mistake.
            corrupt("percent_bp line with no adjustment_value")
        })?),
        "amount" => Magnitude::Amount(amount_set.clone()),
        _ => return Err(corrupt("magnitude_kind")),
    };
    let adjustment = match row.adjustment_kind.as_str() {
        "markup" => Adjustment::Markup(magnitude),
        "discount" => Adjustment::Discount(magnitude),
        "fixed" => Adjustment::Fixed(amount_set),
        _ => return Err(corrupt("adjustment_kind")),
    };

    Ok(OverlayLine {
        line_id: row.line_id,
        key,
        adjustment,
    })
}

/// A stored overlay row as the domain reads it.
fn record_of(
    row: &price_overlay::Model,
    lines: Vec<OverlayLine>,
) -> Result<OverlayRecord, RepoError> {
    let corrupt = |what: &str| {
        RepoError::CorruptRow(format!(
            "pricing_price_overlay {}/{} carries an unusable {what}",
            row.price_overlay_id, row.revision
        ))
    };
    let class = ScopeClass::parse(&row.scope_class).ok_or_else(|| corrupt("scope_class"))?;
    let scope = if class == ScopeClass::Global {
        ScopeSelector::Global
    } else {
        let value = ScopeValue::new(&row.scope_value).ok_or_else(|| corrupt("scope_value"))?;
        ScopeSelector::scoped(class, value).ok_or_else(|| corrupt("scope pairing"))?
    };

    Ok(OverlayRecord {
        price_overlay_id: row.price_overlay_id,
        revision: u64::try_from(row.revision).map_err(|_| corrupt("revision"))?,
        lifecycle_state: OverlayLifecycle::parse(&row.lifecycle_state)
            .ok_or_else(|| corrupt("lifecycle_state"))?,
        scope,
        precedence: row.precedence,
        interval: OverlayInterval {
            from: row.effective_from,
            to: row.effective_to,
        },
        tax_basis: TaxBasis::parse(&row.tax_basis).ok_or_else(|| corrupt("tax_basis"))?,
        disclosure: Disclosure::parse(&row.disclosure).ok_or_else(|| corrupt("disclosure"))?,
        target_ref: parse_target_ref(&row.target_ref),
        row_version: row.row_version,
        lines,
    })
}

/// `target_ref` as the column holds it: `{"plans": ["<uuid>", ...]}`.
///
/// A document rather than a child table because it is a **reference set** and
/// not content: nothing joins to it, no rule ranges over it per row, and §6
/// types it `jsonb`.
fn render_target_ref(target_ref: &TargetRef) -> sea_orm::JsonValue {
    serde_json::json!({
        "plans": target_ref.plans.iter().map(|p| p.get().to_string()).collect::<Vec<_>>(),
    })
}

/// The inverse, tolerantly: an unreadable document reads as *no targets*, which
/// is fail-closed — every non-default line is then outside the scope and the
/// overlay cannot publish.
fn parse_target_ref(value: &sea_orm::JsonValue) -> TargetRef {
    let plans = value
        .get("plans")
        .and_then(|p| p.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .filter_map(|raw| Uuid::parse_str(raw).ok())
                .map(PlanId::new)
                .collect()
        })
        .unwrap_or_default();
    TargetRef { plans }
}

// ---------------------------------------------------------------------------
// The world `PriceOverlayValidator` is judged against.
// ---------------------------------------------------------------------------

impl OverlayRepo {
    /// Resolve every fact [`crate::domain::overlay_rules::validate`] needs.
    ///
    /// The rules are pure with respect to what they are handed (§4.2), because
    /// the same set runs twice — as a pre-check at submit and again inside the
    /// publish-commit transaction — and the world moves between the two runs. So
    /// the reads live here and the judgement lives there, and this function is
    /// the only place that decides *what* a rule gets to see.
    ///
    /// # Two facts have no source in this crate, and they are named rather than
    /// # guessed
    ///
    /// * **`published_skus`** is derived from `pricing_plan.sku_id` — the SKU a
    ///   published plan revision publishes under — rendered as a string, because
    ///   §6 types the line's `target_sku` as one. There is no SKU *registry* in
    ///   this repository, so "a SKU this plan publishes" is exactly "the id on
    ///   the plan row" and nothing richer. A line naming any other SKU is
    ///   refused `OVERLAY_LINE_TARGET_UNKNOWN`, which is the fail-closed
    ///   direction; when the registry lands, this is the read that widens.
    /// * **`lower_precedence_matchers`** is D-138's warning domain, and it is
    ///   computed under the reading D-138's own words state — *"the
    ///   lowest-precedence layer able to match its target"*, i.e. numerically
    ///   lower. §F.1 leaves the stack's **sort direction** undecided, and under
    ///   the other reading this set is the complement of itself. See
    ///   `domain::overlay_rules`' module doc; it is an `[H]` owed entry.
    ///
    /// # Errors
    /// [`RepoError::Db`] on a scope or storage failure.
    pub async fn world_for(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        candidate: &OverlayRecord,
    ) -> Result<OverlayWorld, RepoError> {
        let conn = self
            .db
            .conn()
            .map_err(|e| RepoError::Db(format!("overlay world conn: {e}")))?;

        let scope_value_declared =
            declares_selector(&conn, scope, tenant_id, &candidate.scope).await?;
        let plans = plan_facts(&conn, scope, tenant_id, &candidate.target_ref.plans).await?;
        let markets = price_facts(&conn, scope, tenant_id, &candidate.target_ref.plans).await?;
        let overlays = overlay_facts(&conn, scope, tenant_id, candidate).await?;

        Ok(OverlayWorld {
            scope_value_declared,
            published_plans: plans.published,
            published_skus: plans.skus,
            retired_plans: plans.retired,
            sold_currencies: markets.currencies,
            published_cohorts: markets.cohorts,
            precedence_holder: overlays.precedence_holder,
            interval_holders: overlays.interval_holders,
            lower_precedence_matchers: overlays.lower_precedence_matchers,
        })
    }
}

/// What the **plan** plane says about an overlay's targets.
struct PlanFacts {
    published: BTreeSet<PlanId>,
    retired: BTreeSet<PlanId>,
    skus: BTreeMap<PlanId, BTreeSet<TargetSku>>,
}

/// Which targets are published, which are retired, and under which SKU.
async fn plan_facts(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    targets: &[PlanId],
) -> Result<PlanFacts, RepoError> {
    let mut facts = PlanFacts {
        published: BTreeSet::new(),
        retired: BTreeSet::new(),
        skus: BTreeMap::new(),
    };
    for plan_id in targets {
        let revisions = plan::Entity::find()
            .secure()
            .scope_with(scope)
            .filter(
                Condition::all()
                    .add(plan::Column::TenantId.eq(tenant_id))
                    .add(plan::Column::PlanId.eq(plan_id.get())),
            )
            .all(runner)
            .await
            .map_err(|e| RepoError::Db(format!("read pricing_plan for overlay world: {e}")))?;
        for revision in revisions {
            match revision.lifecycle_state.as_str() {
                "published" => {
                    facts.published.insert(*plan_id);
                    if let Some(sku) = revision.sku_id
                        && let Some(named) = TargetSku::new(&sku.to_string())
                    {
                        facts.skus.entry(*plan_id).or_default().insert(named);
                    }
                }
                "retired" => {
                    facts.retired.insert(*plan_id);
                }
                _ => {}
            }
        }
    }
    Ok(facts)
}

/// What the **price** plane says about an overlay's targets.
struct MarketFacts {
    currencies: BTreeMap<PlanId, BTreeSet<CurrencyCode>>,
    cohorts: BTreeMap<PlanId, BTreeSet<chrono::DateTime<chrono::Utc>>>,
}

/// Which markets each target sells, and which grandfathered generations it has
/// published (D-78).
async fn price_facts(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    targets: &[PlanId],
) -> Result<MarketFacts, RepoError> {
    let mut facts = MarketFacts {
        currencies: BTreeMap::new(),
        cohorts: BTreeMap::new(),
    };
    for plan_id in targets {
        let rows = price::Entity::find()
            .secure()
            .scope_with(scope)
            .filter(
                Condition::all()
                    .add(price::Column::TenantId.eq(tenant_id))
                    .add(price::Column::PlanId.eq(plan_id.get()))
                    .add(price::Column::LifecycleState.eq("published")),
            )
            .all(runner)
            .await
            .map_err(|e| RepoError::Db(format!("read pricing_price for overlay world: {e}")))?;
        for row in rows {
            if let Ok(currency) = CurrencyCode::new(&row.currency) {
                facts
                    .currencies
                    .entry(*plan_id)
                    .or_default()
                    .insert(currency);
            }
            if row.price_eligibility == "existing_grandfathered"
                && let Some(at) = published_generation(&row.cohort)
            {
                facts.cohorts.entry(*plan_id).or_default().insert(at);
            }
        }
    }
    Ok(facts)
}

/// What the **overlay** plane says about a candidate.
struct OverlayFacts {
    precedence_holder: Option<Uuid>,
    interval_holders: Vec<PublishedLineInterval>,
    lower_precedence_matchers: BTreeSet<PlanId>,
}

/// The precedence slot, the collision domain and D-138's lower layers.
///
/// One read of the tenant's published overlays serves all three, because all
/// three range over the same set. **The candidate's own overlay is skipped** —
/// D-107: an overlay never collides with another revision of itself, on either
/// the precedence slot or the interval.
async fn overlay_facts(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    candidate: &OverlayRecord,
) -> Result<OverlayFacts, RepoError> {
    let published = price_overlay::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(price_overlay::Column::TenantId.eq(tenant_id))
                .add(
                    price_overlay::Column::LifecycleState.eq(OverlayLifecycle::Published.as_str()),
                ),
        )
        .all(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read published overlays: {e}")))?;

    let mut facts = OverlayFacts {
        precedence_holder: None,
        interval_holders: Vec::new(),
        lower_precedence_matchers: BTreeSet::new(),
    };
    for row in published {
        if row.price_overlay_id == candidate.price_overlay_id {
            continue;
        }
        let record = record_of(&row, Vec::new())?;
        if record.scope.class() == candidate.scope.class()
            && record.precedence == candidate.precedence
        {
            facts.precedence_holder = Some(record.price_overlay_id);
        }
        let lines =
            read_lines(runner, scope, row.price_overlay_id, tenant_id, row.revision).await?;
        for line in &lines {
            facts.interval_holders.push(PublishedLineInterval {
                price_overlay_id: record.price_overlay_id,
                scope: record.scope.clone(),
                key: line.key.clone(),
                interval: record.interval,
            });
            if record.precedence < candidate.precedence {
                collect_lower_layer(&mut facts.lower_precedence_matchers, &record, line);
            }
        }
    }
    Ok(facts)
}

/// The targets one published line at a lower precedence would have a `fixed`
/// layer discard (D-138).
///
/// A list-default line matches **every** target of its own overlay, so it
/// contributes that whole set rather than one plan.
fn collect_lower_layer(into: &mut BTreeSet<PlanId>, holder: &OverlayRecord, line: &OverlayLine) {
    match line.key.plan_id() {
        Some(plan_id) => {
            into.insert(plan_id);
        }
        None => {
            for plan_id in &holder.target_ref.plans {
                into.insert(*plan_id);
            }
        }
    }
}

/// [`OverlayRepo::taxonomy_declares`]' body, taken through any runner.
async fn declares_selector(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    selector: &ScopeSelector,
) -> Result<bool, RepoError> {
    match selector.value() {
        None => Ok(true),
        Some(value) => declares(runner, scope, tenant_id, selector.class(), value).await,
    }
}

/// One published generation's cutover instant, as the **price** plane stores it.
///
/// **The two planes render one instant two ways, and this is the seam.**
/// `pricing_price.cohort` is the canonical scope key's cohort axis and is stored
/// as **epoch milliseconds in text** (`price_repo::read_cohort`, D-144's quantum
/// made storable), with the literal `none` for the two non-grandfathered
/// classes. `pricing_price_overlay_line.cohort` is a `timestamptz`, because §6
/// types it one and because a line's cohort is authored as an instant rather
/// than derived from a key.
///
/// So `inst-plv-eligibility` — *"a `cohort` value that no published
/// `existing_grandfathered` row of the line's target plan carries"* — is a
/// comparison **across two renderings**, and it is done here, once, rather than
/// at the comparison site. Doing it there would mean comparing timestamps as
/// text, which is the trap this crate has a standing rule against: `SeaORM`
/// writes ISO 8601 with a `T`, and `'T'` beats `' '` at byte 11.
///
/// `None` is *"not a generation"* and covers both the `none` sentinel and a
/// token no writer of this crate produces.
fn published_generation(token: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::TimeZone;
    let millis: i64 = token.parse().ok()?;
    chrono::Utc.timestamp_millis_opt(millis).single()
}
