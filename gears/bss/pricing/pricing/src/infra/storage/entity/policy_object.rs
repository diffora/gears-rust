//! `SeaORM` entity for `bss.pricing_policy_object` — the per-tenant policy
//! object (`design/01-foundation.md` §3.7).
//!
//! Absence is meaningful on both nullable policies, and in both cases it is the
//! fail-safe reading: no approval threshold means the two-person rule always
//! applies, and no default rounding policy means every published row must carry
//! its own `rounding_policy_ref` or fail publish.

use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "pricing_policy_object")]
#[secure(
    tenant_col = "tenant_id",
    resource_col = "tenant_id",
    no_owner,
    no_type
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    /// Material-change threshold in minor units. `None` => the two-person rule
    /// applies unconditionally.
    pub approval_threshold_minor: Option<i64>,
    pub approval_threshold_currency: Option<String>,
    /// `tax_inclusive` | `tax_exclusive`.
    pub tax_display_mode: String,
    /// The tenant default named rounding-policy id; optional by design.
    pub default_rounding_policy_ref: Option<String>,
    /// Enforced-migration notice period in days; floor 60 (D-49).
    pub enforced_migration_notice_days: i32,
    pub updated_at_utc: DateTime<Utc>,
    pub updated_by: Uuid,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
