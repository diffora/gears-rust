//! The retention sweep's store layer: the candidate reads per record class,
//! the deletes the storage admits, and the tenant discovery all three loop
//! calls share (`dod-retention-clock`, `dod-retention-order`; **P-D-118**
//! items 25–27, **P-D-136**).
//!
//! # Every delete here can be refused, and that is the design
//!
//! **P-D-136**: the flat-refusal class keeps its guard and the GC **holds**
//! what it cannot delete. So a `RepoError::Driver` out of a delete below is
//! not an incident — it is the expected steady state for the evidential
//! stores, and `infra::retention`'s sweep classifies it as
//! `HeldReason::StorageRefused` carrying the engine's own message. The
//! message is passed through rather than paraphrased, because the migration
//! names its guard in it and a paraphrase would drift from the migration on
//! the day one of them changes.
//!
//! Three of the four target tables refuse every `DELETE` at this commit, and
//! the sweep is written knowing it: `products_catalog_version`
//! (`m20260901_000010`, unconditional), `products_catalog_version_entry` and
//! `_capture` (`m20260901_000013`, unconditional but with an interim message
//! naming *"slice 10's manifest retention"* as its future admitter), and the
//! five evidence tables P-D-136 keeps that way. The one opened predicate is
//! `products_entity_version`'s (`m20260829_000007`, **P-D-40**): a row is
//! deletable exactly when no manifest entry references it — which is also
//! `dod-retention-order`'s derive rule enforced by the engine rather than by
//! this module's ordering.
//!
//! # Candidate reads are ids, never rows
//!
//! A pass judges each candidate in its own transaction (P-D-136), so the
//! discovery read hands back keys and the per-candidate work re-reads what it
//! needs. Fetching whole rows would put a decade of retained records in
//! memory to decide that none of them may be deleted.

use chrono::{DateTime, Utc};
use sea_orm::{ColumnTrait, Condition, EntityTrait, FromQueryResult, QuerySelect};
use toolkit_db::secure::{AccessScope, DBRunner, SecureDeleteExt, SecureEntityExt};
use uuid::Uuid;

use super::driver_failure;
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::{
    approval, approval_decision, audit_log, breakglass_session, catalog_version,
    catalog_version_capture, catalog_version_entry, correction_override, entity_version,
    identity_ref,
};

/// One `tenant_id`, projected.
#[derive(Debug, FromQueryResult)]
struct TenantRow {
    tenant_id: Uuid,
}

/// One catalog-version candidate.
#[derive(Debug, FromQueryResult)]
struct VersionIdRow {
    catalog_version_id: i64,
}

/// One entity-version candidate's key.
#[derive(Clone, Debug, PartialEq, Eq, FromQueryResult)]
pub struct EntityVersionKey {
    pub entity_kind: String,
    pub entity_id: Uuid,
    pub published_version: i64,
}

/// One audit-class candidate: the table it lives in and its key, rendered.
///
/// The five audit-class tables have five different primary keys, so a single
/// typed key would need a five-arm enum whose only consumer renders it into a
/// string for the audit row anyway. The class is what the sweep reports and
/// the id is what an operator would chase.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditClassCandidate {
    /// Which store, as the audit row names it.
    pub store: &'static str,
    /// The row's own id, rendered.
    pub id: String,
}

/// Every tenant with at least one row any class's clock could reach.
///
/// A `DISTINCT` projection over the audit log alone: every tenant that has
/// ever done anything in this gear has audit rows, and a sweep that
/// discovered tenants per class would run five discovery reads to find the
/// same set. The audit log is the one table no live tenant is absent from.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn tenants_with_retention_history(
    runner: &impl DBRunner,
    scope: &AccessScope,
) -> Result<Vec<Uuid>, RepoError> {
    let rows: Vec<TenantRow> = audit_log::Entity::find()
        .secure()
        .scope_with(scope)
        .project_all(runner, |q| {
            q.select_only()
                .column(audit_log::Column::TenantId)
                .distinct()
                .into_model::<TenantRow>()
        })
        .await
        .map_err(|e| driver_failure("discover tenants for the retention sweep".to_owned(), e))?;
    Ok(rows.into_iter().map(|row| row.tenant_id).collect())
}

