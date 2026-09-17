//! Pricing's `ProductCatalogClientV1`, served from this gear's browse
//! projection.
//!
//! The local arm: same serving rows the browse door reads
//! ([`repo::browse_read_entities_page`], [`repo::find_read_entity`]), mapped
//! onto [`CatalogSku`]. A product row is skipped, not coerced. Tax categories
//! are not projected here, so [`BrowseCatalogProvider::list_tax_categories`]
//! answers an empty dictionary rather than fabricating codes.

use async_trait::async_trait;
use bss_pricing_sdk::product_catalog::{
    CatalogSku, CatalogSkuPage, CatalogTaxCategory, ProductCatalogClientV1, catalog_unreachable,
};
use toolkit_canonical_errors::CanonicalError;
use toolkit_db::odata::sea_orm_filter::LimitCfg;
use toolkit_db::secure::AccessScope;
use toolkit_db::{DBProvider, DbError};
use toolkit_odata::{CursorV1, ODataQuery, parse_filter_string};
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::domain::read_model::{ReadSurface, VisibilityFilter};
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::read_entity;
use crate::infra::storage::repo::{self, BrowseQuery};

/// Same page bounds the browse door serves (`api::rest::odata::LISTING_LIMIT_CFG`).
const BROWSE_LIMIT_CFG: LimitCfg = LimitCfg {
    default: 50,
    max: 200,
};

/// Why a serving row cannot become a [`CatalogSku`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum MappingError {
    /// `entity_kind` is not `sku`. Callers skip the row; they do not map it.
    #[error("entity_kind is not sku")]
    NotASku,
    /// `entity_code` is `None`. Must not become an empty `sku_code`.
    #[error("sku_code is missing")]
    MissingSkuCode,
    /// `sku_type` is `None`. Pricing's field is required and a rule keys on it.
    #[error("sku_type is missing")]
    MissingSkuType,
    /// `sellable` is `None`. Pricing's field is required and a rule keys on it.
    #[error("sellable is missing")]
    MissingSellable,
}

/// Map one browse-projection row onto pricing's registry SKU.
///
/// `name` is copied from the row's `name`. For a SKU the projector fills that
/// column from `sku_code`; this function does not invent a display label.
///
/// # Errors
///
/// [`MappingError::NotASku`] when `entity_kind` is not `sku` (a product row is
/// skipped, not mapped). [`MappingError::MissingSkuCode`],
/// [`MappingError::MissingSkuType`], or [`MappingError::MissingSellable`] when
/// a required SKU column is `None`.
pub(crate) fn catalog_sku_of(row: &read_entity::Model) -> Result<CatalogSku, MappingError> {
    if row.entity_kind != "sku" {
        return Err(MappingError::NotASku);
    }
    let sku_code = row
        .entity_code
        .clone()
        .ok_or(MappingError::MissingSkuCode)?;
    let sku_type = row.sku_type.clone().ok_or(MappingError::MissingSkuType)?;
    let sellable = row.sellable.ok_or(MappingError::MissingSellable)?;
    Ok(CatalogSku {
        sku_id: row.entity_id,
        sku_code,
        name: row.name.clone(),
        metering_unit: row.metering_unit.clone(),
        status: row.lifecycle_state.clone(),
        plan_tier: row.plan_tier_label.clone(),
        sku_type,
        sellable,
        usage_type_ref: row.usage_type_ref.clone(),
        deprecated: row.deprecated,
    })
}

/// In-process [`ProductCatalogClientV1`] over the browse projection.
#[derive(Clone)]
pub struct BrowseCatalogProvider {
    db: DBProvider<DbError>,
}

impl BrowseCatalogProvider {
    /// Wrap the gear's database. Cheap to clone (`DBProvider` is `Arc`).
    #[must_use]
    pub fn new(db: DBProvider<DbError>) -> Self {
        Self { db }
    }
}

fn tenant_scope(ctx: &SecurityContext) -> (Uuid, AccessScope) {
    let tenant_id = ctx.subject_tenant_id();
    (tenant_id, AccessScope::for_tenant(tenant_id))
}

fn mapped_sku(row: &read_entity::Model) -> Result<Option<CatalogSku>, CanonicalError> {
    match catalog_sku_of(row) {
        Ok(sku) => Ok(Some(sku)),
        Err(MappingError::NotASku) => Ok(None),
        Err(err) => {
            Err(CanonicalError::internal(format!("products catalog mapping: {err}")).create())
        }
    }
}

