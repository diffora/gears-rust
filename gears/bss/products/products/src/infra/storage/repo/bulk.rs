//! The bulk batch and its row ledger — the import door's writes and the
//! batch worker's claim/read/outcome surface (`design/09`, P-D-54).
//!
//! Split out of the foundation repository move-only; every item re-exports
//! through `super` (`crate::infra::storage::repo`) unchanged.
use std::collections::BTreeSet;

use chrono::{DateTime, Duration, Utc};
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, EntityTrait, QuerySelect};
use toolkit_db::secure::{
    AccessScope, DBRunner, SecureEntityExt, SecureInsertExt, SecureUpdateExt,
};
use uuid::Uuid;

use crate::domain::governance::ApprovalId;
use crate::domain::states::BatchState;
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::{bulk_batch, bulk_row, product};

use super::{TenantIdRow, driver_failure};

/// One row as the import door writes it into the ledger.
#[derive(Clone, Debug)]
pub struct NewBulkRow {
    /// The caller's own key, batch-scoped.
    pub row_key: String,
    /// The ledger row's surrogate id — the `internal:bulk-row` lane's
    /// `client_key` (P-D-69).
    pub row_id: Uuid,
    /// `product` or `sku`.
    pub entity_kind: String,
    /// The entity the row targets, for an update-as-draft row.
    pub entity_id: Option<Uuid>,
    /// The revision the row pins, for an update-as-draft row.
    pub pinned_revision: Option<i64>,
    /// The row's imported content, canonically serialized (**P-D-86**).
    pub staged_payload: Option<String>,
    /// The pending operation a row stages and the commit applies
    /// (`governed_live_op`, §4): a lifecycle row's `{"op": ...}`, or the
    /// promotion resolver's `update_as_draft` marker with the fields it
    /// touched, which the abandon procedure reverts.
    pub governed_live_op: Option<String>,
}

/// One batch as the import door writes it.
#[derive(Clone, Debug)]
pub struct NewBulkBatch {
    /// Server-minted.
    pub batch_id: Uuid,
    /// The door's idempotency operand, UNIQUE per tenant.
    pub batch_key: String,
    /// `import` or `promote`.
    pub mode: String,
    /// `import` or `lifecycle`.
    pub lane: String,
    /// The creating act's idempotency key, where one was carried.
    pub operation_key: Option<String>,
    /// The creation instant.
    pub created_at: DateTime<Utc>,
}

/// One batch head, read back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BulkBatchRecord {
    /// The batch's own id.
    pub batch_id: Uuid,
    /// The door's idempotency operand.
    pub batch_key: String,
    /// `import` or `promote`.
    pub mode: String,
    /// `import` or `lifecycle`.
    pub lane: String,
    /// The state machine's current value, typed at the storage boundary.
    pub state: BatchState,
    /// The worker's attempt counter.
    pub attempt: i64,
    /// The `06` coalescing identity the commit's increment request carries
    /// (`dod-operation-key`); the import door writes the batch id.
    pub operation_key: Option<String>,
    /// `05`'s record, written at the `staging -> reported` edge by the
    /// report's submission; `None` before the edge and on a batch that never
    /// reported.
    pub approval_ref: Option<Uuid>,
    /// The worker's claim lease stamp (P-D-54).
    pub claimed_at: Option<DateTime<Utc>>,
    /// The creation instant.
    pub created_at: DateTime<Utc>,
}

/// One ledger row, read back — the `RowLedger` reader's payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BulkRowRecord {
    /// The caller's own key.
    pub row_key: String,
    /// The lane's client key.
    pub row_id: Uuid,
    /// `product` or `sku`.
    pub entity_kind: String,
    /// The entity, once minted.
    pub entity_id: Option<Uuid>,
    /// NULL while in flight.
    pub disposition: Option<String>,
    /// The owning feature's code on a failure.
    pub code: Option<String>,
    /// A closed-set literal.
    pub reason: Option<String>,
    /// The row's imported content, as the worker reads it back.
    pub staged_payload: Option<String>,
    /// The head revision the report pinned the row to (`inst-bk-report`);
    /// `None` before the report edge.
    pub pinned_revision: Option<i64>,
    /// The row is in the batch ceremony's itemised override set
    /// (`inst-bk-override`): its uncomposed-bundle condition was named in
    /// the report and acknowledged by name on the batch's record.
    pub override_acknowledged: bool,
    /// The pending operation the row stages (`governed_live_op`, §4).
    pub governed_live_op: Option<String>,
}

