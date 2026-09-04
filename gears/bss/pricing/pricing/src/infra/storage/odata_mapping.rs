//! `OData` field → `SeaORM` column mappers for pricing collection GETs.

use bss_pricing_sdk::odata::{
    ApprovalFilterField, AuditFilterField, BundleFilterField, HistoryFilterField,
    MembershipFilterField, MigrationFilterField, OverlayFilterField, PlanFilterField,
    PlanPriceFilterField, WindowFilterField,
};
use toolkit_db::odata::sea_orm_filter::{FieldToColumn, LimitCfg, ODataFieldMapping};
use toolkit_odata::filter::{FilterField, FilterOp, ODataValue};

use crate::domain::approval::ApprovalState;
use toolkit_odata::{ODataOrderBy, ODataQuery, OrderKey, Page, SortDir};

use crate::infra::storage::RepoError;

use crate::infra::storage::entity::approval::{
    Column as ApprovalColumn, Entity as ApprovalEntity, Model as ApprovalModel,
};
use crate::infra::storage::entity::audit_log::{
    Column as AuditColumn, Entity as AuditEntity, Model as AuditModel,
};
use crate::infra::storage::entity::bundle::{
    Column as BundleColumn, Entity as BundleEntity, Model as BundleModel,
};
use crate::infra::storage::entity::group_membership::{
    Column as MembershipColumn, Entity as MembershipEntity, Model as MembershipModel,
};
use crate::infra::storage::entity::migration::{
    Column as MigrationColumn, Entity as MigrationEntity, Model as MigrationModel,
};
use crate::infra::storage::entity::plan::{
    Column as PlanColumn, Entity as PlanEntity, Model as PlanModel,
};
use crate::infra::storage::entity::price::{
    Column as PriceColumn, Entity as PriceEntity, Model as PriceModel,
};
use crate::infra::storage::entity::price_overlay::{
    Column as OverlayColumn, Entity as OverlayEntity, Model as OverlayModel,
};
use crate::infra::storage::entity::price_window::{
    Column as WindowColumn, Entity as WindowEntity, Model as WindowModel,
};

fn uuid_opt(id: Option<uuid::Uuid>) -> sea_orm::Value {
    sea_orm::Value::Uuid(id)
}

fn string_opt(value: Option<&String>) -> sea_orm::Value {
    match value {
        Some(s) => sea_orm::Value::String(Some(s.clone())),
        None => sea_orm::Value::String(None),
    }
}

fn datetime_utc(instant: time::OffsetDateTime) -> sea_orm::Value {
    sea_orm::Value::TimeDateTimeWithTimeZone(Some(instant))
}

/// D-125 page size: default 100, hard cap 1 000.
pub const LIST_LIMIT_CFG: LimitCfg = LimitCfg {
    default: 100,
    max: 1_000,
};

/// Whether `$filter` names `field` (any operator). Used for authoring defaults.
pub fn filter_mentions_field<F: FilterField>(
    filter: Option<&toolkit_odata::ast::Expr>,
    field: F,
) -> bool {
    filter.is_some_and(|expr| mentions(expr, field.name()))
}

fn mentions(expr: &toolkit_odata::ast::Expr, field: &str) -> bool {
    use toolkit_odata::ast::Expr;
    match expr {
        Expr::And(a, b) | Expr::Or(a, b) => mentions(a, field) || mentions(b, field),
        Expr::Not(inner) => mentions(inner, field),
        Expr::Compare(left, _, right) => mentions(left, field) || mentions(right, field),
        Expr::In(inner, values) => {
            mentions(inner, field) || values.iter().any(|value| mentions(value, field))
        }
        Expr::Function(_, args) => args.iter().any(|arg| mentions(arg, field)),
        Expr::Identifier(name) => name == field,
        Expr::Value(_) => false,
    }
}

