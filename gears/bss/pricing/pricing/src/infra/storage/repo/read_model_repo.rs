//! The writer of `pricing_read_model` — one per-subject delta of one
//! `CatalogVersion`, and the warm set a re-drive subtracts against.
//!
//! Runner-taking free functions rather than a provider-holding struct, and the
//! reason is the sharpest form of [`audit_repo`](super::audit_repo)'s and
//! [`outbox_repo`](super::outbox_repo)'s: **D-136 requires the frontier's
//! advance to happen in the transaction that sets the last outstanding
//! `warm_completed` marker of the frontier's next version in order.** Every
//! write on this path is therefore somebody else's transaction's, and a
//! repository that opened a connection of its own could not participate in it
//! — inside an open transaction `Db::conn()` is refused outright by the
//! toolkit's transaction-bypass guard.
//!
//! # The row is written **warm**, and that is not a shortcut
//!
//! §4.4's marker is per **row** and discriminates a subject that is resolvable
//! from one that is not. In this gear projection and warming are **one act**:
//! the payload is materialized from the truth rows in the same transaction that
//! writes it, so there is no interval in which a delta row exists and is not
//! complete. What the marker therefore discriminates here is *projected* from
//! *not yet projected* — and it stays per row rather than per version because a
//! version's subjects are projected independently (D-91), so a version's
//! completeness is the question "is every one of its ref rows' subjects warm",
//! which is what [`warm_subjects_at`] answers.
//!
//! `chk_pricing_read_model_warm_marker` already makes the marker and its
//! instant move together, so a half-warm row is not expressible on either
//! backend.
//!
//! # An INSERT, never an upsert
//!
//! A completed version never mutates. So a second projection of one
//! `(version, subject)` is either a duplicated sweep or a re-drive that failed
//! to skip an already-warm subject, and in both cases the primary key refusing
//! it is the guard — an upsert would silently rewrite a frozen version's
//! content, which is the one thing a pinned consumer is entitled to assume
//! cannot happen. The refusal arrives as [`RepoError::Db`] rather than a typed
//! variant because no caller can provoke it: the projector is a background
//! sweep with no client, so a refusal here has nobody to report to and no wire
//! code the design set names (D-146's line about the pin frontier, one table
//! over).
//!
//! # What is deliberately absent, and whose it is
//!
//! **The resolution query.** §4.4's own rule — "resolving `(pin, subject)`
//! reads the subject's row with the greatest `catalog_version <= V` whose
//! `warm_completed` is set" — is **not** built here.
//! `idx_pricing_read_model_resolve` exists for exactly that read and is covered
//! by no query anywhere in this crate.
//!
//! It is absent for the crate's standing reason rather than by oversight: there
//! is no read surface to call it. `bss_pricing_sdk::api` says so in as many
//! words — read-model resolution "arrives with the slices that own those
//! payloads; the Foundation-owned entry point is the frontier a consumer pins
//! **before** resolving anything" — and the payloads such a query would return
//! are Slice 6's and Slice 7's, two of whose facts this gear cannot yet
//! project at all ([`crate::domain::projection`] names them one by one). A
//! method shaped before its reader is a shape nobody agreed to, and a
//! repository method with no call site is dead code, which this crate's
//! repository rule already forbids. **Owed**, with the index that is waiting
//! for it.

use bss_pricing_sdk::CatalogVersion;
use chrono::{DateTime, Utc};
use sea_orm::ActiveValue::Set;
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use serde_json::Value as JsonValue;
use toolkit_db::secure::{AccessScope, DBRunner, SecureEntityExt, SecureInsertExt};
use uuid::Uuid;

use crate::domain::read_model::{SubjectKind, SubjectRef};
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::read_model;
use crate::infra::storage::repo::plan_repo::read_token;

/// One projected delta, as its writer is handed it.
///
/// The subject arrives as a [`SubjectRef`] rather than as a kind and a string,
/// so a writer cannot produce a pair that disagrees — the same property
/// [`PendingVersionRow::for_subject`](super::catalog_version_ref_repo::PendingVersionRow::for_subject)
/// carries one table over, and the two tables are keyed alike on purpose (the
/// ref names the subject the projector will write).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewDelta {
    /// The owning tenant.
    pub tenant_id: Uuid,
    /// The committed version this delta belongs to.
    pub catalog_version: CatalogVersion,
    /// What the delta is about.
    pub subject: SubjectRef,
    /// The frozen payload, as [`crate::domain::projection`] renders it.
    pub payload: JsonValue,
    /// When the subject was projected — and, because the two are one act here,
    /// when it became warm.
    pub projected_at: DateTime<Utc>,
}