/// How many batches this tenant holds outside a terminal state — the
/// per-tenant ceiling's operand (`inst-bm-limits`), read by the import door
/// and re-read by the worker at claim (P-D-54).
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn count_live_batches(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
) -> Result<u64, RepoError> {
    bulk_batch::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(bulk_batch::Column::TenantId.eq(tenant_id))
                .add(
                    bulk_batch::Column::State
                        .is_not_in(BatchState::TERMINAL.map(BatchState::as_str)),
                ),
        )
        .count(runner)
        .await
        .map_err(|e| driver_failure(format!("count live batches of {tenant_id}"), e))
}

/// Read one batch head by its key — the import door's replay operand.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn find_batch_by_key(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    batch_key: &str,
) -> Result<Option<BulkBatchRecord>, RepoError> {
    let row = bulk_batch::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(bulk_batch::Column::TenantId.eq(tenant_id))
                .add(bulk_batch::Column::BatchKey.eq(batch_key)),
        )
        .one(runner)
        .await
        .map_err(|e| driver_failure(format!("read batch {batch_key}"), e))?;
    row.map(into_batch_record).transpose()
}

/// Read one batch head by its id — the ledger reader's operand.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn find_batch(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    batch_id: Uuid,
) -> Result<Option<BulkBatchRecord>, RepoError> {
    let row = bulk_batch::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(bulk_batch::Column::TenantId.eq(tenant_id))
                .add(bulk_batch::Column::BatchId.eq(batch_id)),
        )
        .one(runner)
        .await
        .map_err(|e| driver_failure(format!("read batch {batch_id}"), e))?;
    row.map(into_batch_record).transpose()
}

fn into_batch_record(row: bulk_batch::Model) -> Result<BulkBatchRecord, RepoError> {
    // The CHECK constraint admits only the roster, so a value outside it
    // is a corrupt row, never a default.
    let state = BatchState::parse(&row.state).ok_or_else(|| {
        RepoError::CorruptRow(format!(
            "bulk batch {} carries state {:?} outside the roster",
            row.batch_id, row.state
        ))
    })?;
    Ok(BulkBatchRecord {
        batch_id: row.batch_id,
        batch_key: row.batch_key,
        mode: row.mode,
        lane: row.lane,
        state,
        attempt: row.attempt,
        operation_key: row.operation_key,
        approval_ref: row.approval_ref,
        claimed_at: row.claimed_at,
        created_at: row.created_at,
    })
}

/// Every ledger row of one batch, in row-key order.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn find_batch_rows(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    batch_id: Uuid,
) -> Result<Vec<BulkRowRecord>, RepoError> {
    let rows = bulk_row::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(bulk_row::Column::TenantId.eq(tenant_id))
                .add(bulk_row::Column::BatchId.eq(batch_id)),
        )
        .order_by(bulk_row::Column::RowKey, sea_orm::Order::Asc)
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("read the ledger of {batch_id}"), e))?;
    Ok(rows
        .into_iter()
        .map(|row| BulkRowRecord {
            row_key: row.row_key,
            row_id: row.row_id,
            entity_kind: row.entity_kind,
            entity_id: row.entity_id,
            disposition: row.disposition,
            code: row.code,
            reason: row.reason,
            staged_payload: row.staged_payload,
            pinned_revision: row.pinned_revision,
            override_acknowledged: row.override_acknowledged,
            governed_live_op: row.governed_live_op,
        })
        .collect())
}

