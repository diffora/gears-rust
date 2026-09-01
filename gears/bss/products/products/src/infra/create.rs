//! The shared create transaction — the one path every Product/SKU draft
//! lands through: the idempotency claim, the entity insert, its creation
//! outbox event and the stored answer, in ONE transaction (P-D-22, P-D-42).
//!
//! Infra-owned so both the REST create doors and the batch worker call the
//! same layer — the worker staging a bulk row runs the IDENTICAL
//! transaction an interactive create runs, which is the sentence the bulk
//! feature's correctness rests on — without the worker importing
//! `api::rest`. The doors pass a `render` function that turns the inserted
//! record into the response body their surface answers (and stores as the
//! idempotency answer); the worker, which claims no key and reads no body,
//! passes a discard render.

use axum::http::StatusCode;
use sea_orm::DbErr;
use serde_json::Value as JsonValue;
use uuid::Uuid;

use toolkit_db::secure::{AccessScope, TxConfig};
use toolkit_db::{DBProvider, DbError};

use crate::domain::error::DomainError;
use crate::infra::broker::EventSink;
use crate::infra::events;
use crate::infra::idempotency::{
    ClaimVerdict, IdempotencyClaimInput, claim_idempotency, record_idempotency_answer,
};
use crate::infra::storage::contention_db_err;
use crate::infra::storage::repo::{self, NewProduct, NewSku, ProductRecord, SkuRecord};

/// What a create door's mutation transaction produced.
///
/// Not generic over the entity record, and deliberately so. The idempotency
/// phase runs inside the mutation's own transaction (P-D-42) and can end it
/// before the entity exists, so this type has to be able to say "no row, and
/// here is why" without inventing one — and the created arm carries the
/// **rendered response body**, not the record, because that same body was
/// stored as the answer inside the transaction that built it
/// ([`record_idempotency_answer`]). A variant carrying the record instead
/// would leave the handler free to re-render a view that could differ from
/// the bytes a later replay serves; carrying the value makes the two the
/// same object. `internal_revision` rides beside it because it is the
/// `ETag`'s operand and is the one field the handler still needs that the
/// body cannot supply as a header.
pub(crate) enum CreateOutcome {
    /// The mutation ran: the created view as it will be answered, and the
    /// revision the `ETag` is minted from.
    Created {
        /// The row's `internal_revision`, the `ETag` operand
        /// (`crate::domain::concurrency`).
        internal_revision: i64,
        /// The response body, rendered inside the transaction and stored
        /// there as the idempotency answer when the request carried a key.
        body: JsonValue,
    },
    /// A stored answer was replayed; nothing was written.
    Replay {
        /// The stored status.
        status: i32,
        /// The stored body.
        body: JsonValue,
    },
    /// The idempotency phase refused; nothing was written.
    Refused(DomainError),
}

/// The status a create answers on success, and therefore the status a
/// replay of that create reproduces.
///
/// One spelling, read by the response the door builds **and** by the answer
/// it stores, so the two cannot drift: a stored status that was not the
/// status answered would make every later replay a different response from
/// the original.
pub(crate) const CREATE_RESPONSE_STATUS: StatusCode = StatusCode::CREATED;

