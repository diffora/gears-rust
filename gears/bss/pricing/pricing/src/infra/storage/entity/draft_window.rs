//! `SeaORM` entity for `bss.pricing_draft_window` — one revision-owned window
//! intention (D-374).
//!
//! Keyed `(tenant_id, plan_id, plan_revision, operation_id)`. There is no
//! `lifecycle_state` here: the parent revision's is the referent, and the
//! table's append-only triggers freeze the row when that revision leaves
//! `draft`. Abandoned revisions keep these rows.

use sea_orm::entity::prelude::*;
use time::OffsetDateTime;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "pricing_draft_window")]
#[secure(tenant_col = "tenant_id", resource_col = "plan_id", no_owner, no_type)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub plan_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub plan_revision: i64,
    #[sea_orm(primary_key, auto_increment = false)]
    pub operation_id: Uuid,
    pub target_window_id: Uuid,
    /// `create` | `adjust_end` | `cancel`.
    pub action: String,
    /// Required on `create`; null on live operations.
    pub price_id: Option<Uuid>,
    /// `at` | `at_publish` on `create`; null otherwise.
    pub start_kind: Option<String>,
    /// Present iff `start_kind = at`.
    pub effective_from: Option<OffsetDateTime>,
    pub effective_to: Option<OffsetDateTime>,
    pub reason_code: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
