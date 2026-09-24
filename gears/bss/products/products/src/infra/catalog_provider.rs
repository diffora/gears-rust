//! Authorized pricing catalog reads over the SKU heads.
use crate::{
    api::rest::authz_error_to_canonical,
    authz::{access_scope, actions, resource_types},
    domain::{error::DomainError, validation::ValidationReport},
    infra::storage::{entity::sku, repo},
};
use async_trait::async_trait;
use authz_resolver_sdk::PolicyEnforcer;
use bss_pricing_sdk::product_catalog::{
    CatalogSku, CatalogSkuPage, CatalogTaxCategory, ProductCatalogClientV1, catalog_unreachable,
};
use bss_products_sdk::models::{Lifecycle, Sku};
use sea_orm::{ColumnTrait, Condition};
use std::{collections::BTreeSet, sync::Arc};
use toolkit_canonical_errors::{CanonicalError, resource_error};
#[resource_error(gts_id!("cf.bss.products.sku.v1~"))]
struct SkuResource;
use toolkit_db::odata::sea_orm_filter::{FieldToColumn, filter_node_to_condition};
use toolkit_db::{Db, secure::AccessScope};
use toolkit_odata::filter::{FieldKind, FilterField, parse_odata_filter};
use toolkit_security::SecurityContext;
use uuid::Uuid;

/// In-process catalog transport with the same PDP as the REST door.
#[derive(Clone)]
pub struct BrowseCatalogProvider {
    db: Db,
    enforcer: Arc<PolicyEnforcer>,
}
impl BrowseCatalogProvider {
    /// Bind reads to this database and the caller's authorization policy.
    #[must_use]
    pub fn new(db: Db, enforcer: Arc<PolicyEnforcer>) -> Self {
        Self { db, enforcer }
    }

    pub(crate) async fn scope(&self, ctx: &SecurityContext) -> Result<AccessScope, CanonicalError> {
        access_scope(
            &self.enforcer,
            ctx,
            &resource_types::SKU,
            actions::READ,
            None,
            None,
            true,
        )
        .await
        .map_err(|e| {
            authz_error_to_canonical(e, |reason| {
                SkuResource::permission_denied()
                    .with_reason(reason)
                    .create()
            })
        })
    }

    /// Browse the served lifecycle set with a validated `OData` predicate.
    pub(crate) async fn browse(
        &self,
        ctx: &SecurityContext,
        filter: Option<&str>,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<CatalogSkuPage, CanonicalError> {
        let scope = self.scope(ctx).await?;
        let limit = limit.min(200);
        if limit == 0 {
            return Err(invalid("limit", "limit must be at least one"));
        }
        let mut condition =
            Condition::all().add(sku::Column::Lifecycle.is_in(["published", "deprecated"]));
        if let Some(filter) = filter.filter(|s| !s.is_empty()) {
            if filter.len() > 32_768 {
                return Err(invalid("$filter", "filter is too long"));
            }
            let parsed = parse_odata_filter::<CatalogField>(filter)
                .map_err(|e| invalid("$filter", e.to_string()))?;
            condition = condition.add(
                filter_node_to_condition::<CatalogField, CatalogMapping>(&parsed)
                    .map_err(|e| invalid("$filter", e))?,
            );
        }
        let query = repo::SkuQuery {
            catalog_filter: Some(condition),
            text: None,
            r#type: None,
            category_id: None,
            lifecycle: None,
            limit: u64::from(limit),
            after_code: cursor.filter(|s| !s.is_empty()).map(str::to_owned),
        };
        let conn = self
            .db
            .conn()
            .map_err(|e| catalog_unreachable(e.to_string()))?;
        let mut rows = repo::list_skus(&conn, &scope, ctx.subject_tenant_id(), &query)
            .await
            .map_err(|e| catalog_unreachable(e.to_string()))?;
        let more = rows.len() > limit as usize;
        rows.truncate(limit as usize);
        let next_cursor = if more {
            rows.last().map(|s| s.code.clone())
        } else {
            None
        };
        Ok(CatalogSkuPage {
            items: rows.into_iter().map(catalog_sku_of).collect(),
            next_cursor,
        })
    }
}

pub(crate) fn invalid(field: &str, detail: impl Into<String>) -> CanonicalError {
    let mut report = ValidationReport::new();
    report.violate("VALIDATION", field, detail);
    DomainError::Validation(report).into()
}

fn catalog_sku_of(s: Sku) -> CatalogSku {
    CatalogSku {
        sku_id: s.id,
        sku_code: s.code,
        name: s.name,
        metering_unit: s.unit,
        status: s.lifecycle.as_str().to_owned(),
        plan_tier: None,
        sku_type: s.r#type.as_str().to_owned(),
        sellable: s.sellable,
        usage_type_ref: s.usage_type_ref,
        deprecated: s.lifecycle == Lifecycle::Deprecated,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum CatalogField {
    Id,
    Code,
    Name,
}
impl FilterField for CatalogField {
    const FIELDS: &'static [Self] = &[Self::Id, Self::Code, Self::Name];
    fn name(&self) -> &'static str {
        match self {
            Self::Id => "entity_id",
            Self::Code => "sku_code",
            Self::Name => "name",
        }
    }
    fn kind(&self) -> FieldKind {
        match self {
            Self::Id => FieldKind::Uuid,
            Self::Code | Self::Name => FieldKind::String,
        }
    }
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "entity_id" | "sku_id" => Some(Self::Id),
            "entity_code" | "sku_code" => Some(Self::Code),
            "name" => Some(Self::Name),
            _ => None,
        }
    }
}
struct CatalogMapping;
impl FieldToColumn<CatalogField> for CatalogMapping {
    type Column = sku::Column;
    fn map_field(field: CatalogField) -> Self::Column {
        match field {
            CatalogField::Id => sku::Column::Id,
            CatalogField::Code => sku::Column::Code,
            CatalogField::Name => sku::Column::Name,
        }
    }
}

