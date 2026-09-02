//! The batch worker — `design/09-bulk-promotion.md` §2's stage phase and
//! §4's edge 1 (`dod-stage-phase`; P-D-54, P-D-86).
//!
//! # One worker, one claim, the ceiling re-checked where it can drift
//!
//! [`sweep`] discovers tenants holding `staging` batches under the system
//! scope and narrows to `for_tenant` for the work — the sibling jobs'
//! documented pattern. A batch is taken by a **compare-and-swap claim** on
//! `(state, attempt)`, so two workers racing one batch cannot both believe
//! they hold it, and the tenant's concurrent-batch **ceiling is re-checked
//! at claim** rather than only at the door (**P-D-54**: a ceiling checked
//! only by the door drifts as batches hang).
//!
//! # Staging is the ordinary create door, never a parallel rule set
//!
//! Product and SKU rows are parsed from their `staged_payload` (**P-D-86**)
//! and land as **drafts through the Foundation's own insert path** —
//! `insert_product_with_event` / `insert_sku_with_event`, the very
//! functions the interactive create doors call, so the entity row, its
//! outbox event and its transaction are one act and identical to an
//! interactive create's. **This is the sentence the feature's correctness
//! rests on**: a bulk row that skipped a validator interactive authoring
//! runs would make bulk a governance bypass by omission.
//!
//! What is deliberately **not** here: the live-entity row classes
//! (categories, definitions, recognized-set members) whose stores are
//! `02`/`03`'s and do not ship; the dependency ordering those classes make
//! observable, since with one class there is nothing to order; and the
//! **commit phase**, which `dod-commit-phase` owns and §7 rows 7, 10 and 23
//! block.
//!
//! # A staged row is not a disposed row
//!
//! §1.7's terminal mix is `{published, applied, no_op, failed}` and a
//! staged draft is none of them: a row that lands keeps `disposition NULL`
//! with its `entity_id` stamped, staying in flight for the commit phase to
//! dispose of, while a row the create path refuses takes `failed` with the
//! **owning feature's code verbatim** — bulk introducing no parallel
//! taxonomy. The ledger's own trigger then freezes it.
//!
//! # The resume operand is the ledger, not a marker
//!
//! A re-claimed batch skips every row whose `entity_id` is already stamped
//! or whose disposition is terminal: the ledger IS the record, the same
//! shape the family clone's resume takes (P-D-72). So a crash mid-stage
//! costs the rows it had not reached and nothing else.
//!
//! # Edge 1
//!
//! When every row of the batch is staged or failed, the same pass flips
//! `staging -> reported` under a predicate naming the state it must be in
//! (P-D-54's edge 1). The remaining six edges belong to the approval and
//! the commit phase and are not walked here.
//!
//! # The claim's lease is what makes it a claim
//!
//! `(state, attempt)` alone excludes only a racer reading the **same**
//! attempt. A peer starting a second later reads the bumped attempt and its
//! compare succeeds — **inside** the first pass, two workers staging one
//! batch from two snapshots. P-D-54 calls `claimed_at` the claim's *lease*,
//!
//! # The two `DoD`s these edges reach, and why neither is claimed
//!
//! `dod-resume-abandon`: the **abandon** half ships whole — the
//! `reported → abandoned` edge with its single entry, one path per row kind,
//! the `batch-abandoned` literal — and is probed. Its **resume** half says
//! *"a crash mid-commit resumes from the ledger"*, and the commit phase does
//! not exist: `inst-bk-commit` requires each row to publish in
//! `PreAuthorized(approvalId)` mode, which the shipped gate host refuses
//! outright because no approval record store is registered. So the `DoD` is
//! reached and not met.
//!
//! `dod-coalesced-event`: the summary, its CAS and the exactly-once
//! guarantee ship and are falsified. What no document defines is the
//! **ledger digest** the event must carry — §7 row 31 — and the covered set
//! this executor renders is its own choice. A tick would claim an operand
//! the set has not specified.
//!
//! @cpt-cf-bss-products-dod-resume-abandon
//! @cpt-cf-bss-products-dod-coalesced-event
//! and [`STAGE_LEASE`] is what makes that column readable rather than
//! merely written: a batch claimed within the lease is not re-claimable,
//! and one whose worker died becomes claimable when the lease lapses.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-stage-phase:p1

/// How long a claim holds a batch against a peer.
///
/// Ten minutes: long enough for a large batch's pass to finish inside it
/// (the sizing fixture is 10 000 rows), short enough that a worker killed
/// mid-pass releases the batch without an operator. A pass that outruns it
/// meets the case the resume operand exists for — the ledger records what
/// landed.
const STAGE_LEASE: chrono::Duration = chrono::Duration::minutes(10);