/// Catalog versions published before `cutoff`, oldest first.
///
/// Oldest first because the ordering is the one an operator would expect of a
/// retention pass and because a resumed pass then re-judges the same
/// candidates in the same order — P-D-118 item 25's *"re-judges every
/// candidate from scratch"* is easier to reason about when the order is
/// stable.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn catalog_version_candidates(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    cutoff: DateTime<Utc>,
) -> Result<Vec<i64>, RepoError> {
    let rows: Vec<VersionIdRow> = catalog_version::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(catalog_version::Column::TenantId.eq(tenant_id))
                .add(catalog_version::Column::PublishedAt.lt(cutoff)),
        )
        .order_by(
            catalog_version::Column::CatalogVersionId,
            sea_orm::Order::Asc,
        )
        .project_all(runner, |q| {
            q.select_only()
                .column(catalog_version::Column::CatalogVersionId)
                .into_model::<VersionIdRow>()
        })
        .await
        .map_err(|e| {
            driver_failure(format!("read catalog-version candidates of {tenant_id}"), e)
        })?;
    Ok(rows.into_iter().map(|row| row.catalog_version_id).collect())
}

/// Entity versions published before `cutoff`, oldest first.
///
/// **Unfiltered by manifest reference on purpose.** The derive rule
/// (`dod-retention-order`: version-row retention is never shorter than the
/// catalog version's) is enforced by `m20260829_000007`'s own predicate, and
/// pre-filtering here would make the sweep's own ordering the guarantee — the
/// thing §6's criterion refuses: *"refused **by the guard**, not merely
/// skipped by the GC — the probe passes even when the GC is bypassed
/// entirely"*. So the sweep offers each candidate to the engine and reports
/// the refusal.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn entity_version_candidates(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    cutoff: DateTime<Utc>,
) -> Result<Vec<EntityVersionKey>, RepoError> {
    let rows: Vec<EntityVersionKey> = entity_version::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(entity_version::Column::TenantId.eq(tenant_id))
                .add(entity_version::Column::PublishedAt.lt(cutoff)),
        )
        .order_by(entity_version::Column::PublishedAt, sea_orm::Order::Asc)
        .project_all(runner, |q| {
            q.select_only()
                .column(entity_version::Column::EntityKind)
                .column(entity_version::Column::EntityId)
                .column(entity_version::Column::PublishedVersion)
                .into_model::<EntityVersionKey>()
        })
        .await
        .map_err(|e| driver_failure(format!("read entity-version candidates of {tenant_id}"), e))?;
    Ok(rows)
}

