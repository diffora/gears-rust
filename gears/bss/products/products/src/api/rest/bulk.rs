//! The bulk import door and the `RowLedger` reader — `design/09` §2 rule 1
//! (`dod-import-door`, `dod-bulk-errors`' reporting surface; P-D-61,
//! P-D-69).
//!
//! # Three key scopes, and conflating them is the defect this module exists
//! to avoid
//!
//! The **batch** key is this door's idempotency operand, held by
//! `products_bulk_batch`'s own UNIQUE — so a replay answers the existing
//! batch rather than minting a second. The **row** keys are
//! **batch-scoped** and live in the ledger, which is why a row re-listed in
//! a new batch is a new act whose fate its own stage validation decides. A
//! row reaching the publish door resolves *that* door's key under the
//! reserved lane [`INTERNAL_BULK_ROW_LANE`] with the ledger row's surrogate
//! id as the client key — the Foundation's scope, a third one.
//!
//! # `mode`, and why the default is the strict one
//!
//! `import` (the default) leaves a bound `skuCode` carrying different
//! content as the ordinary `DUPLICATE_CODE`; only `promote` engages the
//! `PromotionResolver`'s update-as-draft (P-D-69). A silent auto-update on
//! collision would convert typos into overwrites, which is why the
//! permissive mode is the one a caller must ask for. The resolver itself
//! arrives with its own `DoD`; this door records the mode the batch runs
//! under.
//!
//! # The bounds are the door's own refusal
//!
//! `BULK_LIMIT` covers both of `inst-bm-limits`' operands — rows per batch
//! and the tenant's concurrent-batch ceiling — and is the one of the five
//! bulk codes that is a **response** rather than a per-row ledger outcome.
//! The worker re-checks the ceiling at claim (P-D-54): a ceiling checked
//! only here drifts as batches hang.
//!
//! # What this door does NOT do
//!
//! It stages nothing: the batch lands `staging` with its whole ledger and
//! the **worker** runs the rows (`dod-stage-phase`). The commit phase, the
//! change report and the promotion resolver are their own `DoD`s and their
//! own §7 rows.
//!
//! # The row's content, and who parses it
//!
//! Each row carries a `content` object the door records **canonically
//! serialized** into the ledger's `staged_payload` (**P-D-86**) and judges
//! only for objecthood. The field names are the **worker's** to parse,
//! through the same shape rules interactive authoring runs — a door that
//! parsed them here would be the second validator this feature's whole
//! correctness argument forbids.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-import-door:p1
//! @cpt-dod:cpt-cf-bss-products-dod-bulk-seams:p1

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::Extension;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use toolkit::api::OpenApiRegistry;
use toolkit::api::canonical_prelude::{CanonicalError, resource_error};
use toolkit::api::operation_builder::OperationBuilder;
use toolkit_db::secure::AccessScope;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::api::rest::{ApiState, repo_error_to_canonical, require_authenticated};
use crate::domain::canonical;
use crate::domain::error::DomainError;
use crate::domain::validation::ValidationReport;
use crate::infra::storage::repo::{self, NewBulkBatch, NewBulkRow, RefusalSubject};

/// `OpenAPI` tag for the bulk surface's operations.
const TAG: &str = "BSS Products";

/// The reserved idempotency lane a per-row publish claims under
/// (**P-D-26**, **P-D-69**) — the constant `dod-idempotency-lane` obliges,
/// the lane having been reserved in prose and in no code until now. The
/// `client_key` under it is the ledger row's **`row_id`**, its own
/// surrogate: a row re-listed in a new batch gets a new one and stays a new
/// act, with no batch column added to the shipped primary key.
///
/// @cpt-dod:cpt-cf-bss-products-dod-idempotency-lane:p1
pub const INTERNAL_BULK_ROW_LANE: &str = "internal:bulk-row";

/// The canonical-error identity of this surface's refusals.
#[resource_error(gts_id!("cf.bss.products.bulk.v1~"))]
struct BulkResource;

