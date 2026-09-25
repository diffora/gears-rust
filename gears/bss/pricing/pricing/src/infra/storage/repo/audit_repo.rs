//! Scoped audit repo; follows the Products implementation.
//!
//! @cpt-dod:cpt-cf-bss-pricing-dod-audit-append-only:p1
use super::driver_failure;
use crate::infra::storage::{RepoError, entity::audit_log};
use sea_orm::{EntityTrait, Set};
use time::OffsetDateTime;
use toolkit_db::secure::{AccessScope, DBRunner, SecureInsertExt};
use uuid::Uuid;
#[derive(Clone, Debug)]
pub enum RefusalSubject {
    /// The refusal happened after the subject was minted.
    Minted {
        /// The subject's id.
        subject_id: Uuid,
        /// The subject's revision at the time of the act, where the door
        /// has one to give.
        subject_revision: Option<i64>,
    },
    /// The refusal happened before the mint: the attempted `name`,
    /// `sku_code` or `product_code`.
    Attempted(String),
}

#[derive(Clone, Debug)]
pub enum AuditEntry {
    /// A door's refusal. Carries the refusal's `error_code` — a column
    /// rather than free text because §3.1 makes the code the attribution
    /// channel — and the subject the refusal names, one way or the other.
    /// Never carries a `session_id`.
    Refusal {
        /// The refusal's stable wire code, e.g. `DUPLICATE_NAME`.
        error_code: String,
        /// The subject the refusal names.
        subject: RefusalSubject,
    },
    /// A committed act the design declares emits no broker event. Carries
    /// the subject it acted on. Carries neither `error_code` nor
    /// `session_id`.
    EventlessAct {
        /// The subject's id — always minted by the time this class is
        /// written, since the act already committed.
        subject_id: Uuid,
        /// The subject's revision at the time of the act, where the door
        /// has one to give.
        subject_revision: Option<i64>,
    },
    /// A committed eventless act whose subject has no minted uuid — the
    /// freeze ledger's `(catalog_version_id, participant)` pair is the
    /// first: the pair rides `attempted_key` exactly as a pre-mint
    /// refusal's subject does, and `subject_id` stays `NULL`. Carries
    /// neither `error_code` nor `session_id`.
    KeyedAct {
        /// The act's subject, rendered as the door's own key string.
        attempted_key: String,
    },
    /// A read served under break-glass elevation. Carries the elevation's
    /// `session_id` — 05 audits every elevated access with it. Carries no
    /// `error_code`.
    ElevatedRead {
        /// The break-glass session under which the read was served.
        session_id: Uuid,
        /// The subject read, where the read named one.
        subject_id: Option<Uuid>,
        /// The subject's revision at the time of the read, where the door
        /// has one to give.
        subject_revision: Option<i64>,
    },
    /// An accepted act under a ceremony — the row carries `ceremony_ref`
    /// (`07`'s break-glass lanes, `dod-reference-audit`).
    CeremonyAct {
        subject_id: Uuid,
        subject_revision: Option<i64>,
        ceremony_ref: Uuid,
    },
}

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
    /// Ties related rows together across a single request, where one
    /// exists.
    ///
    /// The value is the W3C trace id `infra::events::correlation_id` reads
    /// off the ambient span — 32 hex characters, rendered so it stays
    /// grep-equal to the access log, the span and the error envelope. The
    /// column shipped `uuid`, which could hold none of them, so every caller
    /// passed `None` and this doc carried the two shapes the repair could
    /// take. **P-D-118 chose `text`** (2026-09-03) and the migration landed
    /// in place on 2026-09-04; the door writers fill it now. A background act
    /// — the GC, the runner — still writes `None`, because it has no request:
    /// that is a fact about the act, not a hole. Minting a value per row
    /// would be worse than `None`, filling the column with values that
    /// correlate nothing while reading as though they did.
    pub correlation_id: Option<String>,
    /// The commit instant; the operand `10-retention-erasure`'s
    /// `RetentionClock` reads. Taken as a parameter rather than read from
    /// `OffsetDateTime::now_utc()`, matching `resolve_actor_ref`.
    pub written_at: OffsetDateTime,
}

async fn insert_audit_row(
    runner: &impl DBRunner,
    scope: &AccessScope,
    common: AuditCommon,
    entry: AuditEntry,
) -> Result<(), RepoError> {
    let audit_id = common.audit_id;
    let mut ceremony: Option<Uuid> = None;

    let (subject_id, subject_revision, error_code, attempted_key, session_id) = match entry {
        AuditEntry::Refusal {
            error_code,
            subject,
        } => match subject {
            RefusalSubject::Minted {
                subject_id,
                subject_revision,
            } => (
                Some(subject_id),
                subject_revision,
                Some(error_code),
                None,
                None,
            ),
            RefusalSubject::Attempted(key) => (None, None, Some(error_code), Some(key), None),
        },
        AuditEntry::EventlessAct {
            subject_id,
            subject_revision,
        } => (Some(subject_id), subject_revision, None, None, None),
        AuditEntry::KeyedAct { attempted_key } => (None, None, None, Some(attempted_key), None),
        AuditEntry::CeremonyAct {
            subject_id,
            subject_revision,
            ceremony_ref,
        } => {
            ceremony = Some(ceremony_ref);
            (Some(subject_id), subject_revision, None, None, None)
        }
        AuditEntry::ElevatedRead {
            session_id,
            subject_id,
            subject_revision,
        } => (subject_id, subject_revision, None, None, Some(session_id)),
    };

    let model = audit_log::ActiveModel {
        audit_id: Set(common.audit_id),
        tenant_id: Set(common.tenant_id),
        actor_ref: Set(common.actor_ref),
        action: Set(common.action),
        subject_kind: Set(common.subject_kind),
        subject_id: Set(subject_id),
        subject_revision: Set(subject_revision),
        error_code: Set(error_code),
        attempted_key: Set(attempted_key),
        reason: Set(common.reason),
        correlation_id: Set(common.correlation_id),
        written_at: Set(common.written_at),
        session_id: Set(session_id),
        // P-D-129's column; `07`'s ceremony lanes write it through
        // `AuditEntry::CeremonyAct` (P-D-147), every other row is `None`.
        ceremony_ref: Set(ceremony),
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

/// Write eventless act audit.
/// # Errors
/// Returns scoped storage failures, preserving database errors for retry.
pub async fn write_eventless_act_audit(
    runner: &impl DBRunner,
    scope: &AccessScope,
    common: AuditCommon,
    subject_id: Uuid,
    subject_revision: Option<i64>,
) -> Result<(), RepoError> {
    insert_audit_row(
        runner,
        scope,
        common,
        AuditEntry::EventlessAct {
            subject_id,
            subject_revision,
        },
    )
    .await
}
