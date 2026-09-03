//! `10-retention-erasure`'s store layer: the map's erasure act, the compliance
//! export's two reads, and the audit writers those two doors spend.
//!
//! # The erasure act does not go through [`super::resolve_actor_ref`]
//!
//! `features/retention-erasure.md`'s `dod-identity-map` states the constraint
//! and it is not optional: that function **mints on a miss**, so an unknown
//! principal handed to it would gain a fresh live row instead of being
//! refused, and the door would answer success on a DSAR it never served. So
//! [`tombstone_principal`] carries its own resolve, and the miss returns
//! `None` for the door to turn into `ERASURE_UNKNOWN_ACTOR`.
//!
//! It also filters `tombstoned_at IS NULL`, which is why the export cannot
//! use it either: `inst-er-export` returns *"the principal's map entries"*
//! and a tombstoned entry is still an entry. [`identity_entries_of_principal`]
//! is the tombstone-inclusive read that `DoD` names, over the same
//! `(tenant_id, principal_ref)` index.
//!
//! # `identity_payload` is destroyed by an `UPDATE` that no shipped writer
//! can currently make matter
//!
//! Measured at `433edf0cd`: the only statement in the crate that writes the
//! column is [`super::resolve_actor_ref`]'s mint, and it writes `Set(None)`.
//! **No production path ever populates an identity payload**, so a test that
//! erases a row minted the ordinary way proves nothing about the destruction
//! — the column was already `NULL`. `retention_tests` seeds a payload
//! directly for that reason, which is the only way the assertion is armed
//! against the claim it makes.
//!
//! # The audit class these doors write, and why one `AuditEntry` serves two
//!
//! `AuditEntry` discriminates the **row** a writer builds, not the class the
//! decision register recognises: four variants already serve P-D-21's three
//! classes, `KeyedAct` being `EventlessAct`'s keyless twin. Both writers here
//! build the same row — a subject id, no error code, no session — so both
//! spend `AuditEntry::EventlessAct`, and the **class** is carried by the
//! function name and its doc, as it is for the keyed one.
//!
//! Owner call, 2026-09-03, on `features/retention-erasure.md` §7 row 4:
//! P-D-21 class 2 widens from its example (*"reads under elevation"*) to its
//! own stated reason (*"a read writes no outbox row at all"*), which admits
//! the compliance export; and a **fourth class** admits acts whose evidential
//! record must carry a field the event deliberately omits, which admits the
//! erasure act once `ActorErased` lands. Until `dod-retention-events` lands,
//! the erasure act emits nothing and class 3 covers it verbatim — the fourth
//! class is what keeps it admitted afterwards, and that sequencing is the
//! reason this module writes the row today without waiting.

use chrono::{DateTime, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use toolkit_db::secure::{AccessScope, DBRunner, SecureEntityExt, SecureUpdateExt};
use uuid::Uuid;

use super::{AuditCommon, AuditEntry, driver_failure, insert_audit_row};
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::{audit_log, identity_ref};

/// One row of the identity map as the compliance export renders it.
///
/// Carries `identity_payload` because this is the one surface that may return
/// a real identity (`dod-compliance-export`), and carries `tombstoned_at`
/// because a tombstoned entry is part of the answer rather than excluded from
/// it: a DSAR after an erasure must be able to see that the erasure happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdentityEntry {
    /// The pseudonym every immutable record carries.
    pub actor_ref: Uuid,
    /// The identity, where one was ever stored and has not been destroyed.
    pub identity_payload: Option<String>,
    /// Set once, by erasure, and never cleared.
    pub tombstoned_at: Option<DateTime<Utc>>,
    /// When this ref was minted.
    pub first_seen_at: DateTime<Utc>,
    /// When an act last **resolved** it, never when it was minted alone.
    pub last_seen_at: DateTime<Utc>,
}

