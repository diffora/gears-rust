//! `02-taxonomy-attributes`' four wire doors: the taxonomy ops, the
//! attribute-definition ops, the category live-value patch and the entity
//! metadata patch.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-metadata-door:p1
//! @cpt-dod:cpt-cf-bss-products-dod-category-live-value-door:p1
//!
//! # One `operations` door per envelope, not one per verb (P-D-106)
//!
//! The taxonomy ops and the definition ops each get a create route and one
//! `operations` route, because the design already makes their verbs one
//! thing: they ride **one** `GovernedLiveOp` envelope, queue through one
//! gate, share one apply path — step 2 re-validates name uniqueness *"on
//! rename **and** re-parent"* in a single clause — and step 5 has the
//! envelope id ride the event. So the act is the payload's and not the
//! path's, and the non-material label edit rides the same door because
//! materiality is judged by the envelope's kind through `05 inst-mt-inputs`,
//! never by which path was called.
//!
//! # The live-value door is a `PATCH`, and that is a different mechanism
//!
//! `inst-av-category-branch` makes it **non-material** with a precondition of
//! its own — `If-Match` on `products_category.mutation_seq`, a mismatch
//! raising `STALE_CATEGORY_TOKEN` (**P-D-50**), which is this slice's own code
//! and neither `STALE_REVISION` (01's entity head) nor `STALE_LIVE_OP` (the
//! envelope's). So it takes the metadata door's shape applied to the one
//! entity whose content is live rather than versioned.
//!
//! # Every ceiling is read from configuration
//!
//! **P-D-107 arm 1** put five interim numbers in `ProductsConfig` and
//! `ApiState` resolves them once at init. Nothing here inlines one: the
//! values are interim and the NFR workshop overrides them by configuration
//! with no code change.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Extension, Path};
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
use crate::domain::live_op::GovernedLiveOp;
use crate::domain::taxonomy::{DefinitionState, TaxonomyLimits};
use crate::domain::validation::ValidationReport;
use crate::infra::storage::repo::{self, RefusalSubject};

/// The `OpenAPI` tag every door registers under.
const TAG: &str = "BSS Products";

/// How many holders a retire refusal names before it says "at least N".
const RETIRE_SAMPLE: u64 = 5;

/// The canonical-error identity of the category surface's refusals.
#[resource_error(gts_id!("cf.bss.products.category.v1~"))]
struct CategoryResource;

/// The canonical-error identity of the attribute-definition surface.
#[resource_error(gts_id!("cf.bss.products.attribute_definition.v1~"))]
struct DefinitionResource;

/// The canonical-error identity of the metadata surface.
#[resource_error(gts_id!("cf.bss.products.metadata.v1~"))]
struct MetadataResource;

/// Which grant a door spends.
#[derive(Clone, Copy)]
enum Gate {
    /// `category × write` — the op doors **and** the live-value door, a
    /// category's values being the category's content and not a resource of
    /// their own.
    Category,
    /// `attribute_definition × write`.
    Definition,
    /// `metadata × write`.
    Metadata,
}

impl Gate {
    fn resource(self) -> authz_resolver_sdk::ResourceType {
        match self {
            Self::Category => crate::authz::resource_types::CATEGORY,
            Self::Definition => crate::authz::resource_types::ATTRIBUTE_DEFINITION,
            Self::Metadata => crate::authz::resource_types::METADATA,
        }
    }

    fn subject_kind(self) -> &'static str {
        match self {
            Self::Category => "category",
            Self::Definition => "attribute_definition",
            Self::Metadata => "metadata",
        }
    }

    fn permission_denied(self, reason: String) -> CanonicalError {
        match self {
            Self::Category => CategoryResource::permission_denied()
                .with_reason(reason)
                .create(),
            Self::Definition => DefinitionResource::permission_denied()
                .with_reason(reason)
                .create(),
            Self::Metadata => MetadataResource::permission_denied()
                .with_reason(reason)
                .create(),
        }
    }
}

/// One category as every door here answers it.
#[toolkit_macros::api_dto(response)]
pub struct CategoryView {
    /// The row's own id.
    pub category_id: Uuid,
    /// The operator-facing name.
    pub name: String,
    /// The parent, absent for a root.
    pub parent_id: Option<Uuid>,
}

/// Create one category.
#[toolkit_macros::api_dto(request)]
pub struct CreateCategoryRequest {
    /// The operator-facing name, unique within the sibling set.
    pub name: String,
    /// The parent, absent for a root.
    pub parent_id: Option<Uuid>,
}

/// One taxonomy operation, as the envelope carries it.
#[toolkit_macros::api_dto(request)]
pub struct CategoryOperationRequest {
    /// `rename`, `reparent`, `retire` or `delete` — the act is the payload's,
    /// not the path's (P-D-106).
    pub op: String,
    /// The category's state as the submitter saw it, re-validated at apply.
    pub expected_state: String,
    /// The new name, for `rename`.
    pub name: Option<String>,
    /// The new parent, for `reparent`; absent means "make it a root".
    pub parent_id: Option<Uuid>,
}