use chrono::{DateTime, Utc};
use serde_json::Value as JsonValue;
use toolkit_db::secure::AccessScope;
use uuid::Uuid;

use crate::domain::error::DomainError;
use crate::domain::name;
use crate::domain::validation::ValidationReport;
use crate::infra::storage::RepoError;
use crate::infra::storage::repo::{self, BulkRowOutcome, NewProduct, NewSku};

/// Everything the batch worker needs, carried on its own type rather than
/// on `api::rest::ApiState`: the worker is infra and the REST layer is not
/// its dependency — `gear.rs`, the composition root, builds this from the
/// same boot state the doors get theirs from.
pub(crate) struct BulkWorkerContext {
    /// The provider the claims, reads and per-row transactions run on.
    pub(crate) db: toolkit_db::DBProvider<toolkit_db::DbError>,
    /// The outbox the create path enqueues creation events through.
    pub(crate) sink: crate::infra::broker::EventSink,
    /// `inst-bm-limits`' second operand, re-checked at claim (P-D-54).
    pub(crate) bulk_max_concurrent_batches_per_tenant: u32,
}

/// Why one row's staging failed — the branch operand the row loop needs: a
/// refusal is the row's own terminal disposition, a storage failure is the
/// pass's and never a verdict on the row.
enum StageRowError {
    /// The create path refused the row: terminal, recorded on the ledger
    /// with the owning feature's code verbatim.
    Refused(DomainError),
    /// The insert failed below the domain — transient or structural
    /// storage trouble. The pass propagates it, the row keeps
    /// `disposition NULL`, and the ledger resume retries it on a later
    /// attempt instead of terminally failing it for a fault it did not
    /// cause.
    Storage(RepoError),
}

/// What one pass over one batch did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StageOutcome {
    /// No `staging` batch for this tenant.
    NoBatch,
    /// A peer holds the batch, or it moved under this pass.
    ClaimLost,
    /// The tenant is over its concurrent-batch ceiling; the batch waits
    /// rather than being failed (P-D-54's re-check is a brake, not a
    /// verdict).
    CeilingHeld,
    /// The batch staged and reported.
    Reported {
        /// The batch.
        batch_id: Uuid,
        /// How many rows landed as drafts.
        staged: usize,
        /// How many the create path refused.
        failed: usize,
    },
}

