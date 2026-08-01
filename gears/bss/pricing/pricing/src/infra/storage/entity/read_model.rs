//! `SeaORM` entity for `bss.pricing_read_model` — one **per-subject delta** of
//! one `CatalogVersion` (`design/01-foundation.md` §3.7, D-86 / D-91).
//!
//! `warm_completed` is per row, not per version: a row is invisible to
//! resolution until both `CatalogVersionPublished` and this marker are present.

use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;
use serde_json::Value as JsonValue;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "pricing_read_model")]
#[secure(
    tenant_col = "tenant_id",
    resource_col = "subject_ref",
    no_owner,
    no_type
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub catalog_version: i64,
    /// `plan` | `price_overlay` | `overlay_index` | `group_membership`; see
    /// `domain::read_model::SubjectKind`.
    #[sea_orm(primary_key, auto_increment = false)]
    pub subject_kind: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub subject_ref: String,
    pub warm_completed: bool,
    pub warm_completed_at: Option<DateTime<Utc>>,
    /// The frozen projected payload for this subject at this version.
    pub payload: JsonValue,
    pub projected_at: DateTime<Utc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