/// Stamp an update-as-draft row with the head it targets: the entity, the
/// revision the save left, and the resolver's marker naming the fields it
/// touched (`dod-promotion-resolver`).
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
#[allow(clippy::too_many_arguments)] // the row's target, all of it the ledger's columns
pub async fn stamp_bulk_row_target(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    batch_id: Uuid,
    row_key: &str,
    entity_id: Uuid,
    pinned_revision: i64,
    governed_live_op: &str,
) -> Result<(), RepoError> {
    bulk_row::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(bulk_row::Column::EntityId, Expr::value(Some(entity_id)))
        .col_expr(
            bulk_row::Column::PinnedRevision,
            Expr::value(Some(pinned_revision)),
        )
        .col_expr(
            bulk_row::Column::GovernedLiveOp,
            Expr::value(Some(governed_live_op.to_owned())),
        )
        .filter(
            Condition::all()
                .add(bulk_row::Column::TenantId.eq(tenant_id))
                .add(bulk_row::Column::BatchId.eq(batch_id))
                .add(bulk_row::Column::RowKey.eq(row_key)),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("stamp row {row_key} of batch {batch_id}"), e))?;
    Ok(())
}

/// Claim one `staging` batch for the worker under a **lease**: stamp
/// `claimed_at` and bump
/// `attempt`, under a predicate matching the state the caller read — so two
/// workers racing one batch cannot both believe they hold it (the
/// compare-and-swap the idempotency takeover uses, applied to the batch
/// head).
///
/// Answers `false` when the row moved under the caller.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
#[allow(clippy::too_many_arguments)] // the CAS's operands: the state it expects joined them (P-D-149)
pub async fn claim_bulk_batch(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    batch_id: Uuid,
    state: BatchState,
    attempt: i64,
    now: DateTime<Utc>,
    lease: Duration,
) -> Result<bool, RepoError> {
    // The lease predicate. `(state, attempt)` alone excludes only a racer that
    // reads the SAME attempt: a peer starting a second after the claim reads
    // the bumped attempt and its compare succeeds, mid-pass. `claimed_at` is
    // what P-D-54 calls the claim's lease, and until this predicate read it
    // the column was written and never read by anything.
    let expiry = now - lease;
    let result = bulk_batch::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(bulk_batch::Column::ClaimedAt, Expr::value(Some(now)))
        .col_expr(bulk_batch::Column::Attempt, Expr::value(attempt + 1))
        .filter(
            Condition::all()
                .add(bulk_batch::Column::TenantId.eq(tenant_id))
                .add(bulk_batch::Column::BatchId.eq(batch_id))
                .add(bulk_batch::Column::State.eq(state.as_str()))
                .add(bulk_batch::Column::Attempt.eq(attempt))
                .add(
                    Condition::any()
                        .add(bulk_batch::Column::ClaimedAt.is_null())
                        .add(bulk_batch::Column::ClaimedAt.lt(expiry)),
                ),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("claim batch {batch_id}"), e))?;
    Ok(result.rows_affected > 0)
}

/// Release the worker's lease on a batch without touching its state: the
/// staging claim is handed back at the commit flip so the commit phase can
/// take its own (`claimed_at` NULL is "claimable now").
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn release_bulk_batch_claim(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    batch_id: Uuid,
) -> Result<(), RepoError> {
    bulk_batch::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            bulk_batch::Column::ClaimedAt,
            Expr::value(Option::<DateTime<Utc>>::None),
        )
        .filter(
            Condition::all()
                .add(bulk_batch::Column::TenantId.eq(tenant_id))
                .add(bulk_batch::Column::BatchId.eq(batch_id)),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("release the claim on batch {batch_id}"), e))?;
    Ok(())
}