/// One string field out of a staged payload.
fn field(payload: &JsonValue, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// Stage one Product row through the Foundation's own insert path.
async fn stage_product(
    ctx: &BulkWorkerContext,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor_ref: Uuid,
    payload: &JsonValue,
    now: DateTime<Utc>,
    stamp: Option<crate::infra::create::BulkRowStamp>,
) -> Result<Uuid, StageRowError> {
    let mut report = ValidationReport::new();
    let name_value = field(payload, "name");
    let brand = field(payload, "brand_id").and_then(|raw| Uuid::parse_str(&raw).ok());
    if name_value.is_none() {
        report.violate("VALIDATION", "name", "name must not be blank");
    }
    if brand.is_none() {
        report.violate("VALIDATION", "brand_id", "brand_id must be a uuid");
    }
    if !report.is_empty() {
        return Err(StageRowError::Refused(DomainError::Validation(report)));
    }
    let (name_value, brand_id) = (
        name_value.unwrap_or_default(),
        brand.unwrap_or_else(Uuid::nil),
    );

    let product_id = Uuid::new_v4();
    let name_normalized = name::normalize(&name_value);
    let new = NewProduct {
        product_id,
        tenant_id,
        brand_id,
        name: name_value,
        name_normalized,
        product_code: field(payload, "product_code"),
        region_scope: field(payload, "region_scope").unwrap_or_default(),
        brand_scope: field(payload, "brand_scope").unwrap_or_default(),
        created_by: actor_ref.to_string(),
        created_at: now,
        cloned_from: None,
        cloned_from_version: None,
    };
    match crate::infra::create::insert_product_with_event(
        &ctx.db,
        &ctx.sink,
        scope.clone(),
        new,
        crate::infra::create::JoinedRecords { claim: None, stamp },
        actor_ref,
        discard_render,
    )
    .await
    {
        Ok(_) => Ok(product_id),
        Err(db_error) => Err(insert_failure(&db_error, "product")),
    }
}

/// The worker's render: it claims no idempotency key and reads no response
/// body, so the created record renders to `Null` instead of dragging a
/// wire view into this layer.
// The Result is the render fn-pointer's contract (`infra::create`), not
// this function's choice.
#[allow(clippy::unnecessary_wraps)]
fn discard_render<T>(_record: T) -> Result<JsonValue, serde_json::Error> {
    Ok(JsonValue::Null)
}

/// Stage one SKU row through the Foundation's own insert path.
async fn stage_sku(
    ctx: &BulkWorkerContext,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor_ref: Uuid,
    payload: &JsonValue,
    now: DateTime<Utc>,
    stamp: Option<crate::infra::create::BulkRowStamp>,
) -> Result<Uuid, StageRowError> {
    let mut report = ValidationReport::new();
    let code = field(payload, "sku_code");
    let parent = field(payload, "product_id").and_then(|raw| Uuid::parse_str(&raw).ok());
    if code.is_none() {
        report.violate("VALIDATION", "sku_code", "sku_code must not be blank");
    }
    if parent.is_none() {
        report.violate("VALIDATION", "product_id", "product_id must be a uuid");
    }
    if !report.is_empty() {
        return Err(StageRowError::Refused(DomainError::Validation(report)));
    }

    let sku_id = Uuid::new_v4();
    let new = NewSku {
        sku_id,
        tenant_id,
        product_id: parent.unwrap_or_else(Uuid::nil),
        sku_code: code.unwrap_or_default(),
        region_scope: field(payload, "region_scope").unwrap_or_default(),
        brand_scope: field(payload, "brand_scope").unwrap_or_default(),
        created_by: actor_ref.to_string(),
        created_at: now,
        cloned_from: None,
        cloned_from_version: None,
    };
    match crate::infra::create::insert_sku_with_event(
        &ctx.db,
        &ctx.sink,
        scope.clone(),
        new,
        crate::infra::create::JoinedRecords { claim: None, stamp },
        actor_ref,
        discard_render,
    )
    .await
    {
        Ok(_) => Ok(sku_id),
        Err(db_error) => Err(insert_failure(&db_error, "sku")),
    }
}

/// Classify an insert failure into the **owning feature's** code — bulk
/// introduces no parallel taxonomy, so a duplicate is the Foundation's own
/// `DUPLICATE_NAME`/`DUPLICATE_CODE`.
///
/// The classification is on the driver's own typed [`sea_orm::SqlErr`],
/// never on a stringified message: only a typed unique-constraint violation
/// is a row verdict, and everything else — a dropped connection, a
/// serialization failure, an enqueue fault — is a storage failure the pass
/// propagates with its cause preserved, instead of collapsing it into a
/// terminal "the row could not be staged" the operator cannot tell from
/// bad data.
fn insert_failure(db_error: &toolkit_db::DbError, kind: &str) -> StageRowError {
    if let toolkit_db::DbError::Sea(sea) = db_error
        && let Some(sea_orm::SqlErr::UniqueConstraintViolation(detail)) = sea.sql_err()
    {
        // The constraint name inside a TYPED unique violation tells the
        // two reservation indexes apart.
        if detail.to_ascii_lowercase().contains("name") {
            return StageRowError::Refused(DomainError::DuplicateName(format!(
                "{kind} name is already reserved"
            )));
        }
        return StageRowError::Refused(DomainError::DuplicateCode(format!(
            "{kind} code is already reserved"
        )));
    }
    StageRowError::Storage(RepoError::Db(format!("stage one {kind}: {db_error}")))
}

/// The stored payload, parsed — or the pass's own refusal: the door stored
/// a canonically serialized object, so a present-but-unparseable payload is
/// store corruption, never operator data. Failed CLOSED with the row named,
/// not coerced to Null and refused as a misleading blank-field validation.
fn parse_staged_payload(
    tenant_id: Uuid,
    batch_id: Uuid,
    row: &repo::BulkRowRecord,
) -> Result<JsonValue, RepoError> {
    match row.staged_payload.as_deref() {
        None => Ok(JsonValue::Null),
        Some(raw) => serde_json::from_str(raw).map_err(|parse_err| {
            tracing::error!(
                %tenant_id,
                %batch_id,
                row_id = %row.row_id,
                error = %parse_err,
                "bss-products: staged_payload failed to parse; the stored row is corrupt"
            );
            RepoError::CorruptRow(format!(
                "staged_payload of row {} in batch {batch_id} is not valid JSON: {parse_err}",
                row.row_id
            ))
        }),
    }
}

/// Stage one in-flight row and record its ledger outcome: `true` when the
/// row landed as a draft, `false` when the create path refused it
/// terminally.
///
/// # Errors
///
/// [`RepoError`] on a storage failure — never a verdict on the row: the row
/// keeps `disposition NULL` and the ledger resume retries it on the next
/// claim.
async fn stage_one_row(
    ctx: &BulkWorkerContext,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor_ref: Uuid,
    batch_id: Uuid,
    row: &repo::BulkRowRecord,
    now: DateTime<Utc>,
) -> Result<bool, RepoError> {
    let payload = parse_staged_payload(tenant_id, batch_id, row)?;
    // The ledger stamp rides the create's own transaction (P-D-42's shape):
    // the entity and its ledger row commit together, so a crash between them
    // is not a state.
    let stamp = Some(crate::infra::create::BulkRowStamp {
        batch_id,
        row_key: row.row_key.clone(),
        now,
    });
    let outcome = match row.entity_kind.as_str() {
        "product" => stage_product(ctx, scope, tenant_id, actor_ref, &payload, now, stamp).await,
        "sku" => stage_sku(ctx, scope, tenant_id, actor_ref, &payload, now, stamp).await,
        other => Err(StageRowError::Refused(DomainError::Validation({
            let mut report = ValidationReport::new();
            report.violate(
                "VALIDATION",
                "entity_kind",
                format!("no staging path for entity_kind {other}"),
            );
            report
        }))),
    };

    let conn = ctx
        .db
        .conn()
        .map_err(|e| RepoError::Db(format!("row outcome connection: {e}")))?;
    match outcome {
        Ok(_entity_id) => {
            // The success stamp already landed, inside the create's own
            // transaction (`BulkRowStamp`). Writing it again here would be
            // the second transaction this fix removed.
            Ok(true)
        }
        Err(StageRowError::Refused(refusal)) => {
            repo::record_bulk_row_outcome(
                &conn,
                scope,
                tenant_id,
                batch_id,
                &row.row_key,
                BulkRowOutcome {
                    entity_id: None,
                    disposition: Some("failed"),
                    code: Some(refusal.code()),
                    reason: None,
                    now,
                },
            )
            .await?;
            Ok(false)
        }
        Err(StageRowError::Storage(storage)) => {
            // Not a row verdict: the pass aborts with the cause named.
            tracing::error!(
                %tenant_id,
                %batch_id,
                row_id = %row.row_id,
                error = %storage,
                "bss-products: staging a row failed below the domain; the pass aborts"
            );
            Err(storage)
        }
    }
}

/// One pass over one tenant's oldest `staging` batch.
///
/// # Errors
///
/// [`RepoError`] as the reads and writes raise it.
pub(crate) async fn stage_next_batch(
    ctx: &BulkWorkerContext,
    tenant_id: Uuid,
    actor_ref: Uuid,
    now: DateTime<Utc>,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<StageOutcome, RepoError> {
    let scope = AccessScope::for_tenant(tenant_id);
    let (batch, rows) = {
        let conn = ctx
            .db
            .conn()
            .map_err(|e| RepoError::Db(format!("batch worker connection: {e}")))?;
        let live = repo::count_live_batches(&conn, &scope, tenant_id).await?;
        if live > u64::from(ctx.bulk_max_concurrent_batches_per_tenant) {
            return Ok(StageOutcome::CeilingHeld);
        }
        let Some(batch) = repo::staging_batches(&conn, &scope, tenant_id)
            .await?
            .into_iter()
            .next()
        else {
            return Ok(StageOutcome::NoBatch);
        };
        let rows = repo::find_batch_rows(&conn, &scope, tenant_id, batch.batch_id).await?;
        (batch, rows)
    };

    {
        let conn = ctx
            .db
            .conn()
            .map_err(|e| RepoError::Db(format!("batch claim connection: {e}")))?;
        if !repo::claim_bulk_batch(
            &conn,
            &scope,
            tenant_id,
            batch.batch_id,
            batch.attempt,
            now,
            STAGE_LEASE,
        )
        .await?
        {
            return Ok(StageOutcome::ClaimLost);
        }
    }

    let mut staged = 0usize;
    let mut failed = 0usize;
    for row in rows {
        // The shutdown seam: a batch can carry tens of thousands of rows,
        // and the gear's stop_timeout must not have to wait for all of
        // them. The ledger is the record, so a pass cut here costs
        // nothing — the next claim resumes from the rows it had not
        // reached.
        if cancel.is_cancelled() {
            return Ok(StageOutcome::ClaimLost);
        }
        // The resume operand: a row already carrying an entity or a
        // terminal disposition was staged by an earlier attempt.
        if row.entity_id.is_some() || row.disposition.is_some() {
            if row.disposition.is_some() {
                failed += 1;
            } else {
                staged += 1;
            }
            continue;
        }
        if stage_one_row(ctx, &scope, tenant_id, actor_ref, batch.batch_id, &row, now).await? {
            staged += 1;
        } else {
            failed += 1;
        }
    }

    // Edge 1 (P-D-54): the pass that stages the last row reports the batch.
    let conn = ctx
        .db
        .conn()
        .map_err(|e| RepoError::Db(format!("batch report connection: {e}")))?;
    repo::move_bulk_batch_state(
        &conn,
        &scope,
        tenant_id,
        batch.batch_id,
        crate::domain::states::BatchState::Staging,
        crate::domain::states::BatchState::Reported,
        now,
    )
    .await?;

    Ok(StageOutcome::Reported {
        batch_id: batch.batch_id,
        staged,
        failed,
    })
}

/// One sweep over every tenant holding a `staging` batch.
///
/// One tenant's failure is logged and the loop continues: tenants iterate
/// in sorted order, so propagating the first error would let one tenant
/// with a deterministic fault (a corrupt staged row, say) permanently
/// block every tenant that sorts after it.
///
/// # Errors
///
/// The last [`RepoError`] raised — only when EVERY tenant's pass failed,
/// which is a whole-sweep fault rather than one tenant's.
/// What an abandon did, in the readings the operator and the ledger need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AbandonOutcome {
    /// The batch was not in `reported`, so the edge did not fire and nothing
    /// was touched. The `reported → abandoned` edge is the machine's only
    /// entry to that state (P-D-69 arm 1), which is what keeps a committing
    /// batch from being abandoned out from under its own row publishes.
    NotReported,
    /// The batch moved to `abandoned` and its rows were disposed of.
    Abandoned {
        /// Rows whose created draft was discarded.
        discarded: usize,
        /// Rows whose pending live-entity operation was dropped, never
        /// applied.
        dropped: usize,
        /// Rows the ledger had already closed, or that staging never
        /// materialised — nothing to undo either way.
        untouched: usize,
    },
}

/// Abandon a reported batch: the `reported → abandoned` edge, then each
/// row's own path (`dod-resume-abandon`, `inst-bb-edge-abandon`).
///
/// # The edge fires first, and that ordering is the guard
///
/// The CAS runs before any row is touched, so a batch a peer already moved
/// leaves this call as [`AbandonOutcome::NotReported`] having written
/// nothing. Disposing rows first and flipping after would let two abandons
/// both discard.
///
/// # No new door, and one path per row kind
///
/// Created drafts **discard** through the repository write the discard door
/// itself uses — the same relationship staging has to the create door, which
/// calls `infra::create` rather than its own HTTP surface. Pending
/// live-entity operations are **dropped**: the ledger records the outcome and
/// nothing is applied. **Update-as-draft rows would revert** through the
/// ordinary save with the last frozen version's content; the import door
/// mints only created drafts today, so no row of that kind can exist yet and
/// `domain::batch::abandon_disposition` carries the arm rather than this
/// executor pretending to exercise it.
///
/// # The reason is a literal
///
/// Every touched row records `batch-abandoned` (**P-D-50**) — a constant from
/// the closed set the migration's `CHECK` pins, never operator text.
///
/// # Errors
///
/// [`RepoError`] as the reads and writes raise it.
#[allow(
    dead_code,
    reason = "the trigger is 05's approval rejection, which does not ship"
)]
pub(crate) async fn abandon_batch(
    ctx: &BulkWorkerContext,
    tenant_id: Uuid,
    batch_id: Uuid,
    now: DateTime<Utc>,
) -> Result<AbandonOutcome, RepoError> {
    let scope = AccessScope::for_tenant(tenant_id);
    let conn = ctx
        .db
        .conn()
        .map_err(|e| RepoError::Db(format!("batch abandon connection: {e}")))?;

    if !repo::move_bulk_batch_state(
        &conn,
        &scope,
        tenant_id,
        batch_id,
        crate::domain::states::BatchState::Reported,
        crate::domain::states::BatchState::Abandoned,
        now,
    )
    .await?
    {
        return Ok(AbandonOutcome::NotReported);
    }

    let rows = repo::find_batch_rows(&conn, &scope, tenant_id, batch_id).await?;
    let mut discarded = 0usize;
    let mut dropped = 0usize;
    let mut untouched = 0usize;
    for row in rows {
        match crate::domain::batch::abandon_disposition(
            &row.entity_kind,
            row.disposition.is_some(),
            row.entity_id.is_some(),
            false,
        ) {
            crate::domain::batch::AbandonDisposition::AlreadyTerminal => {
                untouched += 1;
                continue;
            }
            crate::domain::batch::AbandonDisposition::DiscardDraft => {
                let Some(entity_id) = row.entity_id else {
                    untouched += 1;
                    continue;
                };
                // The staged draft is at revision 1 with `published_version
                // = 0` — the discard write's own admitted shape. A row whose
                // head moved under the batch answers `Unmatched`, and the
                // ledger records the abandon regardless: the head is the
                // operator's now, and re-discarding it is not this
                // procedure's to force.
                let write = match row.entity_kind.as_str() {
                    "product" => {
                        repo::discard_product_head(&conn, &scope, tenant_id, entity_id, 1, now)
                            .await?
                    }
                    _ => {
                        repo::discard_sku_head(&conn, &scope, tenant_id, entity_id, 1, now).await?
                    }
                };
                if write == repo::HeadWrite::Applied {
                    discarded += 1;
                } else {
                    untouched += 1;
                }
            }
            crate::domain::batch::AbandonDisposition::DropPendingOp => dropped += 1,
            crate::domain::batch::AbandonDisposition::RevertToPublished => {
                // No import path mints one of these yet; counted as dropped
                // rather than silently discarded, so a row kind arriving
                // later cannot be swept into the wrong bucket unnoticed.
                dropped += 1;
            }
        }
        repo::record_bulk_row_outcome(
            &conn,
            &scope,
            tenant_id,
            batch_id,
            &row.row_key,
            BulkRowOutcome {
                entity_id: None,
                disposition: Some("no_op"),
                code: None,
                reason: Some(crate::domain::batch::ABANDON_REASON),
                now,
            },
        )
        .await?;
    }

    Ok(AbandonOutcome::Abandoned {
        discarded,
        dropped,
        untouched,
    })
}

