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
use toolkit::api::odata::OData;
use toolkit::api::operation_builder::{OperationBuilder, OperationBuilderODataExt};
use toolkit_db::secure::AccessScope;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::api::rest::odata as odata_seam;
use crate::api::rest::odata::reject_undeclared_query_params;
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

/// The bucket count past which [`ReadPathLimiter::try_acquire`] drops the
/// idle buckets before inserting. A bucket idle for a full second is back at
/// capacity — the refill rate equals the capacity — which is exactly what an
/// absent bucket means, so the eviction is lossless and the map is bounded by
/// the tenants active in the last second rather than every tenant ever seen.
const LIMITER_BUCKET_HIGH_WATER: usize = 4_096;

/// How long a bucket must sit untouched before it is provably full again.
const LIMITER_BUCKET_IDLE: std::time::Duration = std::time::Duration::from_secs(1);

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
    /// without waiting on the default's two hundred per second. Compiled for
    /// the probes only: no door and no boot path sets a ceiling per tenant.
    #[cfg(test)]
    pub(crate) fn set_ceiling_for(&self, tenant_id: Uuid, ceiling: u32) {
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
        if buckets.len() >= LIMITER_BUCKET_HIGH_WATER {
            buckets
                .retain(|_, bucket| now.duration_since(bucket.refilled_at) < LIMITER_BUCKET_IDLE);
        }
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
fn observe_edge(door: &'static str, ctx: &SecurityContext, started: Instant) {
    let tenant_id = ctx.subject_tenant_id();
    // An elevated read names its session on the line (P-D-163): the meter
    // is the one place every served read passes, so a break-glass read is
    // visible in the stream and not only in the gate's audit row.
    let breakglass_session = crate::api::rest::breakglass_session_of(ctx);
    tracing::info!(
        event = "read_edge_latency",
        %tenant_id,
        door,
        breakglass_session = ?breakglass_session,
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

/// The browse door's **own** query operands — everything the caller says
/// that is not the `OData` family (P-D-165).
///
/// Seven parameters went away when the door adopted the platform's query
/// contract, because each of them was a column comparison spelled by hand:
/// `?q=` is `$filter=startswith(name,'...')`, `?category=` is
/// `contains(category_paths,'...')`, and `?skuType=` / `?tier=` /
/// `?sellable=` / `?unit=` are `eq` on their own fields. The four that
/// remain are the ones that are not comparisons — see
/// [`repo::BrowseRowQuery`] for why each cannot be a filter field.
///
/// `limit` and `cursor` are bound by the `OData` extractor (which folds them
/// onto `$top` and `$skiptoken`) and so do not appear here, but they are
/// still the spellings this door has always accepted.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowseParams {
    /// `product` or `sku`; both when absent. An **authorization** operand:
    /// it decides which grants the door requires.
    kind: Option<String>,
    /// Whether deprecated rows are served at all — a choice of visibility
    /// surface, not a row predicate. `$filter=deprecated eq false` narrows
    /// within the surface; this selects it.
    exclude_deprecated: Option<bool>,
    /// A brand claim. Set membership over a comma-joined token set where
    /// empty means unrestricted, not equality.
    brand: Option<String>,
    /// A region claim, on the same footing as `brand`.
    region: Option<String>,
    /// Add the facets over the served set.
    include_facets: Option<bool>,
}

/// The non-`OData` keys this door declares, for the guard that refuses every
/// other one. Kept beside [`BrowseParams`] so a field added there without a
/// spelling added here is visible in one screen.
const BROWSE_PARAMS: [&str; 5] = [
    "kind",
    "excludeDeprecated",
    "brand",
    "region",
    "includeFacets",
];

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

/// The facets over the matching set (`inst-rb-facets`).
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct FacetsView {
    pub categories: Vec<FacetBucketView>,
    pub sku_types: Vec<FacetBucketView>,
    pub tiers: Vec<FacetBucketView>,
    pub sellable: Vec<FacetBucketView>,
    pub units: Vec<FacetBucketView>,
    /// Whether the counts cover the **whole** matching set or only the first
    /// [`BROWSE_FACET_WINDOW`] rows of it.
    ///
    /// Facets are counted in application code because a category facet has
    /// to split a row's stored path set, which no `GROUP BY` over this
    /// column can do — so the count is over a bounded window and past it it
    /// is a lower bound. Saying which of the two a number is costs one field;
    /// not saying it is how the counts were wrong without anyone noticing.
    pub complete: bool,
}

/// The browse answer: rows, facets when asked, the stamp always, and the
/// page this is one of.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct BrowseView {
    pub stamp: StampView,
    pub rows: Vec<BrowseRowView>,
    pub facets: Option<FacetsView>,
    /// `next_cursor`, `prev_cursor` and the `limit` this page was served at
    /// — [`toolkit_odata::PageInfo`], the same envelope every paginated read
    /// on the platform answers with.
    ///
    /// The family keeps its own view rather than answering a bare
    /// `Page<BrowseRowView>` because the response also carries the staleness
    /// stamp and the facets, which `Page<T>` has nowhere to put. The sibling
    /// pricing gear's customer-group listing keeps its own view for the same
    /// reason.
    ///
    /// `next_cursor` is `null` on the last page and **only** there: it is
    /// the answer to "is there more", which is what the door could not say
    /// before this envelope — the ceiling was the whole result set.
    pub page_info: toolkit_odata::PageInfo,
}

fn buckets(counts: BTreeMap<String, u64>) -> Vec<FacetBucketView> {
    counts
        .into_iter()
        .map(|(value, count)| FacetBucketView { value, count })
        .collect()
}

/// The timeline declares no operand of its own: it is addressed by path and
/// paged by the platform's own two spellings.
const TIMELINE_PARAMS: [&str; 0] = [];

/// Neither dashboard declares an operand of its own: both are whole-tenant
/// worklists, narrowed with `$filter` and paged with the platform's two
/// spellings.
const DEFERRED_INTENT_PARAMS: [&str; 0] = [];

/// See [`DEFERRED_INTENT_PARAMS`].
const FREEZE_STATUS_PARAMS: [&str; 0] = [];

/// Why `$filter` is refused here rather than ignored.
const TIMELINE_NO_FILTER: &str = "the version timeline is not a filterable surface: each entry's \
     `changed_keys` is the diff against the version before it, so removing \
     intermediate versions would silently change what `changed` means. Page \
     it with `$top`/`limit` and `$skiptoken`/`cursor`.";

/// Why `$orderby` is refused here rather than ignored.
const TIMELINE_NO_ORDERBY: &str = "the version timeline is served oldest-version-first and in no \
     other order, because each entry's `changed_keys` is the diff against \
     the version before it.";

/// Why `$select` is refused here rather than ignored.
const TIMELINE_NO_SELECT: &str =
    "this door does not project fields; every entry carries its whole shape.";

/// The most rows the facet pass counts over.
///
/// This was the browse door's page ceiling before the door was paginated,
/// when one number had to serve as both — which is why the facet counts were
/// silently a lower bound past 500 matches. The page size is now
/// [`odata_seam::LISTING_LIMIT_CFG`]'s and this is only the facet window,
/// with [`FacetsView::complete`] saying when it was not enough.
pub const BROWSE_FACET_WINDOW: u64 = 500;

fn facets_of(
    rows: &[crate::infra::storage::entity::read_entity::Model],
    complete: bool,
) -> FacetsView {
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
        complete,
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
///
/// `Query<HashMap<..>>` rides beside the two typed extractors on purpose: it
/// is the only way to see the keys **nothing** claimed, which is what
/// [`reject_undeclared_query_params`] refuses. Axum drops an unclaimed key
/// silently, and a dropped filter reads as a correct unfiltered answer.
async fn browse(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    axum::extract::Query(raw): axum::extract::Query<HashMap<String, String>>,
    axum::extract::Query(params): axum::extract::Query<BrowseParams>,
    OData(odata): OData,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    reject_undeclared_query_params(&raw, &BROWSE_PARAMS)?;
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
    let query = BrowseQuery {
        visibility: Some(repo::visibility_condition(VisibilityFilter::for_surface(
            surface,
        ))),
        entity_kind: kind.map(str::to_owned),
        brand_claim: params.brand,
        region_claim: params.region,
        generation,
    };
    let page = repo::browse_read_entities_page(
        &conn,
        &scope,
        tenant_id,
        &query,
        &odata,
        odata_seam::LISTING_LIMIT_CFG,
    )
    .await
    .map_err(|e| odata_seam::odata_error_to_canonical("browse", &e))?;

    // The facet pass runs over its own window of the matching set, not over
    // this page: see `repo::browse_facet_rows`. Skipped entirely when the
    // caller did not ask, so the ordinary browse is still one statement.
    let facets = if want_facets {
        let (rows, complete) = repo::browse_facet_rows(
            &conn,
            &scope,
            tenant_id,
            &query,
            &odata,
            BROWSE_FACET_WINDOW,
        )
        .await
        .map_err(|e| odata_seam::odata_error_to_canonical("browse facets", &e))?;
        Some(facets_of(&rows, complete))
    } else {
        None
    };

    let rows: Vec<BrowseRowView> = page.items.into_iter().map(row_view).collect();
    observe_edge("browse", &ctx, started);
    Ok((
        StatusCode::OK,
        Json(BrowseView {
            stamp,
            rows,
            facets,
            page_info: page.page_info,
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
    /// Where this entity was cloned from, when it was (`dod-clone-lineage`,
    /// P-D-152): the immediate source and the frozen version read — `None`
    /// for a version when the source was a draft read from its head.
    pub lineage: Option<LineageView>,
    /// The entities cloned **from** this one — the reverse lookup `design/11`
    /// §2 promised; drafts included, since a clone is born a draft.
    pub clones: Vec<CloneRefView>,
    /// `next_cursor`, `prev_cursor` and the `limit` this page of `versions`
    /// was served at.
    ///
    /// Only `versions` is paged. `lineage` and `clones` are properties of
    /// the entity rather than of the page, so they are answered whole on
    /// every page — a caller assembling a timeline from several pages gets
    /// the same lineage each time rather than a fragment of it.
    ///
    /// Before P-D-165 this door answered an entity's **entire** publish
    /// history in one body, which for a long-lived SKU is unbounded in the
    /// only dimension that matters here: how many times it was published.
    pub page_info: toolkit_odata::PageInfo,
}

/// The forward lineage pointer.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct LineageView {
    /// The immediate source, one step back (P-D-72).
    pub cloned_from: Uuid,
    /// The source version the clone read; `None` for a draft's head.
    pub cloned_from_version: Option<i64>,
}

/// One clone of the entity.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct CloneRefView {
    /// The clone's id.
    pub entity_id: Uuid,
    /// The version of this entity it read; `None` for a head read.
    pub cloned_from_version: Option<i64>,
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
    raw: &HashMap<String, String>,
    odata: &toolkit_odata::ODataQuery,
) -> Result<Response, CanonicalError> {
    reject_undeclared_query_params(raw, &TIMELINE_PARAMS)?;
    odata_seam::reject_unsupported_odata_options(
        odata,
        Some(TIMELINE_NO_FILTER),
        Some(TIMELINE_NO_ORDERBY),
        Some(TIMELINE_NO_SELECT),
    )?;
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
    let head = if entity_kind == "sku" {
        repo::find_sku(&conn, &scope, tenant_id, entity_id)
            .await
            .map_err(|e| repo_error_to_canonical(&e))?
            .map(|head| {
                (
                    head.lifecycle_state,
                    head.cloned_from,
                    head.cloned_from_version,
                )
            })
    } else {
        repo::find_product(&conn, &scope, tenant_id, entity_id)
            .await
            .map_err(|e| repo_error_to_canonical(&e))?
            .map(|head| {
                (
                    head.lifecycle_state,
                    head.cloned_from,
                    head.cloned_from_version,
                )
            })
    };
    // The history surface serves `retired` (the C2 carve-out) and refuses
    // `draft` and `discarded` like every read; a missing head is the miss.
    let Some((lifecycle_state, cloned_from, cloned_from_version)) = head else {
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
    let (page, predecessor) = repo::entity_versions_page(
        &conn,
        &scope,
        tenant_id,
        versioned,
        entity_id,
        odata,
        odata_seam::LISTING_LIMIT_CFG,
    )
    .await
    .map_err(|e| odata_seam::odata_error_to_canonical("version timeline", &e))?;
    // The diff is against the previous **stored** version, so a page that
    // does not start at version one is seeded with the row before it. A page
    // that does start there gets `None`, which is what makes the first
    // entry's `changed_keys` "every key".
    let mut previous: Option<serde_json::Value> = predecessor
        .as_deref()
        .map(|content| serde_json::from_str(content).unwrap_or_default());
    let page_info = page.page_info;
    let mut versions = Vec::with_capacity(page.items.len());
    for row in page.items {
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
    let clones = repo::clones_of(&conn, &scope, tenant_id, versioned, entity_id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?
        .into_iter()
        .map(|c| CloneRefView {
            entity_id: c.entity_id,
            cloned_from_version: c.cloned_from_version,
        })
        .collect();
    observe_edge("history", ctx, started);
    Ok((
        StatusCode::OK,
        Json(HistoryView {
            stamp,
            entity_kind: entity_kind.to_owned(),
            entity_id,
            lifecycle_state: lifecycle_state.as_str().to_owned(),
            versions,
            lineage: cloned_from.map(|source| LineageView {
                cloned_from: source,
                cloned_from_version,
            }),
            clones,
            page_info,
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
    axum::extract::Query(raw): axum::extract::Query<HashMap<String, String>>,
    OData(odata): OData,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    history(&state, &enforcer, &ctx, "product", id, &raw, &odata).await
}

/// `GET /bss-products/v1/skus/{id}/versions`.
async fn sku_history(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    axum::extract::Query(raw): axum::extract::Query<HashMap<String, String>>,
    OData(odata): OData,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    history(&state, &enforcer, &ctx, "sku", id, &raw, &odata).await
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
    /// `next_cursor`, `prev_cursor` and the `limit` this page was served at.
    ///
    /// Before P-D-165 this dashboard answered every row a tenant had, with
    /// no bound of any kind — and the studio polls it every 30 seconds.
    pub page_info: toolkit_odata::PageInfo,
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
    /// `next_cursor`, `prev_cursor` and the `limit` this page was served at.
    ///
    /// Before P-D-165 this dashboard answered every row a tenant had, with
    /// no bound of any kind — and the studio polls it every 30 seconds.
    pub page_info: toolkit_odata::PageInfo,
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
    axum::extract::Query(raw): axum::extract::Query<HashMap<String, String>>,
    OData(odata): OData,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    reject_undeclared_query_params(&raw, &DEFERRED_INTENT_PARAMS)?;
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
    let page = repo::read_deferred_intents(
        &conn,
        &scope,
        tenant_id,
        &odata,
        odata_seam::LISTING_LIMIT_CFG,
    )
    .await
    .map_err(|e| odata_seam::odata_error_to_canonical("deferred intents", &e))?;
    let page_info = page.page_info;
    let items = page
        .items
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
    observe_edge("deferred-intents", &ctx, started);
    Ok((
        StatusCode::OK,
        Json(DeferredIntentsView {
            stamp,
            items,
            page_info,
        }),
    )
        .into_response())
}

/// `GET /bss-products/v1/read/freeze-status` on `catalog_version × read`.
async fn freeze_status(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    axum::extract::Query(raw): axum::extract::Query<HashMap<String, String>>,
    OData(odata): OData,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    reject_undeclared_query_params(&raw, &FREEZE_STATUS_PARAMS)?;
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
    let page = repo::read_freeze_statuses(
        &conn,
        &scope,
        tenant_id,
        &odata,
        odata_seam::LISTING_LIMIT_CFG,
    )
    .await
    .map_err(|e| odata_seam::odata_error_to_canonical("freeze status", &e))?;
    let page_info = page.page_info;
    let items = page
        .items
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
    observe_edge("freeze-status", &ctx, started);
    Ok((
        StatusCode::OK,
        Json(FreezeStatusesView {
            stamp,
            items,
            page_info,
        }),
    )
        .into_response())
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
    observe_edge("delivery-state", &ctx, started);
    Ok((StatusCode::OK, Json(view)).into_response())
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

/// The read surface's six doors.
#[allow(
    clippy::too_many_lines,
    reason = "one registration per door, and each door now declares its paging \
              and filter surface as well as its own operands (P-D-165). \
              Splitting the function would put a door's route and its \
              parameter documentation in two places, which is exactly the \
              drift the declarations exist to prevent"
)]
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
        .query_param(
            "kind",
            false,
            "product or sku; both when absent. An authorization operand: naming one \
             kind narrows the grants the door requires to that kind's, which is why it \
             is not a $filter field.",
        )
        .query_param(
            "excludeDeprecated",
            false,
            "Choose the visibility surface: whether deprecated rows are served at all. \
             `$filter=deprecated eq false` narrows within a surface; this selects one.",
        )
        .query_param(
            "brand",
            false,
            "A brand claim. Set membership over a token set where empty means \
             unrestricted - not equality, which is why it is not a $filter field.",
        )
        .query_param(
            "region",
            false,
            "A region claim, on the same footing as brand.",
        )
        .query_param(
            "includeFacets",
            false,
            "Add the facets over the matching set. `facets.complete` says whether the \
             counts cover the whole set or only its first 500 rows.",
        )
        .query_param_typed(
            "limit",
            false,
            "Rows per page; default 50, at most 200. Also spelled $top.",
            "integer",
        )
        .query_param(
            "cursor",
            false,
            "The previous page's `page_info.next_cursor`, opaque. Also spelled \
             $skiptoken. A caller MUST NOT change $filter or $orderby between \
             continuation requests carrying the same cursor.",
        )
        .with_odata_filter::<repo::BrowseFilterField>()
        .with_odata_orderby::<repo::BrowseFilterField>()
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
        .query_param_typed(
            "limit",
            false,
            "Versions per page, oldest first; default 50, at most 200. Also spelled \\
             $top. $filter, $orderby and $select are refused: each entry's \\
             `changedKeys` is the diff against the version before it, so the order is \\
             the version order and nothing else.",
            "integer",
        )
        .query_param(
            "cursor",
            false,
            "The previous page's `page_info.next_cursor`, opaque. Also spelled \\
             $skiptoken.",
        )
        .handler(product_history)
        .json_response_with_schema::<HistoryView>(
            openapi,
            StatusCode::OK,
            "One page of the timeline.",
        )
        .error_400(openapi)
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
        .query_param_typed(
            "limit",
            false,
            "Versions per page, oldest first; default 50, at most 200. Also spelled \\
             $top. $filter, $orderby and $select are refused: each entry's \\
             `changedKeys` is the diff against the version before it, so the order is \\
             the version order and nothing else.",
            "integer",
        )
        .query_param(
            "cursor",
            false,
            "The previous page's `page_info.next_cursor`, opaque. Also spelled \\
             $skiptoken.",
        )
        .handler(sku_history)
        .json_response_with_schema::<HistoryView>(
            openapi,
            StatusCode::OK,
            "One page of the timeline.",
        )
        .error_400(openapi)
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
        .query_param_typed(
            "limit",
            false,
            "Rows per page; default 50, at most 200. Also spelled $top.",
            "integer",
        )
        .query_param(
            "cursor",
            false,
            "The previous page's `page_info.next_cursor`, opaque. Also spelled \
             $skiptoken. A caller MUST NOT change $filter or $orderby between \
             continuation requests carrying the same cursor.",
        )
        .with_odata_filter::<repo::DeferredIntentFilterField>()
        .with_odata_orderby::<repo::DeferredIntentFilterField>()
        .handler(deferred_intents)
        .json_response_with_schema::<DeferredIntentsView>(
            openapi,
            StatusCode::OK,
            "One page of the dashboard.",
        )
        .error_400(openapi)
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
        .query_param_typed(
            "limit",
            false,
            "Rows per page; default 50, at most 200. Also spelled $top.",
            "integer",
        )
        .query_param(
            "cursor",
            false,
            "The previous page's `page_info.next_cursor`, opaque. Also spelled \
             $skiptoken. A caller MUST NOT change $filter or $orderby between \
             continuation requests carrying the same cursor.",
        )
        .with_odata_filter::<repo::FreezeStatusFilterField>()
        .with_odata_orderby::<repo::FreezeStatusFilterField>()
        .handler(freeze_status)
        .json_response_with_schema::<FreezeStatusesView>(
            openapi,
            StatusCode::OK,
            "One page of the dashboard.",
        )
        .error_400(openapi)
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