/// Move a batch's state, under a predicate naming the state it must be in
/// — the machine's edges are the worker's to walk and the guard's to
/// refuse.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn move_bulk_batch_state(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    batch_id: Uuid,
    from: BatchState,
    to: BatchState,
    now: DateTime<Utc>,
) -> Result<bool, RepoError> {
    let mut statement = bulk_batch::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            bulk_batch::Column::State,
            Expr::value(to.as_str().to_owned()),
        );
    // `terminal_at` is stamped by the edge that makes the row terminal, and
    // by no other: the column's whole reading is "when this batch stopped",
    // so a non-terminal edge writing it would date a batch still working.
    if BatchState::TERMINAL.contains(&to) {
        statement = statement.col_expr(bulk_batch::Column::TerminalAt, Expr::value(Some(now)));
    }
    let result = statement
        .filter(
            Condition::all()
                .add(bulk_batch::Column::TenantId.eq(tenant_id))
                .add(bulk_batch::Column::BatchId.eq(batch_id))
                .add(bulk_batch::Column::State.eq(from.as_str())),
        )
        .exec(runner)
        .await
        .map_err(|e| {
            driver_failure(
                format!("move batch {batch_id} {} -> {}", from.as_str(), to.as_str()),
                e,
            )
        })?;
    Ok(result.rows_affected > 0)
}

/// Every tenant holding at least one `staging` batch — the worker's
/// discovery read, under the system scope.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn tenants_with_staging_batches(
    runner: &impl DBRunner,
    scope: &AccessScope,
) -> Result<Vec<Uuid>, RepoError> {
    tenants_with_batches_in(runner, scope, &[BatchState::Staging]).await
}

/// The tenants holding at least one batch in any of `states` — the batch
/// worker's discovery read across the machine's live states.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn tenants_with_batches_in(
    runner: &impl DBRunner,
    scope: &AccessScope,
    states: &[BatchState],
) -> Result<Vec<Uuid>, RepoError> {
    // A DISTINCT projection for the same reason as
    // [`tenants_with_pending_requests`]: the per-second discovery read must
    // scale with distinct tenants, not with total staging batches.
    let names: Vec<&str> = states.iter().map(|state| state.as_str()).collect();
    let rows: Vec<TenantIdRow> = bulk_batch::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(bulk_batch::Column::State.is_in(names)))
        .project_all(runner, |q| {
            q.select_only()
                .column(bulk_batch::Column::TenantId)
                .distinct()
                .into_model::<TenantIdRow>()
        })
        .await
        .map_err(|e| driver_failure("discover staging batches".to_owned(), e))?;
    let mut tenants: Vec<Uuid> = rows.into_iter().map(|row| row.tenant_id).collect();
    tenants.sort();
    Ok(tenants)
}

/// One tenant's `staging` batches, oldest first.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn staging_batches(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
) -> Result<Vec<BulkBatchRecord>, RepoError> {
    batches_in_state(runner, scope, tenant_id, BatchState::Staging).await
}

/// One tenant's batches in `state`, oldest first.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn batches_in_state(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    state: BatchState,
) -> Result<Vec<BulkBatchRecord>, RepoError> {
    let rows = bulk_batch::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(bulk_batch::Column::TenantId.eq(tenant_id))
                .add(bulk_batch::Column::State.eq(state.as_str())),
        )
        .order_by(bulk_batch::Column::CreatedAt, sea_orm::Order::Asc)
        .all(runner)
        .await
        .map_err(|e| {
            driver_failure(format!("read {} batches of {tenant_id}", state.as_str()), e)
        })?;
    rows.into_iter().map(into_batch_record).collect()
}

/// Pin the batch's record on its row (`approval_ref`), written once at the
/// `staging -> reported` edge by the report's submission.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn set_bulk_batch_approval_ref(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    batch_id: Uuid,
    approval_id: ApprovalId,
) -> Result<(), RepoError> {
    bulk_batch::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            bulk_batch::Column::ApprovalRef,
            Expr::value(Some(approval_id.get())),
        )
        .filter(
            Condition::all()
                .add(bulk_batch::Column::TenantId.eq(tenant_id))
                .add(bulk_batch::Column::BatchId.eq(batch_id)),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("pin the approval of batch {batch_id}"), e))?;
    Ok(())
}

