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
//! # The machine past the report edge (P-D-149)
//!
//! Edge 1 now carries the report and its `bulk_batch` approval
//! ([`report_and_submit`]); [`advance_batches`] observes the record — a
//! satisfied one starts the commit ([`begin_commit`]: the record consumed
//! **once**, `reported → approved → committing` in one transaction), a
//! rejected one abandons, a pending one past `bulk_batch_ttl_hours` is reaped
//! — and [`commit_rows`] walks the ledger: each row through the Foundation's
//! own door in `PreAuthorized` mode under the stored host, on the
//! `internal:bulk-row` lane, every failure row-local; then the one bulk-lane
//! increment request and edge 4. `dod-resume-abandon`'s resume half is the
//! ledger plus that lane: a re-claimed commit skips disposed rows and replays
//! claimed ones from their stored answer. `dod-coalesced-event`'s digest
//! operand is pinned by P-D-127 row 31 — the set this executor renders.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-resume-abandon:p1
//! @cpt-dod:cpt-cf-bss-products-dod-coalesced-event:p1
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

/// The commit claim's lease: the per-row publishes of a large batch outlast a
/// staging pass, and a peer must not re-claim a batch mid-walk.
const COMMIT_LEASE: chrono::Duration = chrono::Duration::minutes(30);

/// The worker's attempt budget (`inst-ar-failure`'s own arm, **P-D-69**): a
/// batch claimed this many times without reaching its edge is `failed`; row
/// failures never enter it.
pub(crate) const ATTEMPT_BUDGET: i64 = 5;

/// The reserved idempotency lane a bulk row's publish claims on
/// (`inst-bk-keys`, P-D-69): the ledger outcome is the stored answer, so a
/// resumed commit replays a published row instead of publishing it twice.
pub(crate) const BULK_ROW_LANE: &str = "internal:bulk-row";

/// The increment request's `source` for a batch's one bulk-lane request
/// (`dod-operation-key`): an internal requester, never the wire roster's.
///
/// @cpt-dod:cpt-cf-bss-products-dod-operation-key:p1
pub(crate) const BULK_REQUEST_SOURCE: &str = "bulk";

/// The itemised override condition's shape on the batch's record: the code
/// and the row's `skuCode`, which is what an approver acknowledges by name
/// (`inst-bk-override`).
pub(crate) const BUNDLE_OVERRIDE_CONDITION: &str = "BUNDLE_OVERRIDE_REQUIRED";

/// The closed-set reason a row carries when its override condition could
/// not be acknowledged: at effective quorum zero there is no approver to
/// name it and the worker is not an author (P-D-50: reasons are literals).
pub(crate) const NO_ACKNOWLEDGER_REASON: &str = "no-acknowledger-at-quorum-zero";

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde_json::Value as JsonValue;
use toolkit_db::secure::AccessScope;
use uuid::Uuid;

use crate::domain::approval::{ApprovalState, StoredApprovalGate};
use crate::domain::error::DomainError;
use crate::domain::governance::{ApprovalId, GateMode, GateSubject, GovernanceGate as _};
use crate::domain::materiality::{MaterialAct, MaterialityEvaluator, Resolution};
use crate::domain::name;
use crate::domain::validation::ValidationReport;
use crate::infra::idempotency::{self, ClaimVerdict, IdempotencyClaimInput};
use crate::infra::storage::RepoError;
use crate::infra::storage::repo::{
    self, ApprovalStoreError, BulkRowOutcome, NewApproval, NewIncrementRequest, NewProduct, NewSku,
};

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
    /// The `internal:bulk-row` claims' retention, the doors' own window.
    pub(crate) idempotency_retention_hours: u32,
    /// The reaper's operand (P-D-127 row 6).
    pub(crate) batch_ttl_hours: u32,
    /// `04`'s EOL flag, handed to the lifecycle lane's retire acts.
    pub(crate) eol_enabled: bool,
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
    /// The batch's attempt budget ran out: `staging -> failed` (P-D-69).
    BudgetExhausted {
        /// The failed batch.
        batch_id: Uuid,
    },
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
        crate::infra::create::JoinedRecords {
            claim: None,
            stamp,
            content: None,
        },
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
        // 03's classification (P-D-145) as the row carries it; a row naming
        // none is a `product` on the `standard` tier — the row shape that
        // carries these by contract is 09's (group 6).
        sku_type: field(payload, "sku_type").unwrap_or_else(|| {
            crate::domain::recognized::SkuType::Product
                .as_str()
                .to_owned()
        }),
        sellable: payload
            .get("sellable")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true),
        plan_tier: field(payload, "plan_tier")
            .unwrap_or_else(|| crate::domain::recognized::DEFAULT_PLAN_TIER.to_owned()),
        tax_category_ref: field(payload, "tax_category_ref"),
        gl_code_ref: field(payload, "gl_code_ref"),
        metering_unit: None,
        usage_type_ref: None,
    };
    match crate::infra::create::insert_sku_with_event(
        &ctx.db,
        &ctx.sink,
        scope.clone(),
        new,
        crate::infra::create::JoinedRecords {
            claim: None,
            stamp,
            content: None,
        },
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
    batch: &repo::BulkBatchRecord,
    row: &repo::BulkRowRecord,
    now: DateTime<Utc>,
) -> Result<bool, RepoError> {
    let batch_id = batch.batch_id;
    // A lifecycle row stages no draft: it names a live head and the op the
    // commit will drive through the ordinary `04` door (`dod-bulk-lifecycle`).
    if row.governed_live_op.is_some() {
        return stage_lifecycle_row(ctx, scope, tenant_id, batch_id, row, now).await;
    }
    let payload = parse_staged_payload(tenant_id, batch_id, row)?;
    // Under `promote` the resolver runs first (`dod-promotion-resolver`,
    // P-D-69): an identity already bound in this tenant is a no-op, an
    // update-as-draft or a conflict, never a second create.
    if batch.mode == "promote" {
        match resolve_promotion(ctx, scope, tenant_id, row, &payload).await? {
            Promotion::Create => {}
            Promotion::NoOp { entity_id } => {
                let conn = ctx
                    .db
                    .conn()
                    .map_err(|e| RepoError::Db(format!("row outcome connection: {e}")))?;
                repo::record_bulk_row_outcome(
                    &conn,
                    scope,
                    tenant_id,
                    batch_id,
                    &row.row_key,
                    BulkRowOutcome {
                        entity_id: Some(entity_id),
                        disposition: Some("no_op"),
                        code: None,
                        reason: None,
                        now,
                    },
                )
                .await?;
                return Ok(true);
            }
            Promotion::UpdateAsDraft {
                entity_id,
                revision,
                fields,
            } => {
                return stage_update_as_draft(
                    ctx, scope, tenant_id, actor_ref, batch_id, row, entity_id, revision, fields,
                    now,
                )
                .await;
            }
            Promotion::Conflict(refusal) => {
                let conn = ctx
                    .db
                    .conn()
                    .map_err(|e| RepoError::Db(format!("row outcome connection: {e}")))?;
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
                return Ok(false);
            }
        }
    }
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
        if batch.attempt >= ATTEMPT_BUDGET {
            // `staging -> failed`: the attempt budget is the machine's own
            // failure arm (P-D-69), never a row's.
            repo::move_bulk_batch_state(
                &conn,
                &scope,
                tenant_id,
                batch.batch_id,
                crate::domain::states::BatchState::Staging,
                crate::domain::states::BatchState::Failed,
                now,
            )
            .await?;
            return Ok(StageOutcome::BudgetExhausted {
                batch_id: batch.batch_id,
            });
        }
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
            crate::domain::states::BatchState::Staging,
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
        // A lifecycle row names its head from birth, so its "already staged"
        // marker is the pin the stage pass writes, not the id.
        let already_staged = row.disposition.is_some()
            || (row.entity_id.is_some()
                && (row.governed_live_op.is_none() || row.pinned_revision.is_some()));
        if already_staged {
            if row.disposition.is_some() {
                failed += 1;
            } else {
                staged += 1;
            }
            continue;
        }
        if stage_one_row(ctx, &scope, tenant_id, actor_ref, &batch, &row, now).await? {
            staged += 1;
        } else {
            failed += 1;
        }
    }

    // Edge 1 (P-D-54): the pass that stages the last row reports the batch.
    // Edge 1 carries the report: rendered from the ledger, submitted as the
    // batch's one `bulk_batch` approval, its id pinned on the batch before
    // the state moves (`inst-bk-report`, `dod-change-report`).
    let approval_id = report_and_submit(ctx, &scope, tenant_id, &batch, actor_ref, now).await?;
    let conn = ctx
        .db
        .conn()
        .map_err(|e| RepoError::Db(format!("batch report connection: {e}")))?;
    repo::set_bulk_batch_approval_ref(&conn, &scope, tenant_id, batch.batch_id, approval_id)
        .await?;
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