/// The audit class's candidates, across all five of its stores.
///
/// One function rather than five, because the sweep treats them as one class
/// with one window and the only per-store difference is which column carries
/// the clock: `written_at`, `submitted_at`, `decided_at`, `opened_at`,
/// `recorded_at`. Naming them here keeps that mapping in one place instead of
/// in five call sites.
///
/// `bound` caps how many of each store's rows are returned. A retention sweep
/// over a decade of audit rows would otherwise read every one of them to
/// decide that none may be deleted.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn audit_class_candidates(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    cutoff: DateTime<Utc>,
    bound: u64,
) -> Result<Vec<AuditClassCandidate>, RepoError> {
    let mut out = Vec::new();

    let audits = audit_log::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(audit_log::Column::TenantId.eq(tenant_id))
                .add(audit_log::Column::WrittenAt.lt(cutoff)),
        )
        .order_by(audit_log::Column::WrittenAt, sea_orm::Order::Asc)
        .limit(bound)
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("read audit candidates of {tenant_id}"), e))?;
    out.extend(audits.into_iter().map(|row| AuditClassCandidate {
        store: "products_audit_log",
        id: row.audit_id.to_string(),
    }));

    let approvals = approval::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(approval::Column::TenantId.eq(tenant_id))
                .add(approval::Column::SubmittedAt.lt(cutoff)),
        )
        .order_by(approval::Column::SubmittedAt, sea_orm::Order::Asc)
        .limit(bound)
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("read approval candidates of {tenant_id}"), e))?;
    out.extend(approvals.into_iter().map(|row| AuditClassCandidate {
        store: "products_approval",
        id: row.approval_id.to_string(),
    }));

    let decisions = approval_decision::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(approval_decision::Column::TenantId.eq(tenant_id))
                .add(approval_decision::Column::DecidedAt.lt(cutoff)),
        )
        .order_by(approval_decision::Column::DecidedAt, sea_orm::Order::Asc)
        .limit(bound)
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("read decision candidates of {tenant_id}"), e))?;
    out.extend(decisions.into_iter().map(|row| AuditClassCandidate {
        store: "products_approval_decision",
        id: format!("{}/{}", row.approval_id, row.approver_principal),
    }));

    // Keyed on `target_tenant`, not `tenant_id`: a break-glass session is
    // opened by a platform principal against a tenant, and the tenant whose
    // records it reaches is the one whose sweep must account for it.
    let sessions = breakglass_session::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(breakglass_session::Column::TargetTenant.eq(tenant_id))
                .add(breakglass_session::Column::OpenedAt.lt(cutoff)),
        )
        .order_by(breakglass_session::Column::OpenedAt, sea_orm::Order::Asc)
        .limit(bound)
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("read break-glass candidates of {tenant_id}"), e))?;
    out.extend(sessions.into_iter().map(|row| AuditClassCandidate {
        store: "products_breakglass_session",
        id: row.session_id.to_string(),
    }));

    let overrides = correction_override::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(correction_override::Column::TenantId.eq(tenant_id))
                .add(correction_override::Column::RecordedAt.lt(cutoff)),
        )
        .order_by(correction_override::Column::RecordedAt, sea_orm::Order::Asc)
        .limit(bound)
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("read override candidates of {tenant_id}"), e))?;
    out.extend(overrides.into_iter().map(|row| AuditClassCandidate {
        store: "products_correction_override",
        id: row.override_id.to_string(),
    }));

    Ok(out)
}

/// Delete one catalog version whole, in the caller's transaction.
///
/// **P-D-118 item 25's boundary**: captures and entry rows first, then the
/// manifest row — so the intermediate state the item describes, a surviving
/// manifest whose entries are gone, cannot exist for a backup to capture.
/// The entity-version rows the manifest referenced are **not** deleted here:
/// they are the version class's own candidates and reach the engine's
/// referential predicate on their own pass, which is the only place the
/// derive rule is enforced rather than merely ordered.
///
/// # `runner` MUST be one transaction
///
/// The whole point of the boundary is that the three statements commit
/// together. A caller handing a plain connection gets the intermediate state
/// the decision exists to forbid.
///
/// # Errors
///
/// [`RepoError::Driver`] on a storage refusal — which is the expected answer
/// at this commit, since both migrations in this chain refuse every `DELETE`.
pub async fn delete_catalog_version(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    catalog_version_id: i64,
) -> Result<(), RepoError> {
    catalog_version_capture::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(catalog_version_capture::Column::TenantId.eq(tenant_id))
                .add(catalog_version_capture::Column::CatalogVersionId.eq(catalog_version_id)),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("delete captures of {catalog_version_id}"), e))?;

    catalog_version_entry::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(catalog_version_entry::Column::TenantId.eq(tenant_id))
                .add(catalog_version_entry::Column::CatalogVersionId.eq(catalog_version_id)),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("delete entries of {catalog_version_id}"), e))?;

    catalog_version::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(catalog_version::Column::TenantId.eq(tenant_id))
                .add(catalog_version::Column::CatalogVersionId.eq(catalog_version_id)),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("delete catalog version {catalog_version_id}"), e))?;
    Ok(())
}

/// Delete one entity-version row, in the caller's transaction.
///
/// Offered to the engine unconditionally: `m20260829_000007`'s referential
/// predicate is what admits or refuses it, and asking it is how the sweep
/// learns which. A pre-check here would move the guarantee from the engine to
/// this module, which §6's criterion refuses by name.
///
/// # Errors
///
/// [`RepoError::Driver`] when a manifest entry still references the row.
pub async fn delete_entity_version(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    key: &EntityVersionKey,
) -> Result<(), RepoError> {
    entity_version::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(entity_version::Column::TenantId.eq(tenant_id))
                .add(entity_version::Column::EntityKind.eq(key.entity_kind.as_str()))
                .add(entity_version::Column::EntityId.eq(key.entity_id))
                .add(entity_version::Column::PublishedVersion.eq(key.published_version)),
        )
        .exec(runner)
        .await
        .map_err(|e| {
            driver_failure(
                format!(
                    "delete entity version {}/{}/{}",
                    key.entity_kind, key.entity_id, key.published_version
                ),
                e,
            )
        })?;
    Ok(())
}

