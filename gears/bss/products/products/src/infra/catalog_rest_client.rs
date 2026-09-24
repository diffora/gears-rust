//! REST transport retained for the provides macro until phase 1c restores browse.
use async_trait::async_trait;
use bss_pricing_sdk::product_catalog::{
    CatalogSku, CatalogSkuPage, CatalogTaxCategory, ProductCatalogClientV1, catalog_unreachable,
};
use toolkit::contract_support::runtime::config::ClientConfig;
use toolkit_canonical_errors::CanonicalError;
use toolkit_http::HttpClient;
use toolkit_security::SecurityContext;
use uuid::Uuid;

/// REST factory contract used by the gear's provides macro.
pub struct ProductCatalogRestClient {
    #[allow(dead_code)] // Phase 1c restores the HTTP browse transport.
    http: HttpClient,
    #[allow(dead_code)] // The macro still configures the future transport.
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

#[async_trait]
impl ProductCatalogClientV1 for ProductCatalogRestClient {
    async fn get_skus(
        &self,
        _ctx: &SecurityContext,
        _ids: &[Uuid],
    ) -> Result<Vec<CatalogSku>, CanonicalError> {
        Err(catalog_unreachable(
            "bss-products browse route returns in phase 1c",
        ))
    }
    async fn search_skus(
        &self,
        _ctx: &SecurityContext,
        _q: Option<&str>,
        _limit: u32,
        _cursor: Option<&str>,
    ) -> Result<CatalogSkuPage, CanonicalError> {
        Err(catalog_unreachable(
            "bss-products browse route returns in phase 1c",
        ))
    }
    async fn list_tax_categories(
        &self,
        _ctx: &SecurityContext,
    ) -> Result<Vec<CatalogTaxCategory>, CanonicalError> {
        Err(catalog_unreachable(
            "bss-products browse route returns in phase 1c",
        ))
    }
}
