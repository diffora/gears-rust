//! The SKU list on the toolkit's `OData` pager, and its tab counts (P-D-210, P-D-211).
//!
//! One field vocabulary, [`SkuListField`], serves the pager: every field a `$filter` may name and
//! every field an `$orderby` may name. The REST door publishes two narrower views of it
//! (`api::rest::sku_list`): the filter fields (no `updated_at`) and the order fields (`code`,
//! `name`, `updated_at`, and the tie-break `id`). The mapping below enforces both again, so a
//! caller that skipped the door's check still cannot filter by `updated_at` or order by a field
//! with no honest order.
//! @cpt-dod:cpt-cf-bss-products-dod-list-search:p1
use super::{driver_failure, sku_repo::sku_of};
use crate::infra::storage::{RepoError, entity::sku};
use bss_products_sdk::models::{Lifecycle, Sku, SkuType};
use sea_orm::sea_query::{BinOper, Expr, ExprTrait, Func};
use sea_orm::{
    ColumnTrait, Condition, DbBackend, DbErr, EntityTrait, FromQueryResult, QuerySelect,
};
use toolkit_db::odata::sea_orm_filter::{
    FieldToColumn, LimitCfg, ODataFieldMapping, PaginateOdataTryError, escape_like,
    paginate_odata_try,
};
use toolkit_db::secure::{AccessScope, DBRunner, SecureEntityExt};
use toolkit_odata::filter::{FieldKind, FilterField, FilterOp, ODataValue};
use toolkit_odata::{ODataOrderBy, ODataQuery, OrderKey, Page, SortDir};
use uuid::Uuid;

/// The page size when the caller names none, and the most a page holds (`$top` is clamped).
pub const SKU_PAGE: LimitCfg = LimitCfg {
    default: 50,
    max: 200,
};