/// Inject the list's default keyset order, or reconstruct it from `cursor.s`.
///
/// The extractor clears `$orderby` when a cursor is present (the pair is 400).
/// Page 2 therefore arrives with an empty `order` and must take the sort the
/// token already carries — the same derivation `paginate_odata` uses.
pub fn query_with_default_order<F: FilterField>(query: &ODataQuery, fields: &[F]) -> ODataQuery {
    let out = query.clone();
    if !out.order.is_empty() {
        return out;
    }
    if let Some(cursor) = &out.cursor {
        if let Ok(from_cursor) = ODataOrderBy::from_signed_tokens(&cursor.s) {
            return out.with_order(from_cursor);
        }
        return out;
    }
    out.with_order(ODataOrderBy(
        fields
            .iter()
            .map(|field| OrderKey {
                field: field.name().to_owned(),
                dir: SortDir::Asc,
            })
            .collect(),
    ))
}

/// `paginate_odata` failure split: client `$filter` / cursor vs storage.
#[derive(Debug, thiserror::Error)]
pub enum OdataPageError {
    /// Storage / connection failure.
    #[error("pricing list db error: {0}")]
    Db(String),
    /// Malformed `$filter` / `$orderby` / cursor.
    #[error("pricing list odata error: {0}")]
    Odata(#[from] toolkit_odata::Error),
}

/// Map a `paginate_odata` failure into [`OdataPageError`].
pub fn map_odata_err(err: toolkit_odata::Error) -> OdataPageError {
    match err {
        toolkit_odata::Error::Db(d) => OdataPageError::Db(d),
        other => OdataPageError::Odata(other),
    }
}

/// Project domain-mapped rows onto a page, flattening store decode failures.
pub fn domain_page<T, M>(
    page: Page<M>,
    map: impl Fn(M) -> Result<T, RepoError>,
) -> Result<Page<T>, OdataPageError> {
    let items = page
        .items
        .into_iter()
        .map(map)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| OdataPageError::Db(e.to_string()))?;
    Ok(Page {
        items,
        page_info: page.page_info,
    })
}

pub struct PlanODataMapper;

impl FieldToColumn<PlanFilterField> for PlanODataMapper {
    type Column = PlanColumn;

    fn map_field(field: PlanFilterField) -> PlanColumn {
        match field {
            PlanFilterField::PlanId => PlanColumn::PlanId,
            PlanFilterField::LifecycleState => PlanColumn::LifecycleState,
            PlanFilterField::SkuId => PlanColumn::SkuId,
            PlanFilterField::PlanTier => PlanColumn::PlanTier,
            PlanFilterField::BillingCycle => PlanColumn::BillingCycle,
            PlanFilterField::CreatedAtUtc => PlanColumn::CreatedAtUtc,
        }
    }

    fn is_orderable(field: PlanFilterField) -> bool {
        matches!(field, PlanFilterField::PlanId)
    }
}

impl ODataFieldMapping<PlanFilterField> for PlanODataMapper {
    type Entity = PlanEntity;

    fn extract_cursor_value(model: &PlanModel, field: PlanFilterField) -> sea_orm::Value {
        match field {
            PlanFilterField::PlanId => sea_orm::Value::Uuid(Some(model.plan_id)),
            PlanFilterField::LifecycleState => {
                sea_orm::Value::String(Some(model.lifecycle_state.clone()))
            }
            PlanFilterField::SkuId => uuid_opt(model.sku_id),
            PlanFilterField::PlanTier => string_opt(model.plan_tier.as_ref()),
            PlanFilterField::BillingCycle => string_opt(model.billing_cycle.as_ref()),
            PlanFilterField::CreatedAtUtc => datetime_utc(model.created_at_utc),
        }
    }
}

pub struct PlanPriceODataMapper;

impl FieldToColumn<PlanPriceFilterField> for PlanPriceODataMapper {
    type Column = PriceColumn;

    fn map_field(field: PlanPriceFilterField) -> PriceColumn {
        match field {
            PlanPriceFilterField::PriceId => PriceColumn::PriceId,
            PlanPriceFilterField::LifecycleState => PriceColumn::LifecycleState,
            PlanPriceFilterField::CreatedAtUtc => PriceColumn::CreatedAtUtc,
        }
    }
}

impl ODataFieldMapping<PlanPriceFilterField> for PlanPriceODataMapper {
    type Entity = PriceEntity;