fn catalog_repo_error(err: RepoError) -> CanonicalError {
    match err {
        RepoError::CorruptRow(detail) => {
            CanonicalError::internal(format!("products catalog: {detail}")).create()
        }
        other => catalog_unreachable(other.to_string()),
    }
}

fn catalog_odata_error(err: toolkit_odata::Error) -> CanonicalError {
    match err {
        toolkit_odata::Error::Db(detail) => catalog_unreachable(detail),
        other => CanonicalError::internal(format!("products catalog search: {other}")).create(),
    }
}

/// Browse's own `OData` paging: `q` as `startswith` on `name`; `limit`/`cursor`
/// as `$top` / `$skiptoken`.
///
/// # Errors
///
/// [`CanonicalError`] when the prefix cannot be parsed as a `$filter` or the
/// continuation token is not a `CursorV1` this walk minted.
fn search_odata(
    q: Option<&str>,
    limit: u32,
    cursor: Option<&str>,
) -> Result<ODataQuery, CanonicalError> {
    let mut odata = ODataQuery::new().with_limit(u64::from(limit));
    if let Some(prefix) = q.filter(|s| !s.is_empty()) {
        let escaped = prefix.replace('\'', "''");
        let parsed =
            parse_filter_string(&format!("startswith(name,'{escaped}')")).map_err(|e| {
                CanonicalError::internal(format!("products catalog search filter: {e}")).create()
            })?;
        odata = odata.with_filter(parsed.into_expr());
    }
    if let Some(token) = cursor.filter(|s| !s.is_empty()) {
        let decoded = CursorV1::decode(token).map_err(|e| {
            CanonicalError::internal(format!("products catalog search cursor: {e}")).create()
        })?;
        odata = odata.with_cursor(decoded);
    }
    Ok(odata)
}

#[async_trait]
impl ProductCatalogClientV1 for BrowseCatalogProvider {
    async fn get_skus(
        &self,
        ctx: &SecurityContext,
        ids: &[Uuid],
    ) -> Result<Vec<CatalogSku>, CanonicalError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let (tenant_id, scope) = tenant_scope(ctx);
        let conn = self
            .db
            .conn()
            .map_err(|e| catalog_unreachable(e.to_string()))?;
        let mut out = Vec::new();
        for id in ids {
            let Some(row) = repo::find_read_entity(&conn, &scope, tenant_id, "sku", *id)
                .await
                .map_err(catalog_repo_error)?
            else {
                continue;
            };
            if let Some(sku) = mapped_sku(&row)? {
                out.push(sku);
            }
        }
        Ok(out)
    }

    async fn search_skus(
        &self,
        ctx: &SecurityContext,
        q: Option<&str>,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<CatalogSkuPage, CanonicalError> {
        let (tenant_id, scope) = tenant_scope(ctx);
        let conn = self
            .db
            .conn()
            .map_err(|e| catalog_unreachable(e.to_string()))?;
        let (_, generation) = repo::load_read_checkpoint(&conn, &scope, tenant_id)
            .await
            .map_err(catalog_repo_error)?
            .unwrap_or((0, 0));
        let query = BrowseQuery {
            visibility: Some(repo::visibility_condition(VisibilityFilter::for_surface(
                ReadSurface::DefaultBrowse,
            ))),
            entity_kind: Some("sku".to_owned()),
            brand_claim: None,
            region_claim: None,
            generation,
        };
        let odata = search_odata(q, limit, cursor)?;
        let page = repo::browse_read_entities_page(
            &conn,
            &scope,
            tenant_id,
            &query,
            &odata,
            BROWSE_LIMIT_CFG,
        )
        .await
        .map_err(catalog_odata_error)?;
        let mut items = Vec::with_capacity(page.items.len());
        for row in page.items {
            if let Some(sku) = mapped_sku(&row)? {
                items.push(sku);
            }
        }
        Ok(CatalogSkuPage {
            items,
            next_cursor: page.page_info.next_cursor,
        })
    }

    async fn list_tax_categories(
        &self,
        _ctx: &SecurityContext,
    ) -> Result<Vec<CatalogTaxCategory>, CanonicalError> {
        // P-D-169 withdrew the tax-category dictionary from this registry.
        // An empty answer is "none published", not a fabricated code list.
        Ok(Vec::new())
    }
}

#[cfg(test)]
#[path = "catalog_provider_tests.rs"]
mod catalog_provider_tests;