/// The completion transaction's error channel: the repository's own error,
/// or the driver failure the retry loop classifies.
///
/// A local type because `transaction_with_retry` requires `From<DbError>`
/// and [`RepoError`] deliberately has no such impl — its `Db` arm carries a
/// rendered string, and a blanket conversion would let any driver error in
/// wearing that arm's meaning.
enum CompleteTxError {
    /// The repository's error, passed through.
    Repo(RepoError),
    /// The provider's own failure, before or around the statements.
    Db(toolkit_db::DbError),
}

impl From<toolkit_db::DbError> for CompleteTxError {
    fn from(error: toolkit_db::DbError) -> Self {
        Self::Db(error)
    }
}

impl From<RepoError> for CompleteTxError {
    fn from(error: RepoError) -> Self {
        Self::Repo(error)
    }
}

impl From<CompleteTxError> for RepoError {
    fn from(error: CompleteTxError) -> Self {
        match error {
            CompleteTxError::Repo(inner) => inner,
            CompleteTxError::Db(inner) => {
                Self::Db(format!("batch completion transaction: {inner}"))
            }
        }
    }
}

/// What the completion edge did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompleteOutcome {
    /// The batch was not in `committing`, or a peer's CAS won — either way
    /// this call flipped nothing and **emitted nothing**, which is where
    /// "exactly one" comes from.
    NotCommitting,
    /// A row is still in flight, so the batch stays open.
    RowsInFlight,
    /// The batch completed and its single summary was enqueued in the same
    /// transaction as the flip.
    Completed {
        /// The digest the summary carried.
        ledger_digest: String,
    },
}

