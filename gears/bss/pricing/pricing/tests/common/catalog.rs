use toolkit_security::SecurityContext;
use uuid::Uuid;

pub const OFFER_SKU: Uuid = Uuid::from_u128(0x5_c1);

pub fn resource_sku(meter: &str) -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_OID, meter.as_bytes())
}

pub fn catalog_sku(
    id: Uuid,
    meter: Option<&str>,
    sellable: bool,
) -> bss_pricing::domain::ports::CatalogSku {
    bss_pricing::domain::ports::CatalogSku {
        sku_id: id,
        sku_code: id.to_string(),
        name: id.to_string(),
        metering_unit: meter.map(str::to_owned),
        status: "published".into(),
        plan_tier: None,
        sku_type: "service".into(),
        sellable,
        usage_type_ref: None,
        deprecated: false,
    }
}

pub struct FixtureCatalog(pub Vec<bss_pricing::domain::ports::CatalogSku>);
impl FixtureCatalog {
    pub fn sku_ids(&self) -> Vec<Uuid> {
        self.0.iter().map(|sku| sku.sku_id).collect()
    }
}
impl Default for FixtureCatalog {
    fn default() -> Self {
        let mut skus = vec![
            catalog_sku(OFFER_SKU, None, true),
            catalog_sku(Uuid::from_u128(5), None, false),
            catalog_sku(Uuid::from_u128(1), None, true),
        ];
        for meter in [
            "egress-gb",
            "storage_bytes",
            "api-calls",
            "storage-gb",
            "api_bytes",
            "GB-hour",
            "GB",
            "gb",
            "GB_month",
            "GiB-hour",
            "vcpu_hour",
            "api_calls",
            "requests",
            "cpu_hour",
            "request",
            "gib_hour",
            "gb_hour",
            "compute_hour",
            "storage_gb",
            "unit",
            "cloudlets",
            "egress_gb",
            "egress.gb",
            "storage.gb",
            "addon_dr",
            "CPU-hour",
            "GB-month",
            "egress-gb",
            "TB-hour",
        ] {
            skus.push(catalog_sku(resource_sku(meter), Some(meter), false));
        }
        Self(skus)
    }
}
#[async_trait::async_trait]
impl bss_pricing::domain::ports::ProductCatalogClientV1 for FixtureCatalog {
    async fn list_skus(
        &self,
        _ctx: &SecurityContext,
    ) -> Result<
        Vec<bss_pricing::domain::ports::CatalogSku>,
        toolkit::api::canonical_prelude::CanonicalError,
    > {
        Ok(self.0.clone())
    }
    async fn get_skus(
        &self,
        _ctx: &SecurityContext,
        ids: &[Uuid],
    ) -> Result<
        Vec<bss_pricing::domain::ports::CatalogSku>,
        toolkit::api::canonical_prelude::CanonicalError,
    > {
        Ok(self
            .0
            .iter()
            .filter(|sku| ids.contains(&sku.sku_id))
            .cloned()
            .collect())
    }
    async fn search_skus(
        &self,
        _ctx: &SecurityContext,
        q: Option<&str>,
        limit: u32,
        _cursor: Option<&str>,
    ) -> Result<
        bss_pricing::domain::ports::CatalogSkuPage,
        toolkit::api::canonical_prelude::CanonicalError,
    > {
        let prefix = q.unwrap_or("");
        let items = self
            .0
            .iter()
            .filter(|sku| sku.name.starts_with(prefix))
            .take(usize::try_from(limit).unwrap_or(usize::MAX))
            .cloned()
            .collect();
        Ok(bss_pricing::domain::ports::CatalogSkuPage {
            items,
            next_cursor: None,
        })
    }
    async fn list_tax_categories(
        &self,
        _ctx: &SecurityContext,
    ) -> Result<
        Vec<bss_pricing::domain::ports::CatalogTaxCategory>,
        toolkit::api::canonical_prelude::CanonicalError,
    > {
        Ok(vec![])
    }
}
