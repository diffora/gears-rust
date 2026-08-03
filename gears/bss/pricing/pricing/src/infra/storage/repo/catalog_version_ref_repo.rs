//! The writer of `pricing_catalog_version_ref` — the pending handle a publish
//! commit leaves behind, and the subject the projector will resolve it to.
//!
//! Runner-taking and stateless, like [`audit_repo`](super::audit_repo) and
//! [`outbox_repo`](super::outbox_repo), and for the sharpest version of their
//! reason: a ref recorded outside the publish transaction is either a pending
//! assignment for a publish that never happened — which trips
//! `pricing.catalogversion.commit_overdue` forever — or a publish whose
//! addressability nothing is tracking.
//!
//! # The subject linkage
//!
//! `pricing_catalog_version_ref` is `(tenant_id, pending_ref, subject_kind,
//! subject_ref, catalog_version, requested_at, committed_at)`, and the two
//! subject columns were added by an in-place amendment to
//! `m20260802_000004` while building this path. The migration's own doc carries
//! the argument and the rejected alternative; what matters here is the shape of
//! the obligation. The projector arrives at `CatalogVersionPublished` holding
//! committed refs and must write **exactly the subjects of the publish units
//! that produced** each version (§4.4, D-86/D-91). This row is the only durable
//! answer to "which subject did this handle publish".
//!
//! [`PendingVersionRow::for_subject`] derives both columns from one
//! [`SubjectRef`], so a writer cannot produce a kind and a reference that
//! disagree — `SubjectRef` already makes an `overlay_index` reference
//! inseparable from its shard key, and this constructor is what carries that
//! property into storage.
//!
//! # The finalize, and the one answer it refuses
//!
//! [`finalize`] is a compare-and-swap whose predicate is `catalog_version IS
//! NULL`, so two sweeps resolving one handle cannot both win. Zero rows
//! affected is **not** silently fine: the row is re-read, and the two cases
//! that produce it are answered differently. Already committed at the **same**
//! version is the idempotent replay a re-drive is entitled to make and returns
//! `Ok`. Already committed at a **different** version means the registry
//! answered one handle two ways — which re-points a pin that posted periods
//! resolve through — and is refused as [`RepoError::CorruptRow`], whose doc
//! already says "an invariant breach, never a caller mistake". No new variant
//! and no wire code: this path has no client to report to.
//!
//! It is the storage-side sibling of
//! [`VersionRef::finalize`](crate::domain::snapshot::VersionRef::finalize),
//! which refuses a re-finalize in the domain for the same reason and in the
//! same words. The two are not one mechanism and cannot be: that one moves a
//! value a caller holds, this one moves a row several sweeps can reach, and the
//! guarantee here has to be a predicate on a statement.
//!
//! # The prefix D-114 speaks of is **this tenant's**, and that is a reading
//!
//! [`next_committed_version_after`] answers "the smallest committed version of
//! this tenant strictly above the frontier". D-114 says pin-eligibility
//! requires "every **earlier version** to be itself pin-eligible", and
//! `CatalogVersion` is minted by a **cross-tenant** registry — so a tenant's
//! committed versions are a *subset* of the global sequence and are not
//! contiguous. On the global reading no tenant could ever advance past a
//! version another tenant's publish consumed, and every frontier in the
//! deployment would stick at the first gap. `pricing_pin_frontier` is keyed
//! `tenant_id` alone, which settles which reading was meant.
//!
//! **The ambiguity is reported**; what is implemented is the only buildable
//! reading — every earlier version *this tenant has a ref for*.
//!
//! # What is deliberately absent, and whose it is
//!
//! - **The overdue query.** `pricing.catalogversion.commit_overdue` measures the
//!   age of a pending ref, and [`list_pending`] carries `requested_at` so the
//!   sweep can evaluate that itself. A second query returning "the overdue
//!   ones" would put the threshold — a config value — inside a SQL predicate,
//!   where the job that owns the threshold cannot see it.
//! - **A per-plan read for the publish status API.** §3.6 says a pending ref
//!   "surfaces on the publish status API", and that surface is G7's. The
//!   repository method arrives with it; one written now would be a shape fixed
//!   before its reader.

