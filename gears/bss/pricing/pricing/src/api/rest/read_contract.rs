//! The consumer read contract (D-419…D-422), mounted beside the authoring router: `GET /resolve`,
//! the chain matrix Rating and Subscriptions bind from, and `GET /prices/{id}`, the pinned price a
//! replay reads.
//!
//! A read writes nothing: no audit row, no idempotency key, no binding. `resolve` reads the stored
//! revision in ONE transaction (the revision, its plan and items, each entry with ALL its prices,
//! the dimension registry, the settings and the `keep_for_bound` ids); an unknown or another
//! tenant's revision is 404 there, before any Products read. The pure model then judges the pins
//! (`domain::resolve::matrix`), and only then, outside the transaction, is each SKU version read
//! as of the date through the detached registry, as the caller (D-421).
pub mod dto;
use super::authoring::{
    AuthoringState, configuration,
    support::{self, DoorError, authz_failure, require_authenticated},
};
use crate::{
    authz::{self, ResourceRef, actions, resource_types},
    domain::{
        book, dimension,
        plan::{RevisionState, Treatment},
        price::PriceState,
        price_book_entry::ChargeKind,
        resolve::{self, ItemResolution, Pin, ResolveContext, Resolved, TenantDefaults},
    },
    infra::{
        reference_registry, reference_work,
        storage::{
            RepoError,
            entity::{plan_revision, price},
            repo::{
                book_repo, dimension_repo, plan_item_repo, plan_repo, plan_revision_repo,
                price_book_entry_repo, price_repo,
            },
        },
    },
};
use authz_resolver_sdk::PolicyEnforcer;
use axum::{Extension, Router, extract::Path, http::StatusCode, response::Response};
use bss_products_sdk::models::SkuVersion;
use dto::{
    PricingPinnedPriceDto, PricingResolveBindingDto, PricingResolveChainDto, PricingResolveDto,
    PricingResolveInputDto, PricingResolveItemDto, PricingResolveMeterDto, PricingResolveQuery,
    PricingResolveSkuVersionDto,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use time::Date;
use toolkit::api::{OpenApiRegistry, operation_builder::OperationBuilder};
use toolkit_canonical_errors::CanonicalError;
use toolkit_db::secure::{AccessScope, DBRunner};
use toolkit_security::SecurityContext;
use uuid::Uuid;

/// Mount the consumer reads.
pub fn router(state: Arc<AuthoringState>, openapi: &dyn OpenApiRegistry) -> Router {
    let router = Router::new();
    let router = OperationBuilder::get("/bss-pricing/v1/resolve")
        .operation_id("bss_pricing.resolve")
        .summary("Resolve a plan revision on a date")
        .description(
            "Returns, per item of a published or superseded plan revision, the chain matrix \
             (the default chain and every dimension value) with the price bound on the date, \
             the SKU version in force and the resolved invoice inputs; pins renew a \
             subscription's bindings. No totals. Refusals: 400 DATE_INVALID, PIN_FOREIGN, \
             PIN_DUPLICATE, PINS_TOO_MANY; 404 for an unknown revision or item; 409 \
             REVISION_NOT_PUBLISHED; 503 when Products cannot answer.",
        )
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .query_param(
            "plan_revision_id",
            true,
            "A published or superseded plan revision",
        )
        .query_param("date", true, "The date resolved, YYYY-MM-DD")
        .query_param("item_id", false, "Resolve this one item of the revision")
        .query_param(
            "pins",
            false,
            "Comma-separated current bindings: price_id, or price_id:dim_value for a \
             default-chain price that value was bound to; at most 1000",
        )
        .handler(resolve)
        .json_response_with_schema::<PricingResolveDto>(openapi, StatusCode::OK, "Response")
        .standard_errors(openapi)
        .register(router, openapi);
    let router = OperationBuilder::get("/bss-pricing/v1/prices/{id}")
        .operation_id("bss_pricing.get_price")
        .summary("Read a pinned price")
        .description(
            "Returns an approved price of the tenant with its original money whatever its \
             window (closed, followed by a later price, kept for bound subscriptions), with its \
             entry's SKU, charge kind, period, book and currency: stored facts only. A draft, \
             pending, rejected, unknown or foreign price is one and the same 404.",
        )
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Price id")
        .handler(get_price)
        .json_response_with_schema::<PricingPinnedPriceDto>(openapi, StatusCode::OK, "Response")
        .standard_errors(openapi)
        .register(router, openapi);
    router.layer(Extension(state))
}

async fn resolve(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    uri: axum::http::Uri,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = authz::access_scope(
        &enforcer,
        &ctx,
        &resource_types::PLAN,
        actions::READ,
        None,
        None,
    )
    .await
    .map_err(authz_failure)?;
    let request = ResolveRequest::parse(&uri)?;
    resolution(&state, scope, &ctx, request).await
}

async fn get_price(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = authz::access_scope(
        &enforcer,
        &ctx,
        &resource_types::PRICE,
        actions::READ,
        None,
        Some(ResourceRef(id)),
    )
    .await
    .map_err(authz_failure)?;
    let tenant = ctx.subject_tenant_id();
    let body = support::transaction(&state.db.db(), move |tx| {
        let scope = scope.clone();
        Box::pin(async move { pinned_price(tx, &scope, tenant, id).await })
    })
    .await?;
    support::response(StatusCode::OK, &body, None)
}
/// `GET /prices/{id}` below its door (D-422): an approved price of the tenant, as stored.
/// # Errors
/// One and the same 404 for a draft, pending or rejected price, an unknown id and another
/// tenant's id.
async fn pinned_price(
    tx: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
) -> Result<PricingPinnedPriceDto, DoorError> {
    let row = price_repo::find(tx, scope, tenant, id)
        .await?
        .filter(|p| p.state == PriceState::Approved.as_str())
        .ok_or_else(|| support::missing_what("price"))?;
    let children = AccessScope::for_tenant(tenant);
    let entry = price_book_entry_repo::find(tx, &children, tenant, row.price_book_entry_id)
        .await?
        .ok_or_else(|| corrupt(format!("price {id} has no entry")))?;
    let book = book_repo::find(tx, &children, tenant, entry.book_id)
        .await?
        .ok_or_else(|| corrupt(format!("entry {} has no book", entry.id)))?;
    Ok(PricingPinnedPriceDto {
        price_id: row.id,
        price_book_entry_id: entry.id,
        sku_id: entry.sku_id,
        charge_kind: entry.charge_kind,
        period: entry.period,
        book_id: book.id,
        currency: book.currency,
        version_no: row.version_no,
        dim_value: row.dim_value,
        model: row.model,
        price: row.price_json,
        min_fee: row.min_fee,
        eligibility: row.eligibility,
        effective_from: row.effective_from.to_string(),
        effective_to: row.effective_to.map(|d| d.to_string()),
        temporary_until: row.temporary_until.map(|d| d.to_string()),
        keep_for_bound: row.keep_for_bound,
        closed_explicitly: row.closed_explicitly,
        paired_price_id: row.paired_price_id,
        return_of_price_id: row.return_of_price_id,
        approved_by_unit_id: row.approved_by_unit_id,
        approved_at: row.approved_at,
    })
}

/// A parsed `GET /resolve` query.
struct ResolveRequest {
    revision: Uuid,
    date: Date,
    item: Option<Uuid>,
    pins: Vec<Pin>,
}
impl ResolveRequest {
    /// # Errors
    /// 400 `QUERY_INVALID` for an unknown or repeated parameter and a malformed id; 400
    /// `DATE_INVALID` for a missing or malformed date; 400 `PIN_FOREIGN` for a pin that does not
    /// parse.
    fn parse(uri: &axum::http::Uri) -> Result<Self, CanonicalError> {
        let axum::extract::Query(query) =
            axum::extract::Query::<PricingResolveQuery>::try_from_uri(uri)
                .map_err(|_| support::invalid("query", "QUERY_INVALID"))?;
        let id = |field: &str, text: Option<&str>| {
            text.map(|t| Uuid::parse_str(t).map_err(|_| support::invalid(field, "QUERY_INVALID")))
                .transpose()
        };
        let revision = id("plan_revision_id", query.plan_revision_id.as_deref())?
            .ok_or_else(|| support::invalid("plan_revision_id", "QUERY_INVALID"))?;
        let date = support::date(Some(query.date.unwrap_or_default()), "date")?
            .ok_or_else(|| support::invalid("date", "DATE_INVALID"))?;
        let item = id("item_id", query.item_id.as_deref())?;
        let pins = match query.pins.as_deref() {
            None | Some("") => Vec::new(),
            Some(text) => text.split(',').map(pin).collect::<Result<_, _>>()?,
        };
        Ok(Self {
            revision,
            date,
            item,
            pins,
        })
    }
}
/// One pin: `price_id`, or `price_id:dim_value` with a value spelled as a dimension value is.
fn pin(text: &str) -> Result<Pin, CanonicalError> {
    let foreign = || support::invalid("pins", "PIN_FOREIGN");
    let (id, value) = match text.split_once(':') {
        Some((id, value)) if dimension::is_value(value) => (id, Some(value.to_owned())),
        Some(_) => return Err(foreign()),
        None => (text, None),
    };
    Ok(Pin {
        price_id: Uuid::parse_str(id).map_err(|_| foreign())?,
        dim_value: value,
    })
}

/// Everything the transaction reads, as the pure model and the renderer take it.
struct Stored {
    revision: plan_revision::Model,
    currency: String,
    context: ResolveContext,
    /// Every price of the entries the items name, as stored: a binding renders its row.
    rows: BTreeMap<Uuid, price::Model>,
    defaults: TenantDefaults,
    rounding: String,
}

/// `GET /resolve` below its door (D-419, D-420, D-421).
/// # Errors
/// 404 for a revision the tenant does not hold or an `item_id` the revision lacks; 409
/// `REVISION_NOT_PUBLISHED`; 400 for the pins; Products' definite refusal as it gave it; 503
/// `REGISTRY_UNAVAILABLE` when Products cannot answer.
async fn resolution(
    state: &AuthoringState,
    scope: AccessScope,
    ctx: &SecurityContext,
    request: ResolveRequest,
) -> Result<Response, CanonicalError> {
    let tenant = ctx.subject_tenant_id();
    let (revision, item) = (request.revision, request.item);
    let stored = support::transaction(&state.db.db(), move |tx| {
        let scope = scope.clone();
        Box::pin(async move { read_stored(tx, &scope, tenant, revision, item).await })
    })
    .await?;
    let resolved: Vec<ItemResolution> =
        resolve::matrix(&stored.context, request.date, &request.pins)
            .map_err(|e| support::invalid("pins", e.code))?
            .into_iter()
            .filter(|r| item.is_none_or(|id| r.item_id == id))
            .collect();
    let versions = versions_as_of(
        &state.hub,
        ctx,
        resolved.iter().map(|r| r.sku_id),
        request.date,
    )
    .await?;
    let body = render(&stored, request.date, resolved, &versions)?;
    support::response(StatusCode::OK, &body, None)
}

fn corrupt(what: String) -> DoorError {
    RepoError::CorruptRow(what).into()
}
/// The one read transaction of a resolve.
async fn read_stored(
    tx: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
    item: Option<Uuid>,
) -> Result<Stored, DoorError> {
    let children = AccessScope::for_tenant(tenant);
    let revision = plan_revision_repo::find(tx, scope, tenant, id)
        .await?
        .ok_or_else(|| support::missing_what("plan_revision"))?;
    let state: RevisionState = revision
        .state
        .parse()
        .map_err(|_| corrupt(format!("revision {id} state")))?;
    if !matches!(state, RevisionState::Published | RevisionState::Superseded) {
        return Err(support::conflict("REVISION_NOT_PUBLISHED").into());
    }
    plan_repo::find(tx, &children, tenant, revision.plan_id)
        .await?
        .ok_or_else(|| corrupt(format!("revision {id} has no plan")))?;
    let rows = plan_item_repo::for_revision(tx, &children, tenant, revision.id).await?;
    if item.is_some_and(|wanted| !rows.iter().any(|r| r.id == wanted)) {
        return Err(support::missing_what("plan_item").into());
    }
    let book = book_repo::find(tx, &children, tenant, revision.book_id)
        .await?
        .ok_or_else(|| corrupt(format!("revision {id} has no book")))?;
    let mut registry = BTreeMap::new();
    for d in dimension_repo::list(tx, &children, tenant).await? {
        let values: Vec<String> = serde_json::from_value(d.values)
            .map_err(|_| corrupt(format!("dimension {} values", d.key)))?;
        registry.insert(d.key, values);
    }
    let mut entries: BTreeMap<Uuid, resolve::Entry> = BTreeMap::new();
    let mut prices = BTreeMap::new();
    let mut keep_for_bound = BTreeSet::new();
    for entry_id in rows.iter().filter_map(|r| r.price_book_entry_id) {
        if entries.contains_key(&entry_id) {
            continue;
        }
        let e = price_book_entry_repo::find(tx, &children, tenant, entry_id)
            .await?
            .ok_or_else(|| corrupt(format!("entry {entry_id} of revision {id}")))?;
        let of_entry = price_repo::for_entry(tx, &children, tenant, e.id).await?;
        keep_for_bound.extend(of_entry.iter().filter(|p| p.keep_for_bound).map(|p| p.id));
        let domain = of_entry
            .iter()
            .map(price_repo::to_domain)
            .collect::<Result<Vec<_>, _>>()?;
        prices.extend(of_entry.into_iter().map(|p| (p.id, p)));
        let values = e
            .dimension_key
            .as_ref()
            .and_then(|key| registry.get(key))
            .cloned()
            .unwrap_or_default();
        let charge_kind: ChargeKind = e
            .charge_kind
            .parse()
            .map_err(|_| corrupt(format!("entry {} charge_kind", e.id)))?;
        entries.insert(
            e.id,
            resolve::Entry {
                id: e.id,
                charge_kind,
                period: e.period,
                invoice_line_override: e.invoice_line_override,
                values,
                prices: domain,
            },
        );
    }
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let bad = |what: &str| corrupt(format!("plan item {} {what}", row.id));
        let treatment: Treatment = row.treatment.parse().map_err(|_| bad("treatment"))?;
        items.push(resolve::Item {
            id: row.id,
            sku_id: row.sku_id,
            treatment,
            included_qty: row
                .included_qty
                .as_deref()
                .map(str::parse)
                .transpose()
                .map_err(|_| bad("included_qty"))?,
            qty_min: row.qty_min,
            entry: row
                .price_book_entry_id
                .and_then(|entry| entries.get(&entry).cloned()),
        });
    }
    let settings = configuration::settings(tx, &children, tenant).await?;
    let invoice_line_templates =
        serde_json::from_value(settings.invoice_line_templates).map_err(|_| {
            corrupt(format!(
                "settings of tenant {tenant} invoice_line_templates"
            ))
        })?;
    Ok(Stored {
        revision,
        currency: book.currency,
        context: ResolveContext {
            items,
            keep_for_bound,
        },
        rows: prices,
        defaults: TenantDefaults {
            default_timing: settings.default_timing,
            default_gl: settings.default_gl,
            default_tax_category: settings.default_tax_category,
            invoice_line_templates,
        },
        rounding: settings.default_rounding,
    })
}