/// Tombstone a principal's live map entry, in the caller's transaction.
///
/// Answers the tombstoned `actor_ref`, or `None` when the principal has no
/// live ref in this tenant — which is the door's `ERASURE_UNKNOWN_ACTOR`.
///
/// # `runner` MUST be the door's own transaction
///
/// `dod-erasure-door` requires the overwrite happen **in one transaction**,
/// and `chk_products_identity_ref_tombstone` does not supply that atomicity:
/// it is one-directional, refusing a row that carries both a payload and a
/// `tombstoned_at` while **admitting** a payload destroyed with no tombstone
/// stamped. So a half-done erasure is a shape the CHECK permits and only the
/// transaction forbids.
///
/// # The resolve and the write are separate statements, and the predicate is
/// repeated
///
/// The `UPDATE` re-asserts `tombstoned_at IS NULL`. Two erasures of one
/// principal racing inside their own transactions would otherwise both report
/// success, and the second would restamp a `tombstoned_at` the first had
/// already set — moving a column whose entity doc says it is *"set once, by
/// erasure, and never cleared"*. The loser reads zero rows affected and
/// answers `None`, so the door refuses it as unknown: after the winner
/// commits the principal genuinely has no live ref, which is what that
/// refusal means.
///
/// # Errors
///
/// [`RepoError::Driver`] on a storage failure, or [`RepoError::Db`] on a
/// scope refusal that raised no driver error.
pub async fn tombstone_principal(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    principal_ref: &str,
    now: DateTime<Utc>,
) -> Result<Option<Uuid>, RepoError> {
    let live = identity_ref::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(identity_ref::Column::TenantId.eq(tenant_id))
                .add(identity_ref::Column::PrincipalRef.eq(principal_ref))
                .add(identity_ref::Column::TombstonedAt.is_null()),
        )
        .one(runner)
        .await
        .map_err(|e| {
            driver_failure(
                format!("resolve principal for erasure, tenant {tenant_id}"),
                e,
            )
        })?;

    let Some(row) = live else {
        return Ok(None);
    };

    let tombstoned = identity_ref::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            identity_ref::Column::IdentityPayload,
            Expr::value(Option::<String>::None),
        )
        .col_expr(identity_ref::Column::TombstonedAt, Expr::value(now))
        .filter(
            Condition::all()
                .add(identity_ref::Column::TenantId.eq(tenant_id))
                .add(identity_ref::Column::ActorRef.eq(row.actor_ref))
                .add(identity_ref::Column::TombstonedAt.is_null()),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("tombstone actor ref {}", row.actor_ref), e))?;

    if tombstoned.rows_affected == 0 {
        // The race above was lost. Not an error: the principal has no live
        // ref now, which is exactly what the door's refusal asserts.
        return Ok(None);
    }

    Ok(Some(row.actor_ref))
}

/// Every map entry a principal has ever held in this tenant, tombstoned ones
/// included.
///
/// This is `dod-identity-map`'s *"second, tombstone-inclusive read over the
/// same `(tenant_id, principal_ref)` index, for the export alone"*. Ordered
/// by `first_seen_at` so a DSAR response is reproducible across calls; the
/// column is `NOT NULL` and a principal that re-appears after an erasure
/// mints a strictly later row, so the order is total in practice without
/// leaning on it being unique.
///
/// # Errors
///
/// [`RepoError::Driver`] on a storage failure, or [`RepoError::Db`] on a
/// scope refusal that raised no driver error.
pub async fn identity_entries_of_principal(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    principal_ref: &str,
) -> Result<Vec<IdentityEntry>, RepoError> {
    let rows = identity_ref::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(identity_ref::Column::TenantId.eq(tenant_id))
                .add(identity_ref::Column::PrincipalRef.eq(principal_ref)),
        )
        .order_by(identity_ref::Column::FirstSeenAt, sea_orm::Order::Asc)
        .all(runner)
        .await
        .map_err(|e| {
            driver_failure(
                format!("read identity entries for export, tenant {tenant_id}"),
                e,
            )
        })?;

    Ok(rows
        .into_iter()
        .map(|row| IdentityEntry {
            actor_ref: row.actor_ref,
            identity_payload: row.identity_payload,
            tombstoned_at: row.tombstoned_at,
            first_seen_at: row.first_seen_at,
            last_seen_at: row.last_seen_at,
        })
        .collect())
}

