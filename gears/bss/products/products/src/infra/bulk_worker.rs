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
//! @cpt-dod:cpt-cf-bss-products-dod-stage-phase:p1

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde_json::Value as JsonValue;
use toolkit_db::secure::AccessScope;
use uuid::Uuid;

use crate::api::rest::ApiState;
use crate::domain::error::DomainError;
use crate::domain::name;
use crate::domain::validation::ValidationReport;
use crate::infra::storage::RepoError;
use crate::infra::storage::repo::{self, BulkRowOutcome, NewProduct, NewSku};

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
    state: &ApiState,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor_ref: Uuid,
    payload: &JsonValue,
    now: DateTime<Utc>,
) -> Result<Uuid, DomainError> {
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
        return Err(DomainError::Validation(report));
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
    match crate::api::rest::products::insert_product_with_event(
        state,
        scope.clone(),
        new,
        None,
        actor_ref,
    )
    .await
    {
        Ok(_) => Ok(product_id),
        Err(db_error) => Err(insert_failure(&db_error.to_string(), "product")),
    }
}

/// Stage one SKU row through the Foundation's own insert path.
async fn stage_sku(
    state: &ApiState,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor_ref: Uuid,
    payload: &JsonValue,
    now: DateTime<Utc>,
) -> Result<Uuid, DomainError> {
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
        return Err(DomainError::Validation(report));
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
    match crate::api::rest::skus::insert_sku_with_event(state, scope.clone(), new, None, actor_ref)
        .await
    {
        Ok(_) => Ok(sku_id),
        Err(db_error) => Err(insert_failure(&db_error.to_string(), "sku")),
    }
}

/// Classify an insert failure into the **owning feature's** code — bulk
/// introduces no parallel taxonomy, so a duplicate is the Foundation's own
/// `DUPLICATE_NAME`/`DUPLICATE_CODE` and everything else stays a storage
/// failure the pass reports as such.
fn insert_failure(message: &str, kind: &str) -> DomainError {
    let lower = message.to_ascii_lowercase();
    let unique = lower.contains("unique constraint") || lower.contains("duplicate key");
    if unique && lower.contains("name") {
        return DomainError::DuplicateName(format!("{kind} name is already reserved"));
    }
    if unique {
        return DomainError::DuplicateCode(format!("{kind} code is already reserved"));
    }
    DomainError::Validation({
        let mut report = ValidationReport::new();
        report.violate("VALIDATION", "row", "the row could not be staged");
        report
    })
}

/// One pass over one tenant's oldest `staging` batch.
///
/// # Errors
///
/// [`RepoError`] as the reads and writes raise it.
pub(crate) async fn stage_next_batch(
    state: &Arc<ApiState>,
    tenant_id: Uuid,
    actor_ref: Uuid,
    now: DateTime<Utc>,
) -> Result<StageOutcome, RepoError> {
    let scope = AccessScope::for_tenant(tenant_id);
    let (batch, rows) = {
        let conn = state
            .db
            .conn()
            .map_err(|e| RepoError::Db(format!("batch worker connection: {e}")))?;
        let live = repo::count_live_batches(&conn, &scope, tenant_id).await?;
        if live > u64::from(state.bulk_max_concurrent_batches_per_tenant) {
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
        let conn = state
            .db
            .conn()
            .map_err(|e| RepoError::Db(format!("batch claim connection: {e}")))?;
        if !repo::claim_bulk_batch(&conn, &scope, tenant_id, batch.batch_id, batch.attempt, now)
            .await?
        {
            return Ok(StageOutcome::ClaimLost);
        }
    }

    let mut staged = 0usize;
    let mut failed = 0usize;
    for row in rows {
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
        let payload: JsonValue = row
            .staged_payload
            .as_deref()
            .and_then(|raw| serde_json::from_str(raw).ok())
            .unwrap_or(JsonValue::Null);

        let outcome = match row.entity_kind.as_str() {
            "product" => stage_product(state, &scope, tenant_id, actor_ref, &payload, now).await,
            "sku" => stage_sku(state, &scope, tenant_id, actor_ref, &payload, now).await,
            other => Err(DomainError::Validation({
                let mut report = ValidationReport::new();
                report.violate(
                    "VALIDATION",
                    "entity_kind",
                    format!("no staging path for entity_kind {other}"),
                );
                report
            })),
        };

        let conn = state
            .db
            .conn()
            .map_err(|e| RepoError::Db(format!("row outcome connection: {e}")))?;
        match outcome {
            Ok(entity_id) => {
                staged += 1;
                repo::record_bulk_row_outcome(
                    &conn,
                    &scope,
                    tenant_id,
                    batch.batch_id,
                    &row.row_key,
                    BulkRowOutcome {
                        entity_id: Some(entity_id),
                        disposition: None,
                        code: None,
                        now,
                    },
                )
                .await?;
            }
            Err(refusal) => {
                failed += 1;
                repo::record_bulk_row_outcome(
                    &conn,
                    &scope,
                    tenant_id,
                    batch.batch_id,
                    &row.row_key,
                    BulkRowOutcome {
                        entity_id: None,
                        disposition: Some("failed"),
                        code: Some(refusal.code()),
                        now,
                    },
                )
                .await?;
            }
        }
    }

    // Edge 1 (P-D-54): the pass that stages the last row reports the batch.
    let conn = state
        .db
        .conn()
        .map_err(|e| RepoError::Db(format!("batch report connection: {e}")))?;
    repo::move_bulk_batch_state(
        &conn,
        &scope,
        tenant_id,
        batch.batch_id,
        "staging",
        "reported",
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
/// # Errors
///
/// The first [`RepoError`] a tenant's pass raises; later tenants run next
/// tick.
pub(crate) async fn sweep(
    state: &Arc<ApiState>,
    actor_ref: Uuid,
    now: DateTime<Utc>,
) -> Result<(), RepoError> {
    let tenants = {
        let conn = state
            .db
            .conn()
            .map_err(|e| RepoError::Db(format!("batch sweep connection: {e}")))?;
        repo::tenants_with_staging_batches(&conn, &AccessScope::allow_all()).await?
    };
    for tenant in tenants {
        stage_next_batch(state, tenant, actor_ref, now).await?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "bulk_worker_tests.rs"]
mod bulk_worker_tests;
