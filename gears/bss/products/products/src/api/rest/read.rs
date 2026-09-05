//! `08`'s read surface (P-D-150): the browse door with its facets
//! (`inst-rb-query`, `inst-rb-visibility`, `inst-rb-stamp`, `inst-rb-facets`),
//! the version-history timeline (`inst-rh-timeline`), the three polled
//! dashboards (`inst-ps-dashboards`) — every one behind the single
//! per-tenant `ReadPathLimiter` (`inst-dg-shed`) and every response carrying
//! the `StalenessStamp`, degraded and empty alike.
//!
//! # Scope and visibility are in the query
//!
//! The tenant predicate is the PEP's `AccessScope`; the per-state contract is
//! `VisibilityFilter`'s `Condition`; brand and region claims, when a caller
//! passes them, are `scope_condition`s — all built into the statement, so a
//! shed row is never fetched (`dod-browse-door`).
//!
//! # The limiter is one component, in front of every door
//!
//! `ReadPathLimiter` is a per-tenant token bucket installed once at boot from
//! `ProductsConfig::read_path_qps_ceiling`; above the ceiling a door answers
//! `503 READ_MODEL_OVERLOADED` with `Retry-After`, no content, no counts.
//! Under **lag** the doors keep serving — the stamp labels the staleness and
//! the projector raises `read_model_lag` (`inst-dg-lag`).
//!
//! @cpt-dod:cpt-cf-bss-products-dod-browse-door:p1
//! @cpt-dod:cpt-cf-bss-products-dod-degradation:p1
//! @cpt-dod:cpt-cf-bss-products-dod-facets:p2

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use axum::Json;
use axum::Router;
use axum::extract::Extension;
use axum::http::StatusCode;
use axum::http::header::{HeaderValue, RETRY_AFTER};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use toolkit::api::OpenApiRegistry;
use toolkit::api::canonical_prelude::{CanonicalError, resource_error};
use toolkit::api::operation_builder::OperationBuilder;
use toolkit_db::secure::AccessScope;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::api::rest::{
    ApiState, authz_error_to_canonical, repo_error_to_canonical, require_authenticated,
};
use crate::domain::error::DomainError;
use crate::domain::read_model::{ReadSurface, StalenessStamp, VisibilityFilter};
use crate::infra::storage::repo::{self, BrowseQuery};

const TAG: &str = "BSS Products";

/// The canonical-error identity of this surface's own refusals.
#[resource_error(gts_id!("cf.bss.products.product.v1~"))]
struct ReadResource;

// ---------------------------------------------------------------------------
// The limiter (`dod-degradation`)
// ---------------------------------------------------------------------------

struct Bucket {
    tokens: f64,
    refilled_at: Instant,
}

/// The single per-tenant-partition limiter in front of every read door.
///
/// A token bucket per tenant: capacity and refill rate are the configured
/// ceiling (requests per second), so one tenant's burst sheds that tenant
/// alone. Installed once at boot; a process that never installed it runs on
/// the shipped default.
pub struct ReadPathLimiter {
    ceiling: u32,
    buckets: Mutex<HashMap<Uuid, Bucket>>,
    overrides: Mutex<HashMap<Uuid, u32>>,
}

static LIMITER: OnceLock<ReadPathLimiter> = OnceLock::new();

impl ReadPathLimiter {
    fn new(ceiling: u32) -> Self {
        Self {
            ceiling: ceiling.max(1),
            buckets: Mutex::new(HashMap::new()),
            overrides: Mutex::new(HashMap::new()),
        }
    }

    /// Install the process-wide limiter with the configured ceiling; a second
    /// call keeps the first (the boot's).
    pub fn install(ceiling: u32) -> &'static Self {
        LIMITER.get_or_init(|| Self::new(ceiling))
    }

    /// The installed limiter, or the shipped default when boot never ran
    /// (tests).
    pub fn global() -> &'static Self {
        LIMITER.get_or_init(|| Self::new(crate::config::READ_PATH_QPS_CEILING_DEFAULT))
    }

    /// A per-tenant ceiling override — the probes' operand for forcing a shed
    /// without waiting on the default's two hundred per second.
    pub fn set_ceiling_for(&self, tenant_id: Uuid, ceiling: u32) {
        self.overrides
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(tenant_id, ceiling.max(1));
        self.buckets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&tenant_id);
    }

    fn ceiling_for(&self, tenant_id: Uuid) -> u32 {
        self.overrides
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&tenant_id)
            .copied()
            .unwrap_or(self.ceiling)
    }

    /// Take one token for `tenant_id`, or answer the seconds to wait.
    pub fn try_acquire(&self, tenant_id: Uuid) -> Result<(), u32> {
        let ceiling = f64::from(self.ceiling_for(tenant_id));
        let now = Instant::now();
        let mut buckets = self
            .buckets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let bucket = buckets.entry(tenant_id).or_insert(Bucket {
            tokens: ceiling,
            refilled_at: now,
        });
        let elapsed = now.duration_since(bucket.refilled_at).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * ceiling).min(ceiling);
        bucket.refilled_at = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Ok(())
        } else {
            Err(1)
        }
    }
}