/// Complete a committing batch: the `committing → completed` CAS **with the
/// summary event inside it** (`inst-bb-edge-complete`, `dod-coalesced-event`).
///
/// # "Exactly one" is the CAS, not a convention
///
/// The transaction that flips the state is the transaction that emits. A
/// re-claim after a lease expiry finds the state already flipped, its CAS
/// matches no row, and it emits nothing — so the guarantee is structural
/// rather than a rule a later step has to remember. A build that emitted
/// after the flip would carry no such property.
///
/// # A batch with failed rows still completes
///
/// The precondition is that every row has reached a **terminal ledger
/// state**, whatever the mix of `published`, `applied`, `no_op` and
/// `failed`. Parts-succeeded is the honest end state, and a machine that
/// refused it would hold the batch in `committing` forever, keeping the
/// tenant's concurrency slot.
///
/// # What the digest covers, and what is open about it
///
/// `design/09` and the `DoD` both say *the ledger digest* and neither
/// defines a computation. This one renders the ledger's own terminal facts —
/// `(row_key, disposition, code, entity_id)` per row, sorted by `row_key` —
/// through `domain::canonical`, the gear's single rendering rule, and takes
/// its `content_digest`. That set is the ledger as a consumer can verify it:
/// it excludes the staged payload (which a `no_op` row never applied) and
/// the timestamps (which differ between a run and its replay). **The covered
/// set is a choice this executor states rather than one a document makes**,
/// and `features/bulk-promotion.md` §7 carries the question.
///
/// # Errors
///
/// [`RepoError`] as the reads and writes raise it; an events failure
/// surfaces as [`RepoError::Db`], rolling the flip back with it.
#[allow(
    dead_code,
    reason = "the commit phase it terminates is blocked on 05's gate host"
)]
pub(crate) async fn complete_batch(
    ctx: &BulkWorkerContext,
    tenant_id: Uuid,
    batch_id: Uuid,
    actor_ref: Uuid,
    now: DateTime<Utc>,
) -> Result<CompleteOutcome, RepoError> {
    let scope = AccessScope::for_tenant(tenant_id);
    let conn = ctx
        .db
        .conn()
        .map_err(|e| RepoError::Db(format!("batch completion connection: {e}")))?;

    let Some(batch) = repo::find_batch(&conn, &scope, tenant_id, batch_id).await? else {
        return Ok(CompleteOutcome::NotCommitting);
    };
    if batch.state != crate::domain::states::BatchState::Committing {
        return Ok(CompleteOutcome::NotCommitting);
    }
    let rows = repo::find_batch_rows(&conn, &scope, tenant_id, batch_id).await?;
    let dispositions: Vec<Option<String>> = rows.iter().map(|r| r.disposition.clone()).collect();
    if !crate::domain::batch::all_rows_terminal(&dispositions) {
        return Ok(CompleteOutcome::RowsInFlight);
    }

    let ledger_digest = ledger_digest(&rows);
    let counts = disposition_counts(&rows);
    if flip_and_announce(
        ctx,
        tenant_id,
        batch_id,
        CompletionSummary {
            batch_key: &batch.batch_key,
            ledger_digest: &ledger_digest,
            counts,
        },
        actor_ref,
        now,
    )
    .await?
    {
        Ok(CompleteOutcome::Completed { ledger_digest })
    } else {
        Ok(CompleteOutcome::NotCommitting)
    }
}