/// Build the Axum router for the import door and the ledger reader.
pub(crate) fn router(state: Arc<ApiState>, openapi: &dyn OpenApiRegistry) -> Router {
    let router = OperationBuilder::post("/bss-products/v1/bulk/imports")
        .operation_id("bss_products.import_batch")
        .summary("Import a batch")
        .description(
            "Records a batch and its whole row ledger, answering 202: the rows are staged by \
             the gear's own batch worker, not by this call. Idempotent on the batch key, which \
             the batch table's own UNIQUE holds, so a replay answers the existing batch rather \
             than minting a second. The batch-level mode is import (default) or promote: only \
             promote engages the promotion resolver's update-as-draft, and under import a bound \
             code carrying different content stays the ordinary DUPLICATE_CODE. Row keys are \
             batch-scoped. Refuses BULK_LIMIT over either configured bound (rows per batch, or \
             the tenant's concurrent-batch ceiling). Gates on bulk x execute.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .json_request::<ImportBatchRequest>(openapi, "The batch and its rows.")
        .handler(import_batch)
        .json_response_with_schema::<BatchAcceptedView>(
            openapi,
            StatusCode::ACCEPTED,
            "The recorded batch.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(Router::new(), openapi);

    OperationBuilder::get("/bss-products/v1/bulk/batches/{id}")
        .operation_id("bss_products.read_batch")
        .summary("Read a batch and its row ledger")
        .description(
            "Returns the batch's state and one ledger entry per row: its key, kind, entity \
             (once minted), disposition, and on a failure the owning feature's code \
             verbatim, bulk introducing no parallel taxonomy. This is the surface that reports \
             the four per-row bulk codes (P-D-61); one route serves both lanes. Gates on \
             bulk x read.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("id", "The batch to read.")
        .handler(read_batch)
        .json_response_with_schema::<BatchLedgerView>(
            openapi,
            StatusCode::OK,
            "The batch and its ledger.",
        )
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi)
        .layer(Extension(state))
}

/// One row of an import request.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request)]
pub struct ImportRowRequest {
    /// The caller's own key for this row, unique **within the batch**.
    pub row_key: String,
    /// `product` or `sku`.
    pub entity_kind: String,
    /// The entity this row targets, for an update-as-draft row.
    pub entity_id: Option<Uuid>,
    /// The revision this row pins, for an update-as-draft row.
    pub pinned_revision: Option<i64>,
    /// The row's content — what the worker parses and stages
    /// (**P-D-86**). A `product` row carries `{name, brand_id,
    /// product_code?, region_scope?, brand_scope?}`; a `sku` row
    /// `{product_id, sku_code, region_scope?, brand_scope?}`. The door
    /// records it canonically serialized and judges only that it is an
    /// object: **the field names are the worker's to parse**, through the
    /// same shape rules interactive authoring runs, which is what keeps
    /// bulk from becoming a second validator.
    pub content: serde_json::Value,
}

/// The import door's body.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request)]
pub struct ImportBatchRequest {
    /// The batch's idempotency key, unique per tenant.
    pub batch_key: String,
    /// `import` (default) or `promote`.
    pub mode: Option<String>,
    /// The rows, in dependency order.
    pub rows: Vec<ImportRowRequest>,
}

/// What the import door answers.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct BatchAcceptedView {
    /// The batch's server-minted id.
    pub batch_id: Uuid,
    /// The caller's key, echoed.
    pub batch_key: String,
    /// The mode the batch runs under.
    pub mode: String,
    /// The batch's state — `staging` on a fresh batch, whatever the worker
    /// has made of it on a replay.
    pub state: String,
    /// How many rows the ledger holds.
    pub row_count: usize,
    /// Whether this answer replayed an existing batch rather than minting
    /// one.
    pub replayed: bool,
}

/// One ledger entry, as the reader reports it.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct RowLedgerEntryView {
    /// The caller's own key.
    pub row_key: String,
    /// The lane's client key.
    pub row_id: Uuid,
    /// `product` or `sku`.
    pub entity_kind: String,
    /// The entity, once minted.
    pub entity_id: Option<Uuid>,
    /// NULL while the row is in flight.
    pub disposition: Option<String>,
    /// The owning feature's code on a failure.
    pub code: Option<String>,
    /// A closed-set literal, never operator text.
    pub reason: Option<String>,
}

/// The ledger reader's answer.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct BatchLedgerView {
    /// The batch.
    pub batch_id: Uuid,
    /// The caller's key.
    pub batch_key: String,
    /// The mode.
    pub mode: String,
    /// The lane.
    pub lane: String,
    /// The state machine's current value.
    pub state: String,
    /// One entry per row — the no-hidden-partial-failure surface.
    pub rows: Vec<RowLedgerEntryView>,
}