/// Create one attribute definition.
#[toolkit_macros::api_dto(request)]
pub struct CreateDefinitionRequest {
    /// The definition's stable key.
    pub key: String,
    /// Its value type.
    pub value_type: String,
    /// Whether values are localized.
    pub localized: bool,
    /// The region scope, `""` for unrestricted (P-D-39).
    pub region_scope: String,
    /// The brand scope, `""` for unrestricted (P-D-39).
    pub brand_scope: String,
}

/// One definition operation.
#[toolkit_macros::api_dto(request)]
pub struct DefinitionOperationRequest {
    /// `deprecate`, `remove`, `relist` or `label`.
    pub op: String,
    /// The definition's state as the submitter saw it.
    pub expected_state: String,
    /// The new display label, for `label` — a **value** on the definition and
    /// not a column (**P-D-108** arm 2).
    pub display_label: Option<String>,
}

/// One attribute definition as the doors answer it.
#[toolkit_macros::api_dto(response)]
pub struct DefinitionView {
    /// The row's own id.
    pub definition_id: Uuid,
    /// Its stable key.
    pub key: String,
    /// Its lifecycle state.
    pub state: String,
}

/// One value written at the category live-value door.
#[toolkit_macros::api_dto(request)]
pub struct CategoryValueWrite {
    /// The definition this value belongs to.
    pub definition_id: Uuid,
    /// The locale coordinate, `""` for the global one (**P-D-102**).
    pub locale: String,
    /// The region coordinate, `""` for the global one.
    pub region: String,
    /// The brand coordinate, `""` for the global one.
    pub brand: String,
    /// The value; `null` removes the coordinate.
    pub value: Option<String>,
}

/// The live-value patch.
#[toolkit_macros::api_dto(request)]
pub struct CategoryValuesPatch {
    /// The token the caller read, matched against
    /// `products_category.mutation_seq`.
    pub expected_seq: i64,
    /// The coordinates to write or remove.
    pub values: Vec<CategoryValueWrite>,
}

/// What the live-value door answers.
#[toolkit_macros::api_dto(response)]
pub struct CategoryValuesView {
    /// The category written.
    pub category_id: Uuid,
    /// The token after the act — **acts, not row writes** (**P-D-50**).
    pub mutation_seq: i64,
}

/// The metadata patch: a per-key merge, `null` removing a key.
#[toolkit_macros::api_dto(request)]
pub struct MetadataPatch {
    /// Keys to set or remove. An absent key is left untouched, which is what
    /// gives a map standing at the cap an exit.
    pub entries: BTreeMap<String, Option<String>>,
}

/// The metadata map as the door answers it.
#[toolkit_macros::api_dto(response)]
pub struct MetadataView {
    /// The entity written.
    pub entity_id: Uuid,
    /// The map after the merge.
    pub entries: BTreeMap<String, String>,
}