    fn extract_cursor_value(model: &PriceModel, field: PlanPriceFilterField) -> sea_orm::Value {
        match field {
            PlanPriceFilterField::PriceId => sea_orm::Value::Uuid(Some(model.price_id)),
            PlanPriceFilterField::LifecycleState => {
                sea_orm::Value::String(Some(model.lifecycle_state.clone()))
            }
            PlanPriceFilterField::CreatedAtUtc => datetime_utc(model.created_at_utc),
        }
    }
}

pub struct OverlayODataMapper;

impl FieldToColumn<OverlayFilterField> for OverlayODataMapper {
    type Column = OverlayColumn;

    fn map_field(field: OverlayFilterField) -> OverlayColumn {
        match field {
            OverlayFilterField::PriceOverlayId => OverlayColumn::PriceOverlayId,
            OverlayFilterField::ScopeClass => OverlayColumn::ScopeClass,
            OverlayFilterField::LifecycleState => OverlayColumn::LifecycleState,
            OverlayFilterField::Precedence => OverlayColumn::Precedence,
            OverlayFilterField::Revision => OverlayColumn::Revision,
        }
    }
}

impl ODataFieldMapping<OverlayFilterField> for OverlayODataMapper {
    type Entity = OverlayEntity;

    fn extract_cursor_value(model: &OverlayModel, field: OverlayFilterField) -> sea_orm::Value {
        match field {
            OverlayFilterField::PriceOverlayId => {
                sea_orm::Value::Uuid(Some(model.price_overlay_id))
            }
            OverlayFilterField::ScopeClass => {
                sea_orm::Value::String(Some(model.scope_class.clone()))
            }
            OverlayFilterField::LifecycleState => {
                sea_orm::Value::String(Some(model.lifecycle_state.clone()))
            }
            OverlayFilterField::Precedence => sea_orm::Value::Int(Some(model.precedence)),
            OverlayFilterField::Revision => sea_orm::Value::BigInt(Some(model.revision)),
        }
    }
}

pub struct WindowODataMapper;

impl FieldToColumn<WindowFilterField> for WindowODataMapper {
    type Column = WindowColumn;

    fn map_field(field: WindowFilterField) -> WindowColumn {
        match field {
            WindowFilterField::PriceId => WindowColumn::PriceId,
            WindowFilterField::WindowId => WindowColumn::WindowId,
        }
    }
}

impl ODataFieldMapping<WindowFilterField> for WindowODataMapper {
    type Entity = WindowEntity;

    fn extract_cursor_value(model: &WindowModel, field: WindowFilterField) -> sea_orm::Value {
        match field {
            WindowFilterField::PriceId => sea_orm::Value::Uuid(Some(model.price_id)),
            WindowFilterField::WindowId => sea_orm::Value::Uuid(Some(model.window_id)),
        }
    }
}

pub struct MembershipODataMapper;

impl FieldToColumn<MembershipFilterField> for MembershipODataMapper {
    type Column = MembershipColumn;

    fn map_field(field: MembershipFilterField) -> MembershipColumn {
        match field {
            MembershipFilterField::PayerId => MembershipColumn::PayerTenantId,
            MembershipFilterField::MembershipId => MembershipColumn::MembershipId,
            MembershipFilterField::EffectiveFrom => MembershipColumn::EffectiveFrom,
        }
    }
}

impl ODataFieldMapping<MembershipFilterField> for MembershipODataMapper {
    type Entity = MembershipEntity;

    fn extract_cursor_value(
        model: &MembershipModel,
        field: MembershipFilterField,
    ) -> sea_orm::Value {
        match field {
            MembershipFilterField::PayerId => sea_orm::Value::Uuid(Some(model.payer_tenant_id)),
            MembershipFilterField::MembershipId => sea_orm::Value::Uuid(Some(model.membership_id)),
            MembershipFilterField::EffectiveFrom => datetime_utc(model.effective_from),
        }
    }
}

pub struct ApprovalODataMapper;

impl FieldToColumn<ApprovalFilterField> for ApprovalODataMapper {
    type Column = ApprovalColumn;

    fn map_field(field: ApprovalFilterField) -> ApprovalColumn {
        match field {
            ApprovalFilterField::State => ApprovalColumn::State,
            ApprovalFilterField::ApprovalId => ApprovalColumn::ApprovalId,
        }
    }

