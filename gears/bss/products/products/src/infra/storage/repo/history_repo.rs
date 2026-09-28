//! A SKU's history on the toolkit's `OData` pager (P-D-213): the audit rows whose subject is the
//! SKU, and those whose subject is one of the SKU's approval units (`ref_id` the SKU), ordered by
//! `audit_id` alone. Every writer mints it as a UUID v7 inside the act's transaction (per attempt),
//! so its order is the order the acts wrote — bytes on `SQLite`, `uuid` on Postgres, both compared
//! in time order. `written_at` is not the order: it is the instant the act began, taken before its
//! transaction and kept across a retry, and on `SQLite` its RFC 3339 text does not sort as time
//! within one second.
use super::{SkuListError, driver_failure};
use crate::infra::storage::{
    RepoError,
    entity::{approval_unit, audit_log},
};
use bss_products_sdk::models::Lifecycle;
use sea_orm::sea_query::Query;
use sea_orm::{ColumnTrait, Condition, DbErr, EntityTrait};
use std::collections::HashMap;
use time::OffsetDateTime;
use toolkit_db::odata::sea_orm_filter::{
    FieldToColumn, LimitCfg, ODataFieldMapping, PaginateOdataTryError, paginate_odata_try,
};
use toolkit_db::secure::{AccessScope, DBRunner, SecureEntityExt};
use toolkit_odata::filter::{FieldKind, FilterField, FilterOp, ODataValue};
use toolkit_odata::{ODataOrderBy, ODataQuery, OrderKey, Page, SortDir};
use uuid::Uuid;

/// The page size when the caller names none, and the most a page holds (`$top` is clamped).
pub const HISTORY_PAGE: LimitCfg = LimitCfg {
    default: 50,
    max: 200,
};

/// The one key the history orders by; it takes no `$filter` and no `$orderby`. A cursor that
/// names any other key (one minted when the history ordered by `written_at` first) is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HistoryField {
    AuditId,
}
impl FilterField for HistoryField {
    const FIELDS: &'static [Self] = &[Self::AuditId];
    fn name(&self) -> &'static str {
        match self {
            Self::AuditId => "audit_id",
        }
    }
    fn kind(&self) -> FieldKind {
        match self {
            Self::AuditId => FieldKind::Uuid,
        }
    }
    fn from_name(name: &str) -> Option<Self> {
        Self::FIELDS.iter().copied().find(|f| f.name() == name)
    }
}
/// How the pager reads the audit row for each key.
pub struct HistoryMapping;
impl FieldToColumn<HistoryField> for HistoryMapping {
    type Column = audit_log::Column;
    fn map_field(field: HistoryField) -> audit_log::Column {
        match field {
            HistoryField::AuditId => audit_log::Column::AuditId,
        }
    }
    /// The history filters by its SKU alone; the door refuses `$filter` before the pager sees it.
    fn map_value(
        field: HistoryField,
        _op: FilterOp,
        _value: &ODataValue,
    ) -> Result<ODataValue, String> {
        Err(format!(
            "`{}` orders the history and is not a filter field",
            field.name()
        ))
    }
}
impl ODataFieldMapping<HistoryField> for HistoryMapping {
    type Entity = audit_log::Entity;
    fn extract_cursor_value(model: &audit_log::Model, field: HistoryField) -> sea_orm::Value {
        match field {
            HistoryField::AuditId => sea_orm::Value::Uuid(Some(model.audit_id)),
        }
    }
}

/// One act in a SKU's history: when, who, what, the lifecycle it found and left (null on a row
/// written before `m20260927_000008`), the unit it concerned, and the note it carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkuHistoryEntry {
    pub at: OffsetDateTime,
    pub actor: Uuid,
    pub action: String,
    pub from_lifecycle: Option<Lifecycle>,
    pub to_lifecycle: Option<Lifecycle>,
    pub unit_id: Option<Uuid>,
    pub unit_kind: Option<String>,
    pub note: Option<String>,
}

