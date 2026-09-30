//! @cpt-dod:cpt-cf-bss-products-dod-category-retire-refused:p1
//! @cpt-dod:cpt-cf-bss-products-dod-category-flat-crud:p1
//! Flat categories, direct authoring with revision checks and transactional audit.
use super::authz_error_to_canonical;
use super::{
    ApiState, TxError, category_tx_config, contention_db_err,
    dto::{CategoryPatchRequest, CategoryRequest, ProductsCategoryDto, ProductsCategoryItem},
    preconditions::{etag, if_match, if_match_param},
    replay, repo_error_to_canonical, require_authenticated,
    sku_list::{RawQuery, UNSUPPORTED, cursor_hash, params, refused},
    tx_to_canonical,
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
        RepoError, RepoRefusal,
        repo::{self, CategoryListField, HeadWrite, SkuListError},
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
    odata::OData,
    operation_builder::{OperationBuilder, OperationBuilderODataExt},
};
use toolkit_db::secure::{AccessScope, TxConfig};
use toolkit_odata::{
    Error as ODataError, Page,
    filter::{FieldKind, FilterField},
};
use toolkit_security::SecurityContext;
use uuid::Uuid;

pub(crate) const CATEGORIES: &str = "/bss-products/v1/categories";
const TAG: &str = "Categories";
/// The stored status of a retired category (P-D-208, P-D-220).
const RETIRED: &str = "retired";
#[resource_error(gts_id!("cf.bss.products.category.v1~"))]
struct CategoryResource;

/// The fields a category list `$orderby` names: never a filter-only field (`status`,
/// `is_default`). `id` is the tie-break every order ends with (P-D-215).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CategoryOrderField {
    SortOrder,
    Code,
    Name,
    Id,
}
impl CategoryOrderField {
    const fn field(self) -> CategoryListField {
        match self {
            Self::SortOrder => CategoryListField::SortOrder,
            Self::Code => CategoryListField::Code,
            Self::Name => CategoryListField::Name,
            Self::Id => CategoryListField::Id,
        }
    }
}
impl FilterField for CategoryOrderField {
    const FIELDS: &'static [Self] = &[Self::SortOrder, Self::Code, Self::Name, Self::Id];
    fn name(&self) -> &'static str {
        self.field().name()
    }
    fn kind(&self) -> FieldKind {
        self.field().kind()
    }
    fn from_name(name: &str) -> Option<Self> {
        Self::FIELDS.iter().copied().find(|f| f.name() == name)
    }
}

