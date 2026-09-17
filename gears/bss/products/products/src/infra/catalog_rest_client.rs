//! REST arm of pricing's `ProductCatalogClientV1`.
//!
//! `#[toolkit::provides]` constructs this with `new(cfg)` where `cfg` is
//! `tuning.apply_to(endpoint).with_internal_token_provider(...)`. The client
//! calls the existing browse door (`GET /bss-products/v1/browse?kind=sku`)
//! rather than a generated contract surface beside the trait.

use async_trait::async_trait;
use bss_pricing_sdk::product_catalog::{
    CatalogSku, CatalogSkuPage, CatalogTaxCategory, ProductCatalogClientV1, catalog_unreachable,
};
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use toolkit::contract_support::runtime::config::ClientConfig;
use toolkit::contract_support::runtime::http::parse_retry_after;
use toolkit_canonical_errors::{CanonicalError, Problem};
use toolkit_http::HttpClient;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::infra::catalog_provider::{MappingError, catalog_sku_of};
use crate::infra::storage::entity::read_entity;

/// Browse door the REST arm calls.
const BROWSE_PATH: &str = "/bss-products/v1/browse";

/// Same ceiling the browse door serves.
const BROWSE_MAX: u32 = 200;

/// REST [`ProductCatalogClientV1`] over `GET /bss-products/v1/browse`.
pub struct ProductCatalogRestClient {
    http: HttpClient,
    config: ClientConfig,
}

impl ProductCatalogRestClient {
    /// Build the transport the provides-macro REST arm expects.
    ///
    /// # Errors
    ///
    /// [`toolkit_http::HttpError`] when the default HTTP client cannot be built.
    pub fn new(config: ClientConfig) -> Result<Self, toolkit_http::HttpError> {
        let http = toolkit::contract_support::runtime::client::build_default_http_client(
            "ProductCatalogRestClient",
            config.require_tls,
        )?;
        Ok(Self { http, config })
    }
}

#[derive(Debug, Deserialize)]
struct BrowseView {
    rows: Vec<BrowseRowView>,
    page_info: BrowsePageInfo,
}

#[derive(Debug, Deserialize)]
struct BrowsePageInfo {
    next_cursor: Option<String>,
}

/// Wire shape of one browse row — field pairing matches [`catalog_sku_of`].
#[derive(Debug, Deserialize)]
struct BrowseRowView {
    entity_kind: String,
    entity_id: Uuid,
    entity_code: Option<String>,
    name: String,
    lifecycle_state: String,
    deprecated: bool,
    composition_pending: bool,
    sellable: Option<bool>,
    deprecation_provenance: Option<String>,
    replaced_by_sku_id: Option<Uuid>,
    #[serde(default)]
    region_scope: String,
    #[serde(default)]
    brand_scope: String,
    sku_type: Option<String>,
    plan_tier_label: Option<String>,
    metering_unit: Option<String>,
    usage_type_ref: Option<String>,
    display_attributes: Option<String>,
    category_paths: Option<String>,
    #[serde(default)]
    published_version: i64,
}

fn row_model(row: BrowseRowView, tenant_id: Uuid) -> read_entity::Model {
    read_entity::Model {
        tenant_id,
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
        usage_type_ref: row.usage_type_ref,
        display_attributes: row.display_attributes,
        category_paths: row.category_paths,
        published_version: row.published_version,
        projected_at: time::OffsetDateTime::UNIX_EPOCH,
        generation: 0,
    }
}

fn map_rows(rows: Vec<BrowseRowView>, tenant_id: Uuid) -> Result<Vec<CatalogSku>, CanonicalError> {
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        match catalog_sku_of(&row_model(row, tenant_id)) {
            Ok(sku) => items.push(sku),
            Err(MappingError::NotASku) => {}
            Err(err) => {
                return Err(
                    CanonicalError::internal(format!("products catalog mapping: {err}")).create(),
                );
            }
        }
    }
    Ok(items)
}