/// D-421: each SKU version as of `date`, one read per distinct SKU, through the detached
/// registry as the caller. A SKU Products does not know (404) has no version.
/// # Errors
/// Any other definite refusal as Products gave it; 503 `REGISTRY_UNAVAILABLE` when Products
/// cannot answer.
async fn versions_as_of(
    hub: &toolkit::ClientHub,
    ctx: &SecurityContext,
    skus: impl IntoIterator<Item = Uuid>,
    date: Date,
) -> Result<BTreeMap<Uuid, SkuVersion>, CanonicalError> {
    let wanted: BTreeSet<Uuid> = skus.into_iter().collect();
    let mut found = BTreeMap::new();
    if wanted.is_empty() {
        return Ok(found);
    }
    let registry = reference_registry::resolve(hub).map_err(|_| support::unavailable())?;
    for sku in wanted {
        match registry
            .sku_version_as_of(ctx, ctx.subject_tenant_id(), sku, date)
            .await
        {
            Ok(Some(version)) => {
                found.insert(sku, version);
            }
            Ok(None) => {}
            Err(error) if error.status_code() == 404 => {}
            Err(error) if reference_work::definite_refusal(&error) => return Err(error),
            Err(_) => return Err(support::unavailable()),
        }
    }
    Ok(found)
}