fn lifecycle(value: Option<&str>) -> Result<Option<Lifecycle>, RepoError> {
    value
        .map(|v| {
            Lifecycle::parse(v)
                .ok_or_else(|| RepoError::CorruptRow(format!("audit row lifecycle {v}")))
        })
        .transpose()
}
fn entry_of(m: audit_log::Model) -> Result<SkuHistoryEntry, RepoError> {
    let from_lifecycle = lifecycle(m.from_lifecycle.as_deref())?;
    let to_lifecycle = lifecycle(m.to_lifecycle.as_deref())?;
    let unit_id = (m.subject_kind == "approval_unit")
        .then_some(m.subject_id)
        .flatten();
    Ok(SkuHistoryEntry {
        at: m.written_at,
        actor: m.actor_ref,
        action: m.action,
        from_lifecycle,
        to_lifecycle,
        unit_id,
        unit_kind: None,
        note: m.reason,
    })
}

/// The SKU's rows: its own, and its units' (`ref_id` the SKU), in the tenant.
fn history_condition(tenant: Uuid, sku: Uuid) -> Condition {
    let units = Query::select()
        .column(approval_unit::Column::Id)
        .from(approval_unit::Entity)
        .and_where(approval_unit::Column::TenantId.eq(tenant))
        .and_where(approval_unit::Column::RefId.eq(sku))
        .to_owned();
    Condition::all()
        .add(audit_log::Column::TenantId.eq(tenant))
        .add(
            Condition::any()
                .add(
                    Condition::all()
                        .add(audit_log::Column::SubjectKind.eq("sku"))
                        .add(audit_log::Column::SubjectId.eq(sku)),
                )
                .add(
                    Condition::all()
                        .add(audit_log::Column::SubjectKind.eq("approval_unit"))
                        .add(audit_log::Column::SubjectId.in_subquery(units)),
                ),
        )
}

/// One page of the SKU's history in `audit_id` order — the order the acts wrote (the query names
/// no order; a cursor carries its own), `$top` 50 by default and clamped at 200, each unit row
/// with its unit's kind from ONE read of the page's units. The caller has found the SKU in its
/// scope.
/// # Errors
/// [`SkuListError::Query`] for a cursor the pager refuses; [`SkuListError::Repo`] for storage and
/// a stored lifecycle outside the five.
pub async fn page_sku_history(
    runner: &impl DBRunner,
    tenant: Uuid,
    sku: Uuid,
    query: &ODataQuery,
) -> Result<Page<SkuHistoryEntry>, SkuListError> {
    let mut query = query.clone();
    if query.cursor.is_none() {
        query.order = ODataOrderBy(vec![OrderKey {
            field: HistoryField::AuditId.name().to_owned(),
            dir: SortDir::Asc,
        }]);
    }
    let scope = AccessScope::for_tenant(tenant);
    let select = audit_log::Entity::find()
        .secure()
        .scope_with(&scope)
        .filter(history_condition(tenant, sku));
    let mut page = paginate_odata_try::<
        HistoryField,
        HistoryMapping,
        audit_log::Entity,
        SkuHistoryEntry,
        _,
        RepoError,
        _,
    >(
        select,
        runner,
        &query,
        (HistoryField::AuditId.name(), SortDir::Asc),
        HISTORY_PAGE,
        entry_of,
    )
    .await
    .map_err(|e| match e {
        // Kept as a driver failure so the retry classifier still reads the driver's message.
        PaginateOdataTryError::OData(toolkit_odata::Error::Db(message)) => {
            SkuListError::Repo(RepoError::Driver {
                context: "read SKU history".into(),
                source: DbErr::Custom(message),
            })
        }
        PaginateOdataTryError::OData(other) => SkuListError::Query(other),
        PaginateOdataTryError::MapError(e) => SkuListError::Repo(e),
    })?;
    let mut units: Vec<Uuid> = page.items.iter().filter_map(|e| e.unit_id).collect();
    units.sort_unstable();
    units.dedup();
    if units.is_empty() {
        return Ok(page);
    }
    let kinds: HashMap<Uuid, String> = approval_unit::Entity::find()
        .secure()
        .scope_with(&scope)
        .filter(
            Condition::all()
                .add(approval_unit::Column::TenantId.eq(tenant))
                .add(approval_unit::Column::Id.is_in(units)),
        )
        .all(runner)
        .await
        .map_err(|e| SkuListError::Repo(driver_failure("read history units".into(), e)))?
        .into_iter()
        .map(|u| (u.id, u.kind))
        .collect();
    for entry in &mut page.items {
        entry.unit_kind = entry.unit_id.and_then(|id| kinds.get(&id).cloned());
    }
    Ok(page)
}

#[cfg(test)]
#[path = "history_repo_tests.rs"]
mod history_repo_tests;