/// The `audit_id`s of every audit row carrying one of these refs.
///
/// `inst-er-export` returns the map entries *"plus the audit-row references
/// carrying their refs"* — references, not the rows: an audit row's `reason`
/// is operator free text inside the content-PII write block, and returning it
/// here would put a second copy of whatever it holds into the export.
///
/// An empty `actor_refs` answers an empty vector **without a query**. A
/// generated `IN ()` is not portable and, worse, the shapes that do parse it
/// tend to match everything; a principal with no refs must export no audit
/// references, not all of the tenant's.
///
/// # Errors
///
/// [`RepoError::Driver`] on a storage failure, or [`RepoError::Db`] on a
/// scope refusal that raised no driver error.
pub async fn audit_refs_of_actors(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor_refs: &[Uuid],
) -> Result<Vec<Uuid>, RepoError> {
    if actor_refs.is_empty() {
        return Ok(Vec::new());
    }

    let rows = audit_log::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(audit_log::Column::TenantId.eq(tenant_id))
                .add(audit_log::Column::ActorRef.is_in(actor_refs.iter().copied())),
        )
        .order_by(audit_log::Column::WrittenAt, sea_orm::Order::Asc)
        .order_by(audit_log::Column::AuditId, sea_orm::Order::Asc)
        .all(runner)
        .await
        .map_err(|e| {
            driver_failure(
                format!("read audit references for export, tenant {tenant_id}"),
                e,
            )
        })?;

    Ok(rows.into_iter().map(|row| row.audit_id).collect())
}

/// Write the compliance export's per-access audit row.
///
/// **P-D-21 class 2, widened from its example to its reason** (owner call,
/// 2026-09-03, §7 row 4). The class was worded *"reads under elevation"* and
/// justified as *"a read writes no outbox row at all"*; the compliance export
/// is a read that writes no outbox row and that `inst-er-export` requires be
/// audited *"individually"*, so it lands in the class the reason describes.
/// It carries no `session_id` because it runs under no elevation — the grant
/// is `compliance × export`, its own pair.
///
/// # `runner` MUST be a transaction of its own, committed before the read is
/// served
///
/// The same discipline as [`super::write_elevated_read_audit`] and for the
/// same reason (P-D-34): a read has no mutation transaction to join, and an
/// audited read the registry did not record is the failure individual
/// auditing exists to prevent. The caller answers a write failure by serving
/// nothing.
///
/// # Errors
///
/// [`RepoError::Driver`] on a storage failure, or [`RepoError::Db`] on a
/// scope refusal that raised no driver error.
pub async fn write_audited_read_audit(
    runner: &impl DBRunner,
    scope: &AccessScope,
    common: AuditCommon,
    subject_id: Uuid,
) -> Result<(), RepoError> {
    insert_audit_row(
        runner,
        scope,
        common,
        AuditEntry::EventlessAct {
            subject_id,
            subject_revision: None,
        },
    )
    .await
}

/// Write the erasure act's evidential audit row.
///
/// **P-D-21's fourth class** (owner call, 2026-09-03, §7 row 4): acts whose
/// evidential record must carry a field the event deliberately omits. The
/// other two arms of that row are both closed by this feature's own `DoD`s —
/// widening `ActorErased` is refused by `dod-retention-events`
/// (*"carries the ref and no identity"*, and the reason is operator free text
/// inside the content-PII write block), and declaring the act eventless is
/// refused by the same `DoD`'s MUST-emit.
///
/// The row carries the reason and the **eraser's own** pseudonymous ref, in
/// `AuditCommon`, which is exactly what `ActorErased(actor_ref)` omits; its
/// `subject_id` is the ref that was tombstoned.
///
/// # `runner` MUST be the erasure's own transaction
///
/// [`super::write_eventless_act_audit`]'s contract, verbatim: the act and its
/// record stand or fall together, so a row that cannot be written fails the
/// tombstone's own commit. A DSAR whose evidence did not commit did not
/// happen.
///
/// # Errors
///
/// [`RepoError::Driver`] on a storage failure, or [`RepoError::Db`] on a
/// scope refusal that raised no driver error.
pub async fn write_evidential_act_audit(
    runner: &impl DBRunner,
    scope: &AccessScope,
    common: AuditCommon,
    subject_id: Uuid,
) -> Result<(), RepoError> {
    insert_audit_row(
        runner,
        scope,
        common,
        AuditEntry::EventlessAct {
            subject_id,
            subject_revision: None,
        },
    )
    .await
}

#[cfg(test)]
#[path = "retention_tests.rs"]
mod retention_tests;