/// The three values the completion summary carries beyond the batch's own
/// identity — grouped because they always travel together and two of the
/// three are strings a call site could transpose without the compiler
/// noticing (`HeadAct`'s own argument, one door over).
struct CompletionSummary<'a> {
    /// The import door's idempotency operand.
    batch_key: &'a str,
    /// The digest over the completed ledger.
    ledger_digest: &'a str,
    /// The per-disposition counts.
    counts: crate::infra::events::BulkCompletedRows,
}

/// The CAS and the emission, in one transaction — **the whole of "exactly
/// one"**.
///
/// Split from [`complete_batch`] so a probe can call it twice without the
/// caller's state pre-check short-circuiting the second call. That matters:
/// the pre-check is a fast path, and a test that only exercised it would go
/// green against a build that emitted outside the CAS — which is exactly
/// what a first revision of this suite did.
///
/// Answers whether this caller was the one that flipped. `false` means a
/// peer won and **nothing was emitted here**.
///
/// # Errors
///
/// [`RepoError`] as the write and the enqueue raise them; an events failure
/// rolls the flip back with it.
#[allow(dead_code, reason = "reached only by `complete_batch` and its probe")]
async fn flip_and_announce(
    ctx: &BulkWorkerContext,
    tenant_id: Uuid,
    batch_id: Uuid,
    summary: CompletionSummary<'_>,
    actor_ref: Uuid,
    now: DateTime<Utc>,
) -> Result<bool, RepoError> {
    let CompletionSummary {
        batch_key,
        ledger_digest,
        counts,
    } = summary;
    let scope = AccessScope::for_tenant(tenant_id);
    let sink = ctx.sink.clone();
    let batch_key = batch_key.to_owned();
    let digest = ledger_digest.to_owned();
    ctx.db
        .db()
        .transaction_with_retry::<bool, CompleteTxError, _, _>(
            toolkit_db::secure::TxConfig::default(),
            |e: &CompleteTxError| match e {
                CompleteTxError::Repo(RepoError::Driver { source, .. }) => Some(source),
                CompleteTxError::Repo(_) | CompleteTxError::Db(_) => None,
            },
            move |tx| {
                let sink = sink.clone();
                let scope = scope.clone();
                let batch_key = batch_key.clone();
                let digest = digest.clone();
                Box::pin(async move {
                    if !repo::move_bulk_batch_state(
                        tx,
                        &scope,
                        tenant_id,
                        batch_id,
                        crate::domain::states::BatchState::Committing,
                        crate::domain::states::BatchState::Completed,
                        now,
                    )
                    .await?
                    {
                        // A peer's CAS won. Nothing is emitted, which is the
                        // whole of "exactly one".
                        return Ok(false);
                    }
                    crate::infra::events::enqueue_bulk_completed(
                        &sink,
                        tx,
                        crate::infra::events::CATALOG_BULK_OPERATION_COMPLETED_PAYLOAD_TYPE,
                        crate::infra::events::BulkCompletedEventBody {
                            tenant_id,
                            batch_id,
                            batch_key: &batch_key,
                            ledger_digest: &digest,
                            rows: counts,
                        },
                        actor_ref,
                    )
                    .await
                    .map_err(|e| {
                        CompleteTxError::Repo(RepoError::Db(format!("batch completion event: {e}")))
                    })?;
                    Ok(true)
                })
            },
        )
        .await
        .map_err(RepoError::from)
}