/// The edge meter (`dod-nfr-meters`): one structured line per served read
/// with the door and the latency, the p95 and the QPS per tenant partition
/// being the metrics backend's aggregation over it.
///
/// @cpt-dod:cpt-cf-bss-products-dod-nfr-meters:p1
fn observe_edge(door: &'static str, tenant_id: Uuid, started: Instant) {
    tracing::info!(
        event = "read_edge_latency",
        %tenant_id,
        door,
        latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        "bss-products: read served"
    );
}

/// The shed answer: `503 READ_MODEL_OVERLOADED` with `Retry-After`, the code
/// on the audit channel, no content and no counts (`inst-dg-shed`).
fn shed(tenant_id: Uuid, retry_after_secs: u32) -> Response {
    let refusal = DomainError::ReadModelOverloaded(format!(
        "tenant {tenant_id} is above its read ceiling; retry after {retry_after_secs}s"
    ));
    let mut response = CanonicalError::from(refusal).into_response();
    response.headers_mut().insert(
        RETRY_AFTER,
        HeaderValue::from_str(&retry_after_secs.to_string())
            .unwrap_or_else(|_| HeaderValue::from_static("1")),
    );
    response
}

// ---------------------------------------------------------------------------
// The stamp (`inst-rb-stamp`)
// ---------------------------------------------------------------------------

/// The `StalenessStamp` every response carries (C3): the catalog version
/// fully reflected (`null` for a tenant with none) and the projection's
/// fine-grained coordinate.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct StampView {
    /// Every catalog version at or below this is fully reflected; `null`
    /// before the first (the anchorless arm).
    pub as_of_catalog_version: Option<i64>,
    /// The projection's last advance; a tenant nothing was projected for yet
    /// reads the request instant, so the stamp is never omitted.
    pub projected_at: DateTime<Utc>,
}

impl From<StalenessStamp> for StampView {
    fn from(stamp: StalenessStamp) -> Self {
        Self {
            as_of_catalog_version: stamp.as_of_catalog_version,
            projected_at: stamp.projected_at,
        }
    }
}

async fn stamp_of(
    conn: &(impl toolkit_db::secure::DBRunner + Sync),
    scope: &AccessScope,
    tenant_id: Uuid,
    now: DateTime<Utc>,
) -> Result<StampView, CanonicalError> {
    let stamp = repo::load_read_stamp(conn, scope, tenant_id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?
        .unwrap_or_else(|| StalenessStamp::anchorless(now));
    Ok(StampView::from(stamp))
}

/// The read grant a door spends, resolved to the tenant's scope.
async fn read_scope(
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
    resource: &authz_resolver_sdk::pep::ResourceType,
    tenant_id: Uuid,
) -> Result<AccessScope, CanonicalError> {
    crate::authz::access_scope(
        enforcer,
        ctx,
        resource,
        crate::authz::actions::READ,
        Some(tenant_id),
        None,
        true,
    )
    .await
    .map_err(|e| {
        authz_error_to_canonical(e, |reason| {
            ReadResource::permission_denied()
                .with_reason(reason)
                .create()
        })
    })
}

// ---------------------------------------------------------------------------
// Browse (`inst-rb-query`, `inst-rb-visibility`, `inst-rb-facets`)
// ---------------------------------------------------------------------------

#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowseParams {
    /// `product` or `sku`; both when absent.
    kind: Option<String>,
    /// A name prefix.
    q: Option<String>,
    /// A category path segment or full path; every assigned category counts.
    category: Option<String>,
    sku_type: Option<String>,
    tier: Option<String>,
    sellable: Option<bool>,
    unit: Option<String>,
    exclude_deprecated: Option<bool>,
    brand: Option<String>,
    region: Option<String>,
    include_facets: Option<bool>,
    limit: Option<u64>,
}