/// Pin one staged row to the head revision the report was rendered from
/// (`inst-bk-report`): the commit re-checks it row-locally as
/// `STALE_REVISION`.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn pin_bulk_row(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    batch_id: Uuid,
    row_key: &str,
    revision: i64,
) -> Result<(), RepoError> {
    bulk_row::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            bulk_row::Column::PinnedRevision,
            Expr::value(Some(revision)),
        )
        .filter(
            Condition::all()
                .add(bulk_row::Column::TenantId.eq(tenant_id))
                .add(bulk_row::Column::BatchId.eq(batch_id))
                .add(bulk_row::Column::RowKey.eq(row_key)),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("pin row {row_key} of batch {batch_id}"), e))?;
    Ok(())
}

/// Mark one row as a member of the batch ceremony's itemised override set
/// (`inst-bk-override`).
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn mark_bulk_row_override_acknowledged(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    batch_id: Uuid,
    row_key: &str,
) -> Result<(), RepoError> {
    bulk_row::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(bulk_row::Column::OverrideAcknowledged, Expr::value(true))
        .filter(
            Condition::all()
                .add(bulk_row::Column::TenantId.eq(tenant_id))
                .add(bulk_row::Column::BatchId.eq(batch_id))
                .add(bulk_row::Column::RowKey.eq(row_key)),
        )
        .exec(runner)
        .await
        .map_err(|e| {
            driver_failure(
                format!("mark row {row_key} of batch {batch_id} acknowledged"),
                e,
            )
        })?;
    Ok(())
}

/// The `(region_scope, brand_id)` values the tenant's Product heads carry
/// outside `excluding` — the operand of the report's scope-values lint
/// (`inst-bk-report`): a staged value unseen here is named, never refused.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn known_scope_values(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    excluding: &[Uuid],
) -> Result<(BTreeSet<String>, BTreeSet<Uuid>), RepoError> {
    #[derive(Debug, sea_orm::FromQueryResult)]
    struct ScopeValueRow {
        region_scope: String,
        brand_id: Uuid,
    }
    let mut condition = Condition::all().add(product::Column::TenantId.eq(tenant_id));
    if !excluding.is_empty() {
        condition = condition.add(product::Column::ProductId.is_not_in(excluding.to_vec()));
    }
    let rows: Vec<ScopeValueRow> = product::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(condition)
        .project_all(runner, |q| {
            q.select_only()
                .column(product::Column::RegionScope)
                .column(product::Column::BrandId)
                .distinct()
                .into_model::<ScopeValueRow>()
        })
        .await
        .map_err(|e| driver_failure(format!("scope values of {tenant_id}"), e))?;
    let mut regions = BTreeSet::new();
    let mut brands = BTreeSet::new();
    for row in rows {
        regions.insert(row.region_scope);
        brands.insert(row.brand_id);
    }
    Ok((regions, brands))
}

/// Record one row's staging outcome: the minted entity on a success, or the
/// terminal `failed` disposition with the owning feature's code on a
/// refusal. The ledger's trigger freezes a row once its disposition lands,
/// so this statement is the row's last write.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn record_bulk_row_outcome(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    batch_id: Uuid,
    row_key: &str,
    outcome: BulkRowOutcome<'_>,
) -> Result<(), RepoError> {
    let mut update = bulk_row::Entity::update_many().secure().scope_with(scope);
    // Only a stamping outcome writes the id. A `failed` outcome carries
    // `None`, and writing it would NULL an id an earlier pass had stamped —
    // erasing the ledger's only pointer to a draft that exists.
    if outcome.entity_id.is_some() {
        update = update.col_expr(bulk_row::Column::EntityId, Expr::value(outcome.entity_id));
    }
    if let Some(disposition) = outcome.disposition {
        update = update
            .col_expr(
                bulk_row::Column::Disposition,
                Expr::value(Some(disposition.to_owned())),
            )
            .col_expr(bulk_row::Column::TerminalAt, Expr::value(Some(outcome.now)))
            .col_expr(
                bulk_row::Column::Code,
                Expr::value(outcome.code.map(str::to_owned)),
            )
            .col_expr(
                bulk_row::Column::Reason,
                Expr::value(outcome.reason.map(str::to_owned)),
            );
    }
    update
        .filter(
            Condition::all()
                .add(bulk_row::Column::TenantId.eq(tenant_id))
                .add(bulk_row::Column::BatchId.eq(batch_id))
                .add(bulk_row::Column::RowKey.eq(row_key))
                // The ledger's trigger freezes a row only once its disposition
                // lands, so a staged-but-undisposed row is NOT frozen and this
                // statement is not automatically its last write. The predicate
                // is what makes it one.
                .add(bulk_row::Column::Disposition.is_null()),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("record row {row_key} of {batch_id}"), e))?;
    Ok(())
}

