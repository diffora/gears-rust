//! Product registry inputs shared by all price write paths.
use crate::domain::{
    error::DomainError, price_record::PriceContent, scope_key::MarketPriceScopeKey,
};
use std::sync::Arc;
use toolkit_security::SecurityContext;
use uuid::Uuid;

/// Deduplicate the SKU ids a write names, keeping first-seen order.
#[must_use]
pub fn named_sku_ids(ids: impl IntoIterator<Item = Uuid>) -> Vec<Uuid> {
    let mut seen = std::collections::HashSet::new();
    ids.into_iter().filter(|id| seen.insert(*id)).collect()
}

/// The plan's own SKU plus every price-row SKU on the assembled unit.
#[must_use]
pub fn sku_ids_of_shape(shape: &crate::domain::plan_shape::PlanShape) -> Vec<Uuid> {
    named_sku_ids(
        std::iter::once(shape.sku_id).chain(
            shape
                .rows
                .iter()
                .map(|row| row.scope_key.sku_id().as_uuid()),
        ),
    )
}

/// Resolve the plan's own SKU and judge the role it carries for this context.
///
/// Returns the resolved index so a caller that needs it again — publish, which
/// judges the rows against the same listing — does not read the registry twice
/// for one request.
///
/// `expected_role` is the **context's**, and the caller reads the context from
/// the composition it can see. `None` means the plan is being staged and either
/// plan-owning role is admitted; see
/// [`validate_plan_sku_at_create`](crate::domain::plan_sku_rules::validate_plan_sku_at_create).
///
/// Called **before** the write transaction opens: a registry outage must leave
/// no half-written plan behind, and the 503 it becomes is the same one every
/// other registry-dependent write answers.
pub async fn require_plan_sku_role(
    catalog: &dyn crate::domain::ports::ProductCatalogClientV1,
    ctx: &SecurityContext,
    sku_id: Uuid,
    expected_role: Option<&str>,
    introducing: bool,
) -> Result<Arc<crate::domain::registry_view::SkuIndex>, DomainError> {
    use crate::domain::plan_sku_rules::{validate_plan_sku, validate_plan_sku_at_create};
    let index = sku_index_for(catalog, ctx, &[sku_id]).await?;
    let subject = crate::domain::scope_key::SkuId::new(sku_id);
    let report = match expected_role {
        Some(role) => validate_plan_sku(subject, &index, role, introducing),
        None => validate_plan_sku_at_create(subject, &index, introducing),
    };
    if let Some(write_stage) = report.write_stage_only() {
        return Err(DomainError::ValidationFailed(write_stage));
    }
    Ok(index)
}

/// Resolve exactly the registry rows this write names: the row's SKU and the
/// plan's own. The rules need no more, and a whole-catalog read per save makes
/// every price write depend on the registry's whole read-model capacity.
pub async fn sku_index_for(
    catalog: &dyn crate::domain::ports::ProductCatalogClientV1,
    ctx: &SecurityContext,
    ids: &[Uuid],
) -> Result<Arc<crate::domain::registry_view::SkuIndex>, DomainError> {
    let listing = catalog.get_skus(ctx, ids).await.map_err(|e| {
        // The 503 this becomes carries no detail (`infra::error_mapping`'s own
        // account of why), so this line is the only record of what the catalog
        // said. The delay, when the catalog named one, travels on the variant.
        tracing::error!(
            error = %e,
            "bss-pricing: the product catalog could not be read; the price write answers 503"
        );
        let retry_after_seconds = match &e {
            toolkit_canonical_errors::CanonicalError::ServiceUnavailable { ctx, .. } => {
                ctx.retry_after_seconds
            }
            _ => None,
        };
        DomainError::CatalogVersionUnavailable {
            detail: e.to_string(),
            retry_after_seconds,
        }
    })?;
    Ok(Arc::new(
        crate::domain::registry_view::SkuIndex::from_listing(listing),
    ))
}

pub fn derive_meter(
    content: &mut PriceContent,
    key: &MarketPriceScopeKey,
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
///
/// `introducing` is true for a successor that has never been published (the
/// supersession and cutover doors). An already-published row is not an
/// introduction.
pub fn validate(
    content: &PriceContent,
    plan_sku: Uuid,
    index: Arc<crate::domain::registry_view::SkuIndex>,
    introducing: bool,
) -> Result<(), DomainError> {
    let report =
        crate::domain::rules::price_row_rules(crate::domain::row_sku_rules::RowSkuContext {
            plan_sku: crate::domain::scope_key::SkuId::new(plan_sku),
            index,
            introducing,
        })
        .run(&content.row);
    if let Some(report) = report.write_stage_only() {
        return Err(DomainError::ValidationFailed(report));
    }
    Ok(())
}