/// Delete one audit-class row, in the caller's transaction.
///
/// Every one of these is refused at this commit (**P-D-136**: evidence rows
/// are not deletable in v1). The statement is still issued, **once per class
/// per pass**, because the hold has to be **measured** rather than declared:
/// a roster of "tables we believe refuse" is exactly the kind of claim that
/// goes stale the day a migration opens an arm, and the sweep would then keep
/// holding rows the storage would have collected.
///
/// # Errors
///
/// [`RepoError::Driver`] on the guard's refusal, carrying its message.
pub async fn delete_audit_class_row(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    candidate: &AuditClassCandidate,
) -> Result<(), RepoError> {
    let store = candidate.store;
    let id = candidate.id.as_str();
    match store {
        "products_audit_log" => {
            let audit_id = Uuid::parse_str(id).map_err(|e| {
                RepoError::CorruptRow(format!("audit candidate {id} is not a uuid: {e}"))
            })?;
            audit_log::Entity::delete_many()
                .secure()
                .scope_with(scope)
                .filter(
                    Condition::all()
                        .add(audit_log::Column::TenantId.eq(tenant_id))
                        .add(audit_log::Column::AuditId.eq(audit_id)),
                )
                .exec(runner)
                .await
                .map_err(|e| driver_failure(format!("delete audit row {id}"), e))?;
        }
        "products_approval" => {
            let approval_id = Uuid::parse_str(id).map_err(|e| {
                RepoError::CorruptRow(format!("approval candidate {id} is not a uuid: {e}"))
            })?;
            approval::Entity::delete_many()
                .secure()
                .scope_with(scope)
                .filter(
                    Condition::all()
                        .add(approval::Column::TenantId.eq(tenant_id))
                        .add(approval::Column::ApprovalId.eq(approval_id)),
                )
                .exec(runner)
                .await
                .map_err(|e| driver_failure(format!("delete approval {id}"), e))?;
        }
        "products_approval_decision" => {
            let (approval_id, approver) = id.split_once('/').ok_or_else(|| {
                RepoError::CorruptRow(format!(
                    "decision candidate {id} is not `approval/approver`"
                ))
            })?;
            let approval_id = Uuid::parse_str(approval_id).map_err(|e| {
                RepoError::CorruptRow(format!("decision candidate {id} is not a uuid pair: {e}"))
            })?;
            let approver = Uuid::parse_str(approver).map_err(|e| {
                RepoError::CorruptRow(format!("decision candidate {id} is not a uuid pair: {e}"))
            })?;
            approval_decision::Entity::delete_many()
                .secure()
                .scope_with(scope)
                .filter(
                    Condition::all()
                        .add(approval_decision::Column::TenantId.eq(tenant_id))
                        .add(approval_decision::Column::ApprovalId.eq(approval_id))
                        .add(approval_decision::Column::ApproverPrincipal.eq(approver)),
                )
                .exec(runner)
                .await
                .map_err(|e| driver_failure(format!("delete decision {id}"), e))?;
        }
        "products_breakglass_session" => {
            let session_id = Uuid::parse_str(id).map_err(|e| {
                RepoError::CorruptRow(format!("session candidate {id} is not a uuid: {e}"))
            })?;
            breakglass_session::Entity::delete_many()
                .secure()
                .scope_with(scope)
                .filter(
                    Condition::all()
                        .add(breakglass_session::Column::TargetTenant.eq(tenant_id))
                        .add(breakglass_session::Column::SessionId.eq(session_id)),
                )
                .exec(runner)
                .await
                .map_err(|e| driver_failure(format!("delete break-glass session {id}"), e))?;
        }
        "products_correction_override" => {
            let override_id = Uuid::parse_str(id).map_err(|e| {
                RepoError::CorruptRow(format!("override candidate {id} is not a uuid: {e}"))
            })?;
            correction_override::Entity::delete_many()
                .secure()
                .scope_with(scope)
                .filter(
                    Condition::all()
                        .add(correction_override::Column::TenantId.eq(tenant_id))
                        .add(correction_override::Column::OverrideId.eq(override_id)),
                )
                .exec(runner)
                .await
                .map_err(|e| driver_failure(format!("delete override {id}"), e))?;
        }
        // Unreachable behind `audit_class_candidates`, which is the only
        // producer of these values; kept so a sixth store added to that
        // function without an arm here is a refusal rather than a silent
        // "collected" on a row nothing deleted.
        other => {
            return Err(RepoError::CorruptRow(format!(
                "{other} is not one of the audit class's five stores"
            )));
        }
    }
    Ok(())
}