use bss_pricing_sdk::CatalogVersion;
use chrono::{DateTime, Utc};
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, EntityTrait, Order};
use toolkit_db::secure::{
    AccessScope, DBRunner, SecureEntityExt, SecureInsertExt, SecureUpdateExt,
};
use uuid::Uuid;

use crate::domain::lifecycle::LifecycleState;
use crate::domain::read_model::{SubjectKind, SubjectRef};
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::catalog_version_ref;
use crate::infra::storage::repo::plan_repo::read_token;

/// One pending `CatalogVersion` request, as the row holds it.
///
/// The two subject columns are carried **as columns** rather than as a
/// [`SubjectRef`], because that is what a reader gets back: three of the four
/// kinds have no writer in this repository yet, and a read that could only
/// reconstruct the fourth would refuse rows a later group legitimately wrote.
/// The write side keeps the pair honest through
/// [`PendingVersionRow::for_subject`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingVersionRow {
    /// The owning tenant.
    pub tenant_id: Uuid,
    /// The registry's handle for the pending assignment.
    pub pending_ref: String,
    /// What kind of subject the publish unit projects.
    pub subject_kind: SubjectKind,
    /// Which one — a plan id for a plan publish; the shard rendering for an
    /// overlay index.
    pub subject_ref: String,
    /// The revision of the subject the publish unit **judged**, when its kind
    /// has one.
    ///
    /// The projector reads this row rather than whatever revision is current
    /// when the sweep arrives, and the difference is a real defect: the
    /// registry batches at up to five minutes (D-47), a second publish of the
    /// same plan inside that window makes its revision current, and projecting
    /// from "current" then freezes content the earlier version's publish never
    /// judged — permanently, since a delta is INSERT-only on the seven-year
    /// horizon and a completed version never mutates.
    ///
    /// `None` for a subject kind that has no revision concept. A `plan` subject
    /// without one is refused by the projector rather than defaulted, which is
    /// the fail-closed shape: a default here is a guess about which content a
    /// frozen version froze.
    pub subject_revision: Option<u64>,
    /// The lifecycle state the publish unit **judged**, frozen with the revision
    /// it judged.
    ///
    /// Read back into the delta rather than the row's state as it now stands,
    /// because that state keeps moving: `published -> superseded` at the next
    /// revision's commit, `published -> retired` at retirement. `superseded` in
    /// particular is a value D-128 does not contemplate for a projected subject
    /// and that `plan_repo::load_current` could never return, so a delta
    /// carrying it — permanently, on an INSERT-only row — reads as unsellable
    /// to a consumer coding sellability predicate (4) as "is published".
    ///
    /// `None` for a subject kind with no lifecycle; a `plan` subject without one
    /// is refused by the projector, for the reason
    /// [`PendingVersionRow::subject_revision`] gives.
    pub subject_lifecycle_state: Option<LifecycleState>,
    /// When addressability was requested, UTC.
    pub requested_at: DateTime<Utc>,
    /// The committed version, once the registry has assigned one.
    ///
    /// `None` is the pending state — the only one the publish commit can write
    /// — and the projector reads the row back to decide what to do with it,
    /// which is why the committed half is carried rather than left to a second
    /// query.
    pub catalog_version: Option<CatalogVersion>,
    /// When the version was assigned, UTC. Moves with
    /// [`PendingVersionRow::catalog_version`] and never separately —
    /// `chk_pricing_catalog_version_ref_commit` enforces the pairing
    /// physically, on both backends.
    pub committed_at: Option<DateTime<Utc>>,
}

impl PendingVersionRow {
    /// Build a row whose subject columns cannot disagree.
    ///
    /// The kind comes from [`SubjectRef::kind`] and the reference from its
    /// `Display`, so there is no call site at which one could be set without the
    /// other.
    ///
    /// The committed half is `None` by construction and there is no parameter
    /// for it: the only writer of this table is the publish commit, which holds
    /// a handle and no version, and a constructor that let a caller supply one
    /// would let a publish claim addressability the registry never granted.
    /// [`finalize`] is the only way the pair moves.
    #[must_use]
    pub fn for_subject(
        tenant_id: Uuid,
        pending_ref: String,
        subject: &SubjectRef,
        subject_revision: Option<u64>,
        subject_lifecycle_state: Option<LifecycleState>,
        requested_at: DateTime<Utc>,
    ) -> Self {
        Self {
            tenant_id,
            pending_ref,
            subject_kind: subject.kind(),
            subject_ref: subject.to_string(),
            subject_revision,
            subject_lifecycle_state,
            requested_at,
            catalog_version: None,
            committed_at: None,
        }
    }
}

