//! SKU draft authoring, registry reference counts, version history and scoped search.
use super::{
    ApiState, TxError, authz_error_to_canonical, category_tx_config, contention_db_err,
    dto::{
        ReferencesDto, SkuCard, SkuDto, SkuList, SkuPatchRequest, SkuRequest, SkuVersionDto,
        parse_token,
    },
    json_body,
    preconditions::{etag, if_match, if_match_param},
    repo_error_to_canonical, require_authenticated, tx_to_canonical,
};
use crate::{
    authz::{access_scope, actions, resource_types},
    domain::{
        concurrency::InternalRevision,
        error::DomainError,
        recognized::UsageTypeAnswer,
        sku::{NewSku, SkuPatch, apply_patch, validate_new},
        validation::ValidationReport,
    },
    infra::storage::{
        RepoError,
        repo::{self, HeadWrite},
    },
};
use authz_resolver_sdk::PolicyEnforcer;
use axum::{
    Extension, Json, Router,
    extract::rejection::{JsonRejection, QueryRejection},
    extract::{Path, Query},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use bss_products_sdk::models::{Lifecycle, Sku, SkuContent, SkuType};
use std::sync::Arc;
use time::{Date, OffsetDateTime};
use toolkit::api::{
    OpenApiRegistry,
    canonical_prelude::{CanonicalError, resource_error},
    operation_builder::OperationBuilder,
};
use toolkit_db::secure::{AccessScope, DBRunner};
use toolkit_security::SecurityContext;
use uuid::Uuid;

const SKUS: &str = "/bss-products/v1/skus";
const TAG: &str = "SKUs";
#[resource_error(gts_id!("cf.bss.products.sku.v1~"))]
struct SkuResource;

/// List query vocabulary mirrors the repository's scoped filters.
#[toolkit_macros::api_dto(request)]
struct ListQuery {
    q: Option<String>,
    r#type: Option<String>,
    category: Option<Uuid>,
    lifecycle: Option<String>,
    limit: Option<u32>,
    after: Option<String>,
}
#[toolkit_macros::api_dto(request)]
struct VersionQuery {
    #[serde(default, with = "crate::infra::serde_date::option")]
    as_of: Option<Date>,
}
/// History is an array; `as_of` selects a single version.
#[toolkit_macros::api_dto(response)]
#[serde(untagged)]
enum VersionsResponse {
    History(Vec<SkuVersionDto>),
    AsOf(Box<SkuVersionDto>),
}
#[toolkit_macros::api_dto(request)]
struct ReferenceQuery {
    #[serde(default)]
    include_released: bool,
}
#[toolkit_macros::api_dto(response)]
struct ReferenceDto {
    id: Uuid,
    owner: String,
    kind: String,
    ref_id: Uuid,
    state: String,
    #[serde(with = "time::serde::rfc3339")]
    reserved_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    released_at: Option<OffsetDateTime>,
    released_by: Option<Uuid>,
    forced: bool,
    release_reason: Option<String>,
}
impl From<repo::SkuReference> for ReferenceDto {
    fn from(r: repo::SkuReference) -> Self {
        Self {
            id: r.id,
            owner: r.owner_gear,
            kind: r.ref_kind,
            ref_id: r.ref_id,
            state: r.state,
            reserved_at: r.reserved_at,
            released_at: r.released_at,
            released_by: r.released_by,
            forced: r.forced,
            release_reason: r.release_reason,
        }
    }
}
#[toolkit_macros::api_dto(response)]
struct ReferenceList {
    summary: ReferencesDto,
    items: Vec<ReferenceDto>,
}

/// Register the six SKU operations and their concrete response schemas.
#[allow(clippy::too_many_lines)] // Keep each operation's complete contract together.
pub(crate) fn router(state: Arc<ApiState>, openapi: &dyn OpenApiRegistry) -> Router {
    let router = OperationBuilder::post(SKUS)
        .operation_id("bss_products.create_sku")
        .summary("Create a draft SKU")
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .json_request::<SkuRequest>(openapi, "Draft business fields")
        .handler(create_sku)
        .json_response_with_schema::<SkuDto>(openapi, StatusCode::CREATED, "Create a draft SKU.")
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(Router::new(), openapi);
    let router = OperationBuilder::get(SKUS)
        .operation_id("bss_products.list_skus")
        .summary("List and search SKUs")
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .query_param("q", false, "q")
        .query_param("type", false, "type")
        .query_param("category", false, "category")
        .query_param("lifecycle", false, "lifecycle")
        .query_param("limit", false, "limit")
        .query_param("after", false, "after")
        .handler(list_skus)
        .json_response_with_schema::<SkuList>(openapi, StatusCode::OK, "List and search SKUs.")
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);
    let router = OperationBuilder::get(format!("{SKUS}/{{id}}"))
        .operation_id("bss_products.get_sku")
        .summary("Read a SKU card")
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("id", "SKU id")
        .handler(get_sku)
        .json_response_with_schema::<SkuCard>(openapi, StatusCode::OK, "Read a SKU card.")
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);
    let router = OperationBuilder::patch(format!("{SKUS}/{{id}}"))
        .operation_id("bss_products.update_sku_draft")
        .summary("Edit a draft SKU")
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("id", "SKU id")
        .param(if_match_param())
        .json_request::<SkuPatchRequest>(openapi, "Draft business fields")
        .handler(update_sku_draft)
        .json_response_with_schema::<SkuDto>(openapi, StatusCode::OK, "Edit a draft SKU.")
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);
    let router = OperationBuilder::get(format!("{SKUS}/{{id}}/versions"))
        .operation_id("bss_products.sku_versions")
        .summary("Read version history or the version in force")
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("id", "SKU id")
        .query_param("as_of", false, "as_of")
        .handler(sku_versions)
        .json_response_with_schema::<VersionsResponse>(
            openapi,
            StatusCode::OK,
            "Read version history or the version in force.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);
    let router = OperationBuilder::get(format!("{SKUS}/{{id}}/references"))
        .operation_id("bss_products.sku_references")
        .summary("Read reference details and optional released history")
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("id", "SKU id")
        .query_param(
            "include_released",
            false,
            "Include released reference history (default false)",
        )
        .handler(sku_references)
        .json_response_with_schema::<ReferenceList>(
            openapi,
            StatusCode::OK,
            "Read reference details and optional released history.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);
    router.layer(Extension(state))
}

/// Authorize reads without an owner hint and writes against the subject tenant.
async fn scope(
    enforcer: &PolicyEnforcer,
    ctx: &SecurityContext,
    write: bool,
) -> Result<AccessScope, CanonicalError> {
    access_scope(
        enforcer,
        ctx,
        &resource_types::SKU,
        if write {
            actions::AUTHOR
        } else {
            actions::READ
        },
        write.then(|| ctx.subject_tenant_id()),
        None,
        true,
    )
    .await
    .map_err(|e| {
        authz_error_to_canonical(e, |reason| {
            SkuResource::permission_denied()
                .with_reason(reason)
                .create()
        })
    })
}
fn response(status: StatusCode, s: Sku) -> Response {
    (
        status,
        [(header::ETAG, etag(InternalRevision::new(s.revision)))],
        Json(SkuDto::from(s)),
    )
        .into_response()
}
/// Translate only known repository business refusals, retaining all driver errors.
fn write_error(e: RepoError, category_id: Uuid) -> TxError {
    match e {
        RepoError::Db(code) if code == "CATEGORY_NOT_FOUND" => {
            TxError::Refused(DomainError::NotFound {
                what: "category",
                id: category_id,
            })
        }
        RepoError::Db(code)
            if matches!(
                code.as_str(),
                "SKU_CODE_TAKEN" | "SKU_NAME_TAKEN" | "CATEGORY_RETIRED"
            ) =>
        {
            let (code, detail) = match code.as_str() {
                "SKU_CODE_TAKEN" => ("SKU_CODE_TAKEN", "a SKU with this code exists"),
                "SKU_NAME_TAKEN" => ("SKU_NAME_TAKEN", "a SKU with this name exists"),
                _ => ("CATEGORY_RETIRED", "the category is retired"),
            };
            TxError::Refused(DomainError::Conflict {
                code,
                detail: detail.into(),
            })
        }
        other => TxError::Repo(other),
    }
}
/// P-D-184: a configured catalog's definite unknown refuses; silence allows draft save.
async fn resolve_draft_ref(
    state: &ApiState,
    ctx: &SecurityContext,
    reference: Option<&str>,
) -> Result<(), CanonicalError> {
    if state.usage_type_catalog_source == crate::gear::USAGE_TYPE_SOURCE_UNCONFIGURED {
        return Ok(());
    }
    if let Some(reference) = reference
        && matches!(
            state.usage_type_catalog.resolve(ctx, reference).await,
            UsageTypeAnswer::Unresolved
        )
    {
        let mut report = ValidationReport::new();
        report.violate(
            "USAGE_TYPE_UNRESOLVED",
            "usage_type_ref",
            "the usage type catalog does not know this ref",
        );
        return Err(DomainError::Validation(report).into());
    }
    Ok(())
}
/// @cpt-cf-bss-products-fr-sku-define
async fn create_sku(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    body: Result<Json<SkuRequest>, JsonRejection>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let actor = ctx.subject_id();
    let scope_tx = scope(&enforcer, &ctx, true).await?;
    let new_tx = NewSku::try_from(json_body(body)?).map_err(DomainError::Validation)?;
    let report = validate_new(&new_tx);
    if !report.is_empty() {
        return Err(DomainError::Validation(report).into());
    }
    resolve_draft_ref(&state, &ctx, new_tx.usage_type_ref.as_deref()).await?;
    let now = OffsetDateTime::now_utc();
    let created = state
        .db
        .db()
        .transaction_with_retry::<Sku, TxError, _, _>(
            category_tx_config(&state),
            contention_db_err,
            move |tx| {
                let scope = scope_tx.clone();
                let new = new_tx.clone();
                Box::pin(async move {
                    let category = new.category_id;
                    let s = repo::insert_sku(tx, &scope, tenant_id, new, actor, now)
                        .await
                        .map_err(|e| write_error(e, category))?;
                    audit(tx, &scope, tenant_id, actor, "sku.create", &s, now).await?;
                    Ok(s)
                })
            },
        )
        .await
        .map_err(tx_to_canonical)?;
    Ok(response(StatusCode::CREATED, created))
}
/// Read a head with a scoped 404 for absence or a foreign tenant.
async fn find(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
) -> Result<Sku, TxError> {
    repo::find_sku(runner, scope, tenant, id)
        .await
        .map_err(TxError::Repo)?
        .ok_or(TxError::Refused(DomainError::NotFound { what: "sku", id }))
}
/// Check editing eligibility before resolving a ref and again within the write transaction.
fn editable(s: &Sku, expected: i64) -> Result<(), TxError> {
    if s.lifecycle != Lifecycle::Draft {
        return Err(TxError::Refused(DomainError::Conflict {
            code: "NOT_A_DRAFT",
            detail: format!("use POST /skus/{}/changes", s.id),
        }));
    }
    if s.pending_unit_id.is_some() {
        return Err(TxError::Refused(DomainError::Conflict {
            code: "ROW_LOCKED_PENDING",
            detail: "a pending approval unit locks this draft".into(),
        }));
    }
    if s.revision != expected {
        return Err(TxError::Refused(DomainError::StaleRevision {
            expected,
            found: s.revision,
        }));
    }
    Ok(())
}
/// @cpt-cf-bss-products-fr-concurrency-idempotency
async fn update_sku_draft(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    body: Result<Json<SkuPatchRequest>, JsonRejection>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let actor = ctx.subject_id();
    let scope_tx = scope(&enforcer, &ctx, true).await?;
    let expected = if_match(&headers)?.get();
    let patch_tx = SkuPatch::try_from(json_body(body)?).map_err(DomainError::Validation)?;
    let mut report = ValidationReport::new();
    if patch_tx.name.as_deref() == Some("") {
        report.violate("VALIDATION", "name", "name must not be blank");
    }
    if patch_tx.lifecycle.is_some_and(|s| s != Lifecycle::Draft) {
        report.violate(
            "VALIDATION",
            "lifecycle",
            "a draft changes lifecycle through its submit door",
        );
    }
    if !report.is_empty() {
        return Err(DomainError::Validation(report).into());
    }
    // Resolve only a changed proposed ref outside the transaction. The revision check inside
    // guarantees this answer cannot be applied to a different head after a concurrent edit.
    let current = {
        super::governance::touch(&state, &scope_tx, ctx.subject_tenant_id(), id).await?;
        let conn = state.db.conn().map_err(|e| tx_to_canonical(e.into()))?;
        find(&conn, &scope_tx, tenant_id, id)
            .await
            .map_err(tx_to_canonical)?
    };
    editable(&current, expected).map_err(tx_to_canonical)?;
    let proposed = apply_patch(&SkuContent::from(&current), &patch_tx);
    if proposed.usage_type_ref != current.usage_type_ref {
        resolve_draft_ref(&state, &ctx, proposed.usage_type_ref.as_deref()).await?;
    }
    let now = OffsetDateTime::now_utc();
    let updated = state
        .db
        .db()
        .transaction_with_retry::<Sku, TxError, _, _>(
            category_tx_config(&state),
            contention_db_err,
            move |tx| {
                let scope = scope_tx.clone();
                let patch = patch_tx.clone();
                Box::pin(async move {
                    let current = find(tx, &scope, tenant_id, id).await?;
                    editable(&current, expected)?;
                    let content = apply_patch(&SkuContent::from(&current), &patch);
                    let s = match repo::update_sku_draft(
                        tx, &scope, tenant_id, id, expected, &content, now,
                    )
                    .await
                    .map_err(|e| write_error(e, content.category_id))?
                    {
                        HeadWrite::Written(s) => s,
                        HeadWrite::Unmatched => {
                            let latest = find(tx, &scope, tenant_id, id).await?;
                            return Err(TxError::Refused(DomainError::StaleRevision {
                                expected,
                                found: latest.revision,
                            }));
                        }
                    };
                    audit(tx, &scope, tenant_id, actor, "sku.draft_update", &s, now).await?;
                    Ok(s)
                })
            },
        )
        .await
        .map_err(tx_to_canonical)?;
    Ok(response(StatusCode::OK, updated))
}
/// Read a SKU and live reference counts from this gear's registry.
async fn get_sku(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let scope = scope(&enforcer, &ctx, false).await?;
    super::governance::touch(&state, &scope, ctx.subject_tenant_id(), id).await?;
    let conn = state.db.conn().map_err(|e| tx_to_canonical(e.into()))?;
    let s = find(&conn, &scope, ctx.subject_tenant_id(), id)
        .await
        .map_err(tx_to_canonical)?;
    let refs = repo::reference_summary(&conn, &scope, ctx.subject_tenant_id(), id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?;
    Ok((
        [(header::ETAG, etag(InternalRevision::new(s.revision)))],
        Json(SkuCard {
            sku: s.into(),
            references: refs.into(),
        }),
    )
        .into_response())
}
/// Convert malformed query values into canonical 400 violations.
fn query<T>(q: Result<Query<T>, QueryRejection>) -> Result<T, CanonicalError> {
    q.map(|Query(q)| q).map_err(|e| {
        let mut r = ValidationReport::new();
        r.violate("VALIDATION", "query", e.body_text());
        DomainError::Validation(r).into()
    })
}
/// List by an exclusive code cursor, returning the last delivered code as continuation.
async fn list_skus(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    q: Result<Query<ListQuery>, QueryRejection>,
) -> Result<Json<SkuList>, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let scope = scope(&enforcer, &ctx, false).await?;
    let q = query(q)?;
    let limit = q.limit.unwrap_or(50).min(200);
    if limit == 0 {
        let mut r = ValidationReport::new();
        r.violate("VALIDATION", "limit", "limit must be at least one");
        return Err(DomainError::Validation(r).into());
    }
    let q = repo::SkuQuery {
        catalog_filter: None,
        text: q.q,
        r#type: q
            .r#type
            .as_deref()
            .map(|s| parse_token(s, "type", SkuType::parse))
            .transpose()
            .map_err(DomainError::Validation)?,
        category_id: q.category,
        lifecycle: q
            .lifecycle
            .as_deref()
            .map(|s| parse_token(s, "lifecycle", Lifecycle::parse))
            .transpose()
            .map_err(DomainError::Validation)?,
        limit: u64::from(limit),
        after_code: q.after,
    };
    let tenant = ctx.subject_tenant_id();
    let ttl = state.fence_ttl_minutes;
    let mut items = state
        .db
        .db()
        .transaction_with_retry(category_tx_config(&state), contention_db_err, move |tx| {
            let scope = scope.clone();
            let q = q.clone();
            Box::pin(async move {
                repo::expire_orphan_fences(
                    tx,
                    &scope,
                    tenant,
                    OffsetDateTime::now_utc() - time::Duration::minutes(i64::from(ttl)),
                )
                .await
                .map_err(TxError::Repo)?;
                repo::list_skus(tx, &scope, tenant, &q)
                    .await
                    .map_err(TxError::Repo)
            })
        })
        .await
        .map_err(tx_to_canonical)?;
    let limit =
        usize::try_from(limit).map_err(|e| CanonicalError::internal(e.to_string()).create())?;
    let more = items.len() > limit;
    items.truncate(limit);
    let next = if more {
        items.last().map(|s| s.code.clone())
    } else {
        None
    };
    Ok(Json(SkuList {
        items: items.into_iter().map(Into::into).collect(),
        next,
    }))
}
/// @cpt-cf-bss-products-fr-sku-versions
async fn sku_versions(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
    q: Result<Query<VersionQuery>, QueryRejection>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let scope = scope(&enforcer, &ctx, false).await?;
    let q = query(q)?;
    super::governance::touch(&state, &scope, ctx.subject_tenant_id(), id).await?;
    let conn = state.db.conn().map_err(|e| tx_to_canonical(e.into()))?;
    find(&conn, &scope, ctx.subject_tenant_id(), id)
        .await
        .map_err(tx_to_canonical)?;
    if let Some(as_of) = q.as_of {
        let version = repo::version_as_of(&conn, &scope, ctx.subject_tenant_id(), id, as_of)
            .await
            .map_err(|e| repo_error_to_canonical(&e))?;
        let Some(version) = version else {
            // The toolkit's NotFound context is empty, so attach this door's specified
            // discriminator to its canonical Problem without changing the 404 family.
            let error = SkuResource::not_found(format!("no SKU version is in force on {as_of}"))
                .with_resource(id.to_string())
                .create();
            let mut problem = toolkit_canonical_errors::Problem::from_error(&error)
                .map_err(|e| CanonicalError::internal(e.to_string()).create())?;
            problem.context["reason"] = serde_json::json!("NO_VERSION_IN_FORCE");
            return Ok(problem.into_response());
        };
        return Ok(Json(VersionsResponse::AsOf(Box::new(version.into()))).into_response());
    }
    let versions = repo::versions(&conn, &scope, ctx.subject_tenant_id(), id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?;
    Ok(Json(VersionsResponse::History(
        versions.into_iter().map(Into::into).collect(),
    ))
    .into_response())
}
/// @cpt-cf-bss-products-fr-reference-registry
async fn sku_references(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
    q: Result<Query<ReferenceQuery>, QueryRejection>,
) -> Result<Json<ReferenceList>, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let scope = scope(&enforcer, &ctx, false).await?;
    let q = query(q)?;
    let tenant = ctx.subject_tenant_id();
    super::governance::touch(&state, &scope, ctx.subject_tenant_id(), id).await?;
    let conn = state.db.conn().map_err(|e| tx_to_canonical(e.into()))?;
    find(&conn, &scope, tenant, id)
        .await
        .map_err(tx_to_canonical)?;
    let summary = repo::reference_summary(&conn, &scope, tenant, id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?;
    let items = repo::list_references(&conn, &scope, tenant, id, q.include_released)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?;
    Ok(Json(ReferenceList {
        summary: summary.into(),
        items: items.into_iter().map(Into::into).collect(),
    }))
}
/// Commit audit attribution atomically with the draft mutation.
async fn audit(
    tx: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor_ref: Uuid,
    action: &str,
    s: &Sku,
    written_at: OffsetDateTime,
) -> Result<(), TxError> {
    repo::write_eventless_act_audit(
        tx,
        scope,
        repo::AuditCommon {
            audit_id: Uuid::now_v7(),
            tenant_id,
            actor_ref,
            action: action.to_owned(),
            subject_kind: "sku".to_owned(),
            reason: None,
            correlation_id: None,
            written_at,
        },
        s.id,
        Some(s.revision),
    )
    .await
    .map_err(TxError::Repo)
}
#[cfg(test)]
#[path = "skus_tests.rs"]
mod skus_tests;