/// One browse row.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct BrowseRowView {
    pub entity_kind: String,
    pub entity_id: Uuid,
    pub entity_code: Option<String>,
    pub name: String,
    pub lifecycle_state: String,
    /// The machine-readable flag `deprecated` rows carry (C2).
    pub deprecated: bool,
    pub composition_pending: bool,
    pub sellable: Option<bool>,
    pub deprecation_provenance: Option<String>,
    pub replaced_by_sku_id: Option<Uuid>,
    pub region_scope: String,
    pub brand_scope: String,
    pub sku_type: Option<String>,
    pub plan_tier_label: Option<String>,
    pub metering_unit: Option<String>,
    /// Per active locale, the definition key to its display value (JSON).
    pub display_attributes: Option<String>,
    /// Every assigned category's path, primary and secondary (JSON array).
    pub category_paths: Option<String>,
    pub published_version: i64,
}

/// One facet value and its count.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct FacetBucketView {
    pub value: String,
    pub count: u64,
}

/// The facets over the served set (`inst-rb-facets`).
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct FacetsView {
    pub categories: Vec<FacetBucketView>,
    pub sku_types: Vec<FacetBucketView>,
    pub tiers: Vec<FacetBucketView>,
    pub sellable: Vec<FacetBucketView>,
    pub units: Vec<FacetBucketView>,
}

/// The browse answer: rows, facets when asked, the stamp always.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct BrowseView {
    pub stamp: StampView,
    pub rows: Vec<BrowseRowView>,
    pub facets: Option<FacetsView>,
}

/// The most rows one browse answers.
pub const BROWSE_LIMIT_MAX: u64 = 500;

fn buckets(counts: BTreeMap<String, u64>) -> Vec<FacetBucketView> {
    counts
        .into_iter()
        .map(|(value, count)| FacetBucketView { value, count })
        .collect()
}

fn facets_of(rows: &[crate::infra::storage::entity::read_entity::Model]) -> FacetsView {
    let mut categories: BTreeMap<String, u64> = BTreeMap::new();
    let mut sku_types: BTreeMap<String, u64> = BTreeMap::new();
    let mut tiers: BTreeMap<String, u64> = BTreeMap::new();
    let mut sellable: BTreeMap<String, u64> = BTreeMap::new();
    let mut units: BTreeMap<String, u64> = BTreeMap::new();
    for row in rows {
        if let Some(paths) = row
            .category_paths
            .as_deref()
            .and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok())
        {
            // Every assigned category, not the primary alone.
            for path in paths {
                *categories.entry(path).or_insert(0) += 1;
            }
        }
        if let Some(kind) = &row.sku_type {
            *sku_types.entry(kind.clone()).or_insert(0) += 1;
        }
        if let Some(tier) = &row.plan_tier_label {
            *tiers.entry(tier.clone()).or_insert(0) += 1;
        }
        if let Some(flag) = row.sellable {
            *sellable.entry(flag.to_string()).or_insert(0) += 1;
        }
        if let Some(unit) = &row.metering_unit {
            *units.entry(unit.clone()).or_insert(0) += 1;
        }
    }
    FacetsView {
        categories: buckets(categories),
        sku_types: buckets(sku_types),
        tiers: buckets(tiers),
        sellable: buckets(sellable),
        units: buckets(units),
    }
}

fn row_view(row: crate::infra::storage::entity::read_entity::Model) -> BrowseRowView {
    BrowseRowView {
        entity_kind: row.entity_kind,
        entity_id: row.entity_id,
        entity_code: row.entity_code,
        name: row.name,
        lifecycle_state: row.lifecycle_state,
        deprecated: row.deprecated,
        composition_pending: row.composition_pending,
        sellable: row.sellable,
        deprecation_provenance: row.deprecation_provenance,
        replaced_by_sku_id: row.replaced_by_sku_id,
        region_scope: row.region_scope,
        brand_scope: row.brand_scope,
        sku_type: row.sku_type,
        plan_tier_label: row.plan_tier_label,
        metering_unit: row.metering_unit,
        display_attributes: row.display_attributes,
        category_paths: row.category_paths,
        published_version: row.published_version,
    }
}

