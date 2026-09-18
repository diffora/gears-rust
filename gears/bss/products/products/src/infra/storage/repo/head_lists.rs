//! Scoped keyset lists of current Product and SKU heads, including every lifecycle state.
//! These queries are independent of the asynchronously published catalog projection.

use sea_orm::{ColumnTrait, EntityTrait};
use toolkit_db::odata::sea_orm_filter::{
    FieldToColumn, LimitCfg, ODataFieldMapping, PaginateOdataTryError, paginate_odata_try,
};
use toolkit_db::secure::{AccessScope, DBRunner, SecureEntityExt};
use toolkit_odata::{ODataQuery, Page, SortDir};
use toolkit_odata_macros::ODataFilterable;
use uuid::Uuid;

use super::{ProductRecord, SkuRecord, effective_odata, into_product_record, into_sku_record};
use crate::infra::storage::{
    RepoError,
    entity::{product, sku},
};

/// Distinguishes invalid queries from malformed persisted heads.
pub type HeadListError = PaginateOdataTryError<RepoError>;

/// Filterable fields of the product authoring collection. Tenant scope is never a filter input.
#[derive(ODataFilterable)]
#[allow(dead_code, reason = "declaration consumed by ODataFilterable")]
pub struct ProductHeadQuery {
    #[odata(filter(kind = "Uuid"))]
    pub product_id: Uuid,
    #[odata(filter(kind = "Uuid"))]
    pub brand_id: Uuid,
    #[odata(filter(kind = "String"))]
    pub name: String,
    #[odata(filter(kind = "String"))]
    pub product_code: String,
    #[odata(filter(kind = "String"))]
    pub lifecycle_state: String,
    #[odata(filter(kind = "I64"))]
    pub internal_revision: i64,
    #[odata(filter(kind = "I64"))]
    pub published_version: i64,
}

pub use ProductHeadQueryFilterField as ProductHeadFilterField;

/// Maps the wire vocabulary to the current head table.
struct ProductHeadMapper;

impl FieldToColumn<ProductHeadFilterField> for ProductHeadMapper {
    type Column = product::Column;

    fn map_field(field: ProductHeadFilterField) -> Self::Column {
        match field {
            ProductHeadFilterField::ProductId => product::Column::ProductId,
            ProductHeadFilterField::BrandId => product::Column::BrandId,
            ProductHeadFilterField::Name => product::Column::Name,
            ProductHeadFilterField::ProductCode => product::Column::ProductCode,
            ProductHeadFilterField::LifecycleState => product::Column::LifecycleState,
            ProductHeadFilterField::InternalRevision => product::Column::InternalRevision,
            ProductHeadFilterField::PublishedVersion => product::Column::PublishedVersion,
        }
    }

    // Nullable keys cannot be safely continued by the platform's keyset predicate.
    fn is_orderable(field: ProductHeadFilterField) -> bool {
        !matches!(field, ProductHeadFilterField::ProductCode)
    }
}

impl ODataFieldMapping<ProductHeadFilterField> for ProductHeadMapper {
    type Entity = product::Entity;

    fn extract_cursor_value(
        model: &product::Model,
        field: ProductHeadFilterField,
    ) -> sea_orm::Value {
        match field {
            ProductHeadFilterField::ProductId => sea_orm::Value::from(model.product_id),
            ProductHeadFilterField::BrandId => sea_orm::Value::from(model.brand_id),
            ProductHeadFilterField::Name => sea_orm::Value::from(model.name.clone()),
            ProductHeadFilterField::ProductCode => sea_orm::Value::from(model.product_code.clone()),
            ProductHeadFilterField::LifecycleState => {
                sea_orm::Value::from(model.lifecycle_state.clone())
            }
            ProductHeadFilterField::InternalRevision => {
                sea_orm::Value::from(model.internal_revision)
            }
            ProductHeadFilterField::PublishedVersion => {
                sea_orm::Value::from(model.published_version)
            }
        }
    }
}

/// List current product heads under both the PDP scope and the caller's tenant.
///
/// # Errors
/// Invalid query/cursor errors and storage failures retain their distinct categories.
pub async fn list_products_page(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    odata: &ODataQuery,
    limits: LimitCfg,
) -> Result<Page<ProductRecord>, HeadListError> {
    let base = product::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(product::Column::TenantId.eq(tenant_id).into());
    let effective = effective_odata(
        odata,
        ("name", SortDir::Asc),
        Some(&format!("products:{tenant_id}")),
    );
    // New collection cursors always carry a stamp, including an unfiltered walk.
    // Refuse a stripped stamp rather than bypassing the helper's optional comparison.
    if effective
        .cursor
        .as_ref()
        .is_some_and(|cursor| cursor.f.is_none())
    {
        return Err(toolkit_odata::Error::InvalidCursor.into());
    }
    paginate_odata_try::<ProductHeadFilterField, ProductHeadMapper, _, _, _, _, _>(
        base,
        runner,
        &effective,
        ("product_id", SortDir::Asc),
        limits,
        into_product_record,
    )
    .await
}