/// Register the five category operations.
pub(crate) fn router(state: Arc<ApiState>, openapi: &dyn OpenApiRegistry) -> Router {
    let router = OperationBuilder::post(CATEGORIES)
        .operation_id("bss_products.create_category")
        .summary("Create a category")
        .description(
            "A flat category; one may be the tenant's default. Creating one with `is_default: true` \
             moves the default to it: the previous default is cleared in the same write \
             (P-D-218). The code is at most 64 characters and the name 200 (P-D-225). Refusals: \
             400 FIELD_TOO_LONG on a code or a name over its cap; 409 CATEGORY_CODE_TAKEN, or \
             CATEGORY_DEFAULT_TAKEN when a concurrent write took the default first.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .json_request::<CategoryRequest>(openapi, "code, name, is_default, sort_order")
        .param(replay::param())
        .handler(create_category)
        .json_response_with_schema::<ProductsCategoryDto>(
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
        .description(
            "One page of the tenant's categories on the toolkit's OData (P-D-215): `$filter` over \
             id, code, name, status (active or retired), is_default and sort_order; `$orderby` \
             sort_order, code or name (tie-break id; default sort_order, then code); `$top` (alias \
             `limit`; default 200, clamped at 200) and `cursor` (alias `$skiptoken`) from \
             `page_info`. Each item carries `sku_count`, the SKUs that are not retired naming it, \
             from one grouped count. Any other key, `$select` and `$count` are 400; a cursor \
             replayed under another `$filter` is 400.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .query_param_typed(
            "limit",
            false,
            "Page size, alias of $top (default 200, clamped at 200)",
            "integer",
        )
        .query_param_typed(
            "cursor",
            false,
            "Continuation from page_info (alias $skiptoken)",
            "string",
        )
        .handler(list_categories)
        .with_odata_filter::<CategoryListField>()
        .with_odata_orderby::<CategoryOrderField>()
        .json_response_with_schema::<Page<ProductsCategoryItem>>(
            openapi,
            StatusCode::OK,
            "One page of categories, by sort order then code unless ordered otherwise.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);
    let router = OperationBuilder::get(format!("{CATEGORIES}/{{id}}"))
        .operation_id("bss_products.get_category")
        .summary("Read a category")
        .description(
            "The category with its `ETag` (its version) and `sku_count`, the SKUs that are not \
             retired naming it (P-D-215). 404 for a category the tenant does not hold.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("id", "Category id")
        .handler(get_category)
        .json_response_with_schema::<ProductsCategoryItem>(
            openapi,
            StatusCode::OK,
            "The category; ETag carries its version.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);
    let router = OperationBuilder::patch(format!("{CATEGORIES}/{{id}}"))
        .operation_id("bss_products.update_category")
        .summary("Rename, reorder or make default")
        .description(
            "Edits a category under If-Match. `is_default: true` moves the tenant's default to it: \
             the previous default is cleared in the same write, gets a new version and its own \
             audit row (P-D-218); a retired category never becomes the default (P-D-220). The \
             name is at most 200 characters (P-D-225). Refusals: 400 FIELD_TOO_LONG on a name \
             over its cap; 404; 409 STALE_REVISION, CATEGORY_RETIRED for `is_default: true` on a \
             retired category, or CATEGORY_DEFAULT_TAKEN when a concurrent write took the default \
             first.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("id", "Category id")
        .param(if_match_param())
        .json_request::<CategoryPatchRequest>(openapi, "Mutable category fields")
        .handler(update_category)
        .json_response_with_schema::<ProductsCategoryDto>(
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
        .description(
            "Retires a category no SKU in draft, published, deprecated or retiring names; retired \
             SKUs do not keep it in use (P-D-208). Retiring the tenant's default clears it in the \
             same transaction, with its own version and audit row, and leaves the tenant without \
             a default (P-D-220). Refusals: 404; 409 CATEGORY_IN_USE, or CATEGORY_RETIRED when it \
             is already retired.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("id", "Category id")
        .param(replay::param())
        .handler(retire_category)
        .json_response_with_schema::<ProductsCategoryDto>(
            openapi,
            StatusCode::OK,
            "Retired category.",
        )
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);
    router.layer(Extension(state))
}

/// Compile category access through the PDP before validation or storage. The call site names
/// the action it asks (`actions::READ` or `actions::AUTHOR`), not a `bool` (RS-54); a write
/// anchors to the subject's tenant.
async fn scope(
    enforcer: &PolicyEnforcer,
    ctx: &SecurityContext,
    action: &'static str,
) -> Result<AccessScope, CanonicalError> {
    access_scope(
        enforcer,
        ctx,
        &resource_types::CATEGORY,
        action,
        (action != actions::READ).then(|| ctx.subject_tenant_id()),
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
/// # Errors
/// A stored status outside the category's closed set (P-D-217): a storage failure.
fn response(status: StatusCode, c: Category) -> Result<Response, CanonicalError> {
    let version = c.version;
    let body = ProductsCategoryDto::try_from(c).map_err(|e| repo_error_to_canonical(&e))?;
    Ok((
        status,
        [(header::ETAG, etag(InternalRevision::new(version)))],
        Json(body),
    )
        .into_response())
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
    let scope_tx = scope(&enforcer, &ctx, actions::AUTHOR).await?;
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
                    if new.is_default {
                        move_default(tx, &scope, tenant_id, actor, None, now).await?;
                    }
                    let c = repo::insert_category(tx, &scope, tenant_id, new, now)
                        .await
                        .map_err(|e| match e {
                            RepoError::Refused(RepoRefusal::CategoryCodeTaken) => {
                                TxError::Refused(DomainError::Conflict {
                                    code: "CATEGORY_CODE_TAKEN",
                                    detail: "a category with this code exists".into(),
                                })
                            }
                            other => default_taken(other),
                        })?;
                    audit(tx, &scope, tenant_id, actor, "category.create", &c, now).await?;
                    replay::finish(
                        tx,
                        tenant_id,
                        claim.as_ref(),
                        StatusCode::CREATED,
                        &ProductsCategoryDto::try_from(c).map_err(TxError::Repo)?,
                    )
                    .await
                })
            },
        )
        .await
        .map_err(tx_to_canonical)?;
    Ok(created)
}
/// One page of the tenant's categories, each with its `sku_count` from ONE grouped count of the
/// tenant's SKUs that are not retired (P-D-215).
/// @cpt-cf-bss-products-fr-category-flat
async fn list_categories(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    query: RawQuery,
    odata: Result<OData, CanonicalError>,
) -> Result<Json<Page<ProductsCategoryItem>>, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    // Authorization first, then the query (a 403 before a 400).
    let scope = scope(&enforcer, &ctx, actions::READ).await?;
    params(query, &["limit", "cursor"], None)?;
    let OData(mut odata) = odata?;
    if odata.select.is_some() {
        refused(&[(
            "$select",
            "a category is not projected; drop `$select`".to_owned(),
            UNSUPPORTED,
        )])?;
    }
    for key in &odata.order.0 {
        if CategoryOrderField::from_name(&key.field).is_none() {
            return Err(ODataError::InvalidOrderByField(key.field.clone()).into());
        }
    }
    // The cursor carries the hash of the `$filter` it was cut under, none included.
    let hash = cursor_hash(&serde_json::json!({ "categories": odata.filter_hash }));
    if let Some(cursor) = &odata.cursor
        && cursor.f.as_deref() != Some(hash.as_str())
    {
        return Err(ODataError::FilterMismatch.into());
    }
    odata.filter_hash = Some(hash);
    let tenant = ctx.subject_tenant_id();
    let conn = state.db.conn().map_err(|e| tx_to_canonical(e.into()))?;
    let page = repo::page_categories(&conn, &scope, tenant, &odata)
        .await
        .map_err(|e| match e {
            SkuListError::Query(e) => CanonicalError::from(e),
            SkuListError::Repo(e) => repo_error_to_canonical(&e),
        })?;
    let counts = repo::count_live_skus_by_category(&conn, tenant, None)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?;
    Ok(Json(Page {
        items: page
            .items
            .into_iter()
            .map(|c| {
                let sku_count = counts.get(&c.id).copied().unwrap_or(0);
                Ok(ProductsCategoryItem {
                    category: c.try_into()?,
                    sku_count,
                })
            })
            .collect::<Result<_, RepoError>>()
            .map_err(|e| repo_error_to_canonical(&e))?,
        page_info: page.page_info,
    }))
}
/// One category with its `ETag` and `sku_count` (P-D-215).
/// @cpt-cf-bss-products-fr-category-flat
async fn get_category(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let scope = scope(&enforcer, &ctx, actions::READ).await?;
    let tenant = ctx.subject_tenant_id();
    let conn = state.db.conn().map_err(|e| tx_to_canonical(e.into()))?;
    let c = repo::find_category(&conn, &scope, tenant, id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?
        .ok_or_else(|| {
            CanonicalError::from(DomainError::NotFound {
                what: "category",
                id,
            })
        })?;
    let sku_count = repo::count_live_skus_by_category(&conn, tenant, Some(id))
        .await
        .map_err(|e| repo_error_to_canonical(&e))?
        .get(&id)
        .copied()
        .unwrap_or(0);
    let version = c.version;
    let category = c.try_into().map_err(|e| repo_error_to_canonical(&e))?;
    Ok((
        [(header::ETAG, etag(InternalRevision::new(version)))],
        Json(ProductsCategoryItem {
            category,
            sku_count,
        }),
    )
        .into_response())
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
    let scope_tx = scope(&enforcer, &ctx, actions::AUTHOR).await?;
    let expected = if_match(&headers)?.get();
    let body = super::json_body(body)?;
    let patch_tx = CategoryPatch {
        name: body.name.map(|s| s.trim().to_owned()),
        is_default: body.is_default,
        sort_order: body.sort_order,
    };
    let mut r = ValidationReport::new();
    if patch_tx.name.as_deref() == Some("") {
        r.violate("VALIDATION", "name", "name must not be blank");
    }
    crate::domain::caps::check(
        &mut r,
        "name",
        patch_tx.name.as_deref(),
        crate::domain::caps::NAME_MAX_CHARS,
    );
    if !r.is_empty() {
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
                    // P-D-220: a retired category is never the default. A stale tag is judged
                    // first, as on every PATCH; nothing is cleared before the refusal.
                    if patch.is_default == Some(true) && current.status == RETIRED {
                        return Err(TxError::Refused(if current.version == expected {
                            DomainError::Conflict {
                                code: "CATEGORY_RETIRED",
                                detail: "a retired category cannot be the tenant's default".into(),
                            }
                        } else {
                            DomainError::StaleRevision {
                                expected,
                                found: current.version,
                            }
                        }));
                    }
                    if patch.is_default == Some(true) {
                        move_default(tx, &scope, tenant_id, actor, Some(id), now).await?;
                    }
                    let c = match repo::update_category(
                        tx, &scope, tenant_id, id, expected, patch, now,
                    )
                    .await
                    .map_err(default_taken)?
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
    response(StatusCode::OK, updated)
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
    let scope_tx = scope(&enforcer, &ctx, actions::AUTHOR).await?;
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
                    // P-D-220: retiring the tenant's default clears it first, a category write of
                    // its own with its own audit row; a refused retirement rolls it back.
                    if let Some(cleared) = repo::clear_default_of(tx, &scope, tenant_id, id, now)
                        .await
                        .map_err(TxError::Repo)?
                    {
                        audit(
                            tx,
                            &scope,
                            tenant_id,
                            actor,
                            "category.update",
                            &cleared,
                            now,
                        )
                        .await?;
                    }
                    let c = match repo::retire_category_if_unused(tx, &scope, tenant_id, id, now)
                        .await
                        .map_err(TxError::Repo)?
                    {
                        Some(HeadWrite::Written(c)) => c,
                        Some(HeadWrite::Unmatched) => {
                            // P-D-208: an already retired category names its own cause.
                            let retired = repo::find_category(tx, &scope, tenant_id, id)
                                .await
                                .map_err(TxError::Repo)?
                                .is_some_and(|c| c.status == RETIRED);
                            return Err(TxError::Refused(if retired {
                                DomainError::Conflict {
                                    code: "CATEGORY_RETIRED",
                                    detail: "the category is already retired".into(),
                                }
                            } else {
                                DomainError::Conflict {
                                    code: "CATEGORY_IN_USE",
                                    detail: "a SKU that is not retired names this category".into(),
                                }
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
                        &ProductsCategoryDto::try_from(c).map_err(TxError::Repo)?,
                    )
                    .await
                })
            },
        )
        .await
        .map_err(tx_to_canonical)?;
    Ok(retired)
}
/// Clear the tenant's other default before a write that sets one (P-D-218): the move is one
/// transaction, and the old holder's write is audited as every category write is.
async fn move_default(
    tx: &impl toolkit_db::secure::DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor: Uuid,
    keep: Option<Uuid>,
    now: OffsetDateTime,
) -> Result<(), TxError> {
    for cleared in repo::clear_default_category(tx, scope, tenant_id, keep, now)
        .await
        .map_err(TxError::Repo)?
    {
        audit(
            tx,
            scope,
            tenant_id,
            actor,
            "category.update",
            &cleared,
            now,
        )
        .await?;
    }
    Ok(())
}
/// A default that another writer took between the clear and the set: the losing side of two
/// concurrent moves, a 409 (P-D-218), never the 500 of an unmapped index.
fn default_taken(e: RepoError) -> TxError {
    match e {
        RepoError::Refused(RepoRefusal::CategoryDefaultTaken) => {
            TxError::Refused(DomainError::Conflict {
                code: "CATEGORY_DEFAULT_TAKEN",
                detail: "another category became the tenant's default; read it and retry".into(),
            })
        }
        other => TxError::Repo(other),
    }
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
            lifecycle: repo::LifecycleMove::NONE,
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