/// `GET /bss-products/v1/browse` — the browse door.
async fn browse(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    axum::extract::Query(params): axum::extract::Query<BrowseParams>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let started = Instant::now();
    let tenant_id = ctx.subject_tenant_id();
    if let Err(retry) = ReadPathLimiter::global().try_acquire(tenant_id) {
        return Ok(shed(tenant_id, retry));
    }
    let kind = params
        .kind
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty());
    if let Some(kind) = kind
        && !matches!(kind, "product" | "sku")
    {
        let mut report = crate::domain::validation::ValidationReport::new();
        report.violate("VALIDATION", "kind", "kind must be product or sku");
        return Err(CanonicalError::from(DomainError::Validation(report)));
    }
    // `product|sku × read`: both grants when both kinds are browsed.
    let scope = if kind == Some("sku") {
        read_scope(
            &enforcer,
            &ctx,
            &crate::authz::resource_types::SKU,
            tenant_id,
        )
        .await?
    } else {
        let scope = read_scope(
            &enforcer,
            &ctx,
            &crate::authz::resource_types::PRODUCT,
            tenant_id,
        )
        .await?;
        if kind.is_none() {
            read_scope(
                &enforcer,
                &ctx,
                &crate::authz::resource_types::SKU,
                tenant_id,
            )
            .await?;
        }
        scope
    };
    let now = crate::domain::canonical::write_instant(Utc::now());
    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&crate::infra::storage::RepoError::Db(e.to_string()))
    })?;
    let stamp = stamp_of(&conn, &scope, tenant_id, now).await?;
    let (_, generation) = repo::load_read_checkpoint(&conn, &scope, tenant_id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?
        .unwrap_or((0, 0));
    let surface = match params.exclude_deprecated {
        Some(exclude) => ReadSurface::FilteredBrowse {
            exclude_deprecated: exclude,
        },
        None => ReadSurface::DefaultBrowse,
    };
    let want_facets = params.include_facets.unwrap_or(false);
    let limit = params.limit.unwrap_or(50).clamp(1, BROWSE_LIMIT_MAX);
    let query = BrowseQuery {
        visibility: Some(VisibilityFilter::for_surface(surface).condition()),
        entity_kind: kind.map(str::to_owned),
        category_path: params.category.filter(|c| !c.trim().is_empty()),
        sku_type: params.sku_type,
        plan_tier_label: params.tier,
        sellable: params.sellable,
        metering_unit: params.unit,
        brand_claim: params.brand,
        region_claim: params.region,
        name_prefix: params.q.filter(|q| !q.trim().is_empty()),
        generation,
        limit: if want_facets { BROWSE_LIMIT_MAX } else { limit },
    };
    let rows = repo::browse_read_entities(&conn, &scope, tenant_id, &query)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?;
    let facets = want_facets.then(|| facets_of(&rows));
    let rows: Vec<BrowseRowView> = rows
        .into_iter()
        .take(usize::try_from(limit).unwrap_or(usize::MAX))
        .map(row_view)
        .collect();
    observe_edge("browse", tenant_id, started);
    Ok((
        StatusCode::OK,
        Json(BrowseView {
            stamp,
            rows,
            facets,
        }),
    )
        .into_response())
}

// ---------------------------------------------------------------------------
// The history timeline (`inst-rh-timeline`)
// ---------------------------------------------------------------------------

/// One frozen version on the timeline.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct VersionEntryView {
    pub published_version: i64,
    pub published_at: DateTime<Utc>,
    /// The record that authorized the publish, when one did.
    pub approval_ref: Option<Uuid>,
    /// The actor's pseudonymous reference (P-D-117: never resolved here).
    pub actor_pseudonym: Uuid,
    /// The content keys whose values differ from the previous frozen version
    /// (every key on the first).
    pub changed_keys: Vec<String>,
}

/// The timeline answer.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct HistoryView {
    pub stamp: StampView,
    pub entity_kind: String,
    pub entity_id: Uuid,
    pub lifecycle_state: String,
    pub versions: Vec<VersionEntryView>,
}

fn changed_keys(previous: Option<&serde_json::Value>, current: &serde_json::Value) -> Vec<String> {
    let Some(map) = current.as_object() else {
        return Vec::new();
    };
    let mut keys: Vec<String> = map
        .iter()
        .filter(|(key, value)| previous.and_then(|p| p.get(*key)) != Some(*value))
        .map(|(key, _)| key.clone())
        .collect();
    keys.sort();
    keys
}

