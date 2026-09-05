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

use crate::api::rest::dto::ManifestCaptureView;
use crate::api::rest::dto::{
    BatchAcceptedView, BatchLedgerView, ImportBatchRequest, RowLedgerEntryView,
};
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

    let router = OperationBuilder::get("/bss-products/v1/bulk/batches/{id}")
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
        .register(router, openapi);
    let router = OperationBuilder::get("/bss-products/v1/bulk/exports")
        .operation_id("bss_products.export_catalog_version")
        .summary("Export a catalog version as a deterministic artifact")
        .description(
            "Rendered from the stored manifest of `catalogVersionId`: every entry's frozen \
             content (its version row, never the head) with its promotion identity, and \
             every capture from the capture store; sorted throughout, byte-identical for a \
             given version (C4, P-D-29), streamed rather than stored. The header carries the \
             artifact's format version. Gates on `bulk x read` (P-D-127); an unknown version is \
             CATALOG_VERSION_UNKNOWN.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .query_param("catalogVersionId", true, "The version to export.")
        .handler(export_catalog_version)
        .json_response_with_schema::<ExportArtifactView>(
            openapi,
            StatusCode::OK,
            "The artifact: header, entries with frozen content, captures.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);
    OperationBuilder::post("/bss-products/v1/bulk/lifecycle")
        .operation_id("bss_products.start_lifecycle_batch")
        .summary("Start a bulk lifecycle batch (mass deprecate or retire-initiate)")
        .description(
            "One batch over an id list, each row the ordinary `04` transition door in \
             PreAuthorized mode under the batch's one approval, material at any size by its \
             transitions; a referenced row defers under the ordinary guard and the batch never \
             force-retires. Gates on `bulk_lifecycle x execute`, its own grant: the import pair \
             does not reach this door (P-D-69).",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .json_request::<LifecycleBatchRequest>(
            openapi,
            "The batch key, the op (deprecate | retire), the entity kind and the ids.",
        )
        .handler(start_lifecycle_batch)
        .json_response_with_schema::<BatchAcceptedView>(
            openapi,
            StatusCode::ACCEPTED,
            "The batch, accepted into staging; a replayed key answers the existing batch.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi)
        .layer(Extension(state))
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
    bulk_scope_on(
        state, enforcer, ctx, tenant_id, actor_ref, action, subject, false,
    )
    .await
}

/// [`bulk_scope`] over either of the surface's two labels: `bulk` for the
/// import pair and the export, `bulk_lifecycle` for the lifecycle door — its
/// own grant, so the gear's most destructive batch act is never reachable
/// with the import pair (`dod-bulk-lifecycle`, P-D-69).
#[allow(clippy::too_many_arguments)] // the gate's operands plus the label switch
async fn bulk_scope_on(
    state: &ApiState,
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
    tenant_id: Uuid,
    actor_ref: Uuid,
    action: &'static str,
    subject: String,
    lifecycle: bool,
) -> Result<AccessScope, CanonicalError> {
    let resource = if lifecycle {
        &crate::authz::resource_types::BULK_LIFECYCLE
    } else {
        &crate::authz::resource_types::BULK
    };
    let label = if lifecycle {
        crate::authz::labels::BULK_LIFECYCLE
    } else {
        crate::authz::labels::BULK
    };
    match crate::authz::access_scope(enforcer, ctx, resource, action, Some(tenant_id), None, true)
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
                    subject_kind: label,
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
                state: existing.state.as_str().to_owned(),
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
            governed_live_op: None,
        })
        .collect();

    let batch_id = Uuid::new_v4();
    let new = NewBulkBatch {
        batch_id,
        batch_key: batch_key.clone(),
        mode: mode.clone(),
        lane: "import".to_owned(),
        // One batch, one catalog version (`dod-operation-key`): the tag the
        // commit's bulk-lane increment request carries is the batch's own id.
        operation_key: Some(batch_id.to_string()),
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
            state: batch.state.as_str().to_owned(),
            approval_ref: batch.approval_ref,
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

// ---------------------------------------------------------------------------
// GET /bss-products/v1/bulk/exports?catalogVersionId= (inst-bk-export)
// ---------------------------------------------------------------------------

/// The export artifact's format version — the header every artifact carries
/// so slice `12`'s vN -> vN+1 discipline has a number to move (P-D-127 row 3).
pub const EXPORT_FORMAT_VERSION: u32 = 1;

#[derive(Debug, serde::Deserialize)]
struct ExportQuery {
    #[serde(rename = "catalogVersionId")]
    catalog_version_id: Option<i64>,
}

/// One manifest entry with its frozen content and its promotion identity.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct ExportEntryView {
    /// `product` or `sku`.
    pub entity_kind: String,
    /// The entity.
    pub entity_id: Uuid,
    /// The frozen version the manifest names.
    pub published_version: i64,
    /// C5's identity for a promotion, canonically rendered: `skuCode` for a
    /// SKU; `productCode`, `brandId` and the name for a Product.
    pub identity: String,
    /// The frozen version row's content, canonically rendered.
    pub content: String,
}

/// The export artifact (`dod-export`): header, entries, captures.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct ExportArtifactView {
    /// [`EXPORT_FORMAT_VERSION`].
    pub format_version: u32,
    /// The exported version.
    pub catalog_version_id: i64,
    /// The version's checksum, as stored.
    pub checksum: String,
    /// The digest rule the checksum was computed under.
    pub digest_version: i32,
    /// The version's publish instant.
    pub published_at: chrono::DateTime<Utc>,
    /// Every entry, sorted by `(entity_kind, entity_id)`.
    pub entries: Vec<ExportEntryView>,
    /// Every capture, sorted by kind.
    pub captures: Vec<ManifestCaptureView>,
}

fn identity_of(entity_kind: &str, content: &serde_json::Value) -> String {
    let pick = |key: &str| content.get(key).cloned().unwrap_or(serde_json::Value::Null);
    let identity = if entity_kind == "sku" {
        serde_json::json!({ "skuCode": pick("sku_code") })
    } else {
        serde_json::json!({
            "productCode": pick("product_code"),
            "brandId": pick("brand_id"),
            "name": pick("name"),
        })
    };
    canonical::canonical_rendering(&identity, canonical::Absence::Omit)
}

/// `GET /bss-products/v1/bulk/exports?catalogVersionId=` — the export.
///
/// @cpt-dod:cpt-cf-bss-products-dod-export:p1
async fn export_catalog_version(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    axum::extract::Query(query): axum::extract::Query<ExportQuery>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(Utc::now());
    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;
    let subject = query
        .catalog_version_id
        .map_or_else(|| "export".to_owned(), |id| format!("export/{id}"));
    let scope = bulk_scope(
        &state,
        &enforcer,
        &ctx,
        tenant_id,
        actor_ref,
        crate::authz::actions::READ,
        subject.clone(),
    )
    .await?;
    let Some(catalog_version_id) = query.catalog_version_id else {
        let mut report = ValidationReport::new();
        report.violate(
            "VALIDATION",
            "catalogVersionId",
            "catalogVersionId is required: an export names the version it renders",
        );
        return Err(refuse_bulk(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            subject,
            DomainError::Validation(report),
        )
        .await);
    };

    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&crate::infra::storage::RepoError::Db(format!(
            "export connection: {e}"
        )))
    })?;
    let Some(version) = repo::find_catalog_version(&conn, &scope, tenant_id, catalog_version_id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?
    else {
        return Err(refuse_bulk(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            subject,
            DomainError::CatalogVersionUnknown(format!(
                "catalog version {catalog_version_id} is not one of this tenant's"
            )),
        )
        .await);
    };
    let (mut refs, mut captures) =
        repo::catalog_version_manifest_rows(&conn, &scope, tenant_id, catalog_version_id)
            .await
            .map_err(|e| repo_error_to_canonical(&e))?;
    refs.sort_by(|a, b| {
        (a.entity_kind.as_str(), a.entity_id).cmp(&(b.entity_kind.as_str(), b.entity_id))
    });
    captures.sort();
    let mut entries = Vec::with_capacity(refs.len());
    for entry in refs {
        let kind = match entry.entity_kind.as_str() {
            "product" => repo::VersionedEntityKind::Product,
            _ => repo::VersionedEntityKind::Sku,
        };
        let content = repo::entity_version_at(
            &conn,
            &scope,
            tenant_id,
            kind,
            entry.entity_id,
            entry.published_version,
        )
        .await
        .map_err(|e| repo_error_to_canonical(&e))?
        .unwrap_or_else(|| "{}".to_owned());
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap_or_default();
        let identity = identity_of(&entry.entity_kind, &parsed);
        entries.push(ExportEntryView {
            entity_kind: entry.entity_kind,
            entity_id: entry.entity_id,
            published_version: entry.published_version,
            identity,
            content,
        });
    }
    Ok((
        StatusCode::OK,
        Json(ExportArtifactView {
            format_version: EXPORT_FORMAT_VERSION,
            catalog_version_id,
            checksum: version.checksum,
            digest_version: version.digest_version,
            published_at: version.published_at,
            entries,
            captures: captures
                .into_iter()
                .map(|(capture_kind, content)| ManifestCaptureView {
                    capture_kind,
                    content,
                })
                .collect(),
        }),
    )
        .into_response())
}

