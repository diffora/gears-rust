//! Product registry inputs shared by all price write paths.
use crate::domain::{error::DomainError, price_record::PriceContent, scope_key::ScopeKey};
use std::sync::Arc;
use toolkit_security::SecurityContext;
/// Resolve one immutable registry listing for the request.
pub async fn sku_index(
    catalog: &dyn crate::domain::ports::ProductCatalogClientV1,
    ctx: &SecurityContext,
) -> Result<Arc<crate::domain::registry_view::SkuIndex>, DomainError> {
    let listing = catalog
        .list_skus(ctx)
        .await
        .map_err(|e| DomainError::CatalogVersionUnavailable(e.to_string()))?;
    Ok(Arc::new(
        crate::domain::registry_view::SkuIndex::from_listing(listing),
    ))
}

pub fn derive_meter(
    content: &mut PriceContent,
    key: &ScopeKey,
    index: &crate::domain::registry_view::SkuIndex,
) {
    content.row.sku_id = key.sku_id();
    content.row.meter = if key.charge_kind().is_usage() {
        index
            .get(key.sku_id())
            .and_then(|sku| sku.metering_unit.clone())
    } else {
        None
    };
}

/// Revalidate an already normalized row against the request's registry snapshot.
pub fn validate(
    content: &PriceContent,
    plan_sku: uuid::Uuid,
    index: Arc<crate::domain::registry_view::SkuIndex>,
) -> Result<(), DomainError> {
    let report =
        crate::domain::rules::price_row_rules(crate::domain::row_sku_rules::RowSkuContext {
            plan_sku: crate::domain::scope_key::SkuId::new(plan_sku),
            index,
        })
        .run(&content.row);
    if let Some(report) = report.write_stage_only() {
        return Err(DomainError::ValidationFailed(report));
    }
    Ok(())
}