/// Record a pending version request inside `runner`'s transaction.
///
/// The row lands with `catalog_version` and `committed_at` both NULL, which is
/// the only shape `chk_pricing_catalog_version_ref_commit` admits before the
/// registry has assigned anything.
///
/// # Errors
/// [`RepoError::Db`] on a scope or storage failure — which **includes** a second
/// record of one `(tenant_id, pending_ref)`, refused by the primary key. That is
/// the right refusal rather than an upsert: the registry is idempotent on
/// `request_id`, so a handle arriving twice means two publish transactions
/// believe they own the same assignment, and silently overwriting the first
/// would hand one publish's subject to the other's version.
pub async fn record_pending(
    runner: &impl DBRunner,
    scope: &AccessScope,
    entry: PendingVersionRow,
) -> Result<(), RepoError> {
    let am = catalog_version_ref::ActiveModel {
        tenant_id: Set(entry.tenant_id),
        pending_ref: Set(entry.pending_ref.clone()),
        subject_kind: Set(entry.subject_kind.as_str().to_owned()),
        subject_ref: Set(entry.subject_ref.clone()),
        subject_revision: Set(stored_revision(entry.subject_revision)?),
        subject_lifecycle_state: Set(entry
            .subject_lifecycle_state
            .map(|state| state.as_str().to_owned())),
        // Both NULL until `CatalogVersionPublished`; the CHECK ties them.
        catalog_version: Set(None),
        requested_at: Set(entry.requested_at),
        committed_at: Set(None),
    };
    catalog_version_ref::Entity::insert(am.clone())
        .secure()
        .scope_with_model(scope, &am)
        .map_err(|e| RepoError::Db(format!("pricing_catalog_version_ref scope: {e}")))?
        .exec(runner)
        .await
        .map_err(|e| RepoError::Db(format!("record pending catalog version ref: {e}")))?;
    Ok(())
}

/// Read one pending ref back by its composite identity.
///
/// SQL-level BOLA: a foreign tenant's ref yields `None`.
///
/// # Errors
/// [`RepoError::Db`] on a scope or storage failure;
/// [`RepoError::CorruptRow`] when `subject_kind` holds a token outside the
/// enumeration its `CHECK` constrains it to — which means something reached the
/// table around this gear.
pub async fn find(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    pending_ref: &str,
) -> Result<Option<PendingVersionRow>, RepoError> {
    let row = catalog_version_ref::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(catalog_version_ref::Column::TenantId.eq(tenant_id))
                .add(catalog_version_ref::Column::PendingRef.eq(pending_ref)),
        )
        .one(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read pending catalog version ref: {e}")))?;
    row.map(to_domain).transpose()
}