/// The bulk surface's gate, one action per door.
async fn bulk_scope(
    state: &ApiState,
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
    tenant_id: Uuid,
    actor_ref: Uuid,
    action: &'static str,
    subject: String,
) -> Result<AccessScope, CanonicalError> {
    match crate::authz::access_scope(
        enforcer,
        ctx,
        &crate::authz::resource_types::BULK,
        action,
        Some(tenant_id),
        None,
        true,
    )
    .await
    {
        Ok(scope) => Ok(scope),
        Err(crate::authz::AuthzError::Denied(reason)) => {
            let self_scope = AccessScope::for_tenant(tenant_id);
            Err(crate::api::rest::audit_refusal_and_report(
                state,
                &self_scope,
                crate::api::rest::RefusalAuditContext {
                    tenant_id,
                    actor_ref,
                    subject_kind: crate::authz::labels::BULK,
                    error_code: "PERMISSION_DENIED",
                },
                RefusalSubject::Attempted(subject),
                BulkResource::permission_denied()
                    .with_reason(reason)
                    .create(),
            )
            .await)
        }
        Err(err @ crate::authz::AuthzError::Unavailable(_)) => {
            Err(crate::api::rest::authz_error_to_canonical(err, |reason| {
                BulkResource::permission_denied()
                    .with_reason(reason)
                    .create()
            }))
        }
    }
}

/// One audited refusal of the bulk surface.
async fn refuse_bulk(
    state: &ApiState,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor_ref: Uuid,
    subject: String,
    refusal: DomainError,
) -> CanonicalError {
    let code = refusal.code();
    crate::api::rest::audit_refusal_and_report(
        state,
        scope,
        crate::api::rest::RefusalAuditContext {
            tenant_id,
            actor_ref,
            subject_kind: crate::authz::labels::BULK,
            error_code: code,
        },
        RefusalSubject::Attempted(subject),
        CanonicalError::from(refusal),
    )
    .await
}

/// The body's shape, judged in one collected report (P-D-33).
fn validate_import_shape(body: &ImportBatchRequest, max_rows: u32) -> ValidationReport {
    let mut report = ValidationReport::new();
    if body.batch_key.trim().is_empty() {
        report.violate("VALIDATION", "batch_key", "batch_key must not be blank");
    }
    match body.mode.as_deref().map(str::trim) {
        None | Some("" | "import" | "promote") => {}
        Some(_) => {
            report.violate("VALIDATION", "mode", "mode must be import or promote");
        }
    }
    if body.rows.len() > max_rows as usize {
        // The bound's own refusal is BULK_LIMIT, raised by the caller; this
        // only keeps the shape pass from walking a set it will refuse.
        return report;
    }
    // A HashSet, not a Vec scan: the batch admits up to max_rows
    // caller-controlled keys, and a linear `contains` inside the loop is
    // O(n²) string comparisons on the request path.
    let mut seen: std::collections::HashSet<&str> =
        std::collections::HashSet::with_capacity(body.rows.len());
    for row in &body.rows {
        let key = row.row_key.trim();
        if key.is_empty() {
            report.violate("VALIDATION", "rows.row_key", "row_key must not be blank");
        } else if !seen.insert(key) {
            report.violate(
                "VALIDATION",
                "rows.row_key",
                "row keys are batch-scoped and must be unique within the batch",
            );
        }
        if !matches!(row.entity_kind.trim(), "product" | "sku") {
            report.violate(
                "VALIDATION",
                "rows.entity_kind",
                "entity_kind must be product or sku; live-entity kinds arrive with their \
                 own stores",
            );
        }
        if !row.content.is_object() {
            report.violate(
                "VALIDATION",
                "rows.content",
                "content must be an object carrying the row's fields",
            );
        }
    }
    report
}