/// What the report edge saw of one staged row.
struct RowFacts {
    row_key: String,
    entity_id: Uuid,
    revision: i64,
    first_publish: bool,
    finance_material: bool,
    /// The row's `skuCode` when its head is an uncomposed bundle: the
    /// itemised override set's entry (`inst-bk-override`).
    override_code: Option<String>,
    lint: Vec<(String, String, String)>,
    region: String,
    brand: Option<Uuid>,
}

/// Read one staged row's head and lint it through the same functions the
/// dry-run doors run (P-D-125): the report's per-row facts.
async fn row_facts(
    conn: &(impl toolkit_db::secure::DBRunner + Sync),
    scope: &AccessScope,
    tenant_id: Uuid,
    row: &repo::BulkRowRecord,
) -> Result<Option<RowFacts>, RepoError> {
    let Some(entity_id) = row.entity_id else {
        return Ok(None);
    };
    if lifecycle_op(row).is_some() {
        // A transition row carries no lint and no override: its facts are the
        // head's revision, and the batch is material at any size by its
        // transitions (`dod-bulk-lifecycle`).
        return Ok(Some(RowFacts {
            row_key: row.row_key.clone(),
            entity_id,
            revision: row.pinned_revision.unwrap_or_default(),
            first_publish: true,
            finance_material: false,
            override_code: None,
            lint: Vec::new(),
            region: String::new(),
            brand: None,
        }));
    }
    match row.entity_kind.as_str() {
        "sku" => {
            let Some(head) = repo::find_sku(conn, scope, tenant_id, entity_id).await? else {
                return Ok(None);
            };
            let findings =
                crate::api::rest::skus::lint_sku_publish(conn, scope, tenant_id, entity_id).await?;
            let mut override_code = None;
            let mut lint = Vec::new();
            for finding in findings {
                if finding.code == BUNDLE_OVERRIDE_CONDITION {
                    override_code = Some(head.sku_code.clone());
                } else {
                    lint.push((finding.code, finding.subject, finding.detail));
                }
            }
            Ok(Some(RowFacts {
                row_key: row.row_key.clone(),
                entity_id,
                revision: head.internal_revision,
                first_publish: head.published_version == 0,
                finance_material: head.tax_category_ref.is_some() || head.gl_code_ref.is_some(),
                override_code,
                lint,
                region: head.region_scope,
                brand: None,
            }))
        }
        "product" => {
            let Some(head) = repo::find_product(conn, scope, tenant_id, entity_id).await? else {
                return Ok(None);
            };
            let findings =
                crate::api::rest::products::lint_product_publish(conn, scope, tenant_id, entity_id)
                    .await?;
            Ok(Some(RowFacts {
                row_key: row.row_key.clone(),
                entity_id,
                revision: head.internal_revision,
                first_publish: head.published_version == 0,
                finance_material: false,
                override_code: None,
                lint: findings
                    .into_iter()
                    .map(|f| (f.code, f.subject, f.detail))
                    .collect(),
                region: head.region_scope,
                brand: Some(head.brand_id),
            }))
        }
        _ => Ok(None),
    }
}

/// Render the `ChangeReport` from the ledger and submit it as the batch's
/// one `bulk_batch` approval (`inst-bk-report`, `dod-change-report`,
/// `dod-bulk-override-ceremony`).
///
/// The report is derived, never stored on its own: counts, a per-kind
/// summary, a deterministic sample (the first five row keys), the dry-run
/// lint per staged row, the scope-values lint (region and brand values the
/// tenant's heads outside this batch do not carry), the **itemised**
/// override-carrying rows by `skuCode`, and every staged row's pinned
/// revision. Its `ledgerDigest` is the record's pin (`SubjectPin::LedgerDigest`).
///
/// The override ceremony rides the record's `overrideConditions` as one
/// `BUNDLE_OVERRIDE_REQUIRED/{skuCode}` entry per itemised row; each
/// itemised row is marked acknowledged so the commit can tell it from a
/// condition that appeared after the report. At **effective quorum zero**
/// nobody can acknowledge by name and the worker is not an author (P-D-68),
/// so the itemised rows fail `BULK_OVERRIDE_UNACKNOWLEDGED` here with the
/// closed-set reason and the rest of the batch proceeds.
///
/// # Errors
///
/// [`RepoError`] as the reads raise it; a refusal of the record itself
/// (the store's own preconditions) surfaces as [`RepoError::Db`] naming the
/// code, aborting the pass — the attempt budget ends a batch that never
/// submits.
///
/// @cpt-dod:cpt-cf-bss-products-dod-change-report:p1
#[allow(clippy::cognitive_complexity)] // the report's facts, gathered in one pass over the ledger
async fn report_and_submit(
    ctx: &BulkWorkerContext,
    scope: &AccessScope,
    tenant_id: Uuid,
    batch: &repo::BulkBatchRecord,
    actor_ref: Uuid,
    now: DateTime<Utc>,
) -> Result<ApprovalId, RepoError> {
    let conn = ctx
        .db
        .conn()
        .map_err(|e| RepoError::Db(format!("batch report connection: {e}")))?;
    let policy = match repo::resolve_materiality_policy(&conn, scope, tenant_id).await? {
        Resolution::Resolved(policy) => policy,
        Resolution::Unresolvable => {
            return Err(RepoError::Db(
                "the materiality policy could not be read: a failed read is not a verdict \
                 (P-D-119 row 3), so the batch is not reported"
                    .to_owned(),
            ));
        }
    };
    let rows = repo::find_batch_rows(&conn, scope, tenant_id, batch.batch_id).await?;
    let staged_ids: Vec<Uuid> = rows.iter().filter_map(|row| row.entity_id).collect();
    let (known_regions, known_brands) =
        repo::known_scope_values(&conn, scope, tenant_id, &staged_ids).await?;

    let mut facts = Vec::new();
    for row in rows.iter().filter(|row| row.disposition.is_none()) {
        if let Some(fact) = row_facts(&conn, scope, tenant_id, row).await? {
            repo::pin_bulk_row(
                &conn,
                scope,
                tenant_id,
                batch.batch_id,
                &fact.row_key,
                fact.revision,
            )
            .await?;
            facts.push(fact);
        }
    }

    let quorum_zero = policy.approver_count() == 0;
    let mut itemised: Vec<(String, String)> = facts
        .iter()
        .filter_map(|f| {
            f.override_code
                .clone()
                .map(|code| (f.row_key.clone(), code))
        })
        .collect();
    itemised.sort();
    if quorum_zero {
        for (row_key, _) in &itemised {
            repo::record_bulk_row_outcome(
                &conn,
                scope,
                tenant_id,
                batch.batch_id,
                row_key,
                BulkRowOutcome {
                    entity_id: None,
                    disposition: Some("failed"),
                    code: Some("BULK_OVERRIDE_UNACKNOWLEDGED"),
                    reason: Some(NO_ACKNOWLEDGER_REASON),
                    now,
                },
            )
            .await?;
        }
        itemised.clear();
    } else {
        for (row_key, _) in &itemised {
            repo::mark_bulk_row_override_acknowledged(
                &conn,
                scope,
                tenant_id,
                batch.batch_id,
                row_key,
            )
            .await?;
        }
    }
    let conditions: Vec<String> = itemised
        .iter()
        .map(|(_, code)| format!("{BUNDLE_OVERRIDE_CONDITION}/{code}"))
        .collect();

    let rows = repo::find_batch_rows(&conn, scope, tenant_id, batch.batch_id).await?;
    let digest = ledger_digest(&rows);
    let mut by_kind: std::collections::BTreeMap<&str, (usize, usize)> =
        std::collections::BTreeMap::new();
    for row in &rows {
        let entry = by_kind.entry(row.entity_kind.as_str()).or_insert((0, 0));
        if row.disposition.as_deref() == Some("failed") {
            entry.1 += 1;
        } else {
            entry.0 += 1;
        }
    }
    let failed = rows
        .iter()
        .filter(|row| row.disposition.as_deref() == Some("failed"))
        .count();
    let mut sample: Vec<&str> = rows.iter().map(|row| row.row_key.as_str()).collect();
    sample.sort_unstable();
    sample.truncate(5);
    let mut unseen_regions: BTreeSet<&str> = BTreeSet::new();
    let mut unseen_brands: BTreeSet<Uuid> = BTreeSet::new();
    for fact in facts.iter().filter(|fact| !fact.region.is_empty()) {
        if !known_regions.contains(&fact.region) {
            unseen_regions.insert(fact.region.as_str());
        }
        if let Some(brand) = fact.brand
            && !known_brands.contains(&brand)
        {
            unseen_brands.insert(brand);
        }
    }
    let report = serde_json::json!({
        "ledgerDigest": digest,
        "batchId": batch.batch_id,
        "batchKey": batch.batch_key,
        "mode": batch.mode,
        "counts": { "total": rows.len(), "staged": rows.len() - failed, "failed": failed },
        "byKind": by_kind.iter().map(|(kind, (staged, failed))| {
            serde_json::json!({ "entityKind": kind, "staged": staged, "failed": failed })
        }).collect::<Vec<_>>(),
        "sample": sample,
        "lint": facts.iter().flat_map(|f| f.lint.iter().map(move |(code, subject, detail)| {
            serde_json::json!({ "rowKey": f.row_key, "code": code, "subject": subject, "detail": detail })
        })).collect::<Vec<_>>(),
        "scopeValues": { "unseenRegions": unseen_regions, "unseenBrands": unseen_brands },
        "overrides": itemised.iter().map(|(row_key, code)| {
            serde_json::json!({ "rowKey": row_key, "skuCode": code })
        }).collect::<Vec<_>>(),
        "rows": facts.iter().map(|f| {
            serde_json::json!({ "rowKey": f.row_key, "entityId": f.entity_id, "pinnedRevision": f.revision })
        }).collect::<Vec<_>>(),
    });

    // Materiality is 05's to decide: a first publish is material at any size,
    // the affected-entity trigger catches the rest (`inst-bk-report`).
    let affected = if facts.iter().any(|f| f.first_publish) {
        u32::MAX
    } else {
        u32::try_from(facts.len()).unwrap_or(u32::MAX)
    };
    let act = MaterialAct::BatchAct { affected };
    let evaluator = MaterialityEvaluator::new(Resolution::Resolved(&policy));
    let subject = GateSubject::bulk_batch(tenant_id, batch.batch_id, digest);
    let snapshot = report.to_string();
    let submitted = repo::submit_approval(
        &conn,
        scope,
        NewApproval {
            approval_id: ApprovalId::new(Uuid::now_v7()),
            subject: &subject,
            internal_revision: 0,
            content_snapshot: &snapshot,
            diff_basis: None,
            act: &act,
            evaluator,
            finance_material: facts.iter().any(|f| f.finance_material),
            approver_count: policy.approver_count(),
            submitter: actor_ref,
            author_override_ack: None,
            override_conditions: conditions,
        },
        now,
    )
    .await
    .map_err(|e| match e {
        ApprovalStoreError::Refused(refusal) => RepoError::Db(format!(
            "the batch approval was refused by the store: {}",
            refusal.code()
        )),
        ApprovalStoreError::Repo(error) => error,
    })?;
    Ok(submitted.approval_id)
}

