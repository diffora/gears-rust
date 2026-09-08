//! The Legal-governed PII allow-list's store (`dod-pii-allowlist`;
//! **P-D-117** items 23 and 31, `design/10-retention-erasure.md`
//! `inst-pp-allowlist`).
//!
//! # Every write is a governed act, so nothing here opens its own transaction
//!
//! A mutation is a `GovernedLiveOp` under the base approver quorum, and its
//! audit row and its `PiiAllowlistChanged` event share the act's transaction.
//! So each function takes the caller's `runner` for the same reason
//! [`super::retention::tombstone_principal`] does — the atomicity is the
//! door's, and a store function that opened its own would break it silently.
//!
//! # Revocation is a `state` flip
//!
//! **P-D-47**'s reasoning: a revoked entry keeps its sign-off on record, so
//! there is no delete in this module and there is no `DELETE` guard on the
//! table. [`revoke_entry`] re-asserts `state = 'active'` in its own `WHERE`
//! for the reason `tombstone_principal`'s `UPDATE` does: two revocations
//! racing would otherwise both report success and the loser would restamp
//! `updated_at` on a row it did not move.
//!
//! # The active read is the detector's operand, and it is values only
//!
//! [`active_allowlist_values`] answers the normalized strings and nothing
//! else. `crate::domain::retention::RegistryPiiDetector` is synchronous by
//! design, so the door reads this before it inspects, and handing it whole
//! rows would put `justification` — operator free text this table exists to
//! keep inside the write block — on a path that has no reason to carry it.

use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, EntityTrait, Set};
use time::OffsetDateTime;
use toolkit_db::odata::sea_orm_filter::{
    FieldToColumn, LimitCfg, ODataFieldMapping, paginate_odata,
};
use toolkit_db::secure::{
    AccessScope, DBRunner, SecureEntityExt, SecureInsertExt, SecureUpdateExt,
};
use toolkit_odata::SortDir;
use toolkit_odata_macros::ODataFilterable;
use uuid::Uuid;

use super::driver_failure;
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::pii_allowlist;

/// One allow-list entry as the Legal review export renders it.
///
/// Carries `state` rather than filtering on it: the review's whole point is
/// to see what was signed off and what was withdrawn, and an export of the
/// active rows alone would hide the second half.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AllowlistEntry {
    /// The entry's stable address, and `PiiAllowlistChanged`'s aggregate.
    pub entry_id: Uuid,
    /// The normalized name the detector matches on.
    pub value_normalized: String,
    /// Why Legal admitted it.
    pub justification: String,
    /// The reference to the external Legal decision.
    pub signed_off_by: String,
    pub signed_off_at: OffsetDateTime,
    /// [`pii_allowlist::STATE_ACTIVE`] or [`pii_allowlist::STATE_REVOKED`].
    pub state: String,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

/// The tenant's active allow-list values, normalized, for the detector.
///
/// # Errors
///
/// [`RepoError::Driver`] on a storage failure, or [`RepoError::Db`] on a
/// scope refusal that raised no driver error.
pub async fn active_allowlist_values(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
) -> Result<Vec<String>, RepoError> {
    let rows = pii_allowlist::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(pii_allowlist::Column::TenantId.eq(tenant_id))
                .add(pii_allowlist::Column::State.eq(pii_allowlist::STATE_ACTIVE)),
        )
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("read active allow-list, tenant {tenant_id}"), e))?;
    Ok(rows.into_iter().map(|row| row.value_normalized).collect())
}