    fn map_value(
        field: ApprovalFilterField,
        _op: FilterOp,
        value: &ODataValue,
    ) -> Result<ODataValue, String> {
        match field {
            ApprovalFilterField::State => match value {
                ODataValue::String(token) => {
                    if ApprovalState::from_token(token).is_none() {
                        return Err(format!("unknown approval state `{token}`"));
                    }
                    Ok(value.clone())
                }
                _ => Err("approval state must be a string token".to_owned()),
            },
            ApprovalFilterField::ApprovalId => Ok(value.clone()),
        }
    }
}

impl ODataFieldMapping<ApprovalFilterField> for ApprovalODataMapper {
    type Entity = ApprovalEntity;

    fn extract_cursor_value(model: &ApprovalModel, field: ApprovalFilterField) -> sea_orm::Value {
        match field {
            ApprovalFilterField::State => sea_orm::Value::String(Some(model.state.clone())),
            ApprovalFilterField::ApprovalId => sea_orm::Value::Uuid(Some(model.approval_id)),
        }
    }
}

pub struct MigrationODataMapper;

impl FieldToColumn<MigrationFilterField> for MigrationODataMapper {
    type Column = MigrationColumn;

    fn map_field(field: MigrationFilterField) -> MigrationColumn {
        match field {
            MigrationFilterField::State => MigrationColumn::State,
            MigrationFilterField::MigrationId => MigrationColumn::MigrationId,
        }
    }
}

impl ODataFieldMapping<MigrationFilterField> for MigrationODataMapper {
    type Entity = MigrationEntity;

    fn extract_cursor_value(model: &MigrationModel, field: MigrationFilterField) -> sea_orm::Value {
        match field {
            MigrationFilterField::State => sea_orm::Value::String(Some(model.state.clone())),
            MigrationFilterField::MigrationId => sea_orm::Value::Uuid(Some(model.migration_id)),
        }
    }
}

pub struct BundleODataMapper;

impl FieldToColumn<BundleFilterField> for BundleODataMapper {
    type Column = BundleColumn;

    fn map_field(field: BundleFilterField) -> BundleColumn {
        match field {
            BundleFilterField::PlanId => BundleColumn::PlanId,
            BundleFilterField::BundleId => BundleColumn::BundleId,
        }
    }
}

impl ODataFieldMapping<BundleFilterField> for BundleODataMapper {
    type Entity = BundleEntity;

    fn extract_cursor_value(model: &BundleModel, field: BundleFilterField) -> sea_orm::Value {
        match field {
            BundleFilterField::PlanId => sea_orm::Value::Uuid(Some(model.plan_id)),
            BundleFilterField::BundleId => sea_orm::Value::Uuid(Some(model.bundle_id)),
        }
    }
}

pub struct HistoryODataMapper;

impl FieldToColumn<HistoryFilterField> for HistoryODataMapper {
    type Column = PriceColumn;

    fn map_field(field: HistoryFilterField) -> PriceColumn {
        match field {
            HistoryFilterField::PriceId => PriceColumn::PriceId,
            HistoryFilterField::PlanId => PriceColumn::PlanId,
            HistoryFilterField::LifecycleState => PriceColumn::LifecycleState,
            HistoryFilterField::AuthoredAt => PriceColumn::CreatedAtUtc,
            HistoryFilterField::Actor => PriceColumn::CreatedBy,
        }
    }
}

impl ODataFieldMapping<HistoryFilterField> for HistoryODataMapper {
    type Entity = PriceEntity;

    fn extract_cursor_value(model: &PriceModel, field: HistoryFilterField) -> sea_orm::Value {
        match field {
            HistoryFilterField::PriceId => sea_orm::Value::Uuid(Some(model.price_id)),
            HistoryFilterField::PlanId => sea_orm::Value::Uuid(Some(model.plan_id)),
            HistoryFilterField::LifecycleState => {
                sea_orm::Value::String(Some(model.lifecycle_state.clone()))
            }
            HistoryFilterField::AuthoredAt => datetime_utc(model.created_at_utc),
            HistoryFilterField::Actor => sea_orm::Value::Uuid(Some(model.created_by)),
        }
    }
}