/// `POST /bss-products/v1/bulk/imports`.
async fn import_batch(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Json(body): Json<ImportBatchRequest>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(Utc::now());
    let batch_key = body.batch_key.trim().to_owned();

    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;
    let scope = bulk_scope(
        &state,
        &enforcer,
        &ctx,
        tenant_id,
        actor_ref,
        crate::authz::actions::EXECUTE,
        batch_key.clone(),
    )
    .await?;

    let report = validate_import_shape(&body, state.bulk_max_rows_per_batch);
    if !report.is_empty() {
        return Err(refuse_bulk(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            batch_key,
            DomainError::Validation(report),
        )
        .await);
    }

    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&crate::infra::storage::RepoError::Db(format!(
            "import door connection: {e}"
        )))
    })?;

    // The replay: the batch key's own UNIQUE is the idempotency, so an
    // existing batch is answered rather than re-minted.
    if let Some(existing) = repo::find_batch_by_key(&conn, &scope, tenant_id, &batch_key)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?
    {
        let rows = repo::find_batch_rows(&conn, &scope, tenant_id, existing.batch_id)
            .await
            .map_err(|e| repo_error_to_canonical(&e))?;
        return Ok((
            StatusCode::ACCEPTED,
            Json(BatchAcceptedView {
                batch_id: existing.batch_id,
                batch_key: existing.batch_key,
                mode: existing.mode,
                state: existing.state,
                row_count: rows.len(),
                replayed: true,
            }),
        )
            .into_response());
    }

    // Both bounds, one code (`inst-bm-limits`).
    let row_count = body.rows.len();
    if row_count > state.bulk_max_rows_per_batch as usize {
        let refusal = DomainError::BulkLimit(format!(
            "the batch carries {row_count} rows; the configured maximum is {}",
            state.bulk_max_rows_per_batch
        ));
        return Err(refuse_bulk(&state, &scope, tenant_id, actor_ref, batch_key, refusal).await);
    }
    let live = repo::count_live_batches(&conn, &scope, tenant_id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?;
    if live >= u64::from(state.bulk_max_concurrent_batches_per_tenant) {
        let refusal = DomainError::BulkLimit(format!(
            "the tenant already holds {live} live batches; the configured ceiling is {}",
            state.bulk_max_concurrent_batches_per_tenant
        ));
        return Err(refuse_bulk(&state, &scope, tenant_id, actor_ref, batch_key, refusal).await);
    }

    let mode = body
        .mode
        .as_deref()
        .map(str::trim)
        .filter(|mode| !mode.is_empty())
        .unwrap_or("import")
        .to_owned();
    let rows: Vec<NewBulkRow> = body
        .rows
        .iter()
        .map(|row| NewBulkRow {
            row_key: row.row_key.trim().to_owned(),
            row_id: Uuid::new_v4(),
            entity_kind: row.entity_kind.trim().to_owned(),
            entity_id: row.entity_id,
            pinned_revision: row.pinned_revision,
            // Canonically serialized through the gear's one rendering rule,
            // so P-D-69 arm 5's digest over "the row's staged payload" is
            // computable and one row hashes alike however its fields
            // arrived ordered (P-D-86).
            staged_payload: Some(canonical::canonical_rendering(
                &row.content,
                canonical::Absence::Omit,
            )),
        })
        .collect();

    let batch_id = Uuid::new_v4();
    let new = NewBulkBatch {
        batch_id,
        batch_key: batch_key.clone(),
        mode: mode.clone(),
        lane: "import".to_owned(),
        operation_key: None,
        created_at: now,
    };
    let scope_for_tx = scope.clone();
    let rows_for_tx = rows.clone();
    state
        .db
        .db()
        .transaction_with_retry::<(), toolkit_db::DbError, _, _>(
            toolkit_db::secure::TxConfig::default(),
            crate::api::rest::contention_db_err,
            move |tx| {
                let scope = scope_for_tx.clone();
                let new = new.clone();
                let rows = rows_for_tx.clone();
                Box::pin(async move {
                    repo::insert_bulk_batch(tx, &scope, tenant_id, new, &rows)
                        .await
                        .map_err(|e| toolkit_db::DbError::Sea(e.to_db_err()))?;
                    Ok(())
                })
            },
        )
        .await
        .map_err(|e| {
            repo_error_to_canonical(&crate::infra::storage::RepoError::Db(e.to_string()))
        })?;

    Ok((
        StatusCode::ACCEPTED,
        Json(BatchAcceptedView {
            batch_id,
            batch_key,
            mode,
            state: "staging".to_owned(),
            row_count,
            replayed: false,
        }),
    )
        .into_response())
}

/// `GET /bss-products/v1/bulk/batches/{id}`.
async fn read_batch(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    axum::extract::Path(batch_id): axum::extract::Path<Uuid>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(Utc::now());

    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;
    let scope = bulk_scope(
        &state,
        &enforcer,
        &ctx,
        tenant_id,
        actor_ref,
        crate::authz::actions::READ,
        batch_id.to_string(),
    )
    .await?;

    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&crate::infra::storage::RepoError::Db(format!(
            "ledger reader connection: {e}"
        )))
    })?;
    let Some(batch) = repo::find_batch(&conn, &scope, tenant_id, batch_id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?
    else {
        return Err(
            BulkResource::not_found("no batch matches this id in the caller's scope")
                .with_resource(batch_id.to_string())
                .create(),
        );
    };
    let rows = repo::find_batch_rows(&conn, &scope, tenant_id, batch_id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?;

    Ok((
        StatusCode::OK,
        Json(BatchLedgerView {
            batch_id: batch.batch_id,
            batch_key: batch.batch_key,
            mode: batch.mode,
            lane: batch.lane,
            state: batch.state,
            rows: rows
                .into_iter()
                .map(|row| RowLedgerEntryView {
                    row_key: row.row_key,
                    row_id: row.row_id,
                    entity_kind: row.entity_kind,
                    entity_id: row.entity_id,
                    disposition: row.disposition,
                    code: row.code,
                    reason: row.reason,
                })
                .collect(),
        }),
    )
        .into_response())
}

#[cfg(test)]
#[path = "bulk_tests.rs"]
mod bulk_tests;
