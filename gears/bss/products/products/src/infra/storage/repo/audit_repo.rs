//! @cpt-dod:cpt-cf-bss-products-dod-audit-append-only:p1
//! Audit row construction, on the caller's runner (P-D-193, P-D-200).
use super::driver_failure;
use crate::infra::storage::{RepoError, entity::audit_log};
use sea_orm::{EntityTrait, Set};
use time::OffsetDateTime;
use toolkit_db::secure::{AccessScope, DBRunner, SecureInsertExt};
use uuid::Uuid;
/// The fields every audit row carries beside its subject.
#[derive(Clone, Debug)]
pub struct AuditCommon {
    /// Server-minted by the caller, never re-derived here.
    pub audit_id: Uuid,
    /// Owning tenant.
    pub tenant_id: Uuid,
    /// The acting `SecurityContext` subject id.
    pub actor_ref: Uuid,
    /// The audit action token.
    pub action: String,
    /// The kind of thing the entry's subject names.
    pub subject_kind: String,
    /// A free-text reason, where the door supplies one.
    pub reason: Option<String>,
    /// The request's correlation, a `text` column (P-D-200). Products writes
    /// `None` on every row: this gear establishes no request correlation.
    pub correlation_id: Option<String>,
    /// The commit instant, taken as a parameter rather than read from
    /// `OffsetDateTime::now_utc()`.
    pub written_at: OffsetDateTime,
}

/// Write one audit row in the caller's mutation transaction: a door's act on
/// `subject_id` (P-D-193).
///
/// Writes `seal_state = "unsealed"` and leaves `chain_id`, `seq`,
/// `prev_hash` and `row_hash` `NULL` on every call, unconditionally: this
/// gear never seals, chains or verifies (P-D-200). `error_code`,
/// `attempted_key`, `session_id` and `ceremony_ref` are carried in the DDL
/// and written `NULL`: no door writes a refusal, keyed, elevated-read or
/// ceremony row.
///
/// # Errors
/// Returns scoped storage failures.
pub async fn write_eventless_act_audit(
    runner: &impl DBRunner,
    scope: &AccessScope,
    common: AuditCommon,
    subject_id: Uuid,
    subject_revision: Option<i64>,
) -> Result<(), RepoError> {
    let audit_id = common.audit_id;
    let model = audit_log::ActiveModel {
        audit_id: Set(common.audit_id),
        tenant_id: Set(common.tenant_id),
        actor_ref: Set(common.actor_ref),
        action: Set(common.action),
        subject_kind: Set(common.subject_kind),
        subject_id: Set(Some(subject_id)),
        subject_revision: Set(subject_revision),
        error_code: Set(None),
        attempted_key: Set(None),
        reason: Set(common.reason),
        correlation_id: Set(common.correlation_id),
        written_at: Set(common.written_at),
        session_id: Set(None),
        ceremony_ref: Set(None),
        seal_state: Set("unsealed".to_owned()),
        chain_id: Set(None),
        seq: Set(None),
        prev_hash: Set(None),
        row_hash: Set(None),
    };

    audit_log::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| driver_failure(format!("audit row {audit_id} scope"), e))?
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("insert audit row {audit_id}"), e))?;

    Ok(())
}

#[cfg(test)]
#[path = "audit_repo_tests.rs"]
mod audit_repo_tests;
