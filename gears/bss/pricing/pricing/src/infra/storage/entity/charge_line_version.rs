//! `SeaORM` entity for `bss.pricing_charge_line_version` — revision-owned shared
//! structure (`ChargeStructure` plus billing timing and the proration contract).

use sea_orm::entity::prelude::*;
use serde_json::Value as JsonValue;
use time::OffsetDateTime;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "pricing_charge_line_version")]
#[secure(
    tenant_col = "tenant_id",
    resource_col = "line_version_id",
    no_owner,
    no_type
)]
#[allow(
    clippy::struct_field_names,
    reason = "`model_kind` is the normative column name (design/03-price-structure.md 6)"
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub line_version_id: Uuid,
    pub charge_line_id: Uuid,
    pub plan_revision: i64,
    pub lifecycle_state: String,
    pub invoice_line_template: Option<String>,
    pub gl_code_ref: Option<String>,
    pub resolved_invoice_line_template: Option<String>,
    pub resolved_gl_code: Option<String>,
    pub model_kind: Option<String>,
    pub package_size: Option<i64>,
    pub quantity_source: Option<String>,
    pub manual_quantity: Option<i64>,
    pub meter: Option<String>,
    pub billing_granularity: Option<String>,
    pub tier_aggregation_window: Option<String>,
    pub tier_qualification_window: Option<String>,
    pub aggregation_function: Option<String>,
    pub aggregation_granularity: Option<String>,
    pub max_hold_granules: Option<i64>,
    pub included_allowance: Option<JsonValue>,
    pub reservation_flavor: Option<String>,
    pub min_qty_purchase: Option<i64>,
    pub min_qty_usage: Option<i64>,
    pub min_qty_usage_fallback: Option<String>,
    pub discount_ref: Option<String>,
    pub billing_timing: Option<String>,
    pub billing_anchor_policy: Option<String>,
    pub anchor_day: Option<i32>,
    pub proration_basis: Option<String>,
    pub credit_on_downgrade: Option<bool>,
    pub created_by: Uuid,
    pub created_at_utc: OffsetDateTime,
    pub row_version: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