/// Register the four doors' seven routes.
pub(crate) fn router(state: Arc<ApiState>, openapi: &dyn OpenApiRegistry) -> Router {
    let router = Router::new();
    let router = OperationBuilder::post("/bss-products/v1/categories")
        .operation_id("bss_products.create_category")
        .summary("Create a category")
        .description(
            "Creates one category under `category x write`, inside the per-tenant taxonomy \
             writer lock: the lock is taken, the tree is read, the configured depth and \
             fan-out ceilings are judged, and only then is the row written. Reading before \
             the lock would judge a chain a peer can still change. A name already taken in \
             the sibling set is `DUPLICATE_CATEGORY_NAME`; a landing place past a ceiling is \
             `TAXONOMY_LIMIT`.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .json_request::<CreateCategoryRequest>(openapi, "The category to create.")
        .handler(create_category)
        .json_response_with_schema::<CategoryView>(
            openapi,
            StatusCode::CREATED,
            "The category as stored.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    let router = OperationBuilder::post("/bss-products/v1/categories/{categoryId}/operations")
        .operation_id("bss_products.execute_category_operation")
        .summary("Rename, re-parent, retire or delete a category")
        .description(
            "One door for the four acts, because they ride one `GovernedLiveOp` envelope, one \
             gate and one apply path (P-D-106): the act is the payload's `op`, not the path. \
             The envelope pins the category's state at submission and re-validates it \
             immediately before the mutation, a mismatch being `STALE_LIVE_OP`. A re-parent \
             that would close a cycle is `TAXONOMY_CYCLE`; one that would break a configured \
             ceiling - at the leaves of the moved subtree, not only at the moved node - is \
             `TAXONOMY_LIMIT`. A retire with a live product assignment or an active child is \
             `CATEGORY_REFERENCED`, naming a bounded sample. A delete applies only to a \
             retired category.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("categoryId", "The category to act on.")
        .json_request::<CategoryOperationRequest>(openapi, "The operation envelope.")
        .handler(execute_category_operation)
        .json_response_with_schema::<CategoryView>(
            openapi,
            StatusCode::OK,
            "The category after the act.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    let router = OperationBuilder::post("/bss-products/v1/attribute-definitions")
        .operation_id("bss_products.create_attribute_definition")
        .summary("Create an attribute definition")
        .description(
            "Creates one definition under `attribute_definition x write`. An empty \
             `regionScope` or `brandScope` means unrestricted and not empty (P-D-39).",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .json_request::<CreateDefinitionRequest>(openapi, "The definition to create.")
        .handler(create_definition)
        .json_response_with_schema::<DefinitionView>(
            openapi,
            StatusCode::CREATED,
            "The definition as stored.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    let router = OperationBuilder::post("/bss-products/v1/attribute-definitions/{key}/operations")
        .operation_id("bss_products.execute_definition_operation")
        .summary("Deprecate, remove, re-list or re-label an attribute definition")
        .description(
            "One door for the state flips and the label edit (P-D-106). Removal is the \
             definition's `removed` state and never a DELETE (P-D-47), and it is **material** \
             (P-D-108 arm 1) - one envelope cannot be material in one direction only, and the \
             list as written priced the irreversible act below the reversible one. The label \
             is not a column: it is an attribute value on the definition itself, keyed \
             `entity_kind = 'attribute_definition'` (P-D-108 arm 2), resolving through the \
             same locale chain every other display name uses.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("key", "The definition's stable key.")
        .json_request::<DefinitionOperationRequest>(openapi, "The operation envelope.")
        .handler(execute_definition_operation)
        .json_response_with_schema::<DefinitionView>(
            openapi,
            StatusCode::OK,
            "The definition after the act.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    let router =
        OperationBuilder::patch("/bss-products/v1/categories/{categoryId}/attribute-values")
            .operation_id("bss_products.patch_category_attribute_values")
            .summary("Write a category's live attribute values")
            .description(
                "A non-material `PATCH` under `category x write` with its own precondition: \
                 `expectedSeq` is matched against `products_category.mutation_seq` and a \
                 mismatch is `STALE_CATEGORY_TOKEN` - this slice's own code, neither 01's \
                 entity-head `STALE_REVISION` nor the envelope's `STALE_LIVE_OP`. The four \
                 value rules run (definition known, definition active, value type, scope); \
                 the three assignment rules do not, having no operand when the subject is a \
                 category (P-D-107 arm 2). A `null` value removes that coordinate.",
            )
            .tag(TAG)
            .authenticated()
            .no_license_required()
            .path_param("categoryId", "The category to write.")
            .json_request::<CategoryValuesPatch>(openapi, "The coordinates to write or remove.")
            .handler(patch_category_values)
            .json_response_with_schema::<CategoryValuesView>(
                openapi,
                StatusCode::OK,
                "The category and its token after the act.",
            )
            .error_400(openapi)
            .error_401(openapi)
            .error_403(openapi)
            .error_404(openapi)
            .error_409(openapi)
            .error_500(openapi)
            .error_503(openapi)
            .register(router, openapi);

    let router = register_metadata_door(
        router,
        openapi,
        "/bss-products/v1/products/{productId}/metadata",
        "bss_products.patch_product_metadata",
        "productId",
        patch_product_metadata,
    );
    let router = register_metadata_door(
        router,
        openapi,
        "/bss-products/v1/skus/{skuId}/metadata",
        "bss_products.patch_sku_metadata",
        "skuId",
        patch_sku_metadata,
    );

    router.layer(Extension(state))
}

/// The metadata door twice, once per entity kind.
///
/// One registration helper rather than two copied blocks: the two routes are
/// the same door — `design/05` §3.2 declares the pair with one path carrying
/// `{products|skus}` — and copying the description is how two halves of one
/// contract drift.
fn register_metadata_door<H, T>(
    router: Router,
    openapi: &dyn OpenApiRegistry,
    path: &str,
    operation_id: &str,
    param: &str,
    handler: H,
) -> Router
where
    H: axum::handler::Handler<T, ()>,
    T: 'static,
{
    OperationBuilder::patch(path)
        .operation_id(operation_id)
        .summary("Merge an entity's metadata map")
        .description(
            "A per-key merge under `metadata x write`: an absent key is left untouched and a \
             `null` value removes its key, which is what gives a map standing at the cap an \
             exit. Three configured ceilings are enforced here and not in the store - key \
             count, key byte length, value byte length - each raising `METADATA_LIMIT`, and \
             the map is outside frozen version content (P-D-06), so this write bumps no \
             version and rides no `If-Match`. Values are operator free text and pass the \
             content-PII write block with no carve-out. A terminal entity is refused \
             `ENTITY_TERMINAL`.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param(param, "The entity whose map is written.")
        .json_request::<MetadataPatch>(openapi, "The keys to set or remove.")
        .handler(handler)
        .json_response_with_schema::<MetadataView>(
            openapi,
            StatusCode::OK,
            "The map after the merge.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi)
}

/// Authorize one grant, auditing a denial as a refusal.
async fn door_scope(
    state: &ApiState,
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
    tenant_id: Uuid,
    actor_ref: Uuid,
    gate: Gate,
    subject: String,
) -> Result<AccessScope, CanonicalError> {
    match crate::authz::access_scope(
        enforcer,
        ctx,
        &gate.resource(),
        crate::authz::actions::WRITE,
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
                    subject_kind: gate.subject_kind(),
                    error_code: "PERMISSION_DENIED",
                },
                RefusalSubject::Attempted(subject),
                gate.permission_denied(reason),
            )
            .await)
        }
        Err(err @ crate::authz::AuthzError::Unavailable(_)) => {
            Err(crate::api::rest::authz_error_to_canonical(err, |reason| {
                gate.permission_denied(reason)
            }))
        }
    }
}

/// Refuse, audit the refusal, and answer.
async fn refuse(
    state: &ApiState,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor_ref: Uuid,
    gate: Gate,
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
            subject_kind: gate.subject_kind(),
            error_code: code,
        },
        RefusalSubject::Attempted(subject),
        CanonicalError::from(refusal),
    )
    .await
}

/// One violation, as a `DomainError` the ladder renders by its own code.
fn violation(code: &'static str, field: &'static str, detail: impl Into<String>) -> DomainError {
    let mut report = ValidationReport::new();
    report.violate(code, field, detail);
    DomainError::Validation(report)
}

/// The configured ceilings as the domain rule wants them.
fn limits_of(state: &ApiState) -> TaxonomyLimits {
    TaxonomyLimits {
        max_depth: Some(state.taxonomy_caps.max_depth),
        max_children: Some(state.taxonomy_caps.max_children_per_node),
    }
}

async fn create_category(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Json(body): Json<CreateCategoryRequest>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(Utc::now());
    let name = body.name.trim().to_owned();
    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;
    let scope = door_scope(
        &state,
        &enforcer,
        &ctx,
        tenant_id,
        actor_ref,
        Gate::Category,
        name.clone(),
    )
    .await?;

    if name.is_empty() {
        return Err(refuse(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            Gate::Category,
            name,
            violation("VALIDATION", "name", "name must not be blank"),
        )
        .await);
    }

    let category_id = Uuid::now_v7();
    let normalized = crate::domain::name::normalize(&name);
    let written = crate::infra::taxonomy::create_under_lock(
        &state.db,
        &scope,
        repo::NewCategory {
            tenant_id,
            category_id,
            parent_id: body.parent_id,
            name: &name,
            name_normalized: &normalized,
        },
        limits_of(&state),
        now,
    )
    .await
    .map_err(|e| repo_error_to_canonical(&e))?;

    match written {
        Ok(()) => Ok((
            StatusCode::CREATED,
            Json(CategoryView {
                category_id,
                name,
                parent_id: body.parent_id,
            }),
        )
            .into_response()),
        Err(refusal) => Err(refuse(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            Gate::Category,
            name,
            refusal,
        )
        .await),
    }
}

async fn execute_category_operation(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(category_id): Path<Uuid>,
    Json(body): Json<CategoryOperationRequest>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(Utc::now());
    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;
    let scope = door_scope(
        &state,
        &enforcer,
        &ctx,
        tenant_id,
        actor_ref,
        Gate::Category,
        category_id.to_string(),
    )
    .await?;

    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&crate::infra::storage::RepoError::Db(e.to_string()))
    })?;
    let states = repo::category_states(&conn, &scope, tenant_id, &[category_id])
        .await
        .map_err(|e| repo_error_to_canonical(&e))?;
    let Some((_, live_state)) = states.into_iter().next() else {
        return Err(
            CategoryResource::not_found("no category with this id in this tenant")
                .with_resource(category_id.to_string())
                .create(),
        );
    };

    // The envelope pins the state the submitter saw and re-validates it
    // immediately before the mutation (`inst-gl-atomic`). The apply itself is
    // the locked operation below, which is why the check runs here and the
    // closure is not this door's shape.
    let op = GovernedLiveOp {
        kind: format!("category.{}", body.op),
        target: category_id.to_string(),
        payload: body.name.clone().unwrap_or_default(),
        expected_state: body.expected_state.clone(),
    };
    if let Err(stale) = op.check_still_current(&live_state.as_str().to_owned()) {
        return Err(refuse(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            Gate::Category,
            category_id.to_string(),
            stale,
        )
        .await);
    }

    let outcome = match body.op.as_str() {
        "rename" => {
            let Some(name) = body
                .name
                .as_deref()
                .map(str::trim)
                .filter(|n| !n.is_empty())
            else {
                return Err(refuse(
                    &state,
                    &scope,
                    tenant_id,
                    actor_ref,
                    Gate::Category,
                    category_id.to_string(),
                    violation("VALIDATION", "name", "rename needs a non-blank name"),
                )
                .await);
            };
            crate::infra::taxonomy::rename_under_lock(
                &state.db,
                &scope,
                tenant_id,
                category_id,
                name,
                now,
            )
            .await
        }
        "reparent" => {
            crate::infra::taxonomy::reparent_under_lock(
                &state.db,
                &scope,
                tenant_id,
                category_id,
                body.parent_id,
                limits_of(&state),
                now,
            )
            .await
        }
        "retire" => crate::infra::taxonomy::retire_under_lock(
            &state.db,
            &scope,
            tenant_id,
            category_id,
            RETIRE_SAMPLE,
            now,
        )
        .await
        .map(|r| r.map(|_| repo::CategoryWrite::Applied)),
        "delete" => {
            crate::infra::taxonomy::delete_under_lock(&state.db, &scope, tenant_id, category_id)
                .await
                .map(Ok)
        }
        other => {
            return Err(refuse(
                &state,
                &scope,
                tenant_id,
                actor_ref,
                Gate::Category,
                category_id.to_string(),
                violation(
                    "VALIDATION",
                    "op",
                    format!("`{other}` is not one of rename, reparent, retire, delete"),
                ),
            )
            .await);
        }
    }
    .map_err(|e| repo_error_to_canonical(&e))?;

    match outcome {
        Ok(repo::CategoryWrite::Applied) => Ok((
            StatusCode::OK,
            Json(CategoryView {
                category_id,
                name: body.name.unwrap_or_default(),
                parent_id: body.parent_id,
            }),
        )
            .into_response()),
        Ok(repo::CategoryWrite::Unmatched) => Err(CategoryResource::not_found(
            "no category matched this act in this tenant, in a state that admits it",
        )
        .with_resource(category_id.to_string())
        .create()),
        Err(refusal) => Err(refuse(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            Gate::Category,
            category_id.to_string(),
            refusal,
        )
        .await),
    }
}

async fn create_definition(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Json(body): Json<CreateDefinitionRequest>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(Utc::now());
    let key = body.key.trim().to_owned();
    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;
    let scope = door_scope(
        &state,
        &enforcer,
        &ctx,
        tenant_id,
        actor_ref,
        Gate::Definition,
        key.clone(),
    )
    .await?;

    if key.is_empty() || body.value_type.trim().is_empty() {
        return Err(refuse(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            Gate::Definition,
            key,
            violation("VALIDATION", "key", "key and valueType must not be blank"),
        )
        .await);
    }

    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&crate::infra::storage::RepoError::Db(e.to_string()))
    })?;
    let definition_id = Uuid::now_v7();
    let stored = repo::insert_attribute_definition(
        &conn,
        &scope,
        repo::NewAttributeDefinition {
            tenant_id,
            definition_id,
            key: &key,
            value_type: body.value_type.trim(),
            localized: body.localized,
            region_scope: body.region_scope.trim(),
            brand_scope: body.brand_scope.trim(),
            // Operator-authored, so no registry provenance: `seeded_by` is
            // the well-known seeds' marker and inventing one here would make
            // an operator's definition undeletable by the seed guard.
            seeded_by: None,
        },
        now,
    )
    .await
    .map_err(|e| repo_error_to_canonical(&e))?;

    Ok((
        StatusCode::CREATED,
        Json(DefinitionView {
            definition_id: stored.definition_id,
            key: stored.key,
            state: stored.state.as_str().to_owned(),
        }),
    )
        .into_response())
}