/// What one pass did to a tenant's `reported`, `approved` and `committing`
/// batches.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AdvanceOutcome {
    /// Batches whose record read `satisfied` and were flipped to `committing`
    /// with the record consumed (edge 3, then edge 2's consequence).
    pub started: usize,
    /// Batches whose rows were walked this pass.
    pub committed: usize,
    /// Batches abandoned: a rejected record, or the reaper's TTL.
    pub abandoned: usize,
    /// Batches whose commit attempt budget ran out (`committing -> failed`).
    pub failed: usize,
}

/// Walk the tenant's batches past `reported`: start the commit of the ones
/// whose record is satisfied (consuming it exactly once), abandon the ones
/// whose record closed or whose TTL lapsed (**P-D-127** rows 6 and 7), and
/// drive the `committing` ones' rows (`dod-batch-state-machine`,
/// `dod-commit-phase`).
///
/// # Errors
///
/// [`RepoError`] as the store raises it; the tenant's pass aborts and the
/// next tick retries, the ledger being the record.
///
/// @cpt-dod:cpt-cf-bss-products-dod-batch-state-machine:p1
pub(crate) async fn advance_batches(
    ctx: &BulkWorkerContext,
    tenant_id: Uuid,
    actor_ref: Uuid,
    now: DateTime<Utc>,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<AdvanceOutcome, RepoError> {
    let scope = AccessScope::for_tenant(tenant_id);
    let mut outcome = AdvanceOutcome::default();
    let reported = {
        let conn = ctx
            .db
            .conn()
            .map_err(|e| RepoError::Db(format!("batch advance connection: {e}")))?;
        repo::batches_in_state(
            &conn,
            &scope,
            tenant_id,
            crate::domain::states::BatchState::Reported,
        )
        .await?
    };
    let ttl = chrono::Duration::hours(i64::from(ctx.batch_ttl_hours));
    for batch in reported {
        if cancel.is_cancelled() {
            return Ok(outcome);
        }
        let record = match batch.approval_ref {
            Some(approval_ref) => {
                let conn = ctx
                    .db
                    .conn()
                    .map_err(|e| RepoError::Db(format!("batch record connection: {e}")))?;
                repo::gate_candidate_by_id(&conn, &scope, tenant_id, ApprovalId::new(approval_ref))
                    .await?
            }
            None => None,
        };
        match record.as_ref().map(|candidate| candidate.state) {
            Some(ApprovalState::Satisfied) => {
                if begin_commit(ctx, tenant_id, batch.batch_id, actor_ref, now).await? {
                    outcome.started += 1;
                }
            }
            Some(ApprovalState::Rejected | ApprovalState::Superseded) => {
                abandon_batch(ctx, tenant_id, batch.batch_id, now).await?;
                outcome.abandoned += 1;
            }
            // Pending (or an unsubmitted report, or a record already spent
            // by a peer): the reaper's TTL is the only other exit.
            _ => {
                if now.signed_duration_since(batch.created_at) >= ttl {
                    abandon_batch(ctx, tenant_id, batch.batch_id, now).await?;
                    outcome.abandoned += 1;
                }
            }
        }
    }

    // A batch left in `approved` by a crash between the two moves of
    // `begin_commit`: its record is already consumed, so the flip to
    // `committing` needs no second consumption.
    let approved = {
        let conn = ctx
            .db
            .conn()
            .map_err(|e| RepoError::Db(format!("batch advance connection: {e}")))?;
        let approved = repo::batches_in_state(
            &conn,
            &scope,
            tenant_id,
            crate::domain::states::BatchState::Approved,
        )
        .await?;
        for batch in &approved {
            repo::move_bulk_batch_state(
                &conn,
                &scope,
                tenant_id,
                batch.batch_id,
                crate::domain::states::BatchState::Approved,
                crate::domain::states::BatchState::Committing,
                now,
            )
            .await?;
            repo::release_bulk_batch_claim(&conn, &scope, tenant_id, batch.batch_id).await?;
        }
        approved.len()
    };
    outcome.started += approved;

    let committing = {
        let conn = ctx
            .db
            .conn()
            .map_err(|e| RepoError::Db(format!("batch advance connection: {e}")))?;
        repo::batches_in_state(
            &conn,
            &scope,
            tenant_id,
            crate::domain::states::BatchState::Committing,
        )
        .await?
    };
    for batch in committing {
        if cancel.is_cancelled() {
            return Ok(outcome);
        }
        match commit_rows(ctx, tenant_id, &batch, actor_ref, now, cancel).await? {
            CommitOutcome::Committed { .. } => outcome.committed += 1,
            CommitOutcome::BudgetExhausted => outcome.failed += 1,
            CommitOutcome::ClaimLost | CommitOutcome::NoRecord => {}
        }
    }
    Ok(outcome)
}

/// Edges 2 and 3 in one transaction (**P-D-127** row 7, `inst-bk-commit`):
/// the batch's record is evaluated and **consumed once**, and the batch
/// moves `reported -> approved -> committing`. A record spent by a peer in
/// the meantime refuses the evaluation and the transaction rolls back with
/// nothing written — the one-shot is enforced here and nowhere else.
async fn begin_commit(
    ctx: &BulkWorkerContext,
    tenant_id: Uuid,
    batch_id: Uuid,
    _actor_ref: Uuid,
    now: DateTime<Utc>,
) -> Result<bool, RepoError> {
    let scope = AccessScope::for_tenant(tenant_id);
    ctx.db
        .db()
        .transaction_with_retry::<bool, CompleteTxError, _, _>(
            toolkit_db::secure::TxConfig::default(),
            |e: &CompleteTxError| match e {
                CompleteTxError::Repo(RepoError::Driver { source, .. }) => Some(source),
                CompleteTxError::Repo(_) | CompleteTxError::Db(_) => None,
            },
            move |tx| {
                let scope = scope.clone();
                Box::pin(async move {
                    let Some(batch) = repo::find_batch(tx, &scope, tenant_id, batch_id).await?
                    else {
                        return Ok(false);
                    };
                    let Some(approval_ref) = batch.approval_ref else {
                        return Ok(false);
                    };
                    let Some(candidate) = repo::gate_candidate_by_id(
                        tx,
                        &scope,
                        tenant_id,
                        ApprovalId::new(approval_ref),
                    )
                    .await?
                    else {
                        return Ok(false);
                    };
                    let subject = candidate.subject.clone();
                    let gate = StoredApprovalGate::governed(vec![candidate]);
                    let authorization = gate
                        .evaluate(subject, GateMode::Gate)
                        .and_then(crate::domain::governance::GateVerdict::into_authorization)
                        .map_err(|refusal| {
                            CompleteTxError::Repo(RepoError::Db(format!(
                                "the batch record no longer authorizes the commit: {}",
                                refusal.code()
                            )))
                        })?;
                    repo::settle_authorization(tx, &scope, tenant_id, &authorization, now)
                        .await
                        .map_err(|e| match e {
                            repo::SettleError::Refused(refusal) => {
                                CompleteTxError::Repo(RepoError::Db(format!(
                                    "consuming the batch record: {}",
                                    refusal.code()
                                )))
                            }
                            repo::SettleError::Repo(error) => CompleteTxError::Repo(error),
                        })?;
                    let moved = repo::move_bulk_batch_state(
                        tx,
                        &scope,
                        tenant_id,
                        batch_id,
                        crate::domain::states::BatchState::Reported,
                        crate::domain::states::BatchState::Approved,
                        now,
                    )
                    .await?
                        && repo::move_bulk_batch_state(
                            tx,
                            &scope,
                            tenant_id,
                            batch_id,
                            crate::domain::states::BatchState::Approved,
                            crate::domain::states::BatchState::Committing,
                            now,
                        )
                        .await?;
                    if !moved {
                        return Err(CompleteTxError::Repo(RepoError::Db(
                            "the batch left `reported` under the commit's claim".to_owned(),
                        )));
                    }
                    // The staging claim's lease is handed back: the commit
                    // phase takes its own on the `committing` row.
                    repo::release_bulk_batch_claim(tx, &scope, tenant_id, batch_id).await?;
                    Ok(true)
                })
            },
        )
        .await
        .map_err(RepoError::from)
}

/// What one commit pass did to a `committing` batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitOutcome {
    /// The rows were walked; the completion edge's own outcome rides along.
    Committed {
        published: usize,
        failed: usize,
        completion: CompleteOutcome,
    },
    /// Another worker holds the batch's lease.
    ClaimLost,
    /// The batch pins no record (it never reported): nothing to commit under.
    NoRecord,
    /// The attempt budget ran out: `committing -> failed`.
    BudgetExhausted,
}