#[async_trait]
impl ProductCatalogClientV1 for BrowseCatalogProvider {
    async fn get_skus(
        &self,
        ctx: &SecurityContext,
        ids: &[Uuid],
    ) -> Result<Vec<CatalogSku>, CanonicalError> {
        let scope = self.scope(ctx).await?;
        let conn = self
            .db
            .conn()
            .map_err(|e| catalog_unreachable(e.to_string()))?;
        let mut rows = Vec::new();
        let mut seen = BTreeSet::new();
        for &id in ids {
            if !seen.insert(id) {
                continue;
            }
            if let Some(s) = repo::find_sku(&conn, &scope, ctx.subject_tenant_id(), id)
                .await
                .map_err(|e| catalog_unreachable(e.to_string()))?
                && matches!(s.lifecycle, Lifecycle::Published | Lifecycle::Deprecated)
            {
                rows.push(s);
            }
        }
        rows.sort_by(|a, b| a.code.cmp(&b.code));
        Ok(rows.into_iter().map(catalog_sku_of).collect())
    }
    async fn search_skus(
        &self,
        ctx: &SecurityContext,
        q: Option<&str>,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<CatalogSkuPage, CanonicalError> {
        let filter = q
            .filter(|s| !s.is_empty())
            .map(|q| format!("startswith(name,'{}')", q.replace('\'', "''")));
        self.browse(ctx, filter.as_deref(), limit, cursor).await
    }
    async fn list_tax_categories(
        &self,
        ctx: &SecurityContext,
    ) -> Result<Vec<CatalogTaxCategory>, CanonicalError> {
        let scope = self.scope(ctx).await?;
        let conn = self
            .db
            .conn()
            .map_err(|e| catalog_unreachable(e.to_string()))?;
        let mut query = repo::SkuQuery {
            catalog_filter: None,
            text: None,
            r#type: None,
            category_id: None,
            lifecycle: Some(Lifecycle::Published),
            limit: 200,
            after_code: None,
        };
        let mut codes = BTreeSet::new();
        loop {
            let mut rows = repo::list_skus(&conn, &scope, ctx.subject_tenant_id(), &query)
                .await
                .map_err(|e| catalog_unreachable(e.to_string()))?;
            let more = rows.len() > 200;
            rows.truncate(200);
            query.after_code = rows.last().map(|s| s.code.clone());
            codes.extend(rows.into_iter().filter_map(|s| s.tax_category));
            if !more {
                break;
            }
        }
        Ok(codes
            .into_iter()
            .map(|code| CatalogTaxCategory {
                display_name: code.clone(),
                code,
            })
            .collect())
    }
}
#[cfg(test)]
#[path = "catalog_provider_tests.rs"]
mod catalog_provider_tests;