/// Insert the entity row and enqueue its `ProductCreated` event, in one
/// transaction (`dod-create-doors`) — and nothing else. Split out of
/// [`create_product`] to keep that function's own body to the steps its doc
/// enumerates rather than their expansion.
///
/// Returns the raw [`DbError`] on failure rather than a [`CanonicalError`]:
/// [`create_product`] still needs the driver text this error carries to
/// distinguish a unique-index collision from an unrelated storage failure
/// (`classify_insert_conflict`), which a [`CanonicalError`] would already
/// have discarded.
///
/// # The claim runs here, on the mutation's own runner
///
/// `claim` is `Some` exactly when the request carried an `Idempotency-Key`
/// (a keyless request skips the phase, P-D-34), and its `INSERT` is executed
/// **inside this closure**, on the same `tx` the entity insert and the
/// outbox enqueue run on. That is P-D-42's whole requirement and this
/// function is where it is discharged: the claim, the entity row and the
/// event commit together or not at all, so a rollback frees the key with no
/// release step of its own. Moving the claim to a runner of its own — a
/// second `state.db.transaction(..)`, or `state.db.conn()` before this call
/// — would leave a claim behind for a mutation that never committed, which
/// is the defect `products_tests
/// ::a_rolled_back_mutation_frees_the_key_for_a_later_create` exists to
/// catch.
///
/// The claim is the closure's **first** statement, so a replay or a refusal
/// ends the transaction before the entity insert runs and nothing is
/// written on either path (P-D-38).
///
/// # The answer is the closure's **last** statement, and why it is last
///
/// `record_idempotency_answer` runs after the entity insert and after the
/// outbox enqueue, because it stores the response body and the body cannot
/// be rendered before the row it renders exists. Everything the answer
/// records is therefore already written when it runs, and it commits with
/// them — `inst-fd-idem-claim-write`'s "together or not at all". Ordering it
/// before the enqueue would be no safer and would store an answer for an
/// event that had not been queued yet; ordering it outside the closure would
/// leave a committed create with a `claimed` key, which is the state the
/// write exists to remove.
///
/// The body is rendered here, once, and travels out on
/// [`CreateOutcome::Created`] — the handler answers **that** value rather
/// than re-rendering the view, so what a later replay serves and what the
/// original caller was told are the same bytes.
///
/// # The mutation runs under `transaction_with_retry`, not a bare transaction
///
/// `DBProvider::transaction` has no contention retry, and the claim `INSERT`
/// being the gate (P-D-42) makes this transaction one that *concurrent
/// duplicates deliberately collide on*. On `SQLite` "the loser is answered
/// `SQLITE_BUSY` rather than blocking, so the door carries a busy timeout and
/// retries" (`design/01-foundation.md` §3.2 `inst-fd-idem-claim-txn`), and on
/// `PostgreSQL` the same collision can surface as a serialization failure.
/// Without a retry that transaction fails outright, and the failure carries
/// neither "unique constraint" nor "duplicate key", so `classify_insert_conflict`
/// does not recognise it either: the client gets a bare 500 instead of the
/// replay or the `409` the store promises it. `toolkit_db::Db::
/// transaction_with_retry` classifies both through
/// `toolkit_db::contention::is_retryable_contention`, and `contention_db_err`
/// is the accessor it asks the caller for.
///
/// **The classifier can only answer `true` because the driver's own error
/// survives the repository.** `is_retryable_contention` matches `DbErr::Exec`
/// and `DbErr::Query` and nothing else, so the flattening this closure used
/// to do — `DbErr::Custom(e.to_string())` over every `RepoError` — made every
/// contention failure unretryable while this section claimed the opposite.
/// This door was written first and both head-act doors inherited the same
/// wrap. It is closed in one place for all of them: the repository raises
/// `RepoError::Driver`, which carries `sea-orm`'s error unchanged, and every
/// door maps it through `RepoError::to_db_err` (the head-act doors through
/// `HeadActError::from_repo`) rather than through its `Display`.
///
/// **The closure is safe to re-run.** Its first statement is the claim, and
/// the claim rolls back with everything after it (P-D-38), so a retried
/// attempt starts against exactly the state the first one started against:
/// no key held, no entity row, no outbox row. Its **inputs** are
/// attempt-independent — `now` and `expires_at` were stamped before the
/// first. The body is `FnMut`, so the inputs are cloned per attempt rather
/// than moved in once.
///
/// **One written value is not attempt-independent**, and saying otherwise was
/// wrong: `infra::events` mints the envelope's `event_id` per enqueue
/// (`Uuid::new_v4()`), so a retried attempt writes a different one. That is
/// deliberate and harmless in this shape — the attempt only re-runs because
/// the prior one rolled back, so the prior id was never committed and reached
/// no consumer. It would stop being harmless if an id were ever minted
/// *outside* the transaction and carried in.
/// The records that **must join the guarded mutation's transaction**, carried
/// as one value because they travel for one reason.
///
/// P-D-42's shape — *"The claim `INSERT` **is** the gate and **MUST** join the
/// guarded mutation's transaction"* — is what puts the idempotency claim
/// here, and `design/09` §4 puts the bulk ledger's row beside it for the same
/// reason: a record that is the act's own outcome cannot commit separately
/// from the act. Both are optional; a plain wire create carries neither.
#[derive(Clone, Default)]
pub(crate) struct JoinedRecords {
    /// The idempotency claim, where the request carried a key.
    pub claim: Option<IdempotencyClaimInput>,
    /// The bulk ledger's row, where the caller is the batch worker.
    pub stamp: Option<BulkRowStamp>,
}