/// Resolve a pending handle to its committed version, inside `runner`'s
/// transaction.
///
/// A compare-and-swap on `catalog_version IS NULL`. See the module doc for why
/// the zero-rows case is re-read rather than accepted, and why one of its two
/// outcomes is an invariant breach.
///
/// # Errors
/// [`RepoError::NotFound`] when the tenant has no such handle — a version was
/// assigned to a publish this store never recorded, which the sweep can only
/// have reached by asking the registry about a ref it read from this table.
/// [`RepoError::CorruptRow`] when the row already carries a **different**
/// committed version, or when `version` exceeds the signed range the column
/// stores. [`RepoError::Db`] on a scope or storage failure.
pub async fn finalize(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    pending_ref: &str,
    version: CatalogVersion,
    committed_at: DateTime<Utc>,
) -> Result<(), RepoError> {
    let target = stored_version(version)?;
    let outcome = catalog_version_ref::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            catalog_version_ref::Column::CatalogVersion,
            Expr::value(target),
        )
        .col_expr(
            catalog_version_ref::Column::CommittedAt,
            Expr::value(committed_at),
        )
        .filter(
            Condition::all()
                .add(catalog_version_ref::Column::TenantId.eq(tenant_id))
                .add(catalog_version_ref::Column::PendingRef.eq(pending_ref))
                // The swap half: a row another sweep already finalized is not
                // matched at all, so two sweeps cannot both write a version.
                .add(catalog_version_ref::Column::CatalogVersion.is_null()),
        )
        .exec(runner)
        .await
        .map_err(|e| RepoError::Db(format!("finalize catalog version ref: {e}")))?;
    if outcome.rows_affected > 0 {
        return Ok(());
    }

    let existing = find(runner, scope, tenant_id, pending_ref)
        .await?
        .ok_or_else(|| RepoError::NotFound {
            subject: "pending catalog version ref".to_owned(),
            id: pending_ref.to_owned(),
        })?;
    match existing.catalog_version {
        Some(already) if already == version => Ok(()),
        Some(already) => Err(RepoError::CorruptRow(format!(
            "pending ref {pending_ref} of tenant {tenant_id} is committed at catalog version {}, \
             and the registry answered {} for the same handle",
            already.get(),
            version.get()
        ))),
        // The CAS matched nothing and the row is still pending: nothing in this
        // gear can produce that, so it is the store disagreeing with itself
        // rather than a race this function lost.
        None => Err(RepoError::CorruptRow(format!(
            "pending ref {pending_ref} of tenant {tenant_id} refused a finalize to catalog \
             version {} while still holding no version",
            version.get()
        ))),
    }
}

/// The oldest `limit` refs still awaiting a version, **across tenants**.
///
/// Cross-tenant by design: the sweep runs under the sanctioned
/// [`AccessScope::allow_all`] system scope and narrows to
/// `AccessScope::for_tenant` before any per-tenant write, the pattern the
/// sibling ledger's jobs document. Oldest first so a backlog drains in the
/// order it accumulated, and bounded so one pass cannot read an unbounded
/// backlog into memory.
///
/// `requested_at` rides on the row because it is what the
/// `pricing.catalogversion.commit_overdue` alarm measures, and the threshold
/// belongs to the job rather than to a SQL predicate here.
///
/// # Errors
/// [`RepoError::Db`] on a scope or storage failure;
/// [`RepoError::CorruptRow`] when a stored row cannot be read as the value its
/// columns are `CHECK`-constrained to hold.
pub async fn list_pending(
    runner: &impl DBRunner,
    scope: &AccessScope,
    limit: u64,
) -> Result<Vec<PendingVersionRow>, RepoError> {
    let rows = catalog_version_ref::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(catalog_version_ref::Column::CatalogVersion.is_null()))
        .order_by(catalog_version_ref::Column::RequestedAt, Order::Asc)
        .order_by(catalog_version_ref::Column::PendingRef, Order::Asc)
        .limit(limit)
        .all(runner)
        .await
        .map_err(|e| RepoError::Db(format!("list pending catalog version refs: {e}")))?;
    rows.into_iter().map(to_domain).collect()
}

/// Every ref of one committed version — the subject set that version must
/// project.
///
/// This is the projector's input, and D-157 is why it can exist at all: the
/// subject columns on this row are the only durable path from a committed
/// handle back to what it published. The result is legitimately a **set**,
/// which is the batched case the version index stopped being unique for.
///
/// # Errors
/// [`RepoError::Db`] on a scope or storage failure;
/// [`RepoError::CorruptRow`] when a stored row cannot be read as the value its
/// columns are `CHECK`-constrained to hold, or when `catalog_version` exceeds
/// the signed range the column stores.
pub async fn list_at_version(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    catalog_version: CatalogVersion,
) -> Result<Vec<PendingVersionRow>, RepoError> {
    let target = stored_version(catalog_version)?;
    let rows = catalog_version_ref::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(catalog_version_ref::Column::TenantId.eq(tenant_id))
                .add(catalog_version_ref::Column::CatalogVersion.eq(target)),
        )
        .order_by(catalog_version_ref::Column::PendingRef, Order::Asc)
        .all(runner)
        .await
        .map_err(|e| RepoError::Db(format!("list catalog version refs at a version: {e}")))?;
    rows.into_iter().map(to_domain).collect()
}

