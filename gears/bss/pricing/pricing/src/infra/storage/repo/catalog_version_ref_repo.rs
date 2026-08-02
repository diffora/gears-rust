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
//! # What is deliberately absent, and whose it is
//!
//! - **The finalize** (`catalog_version` + `committed_at`). It is G6's, it runs
//!   at `CatalogVersionPublished`, and it has no caller in this group.
//!   `chk_pricing_catalog_version_ref_commit` already makes the two columns move
//!   together, so G6 cannot half-finalize a row this module wrote.
//! - **The pending list and the overdue query.** They belong to the projector
//!   and to the `pricing.catalogversion.commit_overdue` alarm respectively.
//!
//! All three would be methods with no call site, and a repository method
//! nothing calls is dead code with a shape fixed before its reader exists.

use chrono::{DateTime, Utc};
use sea_orm::ActiveValue::Set;
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use toolkit_db::secure::{AccessScope, DBRunner, SecureEntityExt, SecureInsertExt};
use uuid::Uuid;

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
    /// When addressability was requested, UTC.
    pub requested_at: DateTime<Utc>,
}

impl PendingVersionRow {
    /// Build a row whose subject columns cannot disagree.
    ///
    /// The kind comes from [`SubjectRef::kind`] and the reference from its
    /// `Display`, so there is no call site at which one could be set without the
    /// other.
    #[must_use]
    pub fn for_subject(
        tenant_id: Uuid,
        pending_ref: String,
        subject: &SubjectRef,
        requested_at: DateTime<Utc>,
    ) -> Self {
        Self {
            tenant_id,
            pending_ref,
            subject_kind: subject.kind(),
            subject_ref: subject.to_string(),
            requested_at,
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

/// Map a stored row into the value the rest of the system reasons about.
fn to_domain(row: catalog_version_ref::Model) -> Result<PendingVersionRow, RepoError> {
    Ok(PendingVersionRow {
        tenant_id: row.tenant_id,
        pending_ref: row.pending_ref,
        subject_kind: read_token(
            "pricing_catalog_version_ref.subject_kind",
            &row.subject_kind,
            SubjectKind::ALL,
            SubjectKind::as_str,
        )?,
        subject_ref: row.subject_ref,
        requested_at: row.requested_at,
    })
}