async fn execute_definition_operation(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(key): Path<String>,
    Json(body): Json<DefinitionOperationRequest>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(Utc::now());
    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;
    let scope = door_scope(
        &state,
        &enforcer,
        &ctx,
        tenant_id,
        actor_ref,
        Gate::Definition,
        key.clone(),
    )
    .await?;

    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&crate::infra::storage::RepoError::Db(e.to_string()))
    })?;
    let Some(record) = repo::attribute_definition_by_key(&conn, &scope, tenant_id, &key)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?
    else {
        return Err(
            DefinitionResource::not_found("no definition with this key in this tenant")
                .with_resource(key)
                .create(),
        );
    };

    let op = GovernedLiveOp {
        kind: format!("attribute_definition.{}", body.op),
        target: key.clone(),
        payload: body.display_label.clone().unwrap_or_default(),
        expected_state: body.expected_state.clone(),
    };
    if let Err(stale) = op.check_still_current(&record.state.as_str().to_owned()) {
        return Err(refuse(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            Gate::Definition,
            key,
            stale,
        )
        .await);
    }

    let flip = match body.op.as_str() {
        "deprecate" => Some(repo::DefinitionFlip {
            expected: DefinitionState::Active,
            to: DefinitionState::Deprecated,
        }),
        "remove" => Some(repo::DefinitionFlip {
            expected: DefinitionState::Deprecated,
            to: DefinitionState::Removed,
        }),
        "relist" => Some(repo::DefinitionFlip {
            expected: DefinitionState::Removed,
            to: DefinitionState::Active,
        }),
        // The label is a **value on the definition** and not a column
        // (P-D-108 arm 2): `products_attribute_definition` has no label
        // column at all, so the non-material label edit had no target until
        // that decision gave it one.
        "label" => None,
        other => {
            return Err(refuse(
                &state,
                &scope,
                tenant_id,
                actor_ref,
                Gate::Definition,
                key,
                violation(
                    "VALIDATION",
                    "op",
                    format!("`{other}` is not one of deprecate, remove, relist, label"),
                ),
            )
            .await);
        }
    };

    let state_after = if let Some(flip) = flip {
        let to = flip.to;
        let moved =
            repo::flip_definition_state(&conn, &scope, tenant_id, record.definition_id, flip, now)
                .await
                .map_err(|e| repo_error_to_canonical(&e))?;
        if !moved {
            return Err(refuse(
                &state,
                &scope,
                tenant_id,
                actor_ref,
                Gate::Definition,
                key,
                DomainError::StaleLiveOp(format!(
                    "the definition was not in the state this `{}` requires",
                    body.op
                )),
            )
            .await);
        }
        to
    } else {
        let Some(label) = body
            .display_label
            .as_deref()
            .map(str::trim)
            .filter(|l| !l.is_empty())
        else {
            return Err(refuse(
                &state,
                &scope,
                tenant_id,
                actor_ref,
                Gate::Definition,
                key,
                violation("VALIDATION", "displayLabel", "a label edit needs a label"),
            )
            .await);
        };
        write_definition_label(&state, &conn, &scope, tenant_id, &record, label, now).await?;
        record.state
    };

    Ok((
        StatusCode::OK,
        Json(DefinitionView {
            definition_id: record.definition_id,
            key,
            state: state_after.as_str().to_owned(),
        }),
    )
        .into_response())
}

