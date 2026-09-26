//! @cpt-dod:cpt-cf-bss-products-dod-category-retire-refused:p1
//! @cpt-dod:cpt-cf-bss-products-dod-category-flat-crud:p1
//! Flat categories, direct authoring with revision checks and transactional audit.
use super::authz_error_to_canonical;
use super::{
    ApiState, TxError, category_tx_config, contention_db_err,
    dto::{CategoryDto, CategoryList, CategoryPatchRequest, CategoryRequest},
    preconditions::{etag, if_match, if_match_param},
    replay, repo_error_to_canonical, require_authenticated, tx_to_canonical,
};
use crate::{
    authz::{access_scope, actions, resource_types},
    domain::{
        category::{CategoryPatch, NewCategory, validate_new_category},
        concurrency::InternalRevision,
        error::DomainError,
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
    extract::Path,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use bss_products_sdk::models::Category;
use std::sync::Arc;
use time::OffsetDateTime;
use toolkit::api::{
    OpenApiRegistry,
    canonical_prelude::{CanonicalError, resource_error},
    operation_builder::OperationBuilder,
};
use toolkit_db::secure::{AccessScope, TxConfig};
use toolkit_security::SecurityContext;
use uuid::Uuid;

pub(crate) const CATEGORIES: &str = "/bss-products/v1/categories";
const TAG: &str = "Categories";
#[resource_error(gts_id!("cf.bss.products.category.v1~"))]
struct CategoryResource;

/// Register the four category operations.
pub(crate) fn router(state: Arc<ApiState>, openapi: &dyn OpenApiRegistry) -> Router {
    let router = OperationBuilder::post(CATEGORIES)
        .operation_id("bss_products.create_category")
        .summary("Create a category")
        .description("A flat category; one may be the tenant's default.")
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .json_request::<CategoryRequest>(openapi, "code, name, is_default, sort_order")
        .param(replay::param())
        .handler(create_category)
        .json_response_with_schema::<CategoryDto>(
            openapi,
            StatusCode::CREATED,
            "Created category; ETag carries its version.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(Router::new(), openapi);
    let router = OperationBuilder::get(CATEGORIES)
        .operation_id("bss_products.list_categories")
        .summary("List categories")
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .handler(list_categories)
        .json_response_with_schema::<CategoryList>(
            openapi,
            StatusCode::OK,
            "Categories by sort order then code.",
        )
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);
    let router = OperationBuilder::patch(format!("{CATEGORIES}/{{id}}"))
        .operation_id("bss_products.update_category")
        .summary("Rename, reorder or make default")
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("id", "Category id")
        .param(if_match_param())
        .json_request::<CategoryPatchRequest>(openapi, "Mutable category fields")
        .handler(update_category)
        .json_response_with_schema::<CategoryDto>(
            openapi,
            StatusCode::OK,
            "Category with its new ETag.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);
    let router = OperationBuilder::post(format!("{CATEGORIES}/{{id}}/retire"))
        .operation_id("bss_products.retire_category")
        .summary("Retire an unused category")
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("id", "Category id")
        .param(replay::param())
        .handler(retire_category)
        .json_response_with_schema::<CategoryDto>(openapi, StatusCode::OK, "Retired category.")
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);
    router.layer(Extension(state))
}

/// Compile category access through the PDP before validation or storage.
async fn scope(
    enforcer: &PolicyEnforcer,
    ctx: &SecurityContext,
    write: bool,
) -> Result<AccessScope, CanonicalError> {
    access_scope(
        enforcer,
        ctx,
        &resource_types::CATEGORY,
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
            CategoryResource::permission_denied()
                .with_reason(reason)
                .create()
        })
    })
}
fn response(status: StatusCode, c: Category) -> Response {
    (
        status,
        [(header::ETAG, etag(InternalRevision::new(c.version)))],
        Json(CategoryDto::from(c)),
    )
        .into_response()
}
/// @cpt-cf-bss-products-fr-category-flat
async fn create_category(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    headers: HeaderMap,
    body: Result<Json<serde_json::Value>, axum::extract::rejection::JsonRejection>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let actor = ctx.subject_id();
    let scope_tx = scope(&enforcer, &ctx, true).await?;
    let payload = super::json_body(body)?;
    let claim = replay::input(
        &state,
        &headers,
        "/bss-products/v1/categories".into(),
        &payload,
    )?;
    let body: CategoryRequest = serde_json::from_value(payload)
        .map_err(|e| CanonicalError::from(super::governance::validation("body", e.to_string())))?;
    let new_tx = NewCategory {
        code: body.code.trim().to_owned(),
        name: body.name.trim().to_owned(),
        is_default: body.is_default,
        sort_order: body.sort_order,
    };
    let report = validate_new_category(&new_tx);
    if !report.is_empty() {
        return Err(DomainError::Validation(report).into());
    }
    let now = OffsetDateTime::now_utc();
    let created = state
        .db
        .db()
        .transaction_with_retry::<Response, TxError, _, _>(
            TxConfig::default(),
            contention_db_err,
            move |tx| {
                let scope = scope_tx.clone();
                let new = new_tx.clone();
                let claim = claim.clone();
                Box::pin(async move {
                    if let Some(response) = replay::begin(tx, tenant_id, claim.as_ref()).await? {
                        return Ok(response);
                    }
                    let c = repo::insert_category(tx, &scope, tenant_id, new, now)
                        .await
                        .map_err(|e| match e {
                            RepoError::Db(code) if code == "CATEGORY_CODE_TAKEN" => {
                                TxError::Refused(DomainError::Conflict {
                                    code: "CATEGORY_CODE_TAKEN",
                                    detail: "a category with this code exists".into(),
                                })
                            }
                            other => TxError::Repo(other),
                        })?;
                    audit(tx, &scope, tenant_id, actor, "category.create", &c, now).await?;
                    replay::finish(
                        tx,
                        tenant_id,
                        claim.as_ref(),
                        StatusCode::CREATED,
                        &CategoryDto::from(c),
                    )
                    .await
                })
            },
        )
        .await
        .map_err(tx_to_canonical)?;
    Ok(created)
}
/// Read categories in display order.
async fn list_categories(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
) -> Result<Json<CategoryList>, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let scope = scope(&enforcer, &ctx, false).await?;
    let conn = state.db.conn().map_err(|e| tx_to_canonical(e.into()))?;
    let items = repo::list_categories(&conn, &scope, ctx.subject_tenant_id())
        .await
        .map_err(|e| repo_error_to_canonical(&e))?;
    Ok(Json(CategoryList {
        items: items.into_iter().map(Into::into).collect(),
    }))
}
/// @cpt-cf-bss-products-fr-concurrency-idempotency
async fn update_category(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    body: Result<Json<CategoryPatchRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let actor = ctx.subject_id();
    let scope_tx = scope(&enforcer, &ctx, true).await?;
    let expected = if_match(&headers)?.get();
    let body = super::json_body(body)?;
    let patch_tx = CategoryPatch {
        name: body.name.map(|s| s.trim().to_owned()),
        is_default: body.is_default,
        sort_order: body.sort_order,
    };
    if patch_tx.name.as_deref() == Some("") {
        let mut r = ValidationReport::new();
        r.violate("VALIDATION", "name", "name must not be blank");
        return Err(DomainError::Validation(r).into());
    }
    let now = OffsetDateTime::now_utc();
    let updated = state
        .db
        .db()
        .transaction_with_retry::<Category, TxError, _, _>(
            TxConfig::default(),
            contention_db_err,
            move |tx| {
                let scope = scope_tx.clone();
                let patch = patch_tx.clone();
                Box::pin(async move {
                    let current = repo::find_category(tx, &scope, tenant_id, id)
                        .await
                        .map_err(TxError::Repo)?
                        .ok_or(TxError::Refused(DomainError::NotFound {
                            what: "category",
                            id,
                        }))?;
                    let c = match repo::update_category(
                        tx, &scope, tenant_id, id, expected, patch, now,
                    )
                    .await
                    .map_err(TxError::Repo)?
                    {
                        HeadWrite::Written(c) => c,
                        HeadWrite::Unmatched => {
                            return Err(TxError::Refused(DomainError::StaleRevision {
                                expected,
                                found: current.version,
                            }));
                        }
                    };
                    audit(tx, &scope, tenant_id, actor, "category.update", &c, now).await?;
                    Ok(c)
                })
            },
        )
        .await
        .map_err(tx_to_canonical)?;
    Ok(response(StatusCode::OK, updated))
}
/// @cpt-cf-bss-products-fr-category-flat
async fn retire_category(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let actor = ctx.subject_id();
    let scope_tx = scope(&enforcer, &ctx, true).await?;
    let claim = replay::input(
        &state,
        &headers,
        format!("/bss-products/v1/categories/{id}/retire"),
        &serde_json::json!({}),
    )?;
    let now = OffsetDateTime::now_utc();
    let retired = state
        .db
        .db()
        .transaction_with_retry::<Response, TxError, _, _>(
            category_tx_config(&state),
            contention_db_err,
            move |tx| {
                let scope = scope_tx.clone();
                let claim = claim.clone();
                Box::pin(async move {
                    if repo::find_category(tx, &scope, tenant_id, id)
                        .await
                        .map_err(TxError::Repo)?
                        .is_none()
                    {
                        return Err(TxError::Refused(DomainError::NotFound {
                            what: "category",
                            id,
                        }));
                    }
                    if let Some(response) = replay::begin(tx, tenant_id, claim.as_ref()).await? {
                        return Ok(response);
                    }
                    let c = match repo::retire_category_if_unused(tx, &scope, tenant_id, id, now)
                        .await
                        .map_err(TxError::Repo)?
                    {
                        Some(HeadWrite::Written(c)) => c,
                        Some(HeadWrite::Unmatched) => {
                            return Err(TxError::Refused(DomainError::Conflict {
                                code: "CATEGORY_IN_USE",
                                detail: "category is in use or already retired".into(),
                            }));
                        }
                        None => {
                            return Err(TxError::Refused(DomainError::NotFound {
                                what: "category",
                                id,
                            }));
                        }
                    };
                    audit(tx, &scope, tenant_id, actor, "category.retire", &c, now).await?;
                    replay::finish(
                        tx,
                        tenant_id,
                        claim.as_ref(),
                        StatusCode::OK,
                        &CategoryDto::from(c),
                    )
                    .await
                })
            },
        )
        .await
        .map_err(tx_to_canonical)?;
    Ok(retired)
}
/// Record the direct category act in the same transaction.
async fn audit(
    tx: &impl toolkit_db::secure::DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor_ref: Uuid,
    action: &str,
    c: &Category,
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
            subject_kind: "category".to_owned(),
            reason: None,
            correlation_id: None,
            written_at,
        },
        c.id,
        Some(c.version),
    )
    .await
    .map_err(TxError::Repo)
}
#[cfg(test)]
#[path = "categories_tests.rs"]
mod categories_tests;