fn input(resolved: Resolved) -> PricingResolveInputDto {
    PricingResolveInputDto {
        value: resolved.value,
        source: resolved.source.map(|s| s.as_str().to_owned()),
    }
}
/// The response, field by field as slice 07 §6 lists it.
fn render(
    stored: &Stored,
    date: Date,
    resolved: Vec<ItemResolution>,
    versions: &BTreeMap<Uuid, SkuVersion>,
) -> Result<PricingResolveDto, CanonicalError> {
    let mut items = Vec::with_capacity(resolved.len());
    for r in resolved {
        let version = versions.get(&r.sku_id);
        let entry_override = stored
            .context
            .items
            .iter()
            .find(|i| i.id == r.item_id)
            .and_then(|i| i.entry.as_ref())
            .and_then(|e| e.invoice_line_override.as_deref());
        let inputs =
            resolve::invoice_inputs(entry_override, version, &stored.defaults, r.charge_kind);
        let mut chains = Vec::with_capacity(r.chains.len());
        for chain in r.chains {
            let uncovered = chain.uncovered();
            let binding = chain
                .binding
                .map(|b| {
                    let row = stored.rows.get(&b.price.id).ok_or_else(|| {
                        CanonicalError::internal("a bound price has no stored row").create()
                    })?;
                    Ok::<_, CanonicalError>(PricingResolveBindingDto {
                        price_id: row.id,
                        dim_used: b.dim_used().map(str::to_owned),
                        pinned_from: b.pinned_from,
                        model: row.model.clone(),
                        price: row.price_json.clone(),
                        min_fee: row.min_fee.clone(),
                        eligibility: row.eligibility.clone(),
                        effective_from: row.effective_from.to_string(),
                        effective_to: row.effective_to.map(|d| d.to_string()),
                        temporary_until: row.temporary_until.map(|d| d.to_string()),
                        keep_for_bound: b.keep_for_bound,
                    })
                })
                .transpose()?;
            chains.push(PricingResolveChainDto {
                dim_value: chain.dim_value,
                uncovered,
                binding,
            });
        }
        items.push(PricingResolveItemDto {
            item_id: r.item_id,
            sku_id: r.sku_id,
            treatment: r.treatment.as_str().to_owned(),
            included_qty: r.included_qty.map(|q| q.to_string()),
            qty_min: r.qty_min,
            price_book_entry_id: r.price_book_entry_id,
            charge_kind: r.charge_kind.map(|k| k.as_str().to_owned()),
            period: r.period,
            sku_version: version.map(|v| PricingResolveSkuVersionDto {
                published_version: v.published_version,
                effective_from: v.effective_from.to_string(),
            }),
            invoice_line_template: input(inputs.invoice_line_template),
            gl_code: input(inputs.gl_code),
            tax_category: input(inputs.tax_category),
            billing_timing: input(inputs.billing_timing),
            meter: PricingResolveMeterDto {
                usage_type_ref: version.and_then(|v| v.content.usage_type_ref.clone()),
                unit: version.and_then(|v| v.content.unit.clone()),
            },
            chains,
        });
    }
    Ok(PricingResolveDto {
        plan_revision_id: stored.revision.id,
        plan_id: stored.revision.plan_id,
        rev_no: stored.revision.rev_no,
        state: stored.revision.state.clone(),
        book_id: stored.revision.book_id,
        currency: stored.currency.clone(),
        currency_minor_digits: book::minor_digits(&stored.currency),
        rounding_policy: stored.rounding.clone(),
        date: date.to_string(),
        items,
    })
}