/// The tenant's smallest committed version strictly above `frontier` — or its
/// smallest committed version at all, when the tenant has no frontier yet.
///
/// This is what "the frontier's **next** version in order" means for a store
/// whose version numbers come from a cross-tenant registry; the module doc
/// carries the argument and reports the ambiguity.
///
/// # Errors
/// [`RepoError::Db`] on a scope or storage failure;
/// [`RepoError::CorruptRow`] when `frontier` or a stored version lies outside
/// the range its column holds.
pub async fn next_committed_version_after(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    frontier: Option<CatalogVersion>,
) -> Result<Option<CatalogVersion>, RepoError> {
    let mut filter = Condition::all()
        .add(catalog_version_ref::Column::TenantId.eq(tenant_id))
        .add(catalog_version_ref::Column::CatalogVersion.is_not_null());
    if let Some(frontier) = frontier {
        filter =
            filter.add(catalog_version_ref::Column::CatalogVersion.gt(stored_version(frontier)?));
    }
    let row = catalog_version_ref::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(filter)
        .order_by(catalog_version_ref::Column::CatalogVersion, Order::Asc)
        .limit(1)
        .one(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read next committed catalog version: {e}")))?;
    let Some(row) = row else {
        return Ok(None);
    };
    read_version(&row)
}

/// Map a stored row into the value the rest of the system reasons about.
fn to_domain(row: catalog_version_ref::Model) -> Result<PendingVersionRow, RepoError> {
    let subject_kind = read_token(
        "pricing_catalog_version_ref.subject_kind",
        &row.subject_kind,
        SubjectKind::ALL,
        SubjectKind::as_str,
    )?;
    let catalog_version = read_version(&row)?;
    let subject_revision = read_revision(&row)?;
    let subject_lifecycle_state = row
        .subject_lifecycle_state
        .as_deref()
        .map(|token| {
            read_token(
                "pricing_catalog_version_ref.subject_lifecycle_state",
                token,
                LifecycleState::ALL,
                LifecycleState::as_str,
            )
        })
        .transpose()?;
    Ok(PendingVersionRow {
        tenant_id: row.tenant_id,
        pending_ref: row.pending_ref,
        subject_kind,
        subject_ref: row.subject_ref,
        subject_revision,
        subject_lifecycle_state,
        requested_at: row.requested_at,
        catalog_version,
        committed_at: row.committed_at,
    })
}

/// The row's judged revision, in the domain's unsigned vocabulary.
fn read_revision(row: &catalog_version_ref::Model) -> Result<Option<u64>, RepoError> {
    row.subject_revision
        .map(|stored| {
            u64::try_from(stored).map_err(|e| {
                RepoError::CorruptRow(format!(
                    "pending ref {} of tenant {} holds subject_revision {stored}: {e}",
                    row.pending_ref, row.tenant_id
                ))
            })
        })
        .transpose()
}

/// A revision as its `bigint` column holds it.
fn stored_revision(revision: Option<u64>) -> Result<Option<i64>, RepoError> {
    revision
        .map(|value| {
            i64::try_from(value).map_err(|e| RepoError::ValueOutOfRange {
                field: "subjectRevision".to_owned(),
                value: format!("{value}: {e}"),
            })
        })
        .transpose()
}

/// The row's committed version, in the SDK's unsigned vocabulary.
///
/// A stored value outside that range is [`RepoError::CorruptRow`] rather than a
/// silent `None`: the column is `CHECK (catalog_version IS NULL OR
/// catalog_version >= 0)`, so a negative one means something reached the table
/// around this gear — and a committed row read back as pending would be
/// re-finalized at whatever the registry says next.
fn read_version(row: &catalog_version_ref::Model) -> Result<Option<CatalogVersion>, RepoError> {
    row.catalog_version
        .map(|stored| {
            u64::try_from(stored).map(CatalogVersion::new).map_err(|e| {
                RepoError::CorruptRow(format!(
                    "pending ref {} of tenant {} holds catalog_version {stored}: {e}",
                    row.pending_ref, row.tenant_id
                ))
            })
        })
        .transpose()
}

/// A version as its `bigint` column holds it.
fn stored_version(version: CatalogVersion) -> Result<i64, RepoError> {
    i64::try_from(version.get()).map_err(|e| {
        RepoError::CorruptRow(format!(
            "catalog version {} exceeds the storable range: {e}",
            version.get()
        ))
    })
}