/// Every field of the SKU list's pager (see the module doc for the published views).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkuListField {
    Id,
    Code,
    Name,
    Lifecycle,
    Type,
    CategoryId,
    PendingUnitId,
    UpdatedAt,
}
impl FilterField for SkuListField {
    const FIELDS: &'static [Self] = &[
        Self::Id,
        Self::Code,
        Self::Name,
        Self::Lifecycle,
        Self::Type,
        Self::CategoryId,
        Self::PendingUnitId,
        Self::UpdatedAt,
    ];
    fn name(&self) -> &'static str {
        match self {
            Self::Id => "id",
            Self::Code => "code",
            Self::Name => "name",
            Self::Lifecycle => "lifecycle",
            Self::Type => "type",
            Self::CategoryId => "category_id",
            Self::PendingUnitId => "pending_unit_id",
            Self::UpdatedAt => "updated_at",
        }
    }
    fn kind(&self) -> FieldKind {
        match self {
            Self::Id | Self::CategoryId | Self::PendingUnitId => FieldKind::Uuid,
            Self::Code | Self::Name | Self::Lifecycle | Self::Type => FieldKind::String,
            Self::UpdatedAt => FieldKind::DateTimeUtc,
        }
    }
    /// Exact names only: the list has no property paths.
    fn from_name(name: &str) -> Option<Self> {
        Self::FIELDS.iter().copied().find(|f| f.name() == name)
    }
}
impl SkuListField {
    /// Whether the column can hold no value: only these compare with `null`.
    #[must_use]
    pub const fn nullable(self) -> bool {
        matches!(self, Self::CategoryId | Self::PendingUnitId)
    }
    /// Whether the field may key an order and a cursor: never a nullable field (the cursor's
    /// comparison has no answer for a null key) and never a filter-only one.
    #[must_use]
    pub const fn orderable(self) -> bool {
        matches!(self, Self::Id | Self::Code | Self::Name | Self::UpdatedAt)
    }
}
/// How the pager reads the SKU head for each field.
pub struct SkuListMapping;
impl FieldToColumn<SkuListField> for SkuListMapping {
    type Column = sku::Column;
    fn map_field(field: SkuListField) -> sku::Column {
        match field {
            SkuListField::Id => sku::Column::Id,
            SkuListField::Code => sku::Column::Code,
            SkuListField::Name => sku::Column::Name,
            SkuListField::Lifecycle => sku::Column::Lifecycle,
            SkuListField::Type => sku::Column::Type,
            SkuListField::CategoryId => sku::Column::CategoryId,
            SkuListField::PendingUnitId => sku::Column::PendingUnitId,
            SkuListField::UpdatedAt => sku::Column::UpdatedAt,
        }
    }
    /// `updated_at` orders only: on `SQLite` a `$filter` would bind chrono's `+00:00` against
    /// the stored RFC 3339 `Z`, and a text comparison lies at the boundary. `lifecycle` and
    /// `type` take their closed values with `eq`, `ne` and `in` only. `null` compares only with
    /// a nullable field.
    fn map_value(
        field: SkuListField,
        op: FilterOp,
        value: &ODataValue,
    ) -> Result<ODataValue, String> {
        if field == SkuListField::UpdatedAt {
            return Err("`updated_at` orders the list and is not a filter field".to_owned());
        }
        if matches!(value, ODataValue::Null) {
            return if field.nullable() {
                Ok(ODataValue::Null)
            } else {
                Err(format!(
                    "`{}` always has a value; only `category_id` and `pending_unit_id` compare \
                     with null",
                    field.name()
                ))
            };
        }
        let closed: Option<fn(&str) -> bool> = match field {
            SkuListField::Lifecycle => Some(|v| Lifecycle::parse(v).is_some()),
            SkuListField::Type => Some(|v| SkuType::parse(v).is_some()),
            _ => None,
        };
        if let Some(known) = closed {
            if !matches!(op, FilterOp::Eq | FilterOp::Ne | FilterOp::In) {
                return Err(format!(
                    "`{}` takes `eq`, `ne` or `in` with one of its values",
                    field.name()
                ));
            }
            match value {
                ODataValue::String(v) if known(v) => {}
                other => return Err(format!("unknown {}: {other}", field.name())),
            }
        }
        Ok(value.clone())
    }
    fn is_orderable(field: SkuListField) -> bool {
        field.orderable()
    }
}
impl ODataFieldMapping<SkuListField> for SkuListMapping {
    type Entity = sku::Entity;
    fn extract_cursor_value(model: &sku::Model, field: SkuListField) -> sea_orm::Value {
        match field {
            SkuListField::Id => sea_orm::Value::Uuid(Some(model.id)),
            SkuListField::Code => sea_orm::Value::String(Some(model.code.clone())),
            SkuListField::Name => sea_orm::Value::String(Some(model.name.clone())),
            SkuListField::Lifecycle => sea_orm::Value::String(Some(model.lifecycle.clone())),
            SkuListField::Type => sea_orm::Value::String(Some(model.r#type.clone())),
            SkuListField::CategoryId => sea_orm::Value::Uuid(model.category_id),
            SkuListField::PendingUnitId => sea_orm::Value::Uuid(model.pending_unit_id),
            SkuListField::UpdatedAt => {
                sea_orm::Value::TimeDateTimeWithTimeZone(Some(model.updated_at))
            }
        }
    }
}

/// What the list and the counts narrow the tenant's SKUs by, besides `$filter`.
#[derive(Debug, Clone, Default)]
pub struct SkuListFilter {
    /// `q`: a case-insensitive substring of the code, name, unit, usage type or GL code.
    pub text: Option<String>,
    /// `priced`: in or out of pricing's priced set (P-D-212).
    pub priced: Option<SetFilter>,
    /// `in_plan`: in or out of pricing's in-plan set (P-D-212).
    pub in_plan: Option<SetFilter>,
}
/// Keep the SKUs in `ids` (`member`), or the SKUs outside it.
#[derive(Debug, Clone)]
pub struct SetFilter {
    pub member: bool,
    pub ids: Vec<Uuid>,
}

/// `id` in `ids`, with ONE bind whatever the set's size: a JSON array read by `json_each` on
/// `SQLite` (a UUID is stored as 16 bytes there, hence `unhex`), a `uuid[]` on Postgres.
fn membership(backend: DbBackend, ids: &[Uuid]) -> Condition {
    let expr = if backend == DbBackend::Postgres {
        let array = format!(
            "{{{}}}",
            ids.iter()
                .map(|id| id.hyphenated().to_string())
                .collect::<Vec<_>>()
                .join(",")
        );
        Expr::cust_with_values(r#""products_sku"."id" = ANY(CAST($1 AS uuid[]))"#, [array])
    } else {
        let hex: Vec<String> = ids
            .iter()
            .map(|id| id.simple().to_string().to_uppercase())
            .collect();
        let json = serde_json::Value::from(hex).to_string();
        Expr::cust_with_values(
            r#""products_sku"."id" IN (SELECT unhex("value") FROM json_each(?))"#,
            [json],
        )
    };
    Condition::all().add(expr)
}
fn set_condition(backend: DbBackend, set: &SetFilter) -> Condition {
    let inside = membership(backend, &set.ids);
    if set.member { inside } else { inside.not() }
}

/// `q` over the five text columns: `lower(column) LIKE lower(pattern) ESCAPE '\'`, the caller's
/// text matched literally (`%`, `_` and `\` escaped). Both sides go through the database's
/// `lower()`, which folds ASCII only on `SQLite` and Unicode on Postgres (P-D-210). An unset
/// column never matches.
fn text_condition(text: &str) -> Condition {
    let pattern = format!("%{}%", escape_like(text));
    [
        sku::Column::Code,
        sku::Column::Name,
        sku::Column::Unit,
        sku::Column::UsageTypeRef,
        sku::Column::GlCode,
    ]
    .into_iter()
    .fold(Condition::any(), |any, column| {
        // `LikeExpr` binds its pattern as it is; the pattern here goes through `lower()` too,
        // so the `LIKE … ESCAPE` is spelled as the nested binary `LikeExpr` itself builds.
        let lowered = Expr::expr(Func::lower(Expr::val(pattern.clone()))).binary(
            BinOper::Escape,
            Expr::Constant(sea_orm::Value::Char(Some('\\'))),
        );
        any.add(
            Expr::expr(Func::lower(Expr::col((sku::Entity, column))))
                .binary(BinOper::Like, lowered),
        )
    })
}

/// The tenant's SKUs narrowed by `filter`, before `$filter`, the cursor and the order.
#[must_use]
pub fn list_condition(tenant: Uuid, filter: &SkuListFilter, backend: DbBackend) -> Condition {
    let mut c = Condition::all().add(sku::Column::TenantId.eq(tenant));
    if let Some(text) = filter.text.as_deref() {
        c = c.add(text_condition(text));
    }
    for set in [&filter.priced, &filter.in_plan].into_iter().flatten() {
        c = c.add(set_condition(backend, set));
    }
    c
}

/// A list read refused or failed.
#[derive(Debug)]
pub enum SkuListError {
    /// The query itself: a filter value, an order field, a cursor (400).
    Query(toolkit_odata::Error),
    /// Storage; a driver failure keeps its message for the retry classifier.
    Repo(RepoError),
}

/// One page of the tenant's SKUs: `filter`'s narrowing, then the query's `$filter`, cursor and
/// order (`code` when it names none), tie-broken by `id`; `$top` defaults to 50 and is clamped
/// at 200.
/// # Errors
/// [`SkuListError::Query`] for a value, order field or cursor the pager refuses;
/// [`SkuListError::Repo`] for storage and corrupt rows.
pub async fn page_skus(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    backend: DbBackend,
    filter: &SkuListFilter,
    query: &ODataQuery,
) -> Result<Page<Sku>, SkuListError> {
    let mut query = query.clone();
    if query.cursor.is_none() && query.order.0.is_empty() {
        query.order = ODataOrderBy(vec![OrderKey {
            field: SkuListField::Code.name().to_owned(),
            dir: SortDir::Asc,
        }]);
    }
    let select = sku::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(list_condition(tenant, filter, backend));
    paginate_odata_try::<SkuListField, SkuListMapping, sku::Entity, Sku, _, RepoError, _>(
        select,
        runner,
        &query,
        (SkuListField::Id.name(), SortDir::Asc),
        SKU_PAGE,
        sku_of,
    )
    .await
    .map_err(|e| match e {
        // The pager renders the driver's error as text; kept as a driver failure so the
        // door's retry still sees a serialization failure or a busy database by its message.
        PaginateOdataTryError::OData(toolkit_odata::Error::Db(message)) => {
            SkuListError::Repo(RepoError::Driver {
                context: "list SKUs".into(),
                source: DbErr::Custom(message),
            })
        }
        PaginateOdataTryError::OData(other) => SkuListError::Query(other),
        PaginateOdataTryError::MapError(e) => SkuListError::Repo(e),
    })
}

/// The tab counts of the list: every SKU, those in each lifecycle, and those in review.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SkuCounts {
    pub all: u64,
    pub draft: u64,
    pub published: u64,
    pub deprecated: u64,
    pub retiring: u64,
    pub retired: u64,
    /// SKUs a pending approval unit locks (`pending_unit_id` set), in any lifecycle.
    pub in_review: u64,
}
#[derive(Debug, FromQueryResult)]
struct LifecycleCount {
    lifecycle: String,
    n: i64,
    in_review: i64,
}
fn count(v: i64) -> Result<u64, RepoError> {
    u64::try_from(v).map_err(|_| RepoError::CorruptRow(format!("a negative count {v}")))
}

/// The counts of the tenant's SKUs narrowed by `filter` and by `condition` (the door's
/// `$filter` without its `lifecycle` terms), in ONE grouped statement.
/// # Errors
/// Storage failures; a stored lifecycle outside the five is a corrupt row.
pub async fn count_skus(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    backend: DbBackend,
    filter: &SkuListFilter,
    condition: Option<Condition>,
) -> Result<SkuCounts, RepoError> {
    let mut c = list_condition(tenant, filter, backend);
    if let Some(condition) = condition {
        c = c.add(condition);
    }
    let rows = sku::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(c)
        .project_all(runner, |q| {
            q.select_only()
                .column(sku::Column::Lifecycle)
                .column_as(Expr::col((sku::Entity, sku::Column::Id)).count(), "n")
                .column_as(
                    Expr::col((sku::Entity, sku::Column::PendingUnitId)).count(),
                    "in_review",
                )
                .group_by(sku::Column::Lifecycle)
                .into_model::<LifecycleCount>()
        })
        .await
        .map_err(|e| driver_failure("count SKUs".into(), e))?;
    let mut counts = SkuCounts::default();
    for row in rows {
        let n = count(row.n)?;
        let slot = match Lifecycle::parse(&row.lifecycle) {
            Some(Lifecycle::Draft) => &mut counts.draft,
            Some(Lifecycle::Published) => &mut counts.published,
            Some(Lifecycle::Deprecated) => &mut counts.deprecated,
            Some(Lifecycle::Retiring) => &mut counts.retiring,
            Some(Lifecycle::Retired) => &mut counts.retired,
            None => {
                return Err(RepoError::CorruptRow(format!(
                    "SKU lifecycle {}",
                    row.lifecycle
                )));
            }
        };
        *slot += n;
        counts.all += n;
        counts.in_review += count(row.in_review)?;
    }
    Ok(counts)
}

#[cfg(test)]
#[path = "sku_list_repo_tests.rs"]
mod sku_list_repo_tests;
