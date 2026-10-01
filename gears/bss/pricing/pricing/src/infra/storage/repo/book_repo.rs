//! Scoped book persistence with conditional versions, and the book list's pager (D-442).
use super::{driver_failure, map_unique, matched};
use crate::infra::storage::{RepoError, entity::price_book as e};
use sea_orm::sea_query::{Expr, ExprTrait};
use sea_orm::{ColumnTrait, Condition, EntityTrait, Order, Set};
use toolkit_db::odata::sea_orm_filter::{
    FieldToColumn, LimitCfg, ODataFieldMapping, PaginateOdataTryError, escape_like,
    paginate_odata_try,
};
use toolkit_db::secure::{
    AccessScope, DBRunner, SecureEntityExt, SecureInsertExt, SecureUpdateExt,
};
use toolkit_odata::filter::{FieldKind, FilterField};
use toolkit_odata::{ODataOrderBy, ODataQuery, OrderKey, Page, SortDir};
use uuid::Uuid;
fn key(tenant: Uuid, id: Uuid) -> Condition {
    Condition::all()
        .add(e::Column::TenantId.eq(tenant))
        .add(e::Column::Id.eq(id))
}
/// Insert a tenant-scoped row in the caller's transaction.
/// # Errors
/// Returns unique conflicts, parent ownership refusals or typed database failures.
pub async fn insert(
    runner: &impl DBRunner,
    scope: &AccessScope,
    m: e::Model,
) -> Result<e::Model, RepoError> {
    let active = e::ActiveModel {
        id: Set(m.id),
        tenant_id: Set(m.tenant_id),
        code: Set(m.code),
        name: Set(m.name),
        currency: Set(m.currency),
        valid_from: Set(m.valid_from),
        valid_until: Set(m.valid_until),
        description: Set(m.description),
        version: Set(m.version),
        created_at: Set(m.created_at),
        updated_at: Set(m.updated_at),
    };
    e::Entity::insert(active.clone())
        .secure()
        .scope_with_model(scope, &active)
        .map_err(|e| driver_failure("insert scope".into(), e))?
        .exec_with_returning(runner)
        .await
        .map_err(|e| map_unique("insert price_book".into(), e))
}
/// Read by tenant and identity within the authorized scope.
/// # Errors
/// Returns typed database failures.
pub async fn find(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
) -> Result<Option<e::Model>, RepoError> {
    e::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(key(tenant, id))
        .one(runner)
        .await
        .map_err(|e| driver_failure("find price_book".into(), e))
}
/// List tenant rows in stable identity order.
/// # Errors
/// Returns typed database failures.
pub async fn list(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
) -> Result<Vec<e::Model>, RepoError> {
    e::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(e::Column::TenantId.eq(tenant)))
        .order_by(e::Column::Code, Order::Asc)
        .all(runner)
        .await
        .map_err(|e| driver_failure("list price_book".into(), e))
}
/// The tenant's books among `ids`, in ONE statement (D-428).
/// # Errors
/// Returns typed database failures.
pub async fn find_many(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    ids: &[Uuid],
) -> Result<Vec<e::Model>, RepoError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    e::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(e::Column::TenantId.eq(tenant))
                .add(e::Column::Id.is_in(ids.iter().copied())),
        )
        .order_by(e::Column::Id, Order::Asc)
        .all(runner)
        .await
        .map_err(|e| driver_failure("list price books by id".into(), e))
}
/// Change business columns only if the caller's version still owns the row.
/// # Errors
/// Zero matches is a typed version conflict; database failures preserve their type.
pub async fn update(
    runner: &impl DBRunner,
    scope: &AccessScope,
    m: e::Model,
) -> Result<(), RepoError> {
    let predicate = key(m.tenant_id, m.id).add(e::Column::Version.eq(m.version));
    let result = e::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(e::Column::Name, Expr::value(m.name))
        .col_expr(e::Column::ValidFrom, Expr::value(m.valid_from))
        .col_expr(e::Column::ValidUntil, Expr::value(m.valid_until))
        .col_expr(e::Column::Description, Expr::value(m.description))
        .col_expr(e::Column::UpdatedAt, Expr::value(m.updated_at))
        .col_expr(e::Column::Version, Expr::col(e::Column::Version).add(1_i64))
        .filter(predicate)
        .exec(runner)
        .await
        .map_err(|e| map_unique("update price_book".into(), e))?;
    matched(result.rows_affected, "STALE_REVISION")
}
/// The book's foreign keys, by the name Postgres gives them, and the conflict a delete that meets
/// each one is (D-444): an entry's is `BOOK_HAS_ENTRIES`, a plan revision's `BOOK_IN_PLAN`.
const REFERENCED_BY: [(&str, &str); 2] = [
    ("pricing_price_book_entry_book_id_fkey", "BOOK_HAS_ENTRIES"),
    ("pricing_plan_revision_book_id_fkey", "BOOK_IN_PLAN"),
];
/// Delete a book at the version the caller read (D-444), in the caller's transaction. The door
/// judges first that no entry and no plan revision names it; a row that a concurrent writer adds
/// after those reads meets the book's foreign key here, and the conflict it names is the door's
/// 409, never a 500: Postgres waits for the writer and fails the delete on the key it names
/// (`postgres_book_writes.rs` measures it). `SQLite` names no key, and there one writer holds the
/// database for the whole transaction, so the door's own reads saw every row: its foreign-key
/// failure stays a storage failure.
/// # Errors
/// `STALE_REVISION` when no row matched (another version, or gone); `BOOK_HAS_ENTRIES` or
/// `BOOK_IN_PLAN` for a named foreign key; database failures keep their type.
pub async fn delete(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
    version: i64,
) -> Result<(), RepoError> {
    use toolkit_db::secure::SecureDeleteExt;
    let result = e::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(key(tenant, id).add(e::Column::Version.eq(version)))
        .exec(runner)
        .await
        .map_err(|error| {
            if error.is_foreign_key_violation() {
                let message = error.to_string();
                if let Some((_, code)) = REFERENCED_BY.iter().find(|(k, _)| message.contains(k)) {
                    return RepoError::Conflict { code };
                }
            }
            driver_failure("delete price_book".into(), error)
        })?;
    matched(result.rows_affected, "STALE_REVISION")
}