/// Filterable fields of the sku authoring collection. Tenant scope is never a filter input.
#[derive(ODataFilterable)]
#[allow(dead_code, reason = "declaration consumed by ODataFilterable")]
pub struct SkuHeadQuery {
    #[odata(filter(kind = "Uuid"))]
    pub sku_id: Uuid,
    #[odata(filter(kind = "Uuid"))]
    pub product_id: Uuid,
    #[odata(filter(kind = "String"))]
    pub sku_code: String,
    #[odata(filter(kind = "String"))]
    pub lifecycle_state: String,
    #[odata(filter(kind = "String"))]
    pub sku_type: String,
    #[odata(filter(kind = "Bool"))]
    pub sellable: bool,
    #[odata(filter(kind = "I64"))]
    pub internal_revision: i64,
    #[odata(filter(kind = "I64"))]
    pub published_version: i64,
}

pub use SkuHeadQueryFilterField as SkuHeadFilterField;

/// Maps the wire vocabulary to the current head table.
struct SkuHeadMapper;

impl FieldToColumn<SkuHeadFilterField> for SkuHeadMapper {
    type Column = sku::Column;

    fn map_field(field: SkuHeadFilterField) -> Self::Column {
        match field {
            SkuHeadFilterField::SkuId => sku::Column::SkuId,
            SkuHeadFilterField::ProductId => sku::Column::ProductId,
            SkuHeadFilterField::SkuCode => sku::Column::SkuCode,
            SkuHeadFilterField::LifecycleState => sku::Column::LifecycleState,
            SkuHeadFilterField::SkuType => sku::Column::SkuType,
            SkuHeadFilterField::Sellable => sku::Column::Sellable,
            SkuHeadFilterField::InternalRevision => sku::Column::InternalRevision,
            SkuHeadFilterField::PublishedVersion => sku::Column::PublishedVersion,
        }
    }

    // Nullable keys cannot be safely continued by the platform's keyset predicate.
    fn is_orderable(field: SkuHeadFilterField) -> bool {
        !matches!(field, SkuHeadFilterField::SkuType)
    }
}

impl ODataFieldMapping<SkuHeadFilterField> for SkuHeadMapper {
    type Entity = sku::Entity;

    fn extract_cursor_value(model: &sku::Model, field: SkuHeadFilterField) -> sea_orm::Value {
        match field {
            SkuHeadFilterField::SkuId => sea_orm::Value::from(model.sku_id),
            SkuHeadFilterField::ProductId => sea_orm::Value::from(model.product_id),
            SkuHeadFilterField::SkuCode => sea_orm::Value::from(model.sku_code.clone()),
            SkuHeadFilterField::LifecycleState => {
                sea_orm::Value::from(model.lifecycle_state.clone())
            }
            SkuHeadFilterField::SkuType => sea_orm::Value::from(model.sku_type.clone()),
            SkuHeadFilterField::Sellable => sea_orm::Value::from(model.sellable),
            SkuHeadFilterField::InternalRevision => sea_orm::Value::from(model.internal_revision),
            SkuHeadFilterField::PublishedVersion => sea_orm::Value::from(model.published_version),
        }
    }
}

/// List current sku heads under both the PDP scope and the caller's tenant.
///
/// # Errors
/// Invalid query/cursor errors and storage failures retain their distinct categories.
pub async fn list_skus_page(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    odata: &ODataQuery,
    limits: LimitCfg,
) -> Result<Page<SkuRecord>, HeadListError> {
    let base = sku::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(sku::Column::TenantId.eq(tenant_id).into());
    let effective = effective_odata(
        odata,
        ("sku_code", SortDir::Asc),
        Some(&format!("skus:{tenant_id}")),
    );
    // New collection cursors always carry a stamp, including an unfiltered walk.
    // Refuse a stripped stamp rather than bypassing the helper's optional comparison.
    if effective
        .cursor
        .as_ref()
        .is_some_and(|cursor| cursor.f.is_none())
    {
        return Err(toolkit_odata::Error::InvalidCursor.into());
    }
    paginate_odata_try::<SkuHeadFilterField, SkuHeadMapper, _, _, _, _, _>(
        base,
        runner,
        &effective,
        ("sku_id", SortDir::Asc),
        limits,
        into_sku_record,
    )
    .await
}