async fn history(
    state: &ApiState,
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
    entity_kind: &str,
    entity_id: Uuid,
) -> Result<Response, CanonicalError> {
    let started = Instant::now();
    let tenant_id = ctx.subject_tenant_id();
    if let Err(retry) = ReadPathLimiter::global().try_acquire(tenant_id) {
        return Ok(shed(tenant_id, retry));
    }
    let (resource, versioned) = if entity_kind == "sku" {
        (
            &crate::authz::resource_types::SKU,
            repo::VersionedEntityKind::Sku,
        )
    } else {
        (
            &crate::authz::resource_types::PRODUCT,
            repo::VersionedEntityKind::Product,
        )
    };
    let scope = read_scope(enforcer, ctx, resource, tenant_id).await?;
    let now = crate::domain::canonical::write_instant(Utc::now());
    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&crate::infra::storage::RepoError::Db(e.to_string()))
    })?;
    let stamp = stamp_of(&conn, &scope, tenant_id, now).await?;
    let lifecycle_state = if entity_kind == "sku" {
        repo::find_sku(&conn, &scope, tenant_id, entity_id)
            .await
            .map_err(|e| repo_error_to_canonical(&e))?
            .map(|head| head.lifecycle_state)
    } else {
        repo::find_product(&conn, &scope, tenant_id, entity_id)
            .await
            .map_err(|e| repo_error_to_canonical(&e))?
            .map(|head| head.lifecycle_state)
    };
    // The history surface serves `retired` (the C2 carve-out) and refuses
    // `draft` and `discarded` like every read; a missing head is the miss.
    let Some(lifecycle_state) = lifecycle_state else {
        return Err(
            ReadResource::not_found("no entity matches this id in the caller's scope")
                .with_resource(entity_id.to_string())
                .create(),
        );
    };
    if !crate::domain::read_model::serves(lifecycle_state, ReadSurface::History) {
        return Err(
            ReadResource::not_found("no entity matches this id in the caller's scope")
                .with_resource(entity_id.to_string())
                .create(),
        );
    }
    let frozen = repo::entity_versions_of(&conn, &scope, tenant_id, versioned, entity_id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?;
    let mut previous: Option<serde_json::Value> = None;
    let mut versions = Vec::with_capacity(frozen.len());
    for row in frozen {
        let current: serde_json::Value = serde_json::from_str(&row.content).unwrap_or_default();
        versions.push(VersionEntryView {
            published_version: row.published_version,
            published_at: row.published_at,
            approval_ref: row.approval_ref,
            actor_pseudonym: row.actor_ref,
            changed_keys: changed_keys(previous.as_ref(), &current),
        });
        previous = Some(current);
    }
    observe_edge("history", tenant_id, started);
    Ok((
        StatusCode::OK,
        Json(HistoryView {
            stamp,
            entity_kind: entity_kind.to_owned(),
            entity_id,
            lifecycle_state: lifecycle_state.as_str().to_owned(),
            versions,
        }),
    )
        .into_response())
}

/// `GET /bss-products/v1/products/{id}/versions`.
///
/// @cpt-dod:cpt-cf-bss-products-dod-history-timeline:p2
async fn product_history(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    history(&state, &enforcer, &ctx, "product", id).await
}

/// `GET /bss-products/v1/skus/{id}/versions`.
async fn sku_history(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    history(&state, &enforcer, &ctx, "sku", id).await
}

// ---------------------------------------------------------------------------
// The dashboards (`inst-ps-dashboards`, P-D-126 row 10)
// ---------------------------------------------------------------------------

/// One deferred retirement intent as the dashboard shows it.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct DeferredIntentView {
    pub product_id: Uuid,
    pub cascade_ref: Uuid,
    pub children_count: i32,
    pub created_at: DateTime<Utc>,
    pub age_secs: i64,
    pub polled_at: DateTime<Utc>,
}

/// The deferred-intent dashboard.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct DeferredIntentsView {
    pub stamp: StampView,
    pub items: Vec<DeferredIntentView>,
}

/// One version's freeze status.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct FreezeStatusView {
    pub catalog_version_id: i64,
    pub freeze_state: String,
    pub pending: i32,
    pub acked: i32,
    pub released: i32,
    pub forced: i32,
    pub published_at: DateTime<Utc>,
    pub polled_at: DateTime<Utc>,
}

/// The freeze-status dashboard.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct FreezeStatusesView {
    pub stamp: StampView,
    pub items: Vec<FreezeStatusView>,
}

