//! Repository for `pricing_bulk_operation` and `pricing_bulk_row_lock` — a bulk
//! run and the rows it holds while it commits
//! (`design/12-operator-efficiency.md` §4, §6; D-260, D-262, D-267).
//!
//! # The state machine is the trigger's, and this repository does not restate it
//!
//! [`advance`] names a target state and writes it. Whether the move is an edge is
//! the table's business, on both engines, and the refusal it raises is the one an
//! operator reads. A `match` here listing the legal edges would be a second
//! spelling of a rule that already exists in two places — and `rejected` arriving
//! four migrations after the table is the demonstration that the two would drift.
//!
//! # The lock's mutual exclusion is its primary key
//!
//! `pricing_bulk_row_lock` is keyed `(tenant_id, price_id)`, so two runs cannot
//! hold one row: the second insert collides. [`take_locks`] therefore does not
//! read first and then write — it writes, and reads only to *name* the holder in
//! the refusal, which is what `fr-concurrent-edit` asks for. The read-then-write
//! arrangement would be the check-then-act race the key exists to close.
//!
//! **A lock row names a holder that is committed.** Unlike the approval
//! register's pending-key conflict, which cannot name its holder because the
//! winning transaction may not have committed, a lock is inserted by a run that
//! is already `committing` — so the row this repository reads back is durable and
//! the refusal can carry it.

use chrono::{DateTime, Utc};
use sea_orm::ActiveValue::Set;
use sea_orm::{ColumnTrait, Condition, EntityTrait, JsonValue};
use toolkit_db::secure::{
    AccessScope, DBRunner, SecureDeleteExt, SecureEntityExt, SecureInsertExt, SecureUpdateExt,
};
use toolkit_db::{DBProvider, DbError};
use uuid::Uuid;

use crate::domain::bulk::{BulkKind, BulkState};
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::{bulk_operation, bulk_row_lock};

/// A run as this crate reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BulkOperationRecord {
    /// The run's durable name.
    pub operation_id: Uuid,
    /// RLS scope.
    pub tenant_id: Uuid,
    /// Which flow it is.
    pub kind: BulkKind,
    /// Where it stands.
    pub state: BulkState,
    /// O4's client key, unique per tenant.
    pub client_key: String,
    /// The per-row report, which grows as the run progresses.
    pub report: JsonValue,
    /// Who submitted it.
    pub submitted_by: Uuid,
    /// When.
    pub submitted_at: DateTime<Utc>,
    /// Set exactly on the terminal states, which the `CHECK` keeps honest.
    pub completed_at: Option<DateTime<Utc>>,
}

/// A run at its birth.
///
/// No `state`: a run is born `validating` and the table's insert trigger refuses
/// any other value outright, so offering the caller a choice would be offering
/// one the store takes back.
#[derive(Clone, Debug)]
pub struct NewBulkOperation {
    /// Minted by the caller, so the audit record of the same act can name it.
    pub operation_id: Uuid,
    /// RLS scope.
    pub tenant_id: Uuid,
    /// Which flow.
    pub kind: BulkKind,
    /// O4's client key.
    pub client_key: String,
    /// The report as it stands at birth — Phase 1 has not run.
    pub report: JsonValue,
    /// Who submitted it.
    pub submitted_by: Uuid,
    /// When.
    pub submitted_at: DateTime<Utc>,
}

/// The repository handle, for callers that own no transaction.
#[derive(Clone)]
pub struct BulkRepo {
    db: DBProvider<DbError>,
}

impl BulkRepo {
    /// Wrap a provider.
    #[must_use]
    pub const fn new(db: DBProvider<DbError>) -> Self {
        Self { db }
    }

    /// [`read`] on a fresh connection.
    ///
    /// # Errors
    /// Exactly [`read`]'s.
    pub async fn read(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        operation_id: Uuid,
    ) -> Result<Option<BulkOperationRecord>, RepoError> {
        let conn = self
            .db
            .conn()
            .map_err(|e| RepoError::Db(format!("pricing_bulk_operation conn: {e}")))?;
        read(&conn, scope, tenant_id, operation_id).await
    }
}

/// Open a run, on a runner the caller owns.
///
/// # Errors
/// [`RepoError::Db`] on a scope or storage failure — which includes the insert
/// trigger's refusal of any birth state but `validating`, and the unique client
/// key's refusal of a second run under one key.
pub async fn open(
    runner: &impl DBRunner,
    scope: &AccessScope,
    new: NewBulkOperation,
) -> Result<BulkOperationRecord, RepoError> {
    let row = bulk_operation::ActiveModel {
        operation_id: Set(new.operation_id),
        tenant_id: Set(new.tenant_id),
        kind: Set(new.kind.as_str().to_owned()),
        state: Set(BulkState::Validating.as_str().to_owned()),
        client_key: Set(new.client_key.clone()),
        report: Set(new.report.clone()),
        submitted_by: Set(new.submitted_by),
        submitted_at: Set(new.submitted_at),
        completed_at: Set(None),
    };
    bulk_operation::Entity::insert(row.clone())
        .secure()
        .scope_with_model(scope, &row)
        .map_err(|e| RepoError::Db(format!("pricing_bulk_operation scope: {e}")))?
        .exec(runner)
        .await
        .map_err(|e| RepoError::Db(format!("insert pricing_bulk_operation: {e}")))?;

    read(runner, scope, new.tenant_id, new.operation_id)
        .await?
        .ok_or_else(|| {
            RepoError::CorruptRow(format!(
                "pricing_bulk_operation {} was inserted and does not read back",
                new.operation_id
            ))
        })
}