/// The allow-list export's **filterable vocabulary** (P-D-165).
///
/// # What is deliberately absent
///
/// * `tenant_id` — `AccessScope`'s.
/// * `justification` — operator free text that rides the content-PII write
///   block. A filter over it would be a substring search over prose a
///   reviewer wrote about a person, which is the one thing this table is
///   careful about.
/// * `updated_at` — the revoke's own stamp; `state` is the question a
///   reviewer actually asks, and `signed_off_at` is the one with meaning.
#[derive(ODataFilterable)]
#[allow(
    dead_code,
    reason = "a declaration read by the derive macro: only the generated \
              `AllowlistEntryQueryFilterField` is ever named in code"
)]
pub struct AllowlistEntryQuery {
    /// The entry. Also the walk's unique tiebreaker.
    #[odata(filter(kind = "Uuid"))]
    pub entry_id: Uuid,
    /// The admitted string, normalized.
    #[odata(filter(kind = "String"))]
    pub value_normalized: String,
    /// The external Legal sign-off reference (P-D-10).
    #[odata(filter(kind = "String"))]
    pub signed_off_by: String,
    #[odata(filter(kind = "DateTimeUtc"))]
    pub signed_off_at: OffsetDateTime,
    /// `active` or `revoked`. `state eq 'active'` is the live allow-list.
    #[odata(filter(kind = "String"))]
    pub state: String,
    /// When the entry was signed on. The default order.
    #[odata(filter(kind = "DateTimeUtc"))]
    pub created_at: OffsetDateTime,
}

/// The vocabulary under the name the rest of the gear uses.
pub use AllowlistEntryQueryFilterField as AllowlistEntryFilterField;

/// The vocabulary's storage mapping.
pub struct AllowlistEntryODataMapper;

impl FieldToColumn<AllowlistEntryFilterField> for AllowlistEntryODataMapper {
    type Column = pii_allowlist::Column;

    fn map_field(field: AllowlistEntryFilterField) -> pii_allowlist::Column {
        use AllowlistEntryFilterField as F;
        match field {
            F::EntryId => pii_allowlist::Column::EntryId,
            F::ValueNormalized => pii_allowlist::Column::ValueNormalized,
            F::SignedOffBy => pii_allowlist::Column::SignedOffBy,
            F::SignedOffAt => pii_allowlist::Column::SignedOffAt,
            F::State => pii_allowlist::Column::State,
            F::CreatedAt => pii_allowlist::Column::CreatedAt,
        }
    }
}

impl ODataFieldMapping<AllowlistEntryFilterField> for AllowlistEntryODataMapper {
    type Entity = pii_allowlist::Entity;

    fn extract_cursor_value(
        model: &pii_allowlist::Model,
        field: AllowlistEntryFilterField,
    ) -> sea_orm::Value {
        use AllowlistEntryFilterField as F;
        match field {
            F::EntryId => sea_orm::Value::from(model.entry_id),
            F::ValueNormalized => sea_orm::Value::from(model.value_normalized.clone()),
            F::SignedOffBy => sea_orm::Value::from(model.signed_off_by.clone()),
            F::SignedOffAt => sea_orm::Value::from(model.signed_off_at),
            F::State => sea_orm::Value::from(model.state.clone()),
            F::CreatedAt => sea_orm::Value::from(model.created_at),
        }
    }
}

/// The order the export answers in when the caller names none: oldest entry
/// first, which is the order a reviewer reads a ledger.
pub const ALLOWLIST_DEFAULT_ORDER: (&str, SortDir) = ("created_at", SortDir::Asc);

/// The walk's unique tiebreaker.
///
/// `created_at` alone is not unique — two entries signed off in one act
/// share it, which the unpaginated read already had to say out loud in its
/// own second `order_by`.
pub const ALLOWLIST_TIEBREAKER: (&str, SortDir) = ("entry_id", SortDir::Asc);

