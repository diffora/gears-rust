//! Local transport of `ProductCatalogClientV1`. Phase 1b: empty answers, so the contract stays
//! registered; phase 1c serves it from `products_sku`.
use async_trait::async_trait;
use bss_pricing_sdk::product_catalog::{
    CatalogSku, CatalogSkuPage, CatalogTaxCategory, ProductCatalogClientV1,
};
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use uuid::Uuid;

pub struct EmptyCatalogProvider;

#[async_trait]
impl ProductCatalogClientV1 for EmptyCatalogProvider {
    async fn get_skus(
        &self,
        _ctx: &SecurityContext,
        _ids: &[Uuid],
    ) -> Result<Vec<CatalogSku>, CanonicalError> {
        Ok(Vec::new())
    }
    async fn search_skus(
        &self,
        _ctx: &SecurityContext,
        _q: Option<&str>,
        _limit: u32,
        _cursor: Option<&str>,
    ) -> Result<CatalogSkuPage, CanonicalError> {
        Ok(CatalogSkuPage {
            items: Vec::new(),
            next_cursor: None,
        })
    }
    async fn list_tax_categories(
        &self,
        _ctx: &SecurityContext,
    ) -> Result<Vec<CatalogTaxCategory>, CanonicalError> {
        Ok(Vec::new())
    }
}