// ---------------------------------------------------------------------------
// POST /bss-products/v1/bulk/lifecycle (the p2 lane)
// ---------------------------------------------------------------------------

/// The lifecycle batch request: one op over an id list.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request)]
pub struct LifecycleBatchRequest {
    /// The caller's idempotency key for the batch.
    pub batch_key: String,
    /// `deprecate` or `retire`.
    pub op: String,
    /// `product` or `sku`.
    pub entity_kind: String,
    /// The heads to transition, each its own row keyed by its id.
    pub entity_ids: Vec<Uuid>,
}

/// The lifecycle ops a batch may carry.
pub(crate) const LIFECYCLE_OPS: [&str; 2] = ["deprecate", "retire"];

fn validate_lifecycle_shape(body: &LifecycleBatchRequest, max_rows: u32) -> ValidationReport {
    let mut report = ValidationReport::new();
    if body.batch_key.trim().is_empty() {
        report.violate("VALIDATION", "batch_key", "batch_key must not be blank");
    }
    if !LIFECYCLE_OPS.contains(&body.op.trim()) {
        report.violate("VALIDATION", "op", "op must be deprecate or retire");
    }
    if !matches!(body.entity_kind.trim(), "product" | "sku") {
        report.violate(
            "VALIDATION",
            "entity_kind",
            "entity_kind must be product or sku",
        );
    }
    if body.entity_ids.is_empty() {
        report.violate(
            "VALIDATION",
            "entity_ids",
            "entity_ids must name at least one head",
        );
    }
    if body.entity_ids.len() > max_rows as usize {
        return report;
    }
    let mut seen = std::collections::HashSet::with_capacity(body.entity_ids.len());
    if body.entity_ids.iter().any(|id| !seen.insert(*id)) {
        report.violate("VALIDATION", "entity_ids", "an id is listed twice");
    }
    report
}

