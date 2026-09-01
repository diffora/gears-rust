//! `SeaORM` entity for `bss.products_freeze_ack` — AC #44's liveness
//! source, one row per `(version, participant)` seeded `pending` by the
//! increment transaction (P-D-67); the six admitted edges live in the
//! migration's trigger (P-D-60).

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_freeze_ack")]
#[secure(
    tenant_col = "tenant_id",
    resource_col = "participant",
    no_owner,
    no_type
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub catalog_version_id: i64,
    #[sea_orm(primary_key, auto_increment = false)]
    pub participant: String,
    /// `pending`, `acked`, `released` or `not_frozen(forced)`.
    pub state: String,
    pub acked_at: Option<ChronoDateTimeUtc>,
    /// The ceremony's alone, write-once (P-D-67).
    pub released_at: Option<ChronoDateTimeUtc>,
    pub forced_at: Option<ChronoDateTimeUtc>,
    pub ceremony_ref: Option<Uuid>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