// ------------------------------------------------------------------ the book list (D-442)

/// The page size when the caller names none, and the most a page holds (`$top` is clamped): the
/// categories' 200, so a tenant's books stay on one page (D-442).
pub const BOOK_PAGE: LimitCfg = LimitCfg {
    default: 200,
    max: 500,
};

/// Every field of the book list's pager: the filter fields the door publishes (`id`, `code`,
/// `name`, `currency`, `valid_from`, `valid_until`) and the order fields (`code`, `name` and the
/// tie-break `id`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BookListField {
    Id,
    Code,
    Name,
    Currency,
    ValidFrom,
    ValidUntil,
}
impl FilterField for BookListField {
    const FIELDS: &'static [Self] = &[
        Self::Id,
        Self::Code,
        Self::Name,
        Self::Currency,
        Self::ValidFrom,
        Self::ValidUntil,
    ];
    fn name(&self) -> &'static str {
        match self {
            Self::Id => "id",
            Self::Code => "code",
            Self::Name => "name",
            Self::Currency => "currency",
            Self::ValidFrom => "valid_from",
            Self::ValidUntil => "valid_until",
        }
    }
    fn kind(&self) -> FieldKind {
        match self {
            Self::Id => FieldKind::Uuid,
            Self::Code | Self::Name | Self::Currency => FieldKind::String,
            Self::ValidFrom | Self::ValidUntil => FieldKind::Date,
        }
    }
    /// The two validity dates may be unset: only they compare with `null`.
    fn nullable(&self) -> bool {
        matches!(self, Self::ValidFrom | Self::ValidUntil)
    }
    fn from_name(name: &str) -> Option<Self> {
        Self::FIELDS.iter().copied().find(|f| f.name() == name)
    }
}
impl BookListField {
    /// Whether the field may key an order and a cursor: never a nullable one (a cursor has no
    /// answer for a null key), never `currency` (it filters only).
    #[must_use]
    pub const fn orderable(self) -> bool {
        matches!(self, Self::Id | Self::Code | Self::Name)
    }
}
/// How the pager reads the book row for each field.
pub struct BookListMapping;
impl FieldToColumn<BookListField> for BookListMapping {
    type Column = e::Column;
    fn map_field(field: BookListField) -> e::Column {
        match field {
            BookListField::Id => e::Column::Id,
            BookListField::Code => e::Column::Code,
            BookListField::Name => e::Column::Name,
            BookListField::Currency => e::Column::Currency,
            BookListField::ValidFrom => e::Column::ValidFrom,
            BookListField::ValidUntil => e::Column::ValidUntil,
        }
    }
    fn is_orderable(field: BookListField) -> bool {
        field.orderable()
    }
}
impl ODataFieldMapping<BookListField> for BookListMapping {
    type Entity = e::Entity;
    fn extract_cursor_value(model: &e::Model, field: BookListField) -> sea_orm::Value {
        match field {
            BookListField::Id => sea_orm::Value::Uuid(Some(model.id)),
            BookListField::Code => sea_orm::Value::String(Some(model.code.clone())),
            BookListField::Name => sea_orm::Value::String(Some(model.name.clone())),
            BookListField::Currency => sea_orm::Value::String(Some(model.currency.clone())),
            BookListField::ValidFrom => sea_orm::Value::TimeDate(model.valid_from),
            BookListField::ValidUntil => sea_orm::Value::TimeDate(model.valid_until),
        }
    }
}

/// What the list narrows the tenant's books by, besides `$filter`.
#[derive(Debug, Clone, Default)]
pub struct BookListFilter {
    /// `q`: a case-insensitive substring of the code or the name, matched literally.
    pub text: Option<String>,
    /// `sku_id`: the books with an entry of this SKU.
    pub sku: Option<Uuid>,
}

