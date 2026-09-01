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