/// Write a definition's display label as an attribute value **on the
/// definition** (P-D-108 arm 2).
///
/// Keyed `entity_kind = 'attribute_definition'`, which is one of the four the
/// tightened `chk_products_attribute_value_entity_kind` admits, and written
/// at the global coordinate so the ordinary fallback chain resolves it for
/// every locale until a narrower one is written. The value's definition is
/// `displayName`, one of the well-known seeds — the label resolves through the
/// same chain every other display name uses, which is the category branch's
/// shape applied one level up.
async fn write_definition_label(
    state: &ApiState,
    conn: &impl toolkit_db::secure::DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    record: &repo::AttributeDefinitionRecord,
    label: &str,
    now: chrono::DateTime<Utc>,
) -> Result<(), CanonicalError> {
    let detector: Arc<dyn crate::domain::taxonomy::PiiDetector + Send + Sync> =
        Arc::new(crate::domain::taxonomy::NoPiiPolicyDetector);
    if let Err(blocked) =
        crate::domain::taxonomy::content_pii_block(detector.as_ref(), "displayLabel", label)
    {
        return Err(CanonicalError::from(DomainError::ContentPiiBlocked(
            blocked.into_detail(),
        )));
    }
    let _ = state;
    repo::upsert_attribute_value(
        conn,
        scope,
        tenant_id,
        repo::AttributeCoordinate {
            entity_kind: "attribute_definition",
            entity_id: record.definition_id,
            definition_id: record.definition_id,
            locale: "",
            region: "",
            brand: "",
        },
        label,
        now,
    )
    .await
    .map_err(|e| repo_error_to_canonical(&e))
}