fn encode_query(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                out.push('%');
                out.push(char::from(HEX[usize::from(byte >> 4)]));
                out.push(char::from(HEX[usize::from(byte & 0x0F)]));
            }
        }
    }
    out
}

fn ids_filter(ids: &[Uuid]) -> String {
    ids.iter()
        .map(|id| format!("entity_id eq {id}"))
        .collect::<Vec<_>>()
        .join(" or ")
}

fn search_filter(q: Option<&str>) -> Option<String> {
    let prefix = q.filter(|s| !s.is_empty())?;
    let escaped = prefix.replace('\'', "''");
    Some(format!("startswith(name,'{escaped}')"))
}

fn browse_url(base: &str, filter: Option<&str>, limit: u32, cursor: Option<&str>) -> String {
    let mut url = format!(
        "{}{BROWSE_PATH}?kind=sku&limit={}",
        base.trim_end_matches('/'),
        limit.min(BROWSE_MAX)
    );
    if let Some(filter) = filter.filter(|s| !s.is_empty()) {
        url.push_str("&$filter=");
        url.push_str(&encode_query(filter));
    }
    if let Some(cursor) = cursor.filter(|s| !s.is_empty()) {
        url.push_str("&cursor=");
        url.push_str(&encode_query(cursor));
    }
    url
}

/// Surface browse's 503 with **its** `Retry-After`. Do not invent a delay.
pub(crate) fn catalog_error_from_http(
    status: u16,
    retry_after: Option<std::time::Duration>,
    body: &[u8],
) -> CanonicalError {
    if status == 503 {
        let mut builder = CanonicalError::service_unavailable();
        if let Some(delay) = retry_after {
            builder = builder.with_retry_after_seconds(delay.as_secs());
        }
        if let Ok(problem) = serde_json::from_slice::<Problem>(body)
            && !problem.detail.is_empty()
        {
            builder = builder.with_detail(problem.detail);
        }
        return builder.create();
    }
    catalog_unreachable(format!("HTTP {status}: {}", String::from_utf8_lossy(body)))
}

impl ProductCatalogRestClient {
    async fn browse(
        &self,
        ctx: &SecurityContext,
        filter: Option<&str>,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<BrowseView, CanonicalError> {
        let url = browse_url(&self.config.base_url, filter, limit, cursor);
        let mut request = self.http.get(&url);
        if let Some(token) = ctx.bearer_token() {
            request = request.bearer_auth(token.expose_secret());
        }
        let response = request
            .send()
            .await
            .map_err(|e| catalog_unreachable(e.to_string()))?;
        let status = response.status().as_u16();
        let retry_after = parse_retry_after(response.headers());
        let bytes = response
            .bytes()
            .await
            .map_err(|e| catalog_unreachable(e.to_string()))?;
        if !(200..300).contains(&status) {
            return Err(catalog_error_from_http(status, retry_after, &bytes));
        }
        serde_json::from_slice(&bytes)
            .map_err(|e| CanonicalError::internal(format!("products catalog browse: {e}")).create())
    }
}

#[async_trait]
impl ProductCatalogClientV1 for ProductCatalogRestClient {
    async fn get_skus(
        &self,
        ctx: &SecurityContext,
        ids: &[Uuid],
    ) -> Result<Vec<CatalogSku>, CanonicalError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let filter = ids_filter(ids);
        let wanted: std::collections::HashSet<Uuid> = ids.iter().copied().collect();
        let page = self
            .browse(
                ctx,
                Some(&filter),
                u32::try_from(ids.len()).unwrap_or(BROWSE_MAX),
                None,
            )
            .await?;
        let tenant_id = ctx.subject_tenant_id();
        Ok(map_rows(page.rows, tenant_id)?
            .into_iter()
            .filter(|sku| wanted.contains(&sku.sku_id))
            .collect())
    }