/// The collation `q` folds case through on Postgres: ICU's root locale, which folds Unicode
/// whatever the database's own locale is (a `C` database's `lower()` folds ASCII only), as
/// products' SKU list folds it (P-D-210). A deployment's Postgres must be built with ICU.
pub const PG_FOLD_COLLATION: &str = "und-x-icu";

/// `lower(expr)`, folded through [`PG_FOLD_COLLATION`] on Postgres and through the database's own
/// `lower()` (ASCII only) on `SQLite`.
//
// Raw SQL, on purpose (whole-branch review PS-29): sea-query has no `COLLATE` on an expression,
// and the fold must name the ICU collation, or a `C`-locale database folds ASCII only (products
// P-D-210). The folded text stays a bound value.
// Upstream gears use the same pattern in repository code: account-management
// `infra/lease/manager.rs` (`Expr::cust("NOW()")`, `INTERVAL`) and
// `infra/storage/repo_impl/retention.rs` (`make_interval`, `julianday`), and settings-service
// `infra/storage/search_repo.rs` (`LIKE … ESCAPE`, the JSON null checks). A toolkit-db helper
// would be a change to a foreign crate, proposed upstream on its own (owner, O3/O4).
fn folded(backend: sea_orm::DbBackend, expr: Expr) -> Expr {
    if backend == sea_orm::DbBackend::Postgres {
        Expr::cust_with_expr(format!(r#"lower($1 COLLATE "{PG_FOLD_COLLATION}")"#), expr)
    } else {
        Expr::expr(sea_orm::sea_query::Func::lower(expr))
    }
}

/// `q` over the code and the name: `lower(column) LIKE lower(pattern) ESCAPE '\'`, the caller's
/// text matched literally (`%`, `_` and `\` escaped), both sides folded the same way.
fn text_condition(text: &str, backend: sea_orm::DbBackend) -> Condition {
    use sea_orm::sea_query::BinOper;
    let pattern = format!("%{}%", escape_like(text));
    [e::Column::Code, e::Column::Name]
        .into_iter()
        .fold(Condition::any(), |any, column| {
            // `LikeExpr` binds its pattern as it is; this pattern goes through `lower()` too, so
            // the `LIKE … ESCAPE` is spelled as the nested binary `LikeExpr` itself builds.
            let lowered = folded(backend, Expr::val(pattern.clone())).binary(
                BinOper::Escape,
                Expr::Constant(sea_orm::Value::Char(Some('\\'))),
            );
            any.add(folded(backend, Expr::col((e::Entity, column))).binary(BinOper::Like, lowered))
        })
}

/// A list read refused or failed.
#[derive(Debug)]
pub enum BookListError {
    /// The query itself: a filter value, an order field, a cursor (400).
    Query(toolkit_odata::Error),
    /// Storage; a driver failure keeps its message for the retry classifier.
    Repo(RepoError),
}

/// One page of the tenant's books under `scope` (D-442): `filter`'s narrowing, then the query's
/// `$filter`, cursor and order (`code` when it names none), tie-broken by `id`; `$top` defaults
/// to 200 and is clamped at 500. ONE statement.
/// # Errors
/// [`BookListError::Query`] for a value, order field or cursor the pager refuses;
/// [`BookListError::Repo`] for storage.
pub async fn page(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    backend: sea_orm::DbBackend,
    filter: &BookListFilter,
    query: &ODataQuery,
) -> Result<Page<e::Model>, BookListError> {
    let mut query = query.clone();
    if query.cursor.is_none() && query.order.0.is_empty() {
        query.order = ODataOrderBy(vec![OrderKey {
            field: BookListField::Code.name().to_owned(),
            dir: SortDir::Asc,
        }]);
    }
    let mut c = Condition::all().add(e::Column::TenantId.eq(tenant));
    if let Some(text) = filter.text.as_deref() {
        c = c.add(text_condition(text, backend));
    }
    if let Some(sku) = filter.sku {
        c = c.add(
            Expr::col((e::Entity, e::Column::Id))
                .in_subquery(super::price_book_entry_repo::books_pricing(tenant, sku)),
        );
    }
    let select = e::Entity::find().secure().scope_with(scope).filter(c);
    paginate_odata_try::<BookListField, BookListMapping, e::Entity, e::Model, _, RepoError, _>(
        select,
        runner,
        &query,
        (BookListField::Id.name(), SortDir::Asc),
        BOOK_PAGE,
        Ok,
    )
    .await
    .map_err(|e| match e {
        // The pager renders the driver's error as text; kept as a driver failure so the door's
        // retry still sees a serialization failure or a busy database by its message.
        PaginateOdataTryError::OData(toolkit_odata::Error::Db(message)) => {
            BookListError::Repo(RepoError::Driver {
                context: "list price books".into(),
                source: sea_orm::DbErr::Custom(message),
            })
        }
        PaginateOdataTryError::OData(other) => BookListError::Query(other),
        PaginateOdataTryError::MapError(e) => BookListError::Repo(e),
    })
}