async fn patch_category_values(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(category_id): Path<Uuid>,
    Json(body): Json<CategoryValuesPatch>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(Utc::now());
    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;
    let scope = door_scope(
        &state,
        &enforcer,
        &ctx,
        tenant_id,
        actor_ref,
        Gate::Category,
        category_id.to_string(),
    )
    .await?;

    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&crate::infra::storage::RepoError::Db(e.to_string()))
    })?;

    // The four value rules, and not the three assignment rules: those are
    // about assigning categories to a Product and have no operand when the
    // subject *is* a category (P-D-107 arm 2).
    let definitions = repo::attribute_definitions(&conn, &scope, tenant_id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?;
    let subject = crate::domain::taxonomy::ContentSaveSubject {
        assignments: Vec::new(),
        values: body
            .values
            .iter()
            .filter_map(|write| {
                let text = write.value.clone()?;
                let found = definitions
                    .iter()
                    .find(|d| d.definition_id == write.definition_id);
                Some(crate::domain::taxonomy::ValueCandidate {
                    definition_key: found
                        .map_or_else(|| write.definition_id.to_string(), |d| d.key.clone()),
                    locale: write.locale.clone(),
                    region: write.region.clone(),
                    brand: write.brand.clone(),
                    value: text,
                    resolved: found.map(|d| crate::domain::taxonomy::ResolvedDefinition {
                        state: d.state,
                        value_type: d.value_type.clone(),
                        localized: d.localized,
                        region_scope: d.region_scope.clone(),
                        brand_scope: d.brand_scope.clone(),
                    }),
                })
            })
            .collect(),
        // A category carries no scope of its own, so the scope rule judges
        // each value against the definition's alone -- an empty scope being
        // unrestricted and not empty (P-D-39).
        entity_region_scope: String::new(),
        entity_brand_scope: String::new(),
    };
    if let Some((_phase, report)) = crate::domain::taxonomy::category_value_pipeline().run(&subject)
    {
        return Err(refuse(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            Gate::Category,
            category_id.to_string(),
            DomainError::Validation(report),
        )
        .await);
    }

    // `inst-av-category-branch`: the **global** default-locale value is
    // required at the **first** write of a definition for that category. The
    // write-time analogue of the publish-time check, and the one rule this
    // door runs that the entity save door does not (P-D-107 arm 2) -- without
    // it a category could carry a `de-DE` value and nothing for a reader with
    // no matching locale, which is the state the fallback chain has no answer
    // for.
    let standing = repo::attribute_values_of(&conn, &scope, tenant_id, "category", category_id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?;
    for write in body.values.iter().filter(|w| w.value.is_some()) {
        let known = standing
            .iter()
            .any(|row| row.definition_id == write.definition_id);
        let carries_global = body.values.iter().any(|w| {
            w.definition_id == write.definition_id
                && w.value.is_some()
                && w.locale.is_empty()
                && w.region.is_empty()
                && w.brand.is_empty()
        });
        if !known && !carries_global {
            return Err(refuse(
                &state,
                &scope,
                tenant_id,
                actor_ref,
                Gate::Category,
                category_id.to_string(),
                violation(
                    crate::domain::taxonomy::DefaultLocaleRequired::CODE,
                    "values",
                    format!(
                        "the first write of definition {} for this category must carry the \
                         global coordinate",
                        write.definition_id
                    ),
                ),
            )
            .await);
        }
    }

    // Every judgement above runs **before** the token is taken. A refusal
    // that had already bumped `mutation_seq` would leave the caller's next
    // attempt -- the corrected one -- refused as stale for a token its own
    // rejected request had moved: measured, when the first shape of this
    // handler put the CAS ahead of the first-write rule and the corrected
    // patch came back 400.
    //
    // The token is taken by **compare-and-set** before a single value moves:
    // `STALE_CATEGORY_TOKEN` is this slice's own code for a caller quoting a
    // token some other act has since moved -- not 01's `STALE_REVISION`, which
    // names an entity head, and not the envelope's `STALE_LIVE_OP`. A read,
    // a comparison and then a write would leave exactly the window the code
    // exists to close.
    if repo::category_mutation_seq(&conn, &scope, tenant_id, category_id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?
        .is_none()
    {
        return Err(
            CategoryResource::not_found("no category with this id in this tenant")
                .with_resource(category_id.to_string())
                .create(),
        );
    }
    let bumped = match repo::bump_category_mutation_seq(
        &conn,
        &scope,
        tenant_id,
        category_id,
        body.expected_seq,
        now,
    )
    .await
    .map_err(|e| repo_error_to_canonical(&e))?
    {
        Ok(seq) => seq,
        Err(mismatch) => {
            return Err(refuse(
                &state,
                &scope,
                tenant_id,
                actor_ref,
                Gate::Category,
                category_id.to_string(),
                violation(
                    crate::domain::taxonomy::StaleCategoryToken::CODE,
                    "expectedSeq",
                    format!(
                        "the category's token is {} and the request carried {}",
                        mismatch.found, mismatch.expected
                    ),
                ),
            )
            .await);
        }
    };

    for write in &body.values {
        let coordinate = repo::AttributeCoordinate {
            entity_kind: "category",
            entity_id: category_id,
            definition_id: write.definition_id,
            locale: &write.locale,
            region: &write.region,
            brand: &write.brand,
        };
        match write.value.as_deref() {
            Some(text) => {
                repo::upsert_attribute_value(&conn, &scope, tenant_id, coordinate, text, now)
                    .await
                    .map_err(|e| repo_error_to_canonical(&e))?;
            }
            None => {
                repo::delete_attribute_value(&conn, &scope, tenant_id, coordinate)
                    .await
                    .map_err(|e| repo_error_to_canonical(&e))?;
            }
        }
    }

    // One act, one bump: `mutation_seq` counts **acts, not row writes**
    // (P-D-50), so a patch carrying six coordinates moves the token by one --
    // which is what makes the number the caller reads back quotable on its
    // next patch.
    Ok((
        StatusCode::OK,
        Json(CategoryValuesView {
            category_id,
            mutation_seq: bumped,
        }),
    )
        .into_response())
}

async fn patch_product_metadata(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(entity_id): Path<Uuid>,
    Json(body): Json<MetadataPatch>,
) -> Result<Response, CanonicalError> {
    merge_metadata(state, enforcer, extension_ctx, "product", entity_id, body).await
}

async fn patch_sku_metadata(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(entity_id): Path<Uuid>,
    Json(body): Json<MetadataPatch>,
) -> Result<Response, CanonicalError> {
    merge_metadata(state, enforcer, extension_ctx, "sku", entity_id, body).await
}

/// The metadata merge, once, for both entity kinds.
async fn merge_metadata(
    state: Arc<ApiState>,
    enforcer: authz_resolver_sdk::PolicyEnforcer,
    extension_ctx: Option<Extension<SecurityContext>>,
    entity_kind: &'static str,
    entity_id: Uuid,
    body: MetadataPatch,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(Utc::now());
    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;
    let scope = door_scope(
        &state,
        &enforcer,
        &ctx,
        tenant_id,
        actor_ref,
        Gate::Metadata,
        entity_id.to_string(),
    )
    .await?;

    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&crate::infra::storage::RepoError::Db(e.to_string()))
    })?;

    // A head that is not there at all is a 404 and not a silent success. The
    // first shape of this door read the state, filtered it for terminality
    // and treated `None` as "fine": a write to an entity of another tenant,
    // or to no entity, would have landed a metadata row keyed on an id
    // nothing owns.
    let Some(state_now) = head_state(&conn, &scope, tenant_id, entity_kind, entity_id).await?
    else {
        return Err(
            MetadataResource::not_found("no entity with this id in this tenant")
                .with_resource(entity_id.to_string())
                .create(),
        );
    };
    if repo::TERMINAL_HEAD_STATES.contains(&state_now.as_str()) {
        return Err(refuse(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            Gate::Metadata,
            entity_id.to_string(),
            DomainError::EntityTerminal(format!(
                "the {entity_kind} is {}, and a terminal entity's metadata is not writable",
                state_now.as_str()
            )),
        )
        .await);
    }

    let standing = repo::metadata_of(&conn, &scope, tenant_id, entity_kind, entity_id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?;

    // The caps are judged against the map the merge **would leave**, not
    // against the request: an absent key is untouched, so a patch that only
    // removes keys must be admitted by a map standing at the ceiling.
    let mut merged: BTreeMap<String, String> = standing
        .into_iter()
        .map(|entry| (entry.key, entry.value))
        .collect();
    for (key, value) in &body.entries {
        match value {
            Some(text) => {
                merged.insert(key.clone(), text.clone());
            }
            None => {
                merged.remove(key);
            }
        }
    }
    if let Some(refusal) = cap_refusal(&state, &merged) {
        return Err(refuse(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            Gate::Metadata,
            entity_id.to_string(),
            refusal,
        )
        .await);
    }

    // Values are operator free text, inside the write block with no carve-out
    // (`inst-md-write`).
    let detector: Arc<dyn crate::domain::taxonomy::PiiDetector + Send + Sync> =
        Arc::new(crate::domain::taxonomy::NoPiiPolicyDetector);
    for (key, value) in &merged {
        if let Err(blocked) = crate::domain::taxonomy::content_pii_block(
            detector.as_ref(),
            &format!("metadata.{key}"),
            value,
        ) {
            return Err(refuse(
                &state,
                &scope,
                tenant_id,
                actor_ref,
                Gate::Metadata,
                entity_id.to_string(),
                DomainError::ContentPiiBlocked(blocked.into_detail()),
            )
            .await);
        }
    }

    for (key, value) in &body.entries {
        match value {
            Some(text) => repo::upsert_metadata(
                &conn,
                &scope,
                tenant_id,
                entity_kind,
                entity_id,
                (key, text),
                now,
            )
            .await
            .map_err(|e| repo_error_to_canonical(&e))?,
            None => {
                repo::delete_metadata_key(&conn, &scope, tenant_id, entity_kind, entity_id, key)
                    .await
                    .map(|_| ())
                    .map_err(|e| repo_error_to_canonical(&e))?;
            }
        }
    }

    Ok((
        StatusCode::OK,
        Json(MetadataView {
            entity_id,
            entries: merged,
        }),
    )
        .into_response())
}

