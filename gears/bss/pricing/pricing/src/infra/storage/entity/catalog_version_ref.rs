//! `SeaORM` entity for `bss.pricing_catalog_version_ref` — the pending-vs-
//! committed `CatalogVersion` linkage of one publish
//! (`design/01-foundation.md` §3.7).
//!
//! `catalog_version` and `committed_at` are set together at commit and never
//! re-pointed: an already-posted period resolves through the pin, so a ref that
//! could move would change what that period was priced from.
//!
//! `subject_kind` / `subject_ref` name **what the publish unit projects**. The
//! projector arrives at `CatalogVersionPublished` holding committed refs and
//! has to write exactly those subjects (§4.4, D-86/D-91); without them there is
//! no path from a pending handle back to what it published. The four tokens are
//! `pricing_read_model`'s, rendered from `domain::read_model::SubjectKind`.

use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "pricing_catalog_version_ref")]
#[secure(
    tenant_col = "tenant_id",
    resource_col = "pending_ref",
    no_owner,
    no_type
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    /// The registry's pending handle, stamped at publish.
    #[sea_orm(primary_key, auto_increment = false)]
    pub pending_ref: String,
    /// What kind of subject this publish unit projects.
    pub subject_kind: String,
    /// Which one.
    pub subject_ref: String,
    /// `None` until `CatalogVersionPublished` resolves the handle.
    pub catalog_version: Option<i64>,
    pub requested_at: DateTime<Utc>,
    pub committed_at: Option<DateTime<Utc>>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