/// The delivery-state dashboard: the projector's own health.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct DeliveryStateView {
    pub stamp: StampView,
    /// Inbox rows above the checkpoint.
    pub inbox_pending: i64,
    /// Poison rows parked and not released.
    pub parked: i64,
    pub oldest_pending_age_secs: i64,
    /// `None` before the first poll.
    pub polled_at: Option<DateTime<Utc>>,
}

/// `GET /bss-products/v1/read/deferred-intents` on `scheduled_transition × read`
/// (04's own grant, P-D-126 row 10).
///
/// @cpt-dod:cpt-cf-bss-products-dod-dashboards:p1
async fn deferred_intents(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let started = Instant::now();
    let tenant_id = ctx.subject_tenant_id();
    if let Err(retry) = ReadPathLimiter::global().try_acquire(tenant_id) {
        return Ok(shed(tenant_id, retry));
    }
    let scope = read_scope(
        &enforcer,
        &ctx,
        &crate::authz::resource_types::SCHEDULED_TRANSITION,
        tenant_id,
    )
    .await?;
    let now = crate::domain::canonical::write_instant(Utc::now());
    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&crate::infra::storage::RepoError::Db(e.to_string()))
    })?;
    let stamp = stamp_of(&conn, &scope, tenant_id, now).await?;
    let items = repo::read_deferred_intents(&conn, &scope, tenant_id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?
        .into_iter()
        .map(|row| DeferredIntentView {
            product_id: row.product_id,
            cascade_ref: row.cascade_ref,
            children_count: row.children_count,
            created_at: row.created_at,
            age_secs: row.age_secs,
            polled_at: row.polled_at,
        })
        .collect();
    observe_edge("deferred-intents", tenant_id, started);
    Ok((StatusCode::OK, Json(DeferredIntentsView { stamp, items })).into_response())
}

/// `GET /bss-products/v1/read/freeze-status` on `catalog_version × read`.
async fn freeze_status(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let started = Instant::now();
    let tenant_id = ctx.subject_tenant_id();
    if let Err(retry) = ReadPathLimiter::global().try_acquire(tenant_id) {
        return Ok(shed(tenant_id, retry));
    }
    let scope = read_scope(
        &enforcer,
        &ctx,
        &crate::authz::resource_types::CATALOG_VERSION,
        tenant_id,
    )
    .await?;
    let now = crate::domain::canonical::write_instant(Utc::now());
    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&crate::infra::storage::RepoError::Db(e.to_string()))
    })?;
    let stamp = stamp_of(&conn, &scope, tenant_id, now).await?;
    let items = repo::read_freeze_statuses(&conn, &scope, tenant_id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?
        .into_iter()
        .map(|row| FreezeStatusView {
            catalog_version_id: row.catalog_version_id,
            freeze_state: row.freeze_state,
            pending: row.pending,
            acked: row.acked,
            released: row.released,
            forced: row.forced,
            published_at: row.published_at,
            polled_at: row.polled_at,
        })
        .collect();
    observe_edge("freeze-status", tenant_id, started);
    Ok((StatusCode::OK, Json(FreezeStatusesView { stamp, items })).into_response())
}