/// Principals whose last stamped activity is older than `cutoff`.
///
/// `dod-erasure-age`'s operand, and it is `last_seen_at` rather than
/// `first_seen_at` for M2's reason: age since first appearance would
/// tombstone an active employee mid-employment. Tombstoned rows are excluded
/// — a retired ref has nothing left to erase, and re-tombstoning one would
/// move a column the entity's doc says is *"set once, by erasure, and never
/// cleared"*.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn principals_older_than(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    cutoff: DateTime<Utc>,
    bound: u64,
) -> Result<Vec<String>, RepoError> {
    let rows = identity_ref::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(identity_ref::Column::TenantId.eq(tenant_id))
                .add(identity_ref::Column::TombstonedAt.is_null())
                .add(identity_ref::Column::LastSeenAt.lt(cutoff)),
        )
        .order_by(identity_ref::Column::LastSeenAt, sea_orm::Order::Asc)
        .limit(bound)
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("read aged principals of {tenant_id}"), e))?;
    Ok(rows.into_iter().map(|row| row.principal_ref).collect())
}

/// One entity-version row's digest operands, for the restore drill.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionDigest {
    /// The canonical rendering the digest covers.
    pub content: String,
    /// The stored digest.
    pub content_digest: Vec<u8>,
    /// The rule it was computed under.
    pub digest_version: i32,
}

/// The newest `bound` catalog versions of one tenant, newest first.
///
/// The drill's sample (`dod-restore-drill`): newest rather than random,
/// because a corrupt backup is found by reading the restore an incident would
/// actually restore from, and because a deterministic sample makes two
/// consecutive runs comparable.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn newest_catalog_versions(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    bound: u64,
) -> Result<Vec<i64>, RepoError> {
    let rows: Vec<VersionIdRow> = catalog_version::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(catalog_version::Column::TenantId.eq(tenant_id)))
        .order_by(
            catalog_version::Column::CatalogVersionId,
            sea_orm::Order::Desc,
        )
        .limit(bound)
        .project_all(runner, |q| {
            q.select_only()
                .column(catalog_version::Column::CatalogVersionId)
                .into_model::<VersionIdRow>()
        })
        .await
        .map_err(|e| driver_failure(format!("read the drill sample of {tenant_id}"), e))?;
    Ok(rows.into_iter().map(|row| row.catalog_version_id).collect())
}

/// One entity-version row's digest operands.
///
/// `None` when the row is absent, which for a manifest entry is corruption
/// rather than a miss — the caller is what knows that.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn entity_version_digest(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    entity_kind: &str,
    entity_id: Uuid,
    published_version: i64,
) -> Result<Option<VersionDigest>, RepoError> {
    let row = entity_version::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(entity_version::Column::TenantId.eq(tenant_id))
                .add(entity_version::Column::EntityKind.eq(entity_kind))
                .add(entity_version::Column::EntityId.eq(entity_id))
                .add(entity_version::Column::PublishedVersion.eq(published_version)),
        )
        .one(runner)
        .await
        .map_err(|e| {
            driver_failure(
                format!("read the digest of {entity_kind}/{entity_id}/{published_version}"),
                e,
            )
        })?;
    Ok(row.map(|row| VersionDigest {
        content: row.content,
        content_digest: row.content_digest,
        digest_version: row.digest_version,
    }))
}