/// A bulk-ledger row to stamp **inside the create transaction**.
///
/// The batch worker used to stamp its ledger row on a second connection after
/// the create had committed, and a crash in that window left the entity
/// created and the row unstamped — the resume then re-staged it, hit the name
/// or code reservation, and recorded a **terminal `DUPLICATE_NAME` on a row
/// that had in fact succeeded**, with its draft unreachable from the ledger.
///
/// `design/09` §4 makes the ledger row the lane's stored outcome record, and
/// P-D-42's shape for such a record is that it *"**MUST** join the guarded
/// mutation's transaction"*. So the stamp travels with the create rather than
/// after it. The coupling is deliberate and narrow: `infra::create` learns one
/// optional ledger coordinate, and the alternative is the two-transaction
/// window above.
#[derive(Clone, Debug)]
pub(crate) struct BulkRowStamp {
    /// The batch the row belongs to.
    pub batch_id: uuid::Uuid,
    /// The row's key within it.
    pub row_key: String,
    /// The act's instant.
    pub now: chrono::DateTime<chrono::Utc>,
}

pub(crate) async fn insert_product_with_event(
    db: &DBProvider<DbError>,
    sink: &EventSink,
    scope: AccessScope,
    new: NewProduct,
    joined: JoinedRecords,
    actor_ref: Uuid,
    render: fn(ProductRecord) -> Result<JsonValue, serde_json::Error>,
) -> Result<CreateOutcome, DbError> {
    let outbox = sink.clone();
    let JoinedRecords { claim, stamp } = joined;
    let tenant_id = new.tenant_id;
    db.db()
        .transaction_with_retry::<CreateOutcome, DbError, _, _>(
            TxConfig::default(),
            contention_db_err,
            move |tx| {
                // `FnMut`: every attempt gets its own copies, so a retried
                // attempt never finds an input the previous one consumed.
                // The inputs are attempt-independent — the claim's
                // `now`/`expires_at` were stamped before the first one. The
                // envelope's `event_id` is not: it is minted per enqueue, so
                // a retried attempt writes a different one. Harmless here —
                // the rolled-back attempt's id was never committed.
                let outbox = outbox.clone();
                let scope = scope.clone();
                let new = new.clone();
                let claim = claim.clone();
                let stamp = stamp.clone();
                Box::pin(async move {
                    if let Some(input) = claim.as_ref() {
                        match claim_idempotency(tx, &scope, tenant_id, input)
                            .await
                            .map_err(|e| DbError::Sea(e.to_db_err()))?
                        {
                            ClaimVerdict::Proceed => {}
                            ClaimVerdict::Replay { status, body } => {
                                return Ok(CreateOutcome::Replay { status, body });
                            }
                            ClaimVerdict::Refused(refusal) => {
                                return Ok(CreateOutcome::Refused(refusal));
                            }
                        }
                    }

                    let record = repo::insert_product(tx, &scope, new)
                        .await
                        .map_err(|e| DbError::Sea(e.to_db_err()))?;
                    let stamped_id = record.product_id;

                    let core = events::EventBodyCore {
                        tenant_id: record.tenant_id,
                        entity_kind: events::EntityKind::Product.as_str(),
                        entity_id: record.product_id,
                        internal_revision: record.internal_revision,
                        lifecycle_state: record.lifecycle_state.as_str(),
                    };
                    events::enqueue(
                        &outbox,
                        tx,
                        record.product_id,
                        events::PRODUCT_CREATED_PAYLOAD_TYPE,
                        &core,
                        actor_ref,
                    )
                    .await
                    .map_err(|e| {
                        DbError::Sea(DbErr::Custom(format!("enqueue ProductCreated: {e}")))
                    })?;

                    let internal_revision = record.internal_revision;
                    let body = render(record).map_err(|e| {
                        DbError::Sea(DbErr::Custom(format!("render the created Product: {e}")))
                    })?;

                    if let Some(input) = claim.as_ref() {
                        record_idempotency_answer(
                            tx,
                            &scope,
                            tenant_id,
                            input,
                            CREATE_RESPONSE_STATUS,
                            &body,
                        )
                        .await
                        .map_err(|e| DbError::Sea(e.to_db_err()))?;
                    }

                    // The bulk ledger's stamp, on this transaction: the row
                    // and the entity commit together or not at all.
                    if let Some(stamp) = stamp.as_ref() {
                        repo::record_bulk_row_outcome(
                            tx,
                            &scope,
                            tenant_id,
                            stamp.batch_id,
                            &stamp.row_key,
                            repo::BulkRowOutcome {
                                entity_id: Some(stamped_id),
                                disposition: None,
                                code: None,
                                now: stamp.now,
                            },
                        )
                        .await
                        .map_err(|e| DbError::Sea(e.to_db_err()))?;
                    }

                    Ok(CreateOutcome::Created {
                        internal_revision,
                        body,
                    })
                })
            },
        )
        .await
}