/// The digest over a completed ledger — see [`complete_batch`] for what it
/// covers and why that set.
#[allow(dead_code, reason = "reached only by `complete_batch`")]
fn ledger_digest(rows: &[repo::BulkRowRecord]) -> String {
    let mut ordered: Vec<&repo::BulkRowRecord> = rows.iter().collect();
    ordered.sort_by(|a, b| a.row_key.cmp(&b.row_key));
    let rendered = JsonValue::Array(
        ordered
            .into_iter()
            .map(|row| {
                let mut entry = serde_json::Map::new();
                entry.insert("row_key".to_owned(), JsonValue::String(row.row_key.clone()));
                entry.insert(
                    "disposition".to_owned(),
                    row.disposition
                        .clone()
                        .map_or(JsonValue::Null, JsonValue::String),
                );
                entry.insert(
                    "code".to_owned(),
                    row.code.clone().map_or(JsonValue::Null, JsonValue::String),
                );
                entry.insert(
                    "entity_id".to_owned(),
                    row.entity_id
                        .map_or(JsonValue::Null, |id| JsonValue::String(id.to_string())),
                );
                JsonValue::Object(entry)
            })
            .collect(),
    );
    let canonical = crate::domain::canonical::canonical_rendering(
        &rendered,
        crate::domain::canonical::Absence::Omit,
    );
    let digest = crate::domain::canonical::content_digest(&canonical);
    digest
        .iter()
        .fold(String::with_capacity(digest.len() * 2), |mut hex, byte| {
            hex.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
            hex.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
            hex
        })
}