/// One run by id.
///
/// # Errors
/// [`RepoError::Db`] on a scope or storage failure; [`RepoError::CorruptRow`] for
/// a stored kind or state token no `CHECK` should have admitted.
pub async fn read(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    operation_id: Uuid,
) -> Result<Option<BulkOperationRecord>, RepoError> {
    let row = bulk_operation::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(bulk_operation::Column::TenantId.eq(tenant_id))
                .add(bulk_operation::Column::OperationId.eq(operation_id)),
        )
        .one(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read pricing_bulk_operation: {e}")))?;
    row.as_ref().map(record_of).transpose()
}

/// The run a client key already opened **for this kind**, if any (O4).
///
/// **`kind` is a filter and not a convenience** (D-307). §5 gives the two flows
/// two different idempotency columns — the import's `Idempotency-Key` and a
/// repricing run's own `run_id` — and without this predicate an import replay
/// would answer `202` describing a *repricing run*, import nothing, and hand back
/// a view carrying no `kind` member to reveal the substitution. The index behind
/// it moved to `(tenant_id, kind, client_key)` in the same wave; either half
/// alone leaves the other's hole open.
///
/// # Errors
/// Exactly [`read`]'s.
pub async fn find_by_client_key(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    kind: BulkKind,
    client_key: &str,
) -> Result<Option<BulkOperationRecord>, RepoError> {
    let row = bulk_operation::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(bulk_operation::Column::TenantId.eq(tenant_id))
                .add(bulk_operation::Column::Kind.eq(kind.as_str()))
                .add(bulk_operation::Column::ClientKey.eq(client_key)),
        )
        .one(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read pricing_bulk_operation by client key: {e}")))?;
    row.as_ref().map(record_of).transpose()
}

/// Move a run to `to`, writing the report it has reached.
///
/// `completed_at` is supplied for a terminal state and `None` otherwise, which
/// the `CHECK` pairs with the state — so a caller that disagrees with itself is
/// refused by the store rather than by a comparison here.
///
/// # Errors
/// [`RepoError::Db`] on a scope or storage failure — which includes the
/// transition trigger's refusal of a move that is not an edge, and the `CHECK`'s
/// refusal of a terminal state without an instant; [`RepoError::NotFound`] when
/// the run does not exist.
pub async fn advance(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    operation_id: Uuid,
    to: BulkState,
    report: JsonValue,
    at: DateTime<Utc>,
) -> Result<BulkOperationRecord, RepoError> {
    let completed_at = to.is_terminal().then_some(at);
    let affected = bulk_operation::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            bulk_operation::Column::State,
            sea_orm::sea_query::Expr::value(to.as_str()),
        )
        .col_expr(
            bulk_operation::Column::Report,
            sea_orm::sea_query::Expr::value(report),
        )
        .col_expr(
            bulk_operation::Column::CompletedAt,
            sea_orm::sea_query::Expr::value(completed_at),
        )
        .filter(
            Condition::all()
                .add(bulk_operation::Column::TenantId.eq(tenant_id))
                .add(bulk_operation::Column::OperationId.eq(operation_id)),
        )
        .exec(runner)
        .await
        .map_err(|e| RepoError::Db(format!("advance pricing_bulk_operation: {e}")))?;
    if affected.rows_affected == 0 {
        return Err(RepoError::NotFound {
            subject: "bulk operation".to_owned(),
            id: operation_id.to_string(),
        });
    }
    read(runner, scope, tenant_id, operation_id)
        .await?
        .ok_or_else(|| {
            RepoError::CorruptRow(format!(
                "pricing_bulk_operation {operation_id} moved and does not read back"
            ))
        })
}

