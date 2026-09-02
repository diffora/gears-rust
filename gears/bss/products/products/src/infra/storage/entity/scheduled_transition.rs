//! `SeaORM` entity for `bss.products_scheduled_transition` — a persisted
//! lifecycle intent (`design/04-lifecycle.md` §4).
//!
//! **Record, not rebuildable state.** Terminal rows
//! (`applied`/`failed`/`superseded`) are frozen by the migration's guard;
//! live rows stay mutable for the runner's claim protocol.

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_scheduled_transition")]
#[secure(
    tenant_col = "tenant_id",
    resource_col = "transition_id",
    no_owner,
    no_type
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub transition_id: Uuid,
    pub tenant_id: Uuid,
    /// `product` or `sku` — `chk_products_scheduled_transition_entity_kind`.
    pub entity_kind: String,
    pub entity_id: Uuid,
    /// `publish` or `retire` — `chk_products_scheduled_transition_kind`.
    pub kind: String,
    /// UTC activation instant.
    pub at: ChronoDateTimeUtc,
    /// The pinned slice-05 approval snapshot, consumed at scheduling.
    pub approval_ref: Uuid,
    /// `pending|running|applied|failed|deferred|superseded`.
    pub state: String,
    pub claimed_at: Option<ChronoDateTimeUtc>,
    /// Claim / reclaim counter; NOT NULL, default 0.
    pub attempt: i32,
    /// Operator text, written once at retirement initiation (**P-D-46**).
    pub retirement_reason: Option<String>,
    /// Runner outcome text on `applied|failed|deferred` (**P-D-46**).
    pub outcome_reason: Option<String>,
    pub created_at: ChronoDateTimeUtc,
    pub updated_at: ChronoDateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