/// `POST /bss-products/v1/bulk/lifecycle` — the lifecycle lane's door.
///
/// @cpt-dod:cpt-cf-bss-products-dod-bulk-lifecycle:p2
#[allow(clippy::too_many_lines)] // the import door's sequence, on the other label
async fn start_lifecycle_batch(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Json(body): Json<LifecycleBatchRequest>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(Utc::now());
    let batch_key = body.batch_key.trim().to_owned();
    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;
    let scope = bulk_scope_on(
        &state,
        &enforcer,
        &ctx,
        tenant_id,
        actor_ref,
        crate::authz::actions::EXECUTE,
        batch_key.clone(),
        true,
    )
    .await?;

    let report = validate_lifecycle_shape(&body, state.bulk_max_rows_per_batch);
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
            "lifecycle door connection: {e}"
        )))
    })?;
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
                state: existing.state.as_str().to_owned(),
                row_count: rows.len(),
                replayed: true,
            }),
        )
            .into_response());
    }
    let row_count = body.entity_ids.len();
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

    let op = body.op.trim().to_owned();
    let entity_kind = body.entity_kind.trim().to_owned();
    let live_op = serde_json::json!({ "op": op }).to_string();
    let rows: Vec<NewBulkRow> = body
        .entity_ids
        .iter()
        .map(|id| NewBulkRow {
            row_key: id.to_string(),
            row_id: Uuid::new_v4(),
            entity_kind: entity_kind.clone(),
            entity_id: Some(*id),
            pinned_revision: None,
            // The row's payload is the op itself: the ledger's shape CHECK
            // wants a payload on every product/sku row, and the op is what
            // this row stages.
            staged_payload: Some(live_op.clone()),
            governed_live_op: Some(live_op.clone()),
        })
        .collect();
    let batch_id = Uuid::new_v4();
    let new = NewBulkBatch {
        batch_id,
        batch_key: batch_key.clone(),
        mode: "import".to_owned(),
        lane: "lifecycle".to_owned(),
        operation_key: Some(batch_id.to_string()),
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
            mode: "import".to_owned(),
            state: "staging".to_owned(),
            row_count,
            replayed: false,
        }),
    )
        .into_response())
}

#[cfg(test)]
#[path = "bulk_tests.rs"]
mod bulk_tests;