pub struct AuditODataMapper;

impl FieldToColumn<AuditFilterField> for AuditODataMapper {
    type Column = AuditColumn;

    fn map_field(field: AuditFilterField) -> AuditColumn {
        match field {
            AuditFilterField::ChainId => AuditColumn::ChainId,
            AuditFilterField::Seq => AuditColumn::Seq,
            AuditFilterField::EntryKind => AuditColumn::EntryKind,
            AuditFilterField::RecordedAt => AuditColumn::RecordedAt,
            AuditFilterField::ActorPrincipalId => AuditColumn::ActorPrincipalId,
            AuditFilterField::Action => AuditColumn::Action,
            AuditFilterField::SubjectKind => AuditColumn::SubjectKind,
        }
    }
}

impl ODataFieldMapping<AuditFilterField> for AuditODataMapper {
    type Entity = AuditEntity;

    fn extract_cursor_value(model: &AuditModel, field: AuditFilterField) -> sea_orm::Value {
        match field {
            AuditFilterField::ChainId => sea_orm::Value::Uuid(Some(model.chain_id)),
            AuditFilterField::Seq => sea_orm::Value::BigInt(Some(model.seq)),
            AuditFilterField::EntryKind => sea_orm::Value::String(Some(model.entry_kind.clone())),
            AuditFilterField::RecordedAt => datetime_utc(model.recorded_at),
            AuditFilterField::ActorPrincipalId => {
                sea_orm::Value::Uuid(Some(model.actor_principal_id))
            }
            AuditFilterField::Action => sea_orm::Value::String(Some(model.action.clone())),
            AuditFilterField::SubjectKind => {
                sea_orm::Value::String(Some(model.subject_kind.clone()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use toolkit_odata::parse_filter_string;
    use toolkit_odata::{CursorV1, ODataQuery, SortDir};

    use super::*;

    fn expr(raw: &str) -> toolkit_odata::ast::Expr {
        parse_filter_string(raw)
            .expect("filter must parse")
            .as_expr()
            .clone()
    }

    #[test]
    fn filter_mentions_lifecycle_state_eq_in_and_not() {
        assert!(!filter_mentions_field(
            None,
            PlanFilterField::LifecycleState
        ));
        assert!(filter_mentions_field(
            Some(&expr("lifecycle_state eq 'draft'")),
            PlanFilterField::LifecycleState
        ));
        assert!(filter_mentions_field(
            Some(&expr("lifecycle_state in ('draft','published')")),
            PlanFilterField::LifecycleState
        ));
        assert!(filter_mentions_field(
            Some(&expr("not lifecycle_state eq 'draft'")),
            PlanFilterField::LifecycleState
        ));
        assert!(!filter_mentions_field(
            Some(&expr("plan_id eq 11111111-1111-1111-1111-111111111111")),
            PlanFilterField::LifecycleState
        ));
    }

    #[test]
    fn default_order_injects_when_empty_and_reconstructs_from_cursor() {
        let injected = query_with_default_order(&ODataQuery::default(), &[PlanFilterField::PlanId]);
        assert_eq!(injected.order.0.len(), 1);
        assert_eq!(injected.order.0[0].field, "plan_id");

        let already = ODataQuery::default().with_order(ODataOrderBy(vec![OrderKey {
            field: "sku_id".to_owned(),
            dir: SortDir::Desc,
        }]));
        let kept = query_with_default_order(&already, &[PlanFilterField::PlanId]);
        assert_eq!(kept.order.0[0].field, "sku_id");

        let with_cursor = ODataQuery::default().with_cursor(CursorV1 {
            k: vec!["11111111-1111-1111-1111-111111111111".to_owned()],
            o: SortDir::Asc,
            s: "plan_id".to_owned(),
            f: None,
            d: "fwd".to_owned(),
        });
        let after_cursor = query_with_default_order(&with_cursor, &[PlanFilterField::PlanId]);
        assert_eq!(after_cursor.order.0.len(), 1);
        assert_eq!(after_cursor.order.0[0].field, "plan_id");
        assert_eq!(after_cursor.order.0[0].dir, SortDir::Asc);
    }
}