    async fn search_skus(
        &self,
        ctx: &SecurityContext,
        q: Option<&str>,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<CatalogSkuPage, CanonicalError> {
        let filter = search_filter(q);
        let page = self.browse(ctx, filter.as_deref(), limit, cursor).await?;
        Ok(CatalogSkuPage {
            items: map_rows(page.rows, ctx.subject_tenant_id())?,
            next_cursor: page.page_info.next_cursor,
        })
    }

    async fn list_tax_categories(
        &self,
        _ctx: &SecurityContext,
    ) -> Result<Vec<CatalogTaxCategory>, CanonicalError> {
        // P-D-169 withdrew the tax-category dictionary from this registry.
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BrowseRowView, browse_url, catalog_error_from_http, ids_filter, map_rows, search_filter,
    };
    use toolkit_canonical_errors::CanonicalError;
    use uuid::Uuid;

    const SKU: Uuid = Uuid::from_u128(0xca_7a_10_91);
    const TENANT: Uuid = Uuid::from_u128(0xca_7a_10_01);

    fn published_row() -> BrowseRowView {
        BrowseRowView {
            entity_kind: "sku".to_owned(),
            entity_id: SKU,
            entity_code: Some("COMP-VCPU-H".to_owned()),
            name: "COMP-VCPU-H".to_owned(),
            lifecycle_state: "published".to_owned(),
            deprecated: false,
            composition_pending: false,
            sellable: Some(true),
            deprecation_provenance: None,
            replaced_by_sku_id: None,
            region_scope: String::new(),
            brand_scope: String::new(),
            sku_type: Some("service".to_owned()),
            plan_tier_label: Some("Pro".to_owned()),
            metering_unit: Some("vCPU-hour".to_owned()),
            usage_type_ref: Some("cf.usage.vcpu-hour".to_owned()),
            display_attributes: None,
            category_paths: None,
            published_version: 3,
        }
    }

    #[test]
    fn browse_query_carries_kind_filter_limit_and_cursor() {
        let url = browse_url(
            "http://bss-products.virtuozzo.svc:8080",
            Some("startswith(name,'COMP')"),
            50,
            Some("tok"),
        );
        assert!(url.starts_with("http://bss-products.virtuozzo.svc:8080/bss-products/v1/browse?"));
        assert!(url.contains("kind=sku"));
        assert!(url.contains("limit=50"));
        assert!(url.contains("$filter="));
        assert!(url.contains("cursor=tok"));
    }

    #[test]
    fn ids_filter_is_odata_eq_joined_with_or() {
        assert_eq!(ids_filter(&[SKU]), format!("entity_id eq {SKU}"));
    }

    #[test]
    fn search_filter_is_startswith_on_name() {
        assert_eq!(
            search_filter(Some("COMP")),
            Some("startswith(name,'COMP')".to_owned())
        );
        assert_eq!(search_filter(Some("")), None);
    }

    #[test]
    fn browse_rows_map_through_catalog_sku_of() {
        let items = map_rows(vec![published_row()], TENANT).expect("map");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].sku_id, SKU);
        assert_eq!(items[0].sku_code, "COMP-VCPU-H");
        assert_eq!(items[0].status, "published");
    }

    #[test]
    fn a_503_keeps_the_server_retry_after_and_does_not_invent_one() {
        let with_header = catalog_error_from_http(
            503,
            Some(std::time::Duration::from_secs(12)),
            br#"{"title":"overloaded","status":503,"detail":"READ_MODEL_OVERLOADED"}"#,
        );
        match &with_header {
            CanonicalError::ServiceUnavailable { ctx, detail, .. } => {
                assert_eq!(ctx.retry_after_seconds, Some(12));
                assert_eq!(detail, "READ_MODEL_OVERLOADED");
            }
            other => panic!("expected ServiceUnavailable, got {other:?}"),
        }

        let without = catalog_error_from_http(503, None, b"{}");
        match &without {
            CanonicalError::ServiceUnavailable { ctx, .. } => {
                assert!(
                    ctx.retry_after_seconds.is_none(),
                    "absent Retry-After must stay absent"
                );
            }
            other => panic!("expected ServiceUnavailable, got {other:?}"),
        }
    }
}