/// Insert the entity row and enqueue its `SkuCreated` event, in one
/// transaction (`dod-create-doors`) — and nothing else. The SKU door's own
/// copy of `products::create_product`'s
/// `insert_product_with_event` — see this module's doc, "What is duplicated
/// from the Product door, and why", for why this is not a shared function.
///
/// Returns the raw [`DbError`] on failure rather than a [`CanonicalError`]:
/// [`create_sku`] still needs the driver text this error carries to
/// distinguish a `sku_code` collision from an unrelated storage failure
/// (`classify_sku_insert_conflict`), which a [`CanonicalError`] would already
/// have discarded.
///
/// # The claim runs here, on the mutation's own runner
///
/// `claim` is `Some` exactly when the request carried an `Idempotency-Key`,
/// and its `INSERT` executes inside this closure on the same `tx` the entity
/// insert and the outbox enqueue use — **P-D-42**'s requirement, so that a
/// rollback frees the key with no release step. See
/// [`crate::api::rest::products`]'s `insert_product_with_event` for the same
/// obligation stated in full, and `crate::api::rest::claim_idempotency` for
/// why a runner of its own would break the one property this mechanism
/// exists to provide.
///
/// # The answer runs here too, last
///
/// `record_idempotency_answer` runs after the entity insert and the outbox
/// enqueue, on that same `tx`: it stores the response body, and the body
/// cannot be rendered before the row it renders exists. Claim, mutation and
/// answer therefore commit together or not at all
/// (`inst-fd-idem-claim-write`), and the value stored is the very value
/// [`create_sku`] answers, carried out on [`CreateOutcome::Created`] rather
/// than re-rendered for the wire.
///
/// # The mutation runs under `transaction_with_retry`, not a bare transaction
///
/// `DBProvider::transaction` has no contention retry, and the claim `INSERT`
/// being the gate (P-D-42) makes this transaction one that *concurrent
/// duplicates deliberately collide on*. On `SQLite` "the loser is answered
/// `SQLITE_BUSY` rather than blocking, so the door carries a busy timeout and
/// retries" (`design/01-foundation.md` §3.2 `inst-fd-idem-claim-txn`), and on
/// `PostgreSQL` the same collision can surface as a serialization failure.
/// Without a retry that transaction fails outright, and the failure carries
/// neither "unique constraint" nor "duplicate key", so `classify_insert_conflict`
/// does not recognise it either: the client gets a bare 500 instead of the
/// replay or the `409` the store promises it. `toolkit_db::Db::
/// transaction_with_retry` classifies both through
/// `toolkit_db::contention::is_retryable_contention`, and `contention_db_err`
/// is the accessor it asks the caller for.
///
/// **The classifier can only answer `true` because the driver's own error
/// survives the repository.** `is_retryable_contention` matches `DbErr::Exec`
/// and `DbErr::Query` and nothing else, so the flattening this closure used
/// to do — `DbErr::Custom(e.to_string())` over every `RepoError` — made every
/// contention failure unretryable while this section claimed the opposite.
/// This door was written first and both head-act doors inherited the same
/// wrap. It is closed in one place for all of them: the repository raises
/// `RepoError::Driver`, which carries `sea-orm`'s error unchanged, and every
/// door maps it through `RepoError::to_db_err` (the head-act doors through
/// `HeadActError::from_repo`) rather than through its `Display`.
///
/// **The closure is safe to re-run.** Its first statement is the claim, and
/// the claim rolls back with everything after it (P-D-38), so a retried
/// attempt starts against exactly the state the first one started against:
/// no key held, no entity row, no outbox row. Its **inputs** are
/// attempt-independent — `now` and `expires_at` were stamped before the
/// first. The body is `FnMut`, so the inputs are cloned per attempt rather
/// than moved in once.
///
/// **One written value is not attempt-independent**, and saying otherwise was
/// wrong: `infra::events` mints the envelope's `event_id` per enqueue
/// (`Uuid::new_v4()`), so a retried attempt writes a different one. That is
/// deliberate and harmless in this shape — the attempt only re-runs because
/// the prior one rolled back, so the prior id was never committed and reached
/// no consumer. It would stop being harmless if an id were ever minted
/// *outside* the transaction and carried in.
pub(crate) async fn insert_sku_with_event(
    db: &DBProvider<DbError>,
    sink: &EventSink,
    scope: AccessScope,
    new: NewSku,
    joined: JoinedRecords,
    actor_ref: Uuid,
    render: fn(SkuRecord) -> Result<JsonValue, serde_json::Error>,
) -> Result<CreateOutcome, DbError> {
    let outbox = sink.clone();
    let JoinedRecords { claim, stamp } = joined;
    let tenant_id = new.tenant_id;
    db.db()
        .transaction_with_retry::<CreateOutcome, DbError, _, _>(
            TxConfig::default(),
            contention_db_err,
            move |tx| {
                // `FnMut`: every attempt gets its own copies, so a retried
                // attempt never finds an input the previous one consumed.
                // The inputs are attempt-independent — the claim's
                // `now`/`expires_at` were stamped before the first one. The
                // envelope's `event_id` is not: it is minted per enqueue, so
                // a retried attempt writes a different one. Harmless here —
                // the rolled-back attempt's id was never committed.
                let outbox = outbox.clone();
                let scope = scope.clone();
                let new = new.clone();
                let claim = claim.clone();
                let stamp = stamp.clone();
                Box::pin(async move {
                    if let Some(input) = claim.as_ref() {
                        match claim_idempotency(tx, &scope, tenant_id, input)
                            .await
                            .map_err(|e| DbError::Sea(e.to_db_err()))?
                        {
                            ClaimVerdict::Proceed => {}
                            ClaimVerdict::Replay { status, body } => {
                                return Ok(CreateOutcome::Replay { status, body });
                            }
                            ClaimVerdict::Refused(refusal) => {
                                return Ok(CreateOutcome::Refused(refusal));
                            }
                        }
                    }

                    let record = repo::insert_sku(tx, &scope, new)
                        .await
                        .map_err(|e| DbError::Sea(e.to_db_err()))?;
                    let stamped_id = record.sku_id;

                    let core = events::EventBodyCore {
                        tenant_id: record.tenant_id,
                        entity_kind: events::EntityKind::Sku.as_str(),
                        entity_id: record.sku_id,
                        internal_revision: record.internal_revision,
                        lifecycle_state: record.lifecycle_state.as_str(),
                    };
                    events::enqueue(
                        &outbox,
                        tx,
                        record.sku_id,
                        events::SKU_CREATED_PAYLOAD_TYPE,
                        &core,
                        actor_ref,
                    )
                    .await
                    .map_err(|e| DbError::Sea(DbErr::Custom(format!("enqueue SkuCreated: {e}"))))?;

                    let internal_revision = record.internal_revision;
                    let body = render(record).map_err(|e| {
                        DbError::Sea(DbErr::Custom(format!("render the created SKU: {e}")))
                    })?;

                    if let Some(input) = claim.as_ref() {
                        record_idempotency_answer(
                            tx,
                            &scope,
                            tenant_id,
                            input,
                            CREATE_RESPONSE_STATUS,
                            &body,
                        )
                        .await
                        .map_err(|e| DbError::Sea(e.to_db_err()))?;
                    }

                    // The bulk ledger's stamp, on this transaction: the row
                    // and the entity commit together or not at all.
                    if let Some(stamp) = stamp.as_ref() {
                        repo::record_bulk_row_outcome(
                            tx,
                            &scope,
                            tenant_id,
                            stamp.batch_id,
                            &stamp.row_key,
                            repo::BulkRowOutcome {
                                entity_id: Some(stamped_id),
                                disposition: None,
                                code: None,
                                now: stamp.now,
                            },
                        )
                        .await
                        .map_err(|e| DbError::Sea(e.to_db_err()))?;
                    }

                    Ok(CreateOutcome::Created {
                        internal_revision,
                        body,
                    })
                })
            },
        )
        .await
}