/// `GET /bss-products/v1/read/delivery-state` on `audit × read` — the
/// projector's health is operator-facing evidence.
async fn delivery_state(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let started = Instant::now();
    let tenant_id = ctx.subject_tenant_id();
    if let Err(retry) = ReadPathLimiter::global().try_acquire(tenant_id) {
        return Ok(shed(tenant_id, retry));
    }
    let scope = read_scope(
        &enforcer,
        &ctx,
        &crate::authz::resource_types::AUDIT,
        tenant_id,
    )
    .await?;
    let now = crate::domain::canonical::write_instant(Utc::now());
    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&crate::infra::storage::RepoError::Db(e.to_string()))
    })?;
    let stamp = stamp_of(&conn, &scope, tenant_id, now).await?;
    let row = repo::read_delivery_state(&conn, &scope, tenant_id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?;
    let view = match row {
        Some(row) => DeliveryStateView {
            stamp,
            inbox_pending: row.inbox_pending,
            parked: row.parked,
            oldest_pending_age_secs: row.oldest_pending_age_secs,
            polled_at: Some(row.polled_at),
        },
        None => DeliveryStateView {
            stamp,
            inbox_pending: 0,
            parked: 0,
            oldest_pending_age_secs: 0,
            polled_at: None,
        },
    };
    observe_edge("delivery-state", tenant_id, started);
    Ok((StatusCode::OK, Json(view)).into_response())
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

/// The read surface's six doors.
pub(crate) fn router(state: Arc<ApiState>, openapi: &dyn OpenApiRegistry) -> Router {
    let router = OperationBuilder::get("/bss-products/v1/browse")
        .operation_id("bss_products.browse")
        .summary("Browse the projected catalog")
        .description(
            "Serves the read projection: published and deprecated rows (deprecated ones flagged, \
             `excludeDeprecated=true` drops them), never drafts, discards or retired heads; \
             scope and visibility are built into the query; `includeFacets=true` adds facets \
             over category paths (every assigned category), type, tier, sellable and unit. \
             Every answer carries the StalenessStamp. Gates on `product x read` and `sku x \
             read`; above the tenant's ceiling answers 503 READ_MODEL_OVERLOADED with \
             Retry-After.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .query_param("kind", false, "product or sku; both when absent.")
        .query_param("q", false, "A name prefix.")
        .query_param(
            "category",
            false,
            "A category path (any assigned category).",
        )
        .query_param("skuType", false, "A SKU type.")
        .query_param("tier", false, "A plan tier label.")
        .query_param("sellable", false, "true or false.")
        .query_param("unit", false, "A metering unit.")
        .query_param("excludeDeprecated", false, "Drop deprecated rows.")
        .query_param("brand", false, "A brand claim value.")
        .query_param("region", false, "A region claim value.")
        .query_param("includeFacets", false, "Add the facets.")
        .query_param("limit", false, "Rows per answer, at most 500.")
        .handler(browse)
        .json_response_with_schema::<BrowseView>(
            openapi,
            StatusCode::OK,
            "The rows, facets and stamp.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(Router::new(), openapi);
    let router = OperationBuilder::get("/bss-products/v1/products/{id}/versions")
        .operation_id("bss_products.product_history")
        .summary("A product's version history")
        .description(
            "The frozen versions of a product, oldest first, each with the keys that changed, \
             the authorizing record and the actor's pseudonym; a retired product is reachable \
             here. Gates on `product x read`; behind the read limiter; carries the stamp.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("id", "The product.")
        .handler(product_history)
        .json_response_with_schema::<HistoryView>(openapi, StatusCode::OK, "The timeline.")
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);
    let router = OperationBuilder::get("/bss-products/v1/skus/{id}/versions")
        .operation_id("bss_products.sku_history")
        .summary("A SKU's version history")
        .description("The SKU twin of the product timeline. Gates on `sku x read`.")
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("id", "The SKU.")
        .handler(sku_history)
        .json_response_with_schema::<HistoryView>(openapi, StatusCode::OK, "The timeline.")
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);
    let router = OperationBuilder::get("/bss-products/v1/read/deferred-intents")
        .operation_id("bss_products.read_deferred_intents")
        .summary("The deferred-intent dashboard")
        .description(
            "Polled from 04's deferred-retirement table (P-D-126 row 10); each row carries its \
             poll instant. Gates on `scheduled_transition x read`; behind the read limiter.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .handler(deferred_intents)
        .json_response_with_schema::<DeferredIntentsView>(openapi, StatusCode::OK, "The dashboard.")
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);
    let router = OperationBuilder::get("/bss-products/v1/read/freeze-status")
        .operation_id("bss_products.read_freeze_status")
        .summary("The freeze-status dashboard")
        .description(
            "Polled from 06's freeze ledger: per version, the participant counts by state. \
             Gates on `catalog_version x read`; behind the read limiter.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .handler(freeze_status)
        .json_response_with_schema::<FreezeStatusesView>(openapi, StatusCode::OK, "The dashboard.")
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);
    OperationBuilder::get("/bss-products/v1/read/delivery-state")
        .operation_id("bss_products.read_delivery_state")
        .summary("The delivery-state dashboard")
        .description(
            "Polled from the projector's inbox and poison park: rows pending above the \
             checkpoint, rows parked, the oldest pending age. Gates on `audit x read`; behind \
             the read limiter.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .handler(delivery_state)
        .json_response_with_schema::<DeliveryStateView>(openapi, StatusCode::OK, "The dashboard.")
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi)
        .layer(Extension(state))
}

#[cfg(test)]
#[path = "read_tests.rs"]
mod read_tests;
