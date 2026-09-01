//! `SeaORM` entity for `bss.products_recognized_set` — the generic set table
//! behind all four recognized sets (`design/03` §3.1, P-D-47, P-D-92).

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_recognized_set")]
#[secure(
    tenant_col = "tenant_id",
    resource_col = "member_code",
    no_owner,
    no_type
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    /// The `DoD` names four — `metering_unit`, `tax_category`, `gl_code`,
    /// `plan_tier` — and the DDL pins **non-emptiness only** (P-D-92): §7
    /// row 5 may delete two of them, so the roster is the membership door's.
    #[sea_orm(primary_key, auto_increment = false)]
    pub set_kind: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub member_code: String,
    /// `plan_tier`'s, ignored elsewhere.
    pub display_label: Option<String>,
    /// `active` and `deprecated` are the set; `removed` is a tombstone
    /// outside it (P-D-47), and no `DELETE` is ever admitted.
    pub state: String,
    /// The registry-seeded marker: a seeded member is deprecatable and never
    /// retired.
    pub seeded_by: Option<String>,
    pub created_at: ChronoDateTimeUtc,
    pub updated_at: ChronoDateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