/// The head's lifecycle state, or `None` when there is no such head here.
///
/// Absence and terminality are answered apart on purpose: they are a 404 and
/// a 409, and a helper that folded them into one `Option<DomainError>` made
/// "no such entity" read as "nothing to refuse".
///
/// The terminal roster the caller compares against is
/// `repo::TERMINAL_HEAD_STATES`, read rather than copied: a second
/// two-element list of terminal states is a second thing to forget the day a
/// third state joins it.
async fn head_state(
    conn: &impl toolkit_db::secure::DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    entity_kind: &str,
    entity_id: Uuid,
) -> Result<Option<bss_products_sdk::models::LifecycleState>, CanonicalError> {
    if entity_kind == "product" {
        Ok(repo::find_product(conn, scope, tenant_id, entity_id)
            .await
            .map_err(|e| repo_error_to_canonical(&e))?
            .map(|row| row.lifecycle_state))
    } else {
        Ok(repo::find_sku(conn, scope, tenant_id, entity_id)
            .await
            .map_err(|e| repo_error_to_canonical(&e))?
            .map(|row| row.lifecycle_state))
    }
}

#[cfg(test)]
#[path = "taxonomy_tests.rs"]
mod taxonomy_tests;

/// `METADATA_LIMIT` when the merged map breaks a configured ceiling.
///
/// Three ceilings, each named separately in the refusal: an operator told
/// only *"a cap was exceeded"* cannot tell a key-count problem from a
/// value-length one, and the three have different fixes.
fn cap_refusal(state: &ApiState, merged: &BTreeMap<String, String>) -> Option<DomainError> {
    let caps = state.taxonomy_caps;
    if merged.len() > caps.metadata_max_keys as usize {
        return Some(DomainError::MetadataLimit(format!(
            "the map would hold {} keys and the configured ceiling is {}",
            merged.len(),
            caps.metadata_max_keys
        )));
    }
    for (key, value) in merged {
        if key.len() > caps.metadata_max_key_bytes as usize {
            return Some(DomainError::MetadataLimit(format!(
                "key `{key}` is {} bytes and the configured ceiling is {}",
                key.len(),
                caps.metadata_max_key_bytes
            )));
        }
        if value.len() > caps.metadata_max_value_bytes as usize {
            return Some(DomainError::MetadataLimit(format!(
                "the value of `{key}` is {} bytes and the configured ceiling is {}",
                value.len(),
                caps.metadata_max_value_bytes
            )));
        }
    }
    None
}