/// Write one subject's delta inside `runner`'s transaction, warm.
///
/// # Errors
/// [`RepoError::Db`] on a scope or storage failure — which **includes** a
/// second delta on one `(tenant, version, subject)`, refused by the primary
/// key. That is the guard, not an inconvenience; see the module doc.
/// [`RepoError::CorruptRow`] when the version exceeds the signed range the
/// column stores.
pub async fn project_subject(
    runner: &impl DBRunner,
    scope: &AccessScope,
    delta: NewDelta,
) -> Result<(), RepoError> {
    let stored_version = stored_version(delta.catalog_version)?;
    let am = read_model::ActiveModel {
        tenant_id: Set(delta.tenant_id),
        catalog_version: Set(stored_version),
        subject_kind: Set(delta.subject.kind().as_str().to_owned()),
        subject_ref: Set(delta.subject.to_string()),
        // The marker and its instant in the same statement: projection and
        // warming are one act here, and the CHECK ties the pair anyway.
        warm_completed: Set(true),
        warm_completed_at: Set(Some(delta.projected_at)),
        payload: Set(delta.payload.clone()),
        projected_at: Set(delta.projected_at),
    };
    read_model::Entity::insert(am.clone())
        .secure()
        .scope_with_model(scope, &am)
        .map_err(|e| RepoError::Db(format!("pricing_read_model scope: {e}")))?
        .exec(runner)
        .await
        .map_err(|e| RepoError::Db(format!("project read model subject: {e}")))?;
    Ok(())
}

/// The `(subject_kind, subject_ref)` pairs already warm at `catalog_version`.
///
/// Two callers, both in the projector: the re-drive subtracts this from the
/// version's ref rows to get exactly what failed last time, and the
/// completeness check compares the two sets to decide whether the version is
/// complete. Returned as the stored pair rather than as a [`SubjectRef`]
/// because that is what both callers compare against — the ref row's own
/// columns — and because three of the four kinds have no writer in this gear,
/// so a read that could only reconstruct the fourth would refuse rows a later
/// group legitimately wrote.
///
/// The `warm_completed` filter is redundant today and is kept deliberately: a
/// row is written warm, but the predicate the callers depend on is "warm at
/// this version", not "present at this version", and a filter dropped because
/// it is currently vacuous is a filter absent the day a slice writes a row that
/// is not.
///
/// # Errors
/// [`RepoError::Db`] on a scope or storage failure;
/// [`RepoError::CorruptRow`] when the version exceeds the signed range the
/// column stores, or when a stored `subject_kind` lies outside the enumeration
/// its `CHECK` constrains it to.
pub async fn warm_subjects_at(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    catalog_version: CatalogVersion,
) -> Result<Vec<(SubjectKind, String)>, RepoError> {
    let stored_version = stored_version(catalog_version)?;
    let rows = read_model::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(read_model::Column::TenantId.eq(tenant_id))
                .add(read_model::Column::CatalogVersion.eq(stored_version))
                .add(read_model::Column::WarmCompleted.eq(true)),
        )
        .all(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read warm read-model subjects: {e}")))?;

    rows.into_iter()
        .map(|row| {
            let kind = read_token(
                "pricing_read_model.subject_kind",
                &row.subject_kind,
                SubjectKind::ALL,
                SubjectKind::as_str,
            )?;
            Ok((kind, row.subject_ref))
        })
        .collect()
}

/// The version as its `bigint` column holds it.
///
/// A value outside the signed range is [`RepoError::CorruptRow`] rather than a
/// caller mistake: `CatalogVersion` is minted by the registry, so a version
/// this large means the sequence itself has left the range every column in this
/// gear stores it in, and no caller can reshape the request.
fn stored_version(version: CatalogVersion) -> Result<i64, RepoError> {
    i64::try_from(version.get()).map_err(|e| {
        RepoError::CorruptRow(format!(
            "catalog version {} exceeds the storable range: {e}",
            version.get()
        ))
    })
}