/// The summary's per-disposition counts, over the ledger's four terminal
/// values.
#[allow(dead_code, reason = "reached only by `complete_batch`")]
fn disposition_counts(rows: &[repo::BulkRowRecord]) -> crate::infra::events::BulkCompletedRows {
    let count = |want: &str| {
        u32::try_from(
            rows.iter()
                .filter(|r| r.disposition.as_deref() == Some(want))
                .count(),
        )
        .unwrap_or(u32::MAX)
    };
    crate::infra::events::BulkCompletedRows {
        published: count("published"),
        applied: count("applied"),
        no_op: count("no_op"),
        failed: count("failed"),
    }
}

pub(crate) async fn sweep(
    ctx: &BulkWorkerContext,
    actor_ref: Uuid,
    now: DateTime<Utc>,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<(), RepoError> {
    let tenants = {
        let conn = ctx
            .db
            .conn()
            .map_err(|e| RepoError::Db(format!("batch sweep connection: {e}")))?;
        repo::tenants_with_staging_batches(&conn, &AccessScope::allow_all()).await?
    };
    let total = tenants.len();
    let mut failed = 0_usize;
    let mut last_err: Option<RepoError> = None;
    for tenant in tenants {
        if cancel.is_cancelled() {
            return Ok(());
        }
        if let Err(e) = stage_next_batch(ctx, tenant, actor_ref, now, cancel).await {
            failed += 1;
            tracing::error!(
                %tenant,
                error = %e,
                "bss-products: batch staging pass failed; later tenants continue"
            );
            last_err = Some(e);
        }
    }
    match last_err {
        Some(e) if failed == total => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
#[path = "bulk_worker_tests.rs"]
mod bulk_worker_tests;