/// Take the run's row locks (`inst-bk-lock`).
///
/// **The run must already be `committing`.** The lock table's own trigger says so
/// — "the bulk lock takes effect only on entry to committing" — and it is not a
/// formality: a lock held by a run that is still validating would exclude every
/// interactive editor for the length of Phase 1, which is a read.
///
/// Writes first and reads only to name a holder, for the module doc's reason: the
/// key is the mutual exclusion, and a read-then-write would be the check-then-act
/// race it exists to close.
///
/// **`runner` must be an autocommit connection, not a transaction.** Postgres
/// aborts an enclosing transaction on a failed statement, and everything this
/// function does after a refused insert is a further statement — so inside a
/// transaction the refusal degrades from [`RepoError::BulkRowLocked`] to
/// [`RepoError::Db`], losing exactly the holder `fr-concurrent-edit` requires it
/// to name.
///
/// **Measured on Postgres rather than reasoned about** (`postgres_bulk_repo`,
/// D-297): the statement that dies is the **release**, not the holder read. The
/// release carries a `?` and runs first, so the read is never reached at all and
/// the error reads `release pricing_bulk_row_lock: … current transaction is
/// aborted`. This paragraph said the holder read was what failed, which was true
/// of the order that stood *before* D-294 put the release in front of it — the
/// same wave's own correction moved the statement and left the sentence
/// describing the code it replaced. The case pins both halves of the message so
/// the two cannot drift apart again.
///
/// Either every lock is taken or none is: a refusal partway releases what this run
/// already took, so a caller cannot be left holding rows it does not know about.
///
/// # Errors
/// [`RepoError::BulkRowLocked`] naming the run that already holds a row;
/// [`RepoError::Db`] on a scope or storage failure.
pub async fn take_locks(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    operation_id: Uuid,
    price_ids: &[Uuid],
    at: DateTime<Utc>,
) -> Result<(), RepoError> {
    for &price_id in price_ids {
        let row = bulk_row_lock::ActiveModel {
            tenant_id: Set(tenant_id),
            price_id: Set(price_id),
            bulk_operation_id: Set(operation_id),
            locked_at: Set(at),
        };
        let taken = bulk_row_lock::Entity::insert(row.clone())
            .secure()
            .scope_with_model(scope, &row)
            .map_err(|e| RepoError::Db(format!("pricing_bulk_row_lock scope: {e}")))?
            .exec(runner)
            .await;
        if let Err(e) = taken {
            // **All or none.** The inserts above are independent statements on
            // whatever runner the caller holds, so a refusal partway leaves this
            // run holding the rows it already took — and if the caller then ends
            // the run, those rows are frozen by an operation that is over. That is
            // the freeze `inst-bs-done`'s "lock released either way" and D-37's
            // release path both exist to prevent, so the release happens here,
            // where the partial set is known to be exactly this run's.
            release_locks(runner, scope, tenant_id, operation_id).await?;

            // The key refused it, so somebody holds it. Read the holder — it is
            // committed, its run being `committing` — and name it.
            let Some(holder) = lock_holder(runner, scope, tenant_id, price_id).await? else {
                return Err(RepoError::Db(format!(
                    "insert pricing_bulk_row_lock for {price_id}: {e}"
                )));
            };
            return Err(RepoError::BulkRowLocked {
                price_id: price_id.to_string(),
                bulk_operation_id: holder.to_string(),
            });
        }
    }
    Ok(())
}

/// Release every lock the run holds, returning how many moved.
///
/// # Errors
/// [`RepoError::Db`] on a scope or storage failure.
pub async fn release_locks(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    operation_id: Uuid,
) -> Result<u64, RepoError> {
    let result = bulk_row_lock::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(bulk_row_lock::Column::TenantId.eq(tenant_id))
                .add(bulk_row_lock::Column::BulkOperationId.eq(operation_id)),
        )
        .exec(runner)
        .await
        .map_err(|e| RepoError::Db(format!("release pricing_bulk_row_lock: {e}")))?;
    Ok(result.rows_affected)
}

/// Which run holds this row, if any.
///
/// The read `fr-concurrent-edit` makes: an interactive edit asks before it
/// writes, so the refusal can name the run rather than surfacing a key collision.
///
/// # Errors
/// [`RepoError::Db`] on a scope or storage failure.
pub async fn lock_holder(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    price_id: Uuid,
) -> Result<Option<Uuid>, RepoError> {
    let row = bulk_row_lock::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(bulk_row_lock::Column::TenantId.eq(tenant_id))
                .add(bulk_row_lock::Column::PriceId.eq(price_id)),
        )
        .one(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read pricing_bulk_row_lock: {e}")))?;
    Ok(row.map(|row| row.bulk_operation_id))
}

/// One stored row, read into the vocabulary the domain uses.
fn record_of(row: &bulk_operation::Model) -> Result<BulkOperationRecord, RepoError> {
    let corrupt = |what: &str, value: &str| {
        RepoError::CorruptRow(format!(
            "pricing_bulk_operation {}: {what} `{value}` is outside its enumeration",
            row.operation_id
        ))
    };
    Ok(BulkOperationRecord {
        operation_id: row.operation_id,
        tenant_id: row.tenant_id,
        kind: BulkKind::parse(&row.kind).map_err(|_| corrupt("kind", &row.kind))?,
        state: BulkState::parse(&row.state).map_err(|_| corrupt("state", &row.state))?,
        client_key: row.client_key.clone(),
        report: row.report.clone(),
        submitted_by: row.submitted_by,
        submitted_at: row.submitted_at,
        completed_at: row.completed_at,
    })
}