/// One page of the tenant's allow-list, active and revoked, for the Legal
/// review export (P-D-165).
///
/// # Errors
///
/// [`toolkit_odata::Error`] on an unservable query, or a driver failure.
pub async fn allowlist_entries(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    odata: &toolkit_odata::ODataQuery,
    limits: LimitCfg,
) -> Result<toolkit_odata::Page<AllowlistEntry>, toolkit_odata::Error> {
    let base = pii_allowlist::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(pii_allowlist::Column::TenantId.eq(tenant_id)));
    let effective = super::effective_odata(odata, ALLOWLIST_DEFAULT_ORDER, None);
    paginate_odata::<AllowlistEntryFilterField, AllowlistEntryODataMapper, _, _, _, _>(
        base,
        runner,
        &effective,
        ALLOWLIST_TIEBREAKER,
        limits,
        |row| AllowlistEntry {
            entry_id: row.entry_id,
            value_normalized: row.value_normalized,
            justification: row.justification,
            signed_off_by: row.signed_off_by,
            signed_off_at: row.signed_off_at,
            state: row.state,
            created_at: row.created_at,
            updated_at: row.updated_at,
        },
    )
    .await
}

/// The columns a caller supplies when signing an entry on.
///
/// A struct rather than five positional arguments because four of them are
/// `String` and a transposed pair would compile.
#[derive(Clone, Debug)]
pub struct NewAllowlistEntry {
    pub tenant_id: Uuid,
    pub entry_id: Uuid,
    /// **Already normalized** by
    /// `crate::domain::retention::normalize_allowlist_value` — the door
    /// normalizes so the refusal it may raise names the field the operator
    /// sent, and the store writes what it is given.
    pub value_normalized: String,
    pub justification: String,
    pub signed_off_by: String,
    pub signed_off_at: OffsetDateTime,
    pub now: OffsetDateTime,
}

/// Sign an entry on, in the caller's transaction.
///
/// The partial unique `uq_products_pii_allowlist_active` is what refuses a
/// second **active** entry for one normalized value; this function does not
/// pre-check it, because a pre-check is a race and the index is not.
///
/// # Errors
///
/// [`RepoError::Driver`] on a storage failure — including the unique-index
/// refusal, which the door turns into its own conflict — or [`RepoError::Db`]
/// on a scope refusal that raised no driver error.
pub async fn insert_entry(
    runner: &impl DBRunner,
    scope: &AccessScope,
    entry: NewAllowlistEntry,
) -> Result<(), RepoError> {
    let row = pii_allowlist::ActiveModel {
        tenant_id: Set(entry.tenant_id),
        entry_id: Set(entry.entry_id),
        value_normalized: Set(entry.value_normalized),
        justification: Set(entry.justification),
        signed_off_by: Set(entry.signed_off_by),
        signed_off_at: Set(entry.signed_off_at),
        state: Set(pii_allowlist::STATE_ACTIVE.to_owned()),
        created_at: Set(entry.now),
        updated_at: Set(entry.now),
    };
    pii_allowlist::Entity::insert(row.clone())
        .secure()
        .scope_with_model(scope, &row)
        .map_err(|e| driver_failure(format!("allow-list scope of {}", entry.tenant_id), e))?
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("insert allow-list entry {}", entry.entry_id), e))?;
    Ok(())
}

/// Flip an active entry to `revoked`, in the caller's transaction.
///
/// Answers `false` when no **active** entry with that id exists — either it
/// was never there or a racer revoked it first. Both are the same fact from
/// the caller's side, and the door refuses on it.
///
/// # Errors
///
/// [`RepoError::Driver`] on a storage failure, or [`RepoError::Db`] on a
/// scope refusal that raised no driver error.
pub async fn revoke_entry(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    entry_id: Uuid,
    now: OffsetDateTime,
) -> Result<bool, RepoError> {
    let outcome = pii_allowlist::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            pii_allowlist::Column::State,
            Expr::value(pii_allowlist::STATE_REVOKED),
        )
        .col_expr(pii_allowlist::Column::UpdatedAt, Expr::value(now))
        .filter(
            Condition::all()
                .add(pii_allowlist::Column::TenantId.eq(tenant_id))
                .add(pii_allowlist::Column::EntryId.eq(entry_id))
                .add(pii_allowlist::Column::State.eq(pii_allowlist::STATE_ACTIVE)),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("revoke allow-list entry {entry_id}"), e))?;
    Ok(outcome.rows_affected > 0)
}