/// What one staged row became.
#[derive(Clone, Copy, Debug)]
pub struct BulkRowOutcome<'a> {
    /// The entity the row minted, where it did.
    pub entity_id: Option<Uuid>,
    /// `failed` at stage, or `None` while the row stays in flight for the
    /// commit phase to dispose of.
    pub disposition: Option<&'a str>,
    /// The owning feature's code, on a failure.
    pub code: Option<&'a str>,
    /// A literal from the closed set the migration's `CHECK` pins — today
    /// only `batch-abandoned` (**P-D-50**). **Never operator text**: this
    /// feature writes no free-text reason anywhere, which is why `02`'s
    /// content-PII enumeration no longer names it.
    pub reason: Option<&'a str>,
    /// The instant the disposition landed.
    pub now: DateTime<Utc>,
}

/// Write the batch head and its whole ledger, one transaction's worth
/// (the caller supplies the runner, so the door's own transaction holds
/// both). A batch with no rows is admitted: emptiness is the caller's
/// business, and the ledger reader reports it honestly.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure — a duplicate `batch_key`
/// surfacing as the driver's unique violation for the door to classify as
/// the replay it is.
pub async fn insert_bulk_batch(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    new: NewBulkBatch,
    rows: &[NewBulkRow],
) -> Result<Uuid, RepoError> {
    let batch_id = new.batch_id;
    let model = bulk_batch::ActiveModel {
        tenant_id: Set(tenant_id),
        batch_id: Set(batch_id),
        batch_key: Set(new.batch_key),
        mode: Set(new.mode),
        lane: Set(new.lane),
        state: Set(BatchState::Staging.as_str().to_owned()),
        operation_key: Set(new.operation_key),
        approval_ref: Set(None),
        claimed_at: Set(None),
        attempt: Set(0),
        created_at: Set(new.created_at),
        terminal_at: Set(None),
    };
    bulk_batch::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| driver_failure(format!("batch scope of {tenant_id}"), e))?
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("insert batch {batch_id}"), e))?;

    for row in rows {
        let model = bulk_row::ActiveModel {
            tenant_id: Set(tenant_id),
            batch_id: Set(batch_id),
            row_key: Set(row.row_key.clone()),
            row_id: Set(row.row_id),
            entity_kind: Set(row.entity_kind.clone()),
            entity_id: Set(row.entity_id),
            pinned_revision: Set(row.pinned_revision),
            staged_payload: Set(row.staged_payload.clone()),
            disposition: Set(None),
            code: Set(None),
            reason: Set(None),
            governed_live_op: Set(row.governed_live_op.clone()),
            override_acknowledged: Set(false),
            terminal_at: Set(None),
        };
        bulk_row::Entity::insert(model.clone())
            .secure()
            .scope_with_model(scope, &model)
            .map_err(|e| driver_failure(format!("ledger scope of {tenant_id}"), e))?
            .exec(runner)
            .await
            .map_err(|e| driver_failure(format!("insert ledger row {}", row.row_key), e))?;
    }
    Ok(batch_id)
}