/// The commit phase (`inst-bk-commit`, `dod-commit-phase`): every staged row
/// publishes through the Foundation's own door in `PreAuthorized` mode naming
/// the batch's consumed record, pinned to its ledger revision, on the
/// `internal:bulk-row` lane; every failure is row-local and coded; the
/// batch's one bulk-lane increment request is enqueued when anything
/// published; then edge 4 completes the batch.
///
/// Products walk before SKUs so a SKU whose parent row failed this pass fails
/// `BULK_DEPENDENCY_FAILED` wrapping the parent's code. A resumed commit
/// skips every disposed row and replays a claimed one from its stored answer.
///
/// # Errors
///
/// [`RepoError`] on a storage failure below the domain; the pass aborts and
/// the next tick resumes from the ledger.
///
/// @cpt-dod:cpt-cf-bss-products-dod-commit-phase:p1
#[allow(clippy::too_many_lines)] // one walk, in ledger order, with its two arms inline
pub(crate) async fn commit_rows(
    ctx: &BulkWorkerContext,
    tenant_id: Uuid,
    batch: &repo::BulkBatchRecord,
    actor_ref: Uuid,
    now: DateTime<Utc>,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<CommitOutcome, RepoError> {
    let scope = AccessScope::for_tenant(tenant_id);
    let Some(approval_ref) = batch.approval_ref else {
        return Ok(CommitOutcome::NoRecord);
    };
    let conn = ctx
        .db
        .conn()
        .map_err(|e| RepoError::Db(format!("batch commit connection: {e}")))?;
    if batch.attempt >= ATTEMPT_BUDGET {
        repo::move_bulk_batch_state(
            &conn,
            &scope,
            tenant_id,
            batch.batch_id,
            crate::domain::states::BatchState::Committing,
            crate::domain::states::BatchState::Failed,
            now,
        )
        .await?;
        return Ok(CommitOutcome::BudgetExhausted);
    }
    if !repo::claim_bulk_batch(
        &conn,
        &scope,
        tenant_id,
        batch.batch_id,
        crate::domain::states::BatchState::Committing,
        batch.attempt,
        now,
        COMMIT_LEASE,
    )
    .await?
    {
        return Ok(CommitOutcome::ClaimLost);
    }
    let approval_id = ApprovalId::new(approval_ref);
    let Some(candidate) = repo::gate_candidate_by_id(&conn, &scope, tenant_id, approval_id).await?
    else {
        return Err(RepoError::Db(format!(
            "batch {} pins approval {approval_ref}, which is gone",
            batch.batch_id
        )));
    };
    let gate = StoredApprovalGate::bulk_row(vec![candidate], approval_id);

    let mut rows = repo::find_batch_rows(&conn, &scope, tenant_id, batch.batch_id).await?;
    // Products first: a SKU's dependency arm reads its parent's outcome.
    rows.sort_by_key(|row| (row.entity_kind != "product", row.row_key.clone()));
    let mut failed_parents: std::collections::BTreeMap<Uuid, String> =
        std::collections::BTreeMap::new();
    let mut published = 0usize;
    let mut failed = 0usize;
    for row in &rows {
        if cancel.is_cancelled() {
            return Ok(CommitOutcome::ClaimLost);
        }
        if row.disposition.is_some() {
            if row.disposition.as_deref() == Some("failed") {
                failed += 1;
                if let Some(entity_id) = row.entity_id
                    && row.entity_kind == "product"
                {
                    failed_parents.insert(entity_id, row.code.clone().unwrap_or_default());
                }
            } else {
                published += 1;
            }
            continue;
        }
        let Some(entity_id) = row.entity_id else {
            record_row(
                &conn,
                &scope,
                tenant_id,
                batch.batch_id,
                &row.row_key,
                None,
                Err("VALIDATION".to_owned()),
                false,
                now,
            )
            .await?;
            failed += 1;
            continue;
        };
        let is_lifecycle = lifecycle_op(row).is_some();
        let result = match row.entity_kind.as_str() {
            _ if is_lifecycle => {
                apply_lifecycle_row(
                    ctx,
                    &conn,
                    &scope,
                    tenant_id,
                    batch.batch_id,
                    row,
                    entity_id,
                    &gate,
                    approval_id,
                    actor_ref,
                    now,
                )
                .await?
            }
            "product" => {
                publish_product_row(
                    ctx,
                    &conn,
                    &scope,
                    tenant_id,
                    batch.batch_id,
                    row,
                    entity_id,
                    &gate,
                    approval_id,
                    actor_ref,
                    now,
                )
                .await?
            }
            "sku" => {
                let parent = parse_staged_payload(tenant_id, batch.batch_id, row)
                    .ok()
                    .and_then(|payload| field(&payload, "product_id"))
                    .and_then(|raw| Uuid::parse_str(&raw).ok());
                match parent.and_then(|parent| failed_parents.get(&parent)) {
                    Some(parent_code) => Err(format!("BULK_DEPENDENCY_FAILED:{parent_code}")),
                    None => {
                        publish_sku_row(
                            ctx,
                            &conn,
                            &scope,
                            tenant_id,
                            batch.batch_id,
                            row,
                            entity_id,
                            &gate,
                            approval_id,
                            actor_ref,
                            now,
                        )
                        .await?
                    }
                }
            }
            _ => Err("VALIDATION".to_owned()),
        };
        match &result {
            Ok(_) => published += 1,
            Err(code) => {
                failed += 1;
                if row.entity_kind == "product" {
                    failed_parents.insert(entity_id, code.clone());
                }
            }
        }
        record_row(
            &conn,
            &scope,
            tenant_id,
            batch.batch_id,
            &row.row_key,
            Some(entity_id),
            result,
            is_lifecycle,
            now,
        )
        .await?;
    }

    if published > 0 {
        // One batch, one catalog version (`dod-operation-key`): the request is
        // keyed by the batch and rides the bulk lane, whose window is what
        // holds the batch's publishes together; a resumed commit replays it.
        let request_key = batch.batch_id.to_string();
        repo::enqueue_increment_request(
            &conn,
            &scope,
            tenant_id,
            NewIncrementRequest {
                source: BULK_REQUEST_SOURCE,
                request_key: &request_key,
                lane: bss_products_sdk::increments::IncrementLane::Bulk,
                operation_key: Some(batch.operation_key.as_deref().unwrap_or(&request_key)),
                requested_at: now,
            },
        )
        .await?;
    }
    drop(rows);
    return_pinned(conn);
    let completion = complete_batch(ctx, tenant_id, batch.batch_id, actor_ref, now).await?;
    Ok(CommitOutcome::Committed {
        published,
        failed,
        completion,
    })
}

fn return_pinned<T>(conn: T) {
    let _returned = conn;
}

/// Write one row's commit outcome to the ledger: `published` with its id, or
/// `failed` with its code (a dependency failure wraps the parent's code
/// after the colon, which the ledger keeps as the code column's text).
#[allow(clippy::too_many_arguments)] // the ledger write's operands, all of them the row's
async fn record_row(
    conn: &(impl toolkit_db::secure::DBRunner + Sync),
    scope: &AccessScope,
    tenant_id: Uuid,
    batch_id: Uuid,
    row_key: &str,
    entity_id: Option<Uuid>,
    result: Result<i64, String>,
    applied: bool,
    now: DateTime<Utc>,
) -> Result<(), RepoError> {
    let (disposition, code) = match &result {
        Ok(_) if applied => ("applied", None),
        Ok(_) => ("published", None),
        Err(code) => ("failed", Some(code.as_str())),
    };
    repo::record_bulk_row_outcome(
        conn,
        scope,
        tenant_id,
        batch_id,
        row_key,
        BulkRowOutcome {
            entity_id,
            disposition: Some(disposition),
            code,
            reason: None,
            now,
        },
    )
    .await
}

/// The `internal:bulk-row` claim of one row's publish: the key is the batch
/// and the row, the answer is the ledger outcome (P-D-69 on P-D-42's shape).
fn row_claim(
    ctx: &BulkWorkerContext,
    batch_id: Uuid,
    row_key: &str,
    now: DateTime<Utc>,
) -> IdempotencyClaimInput {
    IdempotencyClaimInput::new(
        BULK_ROW_LANE,
        format!("{batch_id}/{row_key}"),
        crate::domain::idempotency::payload_digest(&serde_json::json!({})),
        now,
        ctx.idempotency_retention_hours,
    )
}

/// Publish one SKU row through `skus::run_publish` in `PreAuthorized` mode
/// (`inst-bk-commit`). The override arm runs first: an uncomposed bundle
/// whose row is not in the ceremony's itemised set is
/// `BULK_OVERRIDE_UNACKNOWLEDGED` alone (`inst-bk-override`).
///
/// @cpt-dod:cpt-cf-bss-products-dod-bulk-override-ceremony:p1
#[allow(clippy::too_many_arguments)] // the door's operands, threaded from the walk
async fn publish_sku_row(
    ctx: &BulkWorkerContext,
    conn: &(impl toolkit_db::secure::DBRunner + Sync),
    scope: &AccessScope,
    tenant_id: Uuid,
    batch_id: Uuid,
    row: &repo::BulkRowRecord,
    sku_id: Uuid,
    gate: &StoredApprovalGate,
    approval_id: ApprovalId,
    actor_ref: Uuid,
    now: DateTime<Utc>,
) -> Result<Result<i64, String>, RepoError> {
    use crate::api::rest::skus;
    let Some(head) = repo::find_sku(conn, scope, tenant_id, sku_id).await? else {
        return Ok(Err("HEAD_VANISHED".to_owned()));
    };
    let is_bundle =
        head.sku_type.as_deref() == Some(crate::domain::recognized::SkuType::Bundle.as_str());
    if is_bundle
        && (head.published_version == 0 || head.composition_pending)
        && !row.override_acknowledged
    {
        return Ok(Err("BULK_OVERRIDE_UNACKNOWLEDGED".to_owned()));
    }
    let claim = row_claim(ctx, batch_id, &row.row_key, now);
    match idempotency::claim_idempotency(conn, scope, tenant_id, &claim).await? {
        ClaimVerdict::Replay { .. } => return Ok(Ok(head.published_version)),
        ClaimVerdict::Refused(error) => return Ok(Err(error.code().to_owned())),
        ClaimVerdict::Proceed => {}
    }
    let inputs = skus::HeadActInputs {
        scope: scope.clone(),
        tenant_id,
        sku_id,
        actor_ref,
        expected: row.pinned_revision.unwrap_or(head.internal_revision),
        now,
        claim: None,
    };
    let outcome = skus::run_publish(
        conn,
        &inputs,
        gate,
        GateMode::PreAuthorized(approval_id),
        &ctx.sink,
        skus::PublishOperands::default(),
    )
    .await;
    let result = match outcome {
        Ok(skus::MutationOutcome::Applied { .. } | skus::MutationOutcome::Replay { .. }) => {
            let version = repo::find_sku(conn, scope, tenant_id, sku_id)
                .await?
                .map_or(head.published_version, |after| after.published_version);
            Ok(version)
        }
        Err(skus::HeadActError::Refused(refusal)) => Err(refusal.code().to_owned()),
        Err(skus::HeadActError::Vanished) => Err("HEAD_VANISHED".to_owned()),
        Err(skus::HeadActError::Db(error)) => return Err(RepoError::Db(error.to_string())),
    };
    idempotency::record_idempotency_answer(
        conn,
        scope,
        tenant_id,
        &claim,
        axum::http::StatusCode::OK,
        &row_answer(&row.row_key, sku_id, &result),
    )
    .await?;
    Ok(result)
}

/// Publish one Product row through `products::run_publish` in
/// `PreAuthorized` mode (`inst-bk-commit`).
#[allow(clippy::too_many_arguments)] // the door's operands, threaded from the walk
async fn publish_product_row(
    ctx: &BulkWorkerContext,
    conn: &(impl toolkit_db::secure::DBRunner + Sync),
    scope: &AccessScope,
    tenant_id: Uuid,
    batch_id: Uuid,
    row: &repo::BulkRowRecord,
    product_id: Uuid,
    gate: &StoredApprovalGate,
    approval_id: ApprovalId,
    actor_ref: Uuid,
    now: DateTime<Utc>,
) -> Result<Result<i64, String>, RepoError> {
    use crate::api::rest::products;
    let Some(head) = repo::find_product(conn, scope, tenant_id, product_id).await? else {
        return Ok(Err("HEAD_VANISHED".to_owned()));
    };
    let claim = row_claim(ctx, batch_id, &row.row_key, now);
    match idempotency::claim_idempotency(conn, scope, tenant_id, &claim).await? {
        ClaimVerdict::Replay { .. } => return Ok(Ok(head.published_version)),
        ClaimVerdict::Refused(error) => return Ok(Err(error.code().to_owned())),
        ClaimVerdict::Proceed => {}
    }
    let inputs = products::HeadActInputs {
        scope: scope.clone(),
        tenant_id,
        product_id,
        actor_ref,
        expected: row.pinned_revision.unwrap_or(head.internal_revision),
        now,
        claim: None,
    };
    let outcome = products::run_publish(
        conn,
        &inputs,
        gate,
        GateMode::PreAuthorized(approval_id),
        &ctx.sink,
    )
    .await;
    let result = match outcome {
        Ok(products::HeadActOutcome::Applied { .. } | products::HeadActOutcome::Replay { .. }) => {
            let version = repo::find_product(conn, scope, tenant_id, product_id)
                .await?
                .map_or(head.published_version, |after| after.published_version);
            Ok(version)
        }
        Err(products::HeadActError::Refused(refusal)) => Err(refusal.code().to_owned()),
        Err(products::HeadActError::Vanished) => Err("HEAD_VANISHED".to_owned()),
        Err(products::HeadActError::Db(error)) => return Err(RepoError::Db(error.to_string())),
    };
    idempotency::record_idempotency_answer(
        conn,
        scope,
        tenant_id,
        &claim,
        axum::http::StatusCode::OK,
        &row_answer(&row.row_key, product_id, &result),
    )
    .await?;
    Ok(result)
}

/// The `internal:bulk-row` lane's stored answer: the ledger outcome.
fn row_answer(row_key: &str, entity_id: Uuid, result: &Result<i64, String>) -> JsonValue {
    match result {
        Ok(version) => serde_json::json!({
            "rowKey": row_key, "entityId": entity_id, "disposition": "published",
            "publishedVersion": version,
        }),
        Err(code) => serde_json::json!({
            "rowKey": row_key, "entityId": entity_id, "disposition": "failed", "code": code,
        }),
    }
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
        let edits_existing = row
            .governed_live_op
            .as_deref()
            .is_some_and(|raw| raw.contains(UPDATE_AS_DRAFT_OP));
        // A lifecycle row stages no draft: its pending transition is dropped,
        // never applied, and the head it names is untouched.
        let disposition = if lifecycle_op(&row).is_some() {
            crate::domain::batch::AbandonDisposition::DropPendingOp
        } else {
            crate::domain::batch::abandon_disposition(crate::domain::batch::AbandonRow {
                kind: bss_products_sdk::models::EntityKind::parse(&row.entity_kind),
                standing: if row.disposition.is_some() {
                    crate::domain::batch::RowStanding::Terminal
                } else if row.entity_id.is_none() {
                    crate::domain::batch::RowStanding::NeverMaterialised
                } else {
                    crate::domain::batch::RowStanding::Live
                },
                edits_existing,
            })
        };
        match disposition {
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
                // An update-as-draft row reverts through the ordinary save door
                // to the last frozen content (`dod-resume-abandon`); a revert
                // the door refuses is counted as dropped, the head keeping its
                // draft for the operator.
                if let Some(entity_id) = row.entity_id
                    && revert_update_as_draft(ctx, &conn, &scope, tenant_id, &row, entity_id, now)
                        .await?
                {
                    discarded += 1;
                } else {
                    dropped += 1;
                }
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

#[allow(clippy::cognitive_complexity)] // one pass per tenant: stage, then advance, each logged
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
        repo::tenants_with_batches_in(
            &conn,
            &AccessScope::allow_all(),
            &[
                crate::domain::states::BatchState::Staging,
                crate::domain::states::BatchState::Reported,
                crate::domain::states::BatchState::Approved,
                crate::domain::states::BatchState::Committing,
            ],
        )
        .await?
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
            continue;
        }
        if let Err(e) = advance_batches(ctx, tenant, actor_ref, now, cancel).await {
            failed += 1;
            tracing::error!(
                %tenant,
                error = %e,
                "bss-products: batch commit pass failed; later tenants continue"
            );
            last_err = Some(e);
        }
    }
    match last_err {
        Some(e) if failed == total => Err(e),
        _ => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// The promotion resolver (`dod-promotion-resolver`, P-D-69, P-D-127 rows 4, 21)
// ---------------------------------------------------------------------------

/// The four classes C5 names, in the slice's order.
enum Promotion {
    /// An unknown identity: the row creates, ids re-minted.
    Create,
    /// The identity is bound to matching content.
    NoOp { entity_id: Uuid },
    /// The identity is bound to different content: the row saves onto the
    /// existing head as a draft, pinned to the revision the save leaves.
    UpdateAsDraft {
        entity_id: Uuid,
        revision: i64,
        fields: std::collections::BTreeMap<String, JsonValue>,
    },
    /// An incompatible binding: a `retired` holder (revival is clone-only),
    /// a head carrying unpublished edits or an open approval.
    Conflict(DomainError),
}

/// The staged fields the save door recognises — the content a promotion
/// compares and, when it differs, saves. Every recognised field is kept,
/// bucket ii included: a bucket-i/ii difference onto an existing identity
/// classifies as update-as-draft and **fails at the save door** with 01's
/// `ILLEGAL_FIELD_MUTATION` naming 07's correction door (§7 row 2, P-D-127),
/// the door judging rather than the resolver pre-empting it. Create-only
/// and unrecognised keys are dropped: the identity keys matched already.
fn promotable_fields(
    entity_kind: &str,
    payload: &JsonValue,
) -> std::collections::BTreeMap<String, JsonValue> {
    use crate::domain::bucket::{self, FieldClass};
    let Some(map) = payload.as_object() else {
        return std::collections::BTreeMap::new();
    };
    let mut fields = std::collections::BTreeMap::new();
    for (key, value) in map {
        let column = match entity_kind {
            "sku" => {
                if crate::api::rest::skus::SKU_CONTENT_SAVE_KEYS.contains(&key.as_str()) {
                    fields.insert(key.clone(), value.clone());
                    continue;
                }
                crate::api::rest::skus::SkuSaveField::from_wire(key)
                    .map(crate::api::rest::skus::SkuSaveField::column)
            }
            _ => crate::api::rest::products::ProductSaveField::from_wire(key)
                .map(crate::api::rest::products::ProductSaveField::column),
        };
        let Some(column) = column else {
            continue;
        };
        let kind = if entity_kind == "sku" {
            bss_products_sdk::models::EntityKind::Sku
        } else {
            bss_products_sdk::models::EntityKind::Product
        };
        if let Ok(FieldClass::Bucket(_)) = bucket::classify(kind, column) {
            fields.insert(key.clone(), value.clone());
        }
    }
    fields
}

/// Does the bound head carry unpublished local edits? A never-published head
/// does by definition; a published one when its rendering differs from its
/// last version row — the correction door's own clean-head predicate.
async fn head_is_dirty(
    conn: &(impl toolkit_db::secure::DBRunner + Sync),
    scope: &AccessScope,
    tenant_id: Uuid,
    entity_kind: &str,
    entity_id: Uuid,
    published_version: i64,
    rendering: &str,
) -> Result<bool, RepoError> {
    if published_version == 0 {
        return Ok(true);
    }
    let kind = if entity_kind == "sku" {
        repo::VersionedEntityKind::Sku
    } else {
        repo::VersionedEntityKind::Product
    };
    let latest = repo::latest_entity_version(conn, scope, tenant_id, kind, entity_id).await?;
    Ok(!matches!(latest, Some((_, ref frozen)) if frozen == rendering))
}

/// Is there an open publish approval on the head?
async fn head_has_open_approval(
    conn: &(impl toolkit_db::secure::DBRunner + Sync),
    scope: &AccessScope,
    tenant_id: Uuid,
    entity_kind: bss_products_sdk::models::EntityKind,
    entity_id: Uuid,
    revision: i64,
) -> Result<bool, RepoError> {
    let subject = GateSubject::entity_publish(
        crate::domain::governance::EntityRef {
            tenant_id,
            entity_kind,
            entity_id,
        },
        crate::domain::concurrency::InternalRevision::new(revision),
    );
    let candidates = repo::gate_candidates(conn, scope, &subject).await?;
    Ok(candidates
        .iter()
        .any(|candidate| candidate.state == ApprovalState::Pending))
}

/// The head a promotion row resolves to: id, lifecycle state, internal
/// revision, published version, current content, kind.
type BoundHead = (
    Uuid,
    bss_products_sdk::models::LifecycleState,
    i64,
    i64,
    JsonValue,
    bss_products_sdk::models::EntityKind,
);

/// Classify one `promote` row against the tenant's heads: identity is the
/// exported id, then the code, then `(brandId, normalized name)` for a
/// Product (P-D-127 row 4); matching content is canonical equality of the
/// promotable fields (row 21).
///
/// @cpt-dod:cpt-cf-bss-products-dod-promotion-resolver:p1
async fn resolve_promotion(
    ctx: &BulkWorkerContext,
    scope: &AccessScope,
    tenant_id: Uuid,
    row: &repo::BulkRowRecord,
    payload: &JsonValue,
) -> Result<Promotion, RepoError> {
    let conn = ctx
        .db
        .conn()
        .map_err(|e| RepoError::Db(format!("promotion resolver connection: {e}")))?;
    let fields = promotable_fields(&row.entity_kind, payload);
    let bound: Option<BoundHead> = if row.entity_kind == "sku" {
        let mut head = if let Some(id) = row.entity_id {
            repo::find_sku(&conn, scope, tenant_id, id).await?
        } else {
            None
        };
        if head.is_none()
            && let Some(code) = field(payload, "sku_code")
        {
            head = repo::find_sku_by_code(&conn, scope, tenant_id, &code).await?;
        }
        match head {
            Some(h) => {
                let collections =
                    repo::frozen_collections(&conn, scope, tenant_id, "sku", h.sku_id).await?;
                let content = crate::api::rest::skus::sku_version_content(&h, &collections.values);
                Some((
                    h.sku_id,
                    h.lifecycle_state,
                    h.internal_revision,
                    h.published_version,
                    content,
                    bss_products_sdk::models::EntityKind::Sku,
                ))
            }
            None => None,
        }
    } else {
        let mut head = if let Some(id) = row.entity_id {
            repo::find_product(&conn, scope, tenant_id, id).await?
        } else {
            None
        };
        if head.is_none()
            && let Some(code) = field(payload, "product_code")
        {
            head = repo::find_product_by_code(&conn, scope, tenant_id, &code).await?;
        }
        if head.is_none()
            && let (Some(brand), Some(name_value)) = (
                field(payload, "brand_id").and_then(|raw| Uuid::parse_str(&raw).ok()),
                field(payload, "name"),
            )
        {
            head = repo::find_product_by_brand_and_name(
                &conn,
                scope,
                tenant_id,
                brand,
                &name::normalize(&name_value),
            )
            .await?;
        }
        match head {
            Some(h) => {
                let collections =
                    repo::frozen_collections(&conn, scope, tenant_id, "product", h.product_id)
                        .await?;
                let content = crate::api::rest::products::product_content(&h, &collections);
                Some((
                    h.product_id,
                    h.lifecycle_state,
                    h.internal_revision,
                    h.published_version,
                    content,
                    bss_products_sdk::models::EntityKind::Product,
                ))
            }
            None => None,
        }
    };
    let Some((entity_id, state, revision, published_version, content, kind)) = bound else {
        return Ok(Promotion::Create);
    };
    if matches!(
        state,
        bss_products_sdk::models::LifecycleState::Retired
            | bss_products_sdk::models::LifecycleState::Discarded
    ) {
        return Ok(Promotion::Conflict(DomainError::PromotionIdentityConflict(
            format!(
                "row {} resolves to a {} {} ({entity_id}): revival is clone-only (C5), a \
                 promotion never re-animates it",
                row.row_key,
                state.as_str(),
                row.entity_kind
            ),
        )));
    }
    let rendering = if kind == bss_products_sdk::models::EntityKind::Sku {
        crate::domain::canonical::canonical_rendering(
            &content,
            crate::domain::canonical::Absence::Null {
                roster: &crate::api::rest::skus::SKU_VERSION_CONTENT_ROSTER,
            },
        )
    } else {
        crate::domain::canonical::canonical_rendering(
            &content,
            crate::domain::canonical::Absence::Null {
                roster: &crate::api::rest::products::PRODUCT_CONTENT_ROSTER,
            },
        )
    };
    if head_is_dirty(
        &conn,
        scope,
        tenant_id,
        &row.entity_kind,
        entity_id,
        published_version,
        &rendering,
    )
    .await?
        || head_has_open_approval(&conn, scope, tenant_id, kind, entity_id, revision).await?
    {
        return Ok(Promotion::Conflict(DomainError::PromotionDirtyHead(
            format!(
                "row {} resolves to {} {entity_id}, which carries unpublished local edits or an \
             open approval: an import never merges into in-flight work (C5)",
                row.row_key, row.entity_kind
            ),
        )));
    }
    // Only the fields that differ are the promotion's change: an unchanged
    // structural field must not reach a save door that would refuse it after
    // first publish, while a *changed* one must (§7 row 2).
    let mut fields = fields;
    fields.retain(|key, value| content.get(key) != Some(value));
    if fields.is_empty() {
        Ok(Promotion::NoOp { entity_id })
    } else {
        Ok(Promotion::UpdateAsDraft {
            entity_id,
            revision,
            fields,
        })
    }
}

/// The marker an update-as-draft row carries in `governed_live_op`: the op
/// and the fields the save touched, which the abandon procedure reverts.
const UPDATE_AS_DRAFT_OP: &str = "update_as_draft";

/// The PII detector the worker's saves and retirements run under: the
/// tenant's own allowlist, read the way the doors read it.
async fn tenant_detector(
    conn: &(impl toolkit_db::secure::DBRunner + Sync),
    scope: &AccessScope,
    tenant_id: Uuid,
) -> Result<crate::domain::retention::RegistryPiiDetector, RepoError> {
    let values = repo::active_allowlist_values(conn, scope, tenant_id).await?;
    Ok(crate::domain::retention::RegistryPiiDetector::new(values))
}

/// Save a promotion's differing fields onto the bound head through the
/// ordinary save door (ungoverned, as every save is), and stamp the row with
/// the head, the revision the save left and the touched fields.
#[allow(clippy::too_many_arguments)] // the save's operands, threaded from the resolver
async fn stage_update_as_draft(
    ctx: &BulkWorkerContext,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor_ref: Uuid,
    batch_id: Uuid,
    row: &repo::BulkRowRecord,
    entity_id: Uuid,
    revision: i64,
    fields: std::collections::BTreeMap<String, JsonValue>,
    now: DateTime<Utc>,
) -> Result<bool, RepoError> {
    let conn = ctx
        .db
        .conn()
        .map_err(|e| RepoError::Db(format!("update-as-draft connection: {e}")))?;
    let detector = tenant_detector(&conn, scope, tenant_id).await?;
    let gate = StoredApprovalGate::ungoverned();
    let touched: Vec<String> = fields.keys().cloned().collect();
    let saved: Result<i64, String> = if row.entity_kind == "sku" {
        use crate::api::rest::skus;
        let inputs = skus::HeadActInputs {
            scope: scope.clone(),
            tenant_id,
            sku_id: entity_id,
            actor_ref,
            expected: revision,
            now,
            claim: None,
        };
        match skus::run_save(
            &conn,
            &inputs,
            &skus::SaveSkuRequest { fields },
            &gate,
            &detector,
            &ctx.sink,
        )
        .await
        {
            Ok(skus::MutationOutcome::Applied {
                internal_revision, ..
            }) => Ok(internal_revision),
            Ok(skus::MutationOutcome::Replay { .. }) => Ok(revision),
            Err(skus::HeadActError::Refused(refusal)) => Err(refusal.code().to_owned()),
            Err(skus::HeadActError::Vanished) => Err("HEAD_VANISHED".to_owned()),
            Err(skus::HeadActError::Db(error)) => return Err(RepoError::Db(error.to_string())),
        }
    } else {
        use crate::api::rest::products;
        let inputs = products::HeadActInputs {
            scope: scope.clone(),
            tenant_id,
            product_id: entity_id,
            actor_ref,
            expected: revision,
            now,
            claim: None,
        };
        match products::run_save(
            &conn,
            &inputs,
            &products::SaveProductRequest { fields },
            &gate,
            &detector,
            &ctx.sink,
        )
        .await
        {
            Ok(products::HeadActOutcome::Applied {
                internal_revision, ..
            }) => Ok(internal_revision),
            Ok(products::HeadActOutcome::Replay { .. }) => Ok(revision),
            Err(products::HeadActError::Refused(refusal)) => Err(refusal.code().to_owned()),
            Err(products::HeadActError::Vanished) => Err("HEAD_VANISHED".to_owned()),
            Err(products::HeadActError::Db(error)) => {
                return Err(RepoError::Db(error.to_string()));
            }
        }
    };
    match saved {
        Ok(new_revision) => {
            let marker = serde_json::json!({ "op": UPDATE_AS_DRAFT_OP, "touched": touched });
            repo::stamp_bulk_row_target(
                &conn,
                scope,
                tenant_id,
                batch_id,
                &row.row_key,
                entity_id,
                new_revision,
                &marker.to_string(),
            )
            .await?;
            Ok(true)
        }
        Err(code) => {
            repo::record_bulk_row_outcome(
                &conn,
                scope,
                tenant_id,
                batch_id,
                &row.row_key,
                BulkRowOutcome {
                    entity_id: Some(entity_id),
                    disposition: Some("failed"),
                    code: Some(&code),
                    reason: None,
                    now,
                },
            )
            .await?;
            Ok(false)
        }
    }
}

// ---------------------------------------------------------------------------
// The lifecycle lane (`dod-bulk-lifecycle`, p2)
// ---------------------------------------------------------------------------

/// The closed-set reason every lifecycle row's transition carries: this
/// feature writes no operator free text (P-D-50).
pub(crate) const LIFECYCLE_REASON: &str = "bulk-lifecycle";

/// The op a lifecycle row stages, off its `governed_live_op` marker.
fn lifecycle_op(row: &repo::BulkRowRecord) -> Option<String> {
    row.governed_live_op
        .as_deref()
        .and_then(|raw| serde_json::from_str::<JsonValue>(raw).ok())
        .and_then(|value| value.get("op")?.as_str().map(str::to_owned))
        .filter(|op| crate::api::rest::bulk::LIFECYCLE_OPS.contains(&op.as_str()))
}

/// Stage one lifecycle row: the head must exist and the transition must be
/// admissible from its state now (the ordinary `04` guard); the row pins the
/// head's revision and stays in flight for the commit.
async fn stage_lifecycle_row(
    ctx: &BulkWorkerContext,
    scope: &AccessScope,
    tenant_id: Uuid,
    batch_id: Uuid,
    row: &repo::BulkRowRecord,
    now: DateTime<Utc>,
) -> Result<bool, RepoError> {
    use bss_products_sdk::models::LifecycleState;
    let conn = ctx
        .db
        .conn()
        .map_err(|e| RepoError::Db(format!("lifecycle stage connection: {e}")))?;
    let Some(op) = lifecycle_op(row) else {
        return fail_row(&conn, scope, tenant_id, batch_id, row, "VALIDATION", now).await;
    };
    let target = if op == "retire" {
        LifecycleState::Retired
    } else {
        LifecycleState::Deprecated
    };
    let (state, revision) = match (row.entity_kind.as_str(), row.entity_id) {
        ("sku", Some(id)) => repo::find_sku(&conn, scope, tenant_id, id)
            .await?
            .map(|h| (h.lifecycle_state, h.internal_revision)),
        ("product", Some(id)) => repo::find_product(&conn, scope, tenant_id, id)
            .await?
            .map(|h| (h.lifecycle_state, h.internal_revision)),
        _ => None,
    }
    .map_or((None, 0), |(state, revision)| (Some(state), revision));
    let Some(state) = state else {
        return fail_row(&conn, scope, tenant_id, batch_id, row, "HEAD_VANISHED", now).await;
    };
    if let Err(refusal) = crate::domain::transition::guard(state, target) {
        return fail_row(&conn, scope, tenant_id, batch_id, row, refusal.code(), now).await;
    }
    repo::pin_bulk_row(&conn, scope, tenant_id, batch_id, &row.row_key, revision).await?;
    Ok(true)
}

async fn fail_row(
    conn: &(impl toolkit_db::secure::DBRunner + Sync),
    scope: &AccessScope,
    tenant_id: Uuid,
    batch_id: Uuid,
    row: &repo::BulkRowRecord,
    code: &str,
    now: DateTime<Utc>,
) -> Result<bool, RepoError> {
    repo::record_bulk_row_outcome(
        conn,
        scope,
        tenant_id,
        batch_id,
        &row.row_key,
        BulkRowOutcome {
            entity_id: row.entity_id,
            disposition: Some("failed"),
            code: Some(code),
            reason: None,
            now,
        },
    )
    .await?;
    Ok(false)
}

/// Drive one lifecycle row's transition through the ordinary `04` door in
/// `PreAuthorized` mode naming the batch's consumed record, provenance
/// `direct`, the per-head guards intact — a referenced SKU defers under its
/// own guard and the batch never force-retires.
#[allow(clippy::too_many_arguments)] // the door's operands, threaded from the walk
async fn apply_lifecycle_row(
    ctx: &BulkWorkerContext,
    conn: &(impl toolkit_db::secure::DBRunner + Sync),
    scope: &AccessScope,
    tenant_id: Uuid,
    batch_id: Uuid,
    row: &repo::BulkRowRecord,
    entity_id: Uuid,
    gate: &StoredApprovalGate,
    approval_id: ApprovalId,
    actor_ref: Uuid,
    now: DateTime<Utc>,
) -> Result<Result<i64, String>, RepoError> {
    let Some(op) = lifecycle_op(row) else {
        return Ok(Err("VALIDATION".to_owned()));
    };
    let claim = row_claim(ctx, batch_id, &row.row_key, now);
    match idempotency::claim_idempotency(conn, scope, tenant_id, &claim).await? {
        ClaimVerdict::Replay { .. } => return Ok(Ok(0)),
        ClaimVerdict::Refused(error) => return Ok(Err(error.code().to_owned())),
        ClaimVerdict::Proceed => {}
    }
    let detector = tenant_detector(conn, scope, tenant_id).await?;
    let mode = GateMode::PreAuthorized(approval_id);
    let result: Result<i64, String> = if row.entity_kind == "sku" {
        use crate::api::rest::skus;
        let Some(head) = repo::find_sku(conn, scope, tenant_id, entity_id).await? else {
            return Ok(Err("HEAD_VANISHED".to_owned()));
        };
        let inputs = skus::HeadActInputs {
            scope: scope.clone(),
            tenant_id,
            sku_id: entity_id,
            actor_ref,
            expected: row.pinned_revision.unwrap_or(head.internal_revision),
            now,
            claim: None,
        };
        let outcome = if op == "retire" {
            skus::run_retire(
                conn,
                &inputs,
                gate,
                mode,
                ctx.eol_enabled,
                &detector,
                // The batch's one approval is the per-row confirmation the
                // interactive door asks for, aggregated into the report.
                &skus::RetireSkuRequest {
                    reason: LIFECYCLE_REASON.to_owned(),
                    replaced_by: None,
                    effective_at: None,
                    must_migrate_by: None,
                    confirmed: true,
                },
                &ctx.sink,
            )
            .await
        } else {
            skus::run_deprecate(conn, &inputs, gate, mode, &ctx.sink).await
        };
        match outcome {
            Ok(skus::MutationOutcome::Applied {
                internal_revision, ..
            }) => Ok(internal_revision),
            Ok(skus::MutationOutcome::Replay { .. }) => Ok(0),
            Err(skus::HeadActError::Refused(refusal)) => Err(refusal.code().to_owned()),
            Err(skus::HeadActError::Vanished) => Err("HEAD_VANISHED".to_owned()),
            Err(skus::HeadActError::Db(error)) => return Err(RepoError::Db(error.to_string())),
        }
    } else {
        use crate::api::rest::products;
        let Some(head) = repo::find_product(conn, scope, tenant_id, entity_id).await? else {
            return Ok(Err("HEAD_VANISHED".to_owned()));
        };
        let inputs = products::HeadActInputs {
            scope: scope.clone(),
            tenant_id,
            product_id: entity_id,
            actor_ref,
            expected: row.pinned_revision.unwrap_or(head.internal_revision),
            now,
            claim: None,
        };
        let outcome = if op == "retire" {
            products::run_retire(
                conn,
                &inputs,
                scope,
                gate,
                mode,
                ctx.eol_enabled,
                &detector,
                &products::RetireProductRequest {
                    reason: LIFECYCLE_REASON.to_owned(),
                    replaced_by: None,
                    effective_at: None,
                    must_migrate_by: None,
                    confirmed: true,
                    cascade_confirmed: Some(true),
                },
                &ctx.sink,
            )
            .await
        } else {
            products::run_deprecate(conn, &inputs, scope, gate, mode, &ctx.sink).await
        };
        match outcome {
            Ok(products::HeadActOutcome::Applied {
                internal_revision, ..
            }) => Ok(internal_revision),
            Ok(products::HeadActOutcome::Replay { .. }) => Ok(0),
            Err(products::HeadActError::Refused(refusal)) => Err(refusal.code().to_owned()),
            Err(products::HeadActError::Vanished) => Err("HEAD_VANISHED".to_owned()),
            Err(products::HeadActError::Db(error)) => {
                return Err(RepoError::Db(error.to_string()));
            }
        }
    };
    idempotency::record_idempotency_answer(
        conn,
        scope,
        tenant_id,
        &claim,
        axum::http::StatusCode::OK,
        &row_answer(&row.row_key, entity_id, &result),
    )
    .await?;
    Ok(result)
}

/// Revert an abandoned update-as-draft row through the ordinary save door:
/// the fields the resolver touched, at the values the last frozen version
/// carries, so the head returns to its published content with a revision
/// bump (`dod-resume-abandon`).
async fn revert_update_as_draft(
    ctx: &BulkWorkerContext,
    conn: &(impl toolkit_db::secure::DBRunner + Sync),
    scope: &AccessScope,
    tenant_id: Uuid,
    row: &repo::BulkRowRecord,
    entity_id: Uuid,
    now: DateTime<Utc>,
) -> Result<bool, RepoError> {
    let touched: Vec<String> = row
        .governed_live_op
        .as_deref()
        .and_then(|raw| serde_json::from_str::<JsonValue>(raw).ok())
        .and_then(|value| value.get("touched")?.as_array().cloned())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let kind = if row.entity_kind == "sku" {
        repo::VersionedEntityKind::Sku
    } else {
        repo::VersionedEntityKind::Product
    };
    let Some((_, frozen)) =
        repo::latest_entity_version(conn, scope, tenant_id, kind, entity_id).await?
    else {
        return Ok(false);
    };
    let frozen: JsonValue = serde_json::from_str(&frozen).unwrap_or_default();
    let fields: std::collections::BTreeMap<String, JsonValue> = touched
        .into_iter()
        .filter_map(|key| frozen.get(&key).cloned().map(|value| (key, value)))
        .collect();
    if fields.is_empty() {
        return Ok(false);
    }
    let detector = tenant_detector(conn, scope, tenant_id).await?;
    let gate = StoredApprovalGate::ungoverned();
    let actor_ref = Uuid::nil();
    if row.entity_kind == "sku" {
        use crate::api::rest::skus;
        let Some(head) = repo::find_sku(conn, scope, tenant_id, entity_id).await? else {
            return Ok(false);
        };
        let inputs = skus::HeadActInputs {
            scope: scope.clone(),
            tenant_id,
            sku_id: entity_id,
            actor_ref,
            expected: head.internal_revision,
            now,
            claim: None,
        };
        Ok(skus::run_save(
            conn,
            &inputs,
            &skus::SaveSkuRequest { fields },
            &gate,
            &detector,
            &ctx.sink,
        )
        .await
        .is_ok())
    } else {
        use crate::api::rest::products;
        let Some(head) = repo::find_product(conn, scope, tenant_id, entity_id).await? else {
            return Ok(false);
        };
        let inputs = products::HeadActInputs {
            scope: scope.clone(),
            tenant_id,
            product_id: entity_id,
            actor_ref,
            expected: head.internal_revision,
            now,
            claim: None,
        };
        Ok(products::run_save(
            conn,
            &inputs,
            &products::SaveProductRequest { fields },
            &gate,
            &detector,
            &ctx.sink,
        )
        .await
        .is_ok())
    }
}

#[cfg(test)]
#[path = "bulk_worker_tests.rs"]
mod bulk_worker_tests;
