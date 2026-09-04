//! `SeaORM` entity for `bss.products_approval` — the approval record
//! (`design/05` §4, P-D-13, P-D-14, P-D-68).
//!
//! @cpt-dod:cpt-cf-bss-products-dod-approval-store:p1

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_approval")]
#[secure(
    tenant_col = "tenant_id",
    resource_col = "approval_id",
    no_owner,
    no_type
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub approval_id: Uuid,
    /// One of the five §4 kinds.
    pub subject_kind: String,
    /// The subject's own identifier, rendered — the five kinds do not share
    /// one id type, so the column is textual by construction.
    pub subject_ref: String,
    /// The revision the submission pinned.
    pub internal_revision: i64,
    /// Stored at submission and never re-derived (§5's flagship probe).
    pub content_snapshot: String,
    /// The published version the diff renders against.
    pub diff_basis: Option<i64>,
    /// Stored at submission and never re-derived — it carries the `N` in
    /// force then, so a later policy edit cannot change a pending record.
    pub quorum_descriptor: String,
    pub state: String,
    /// Pseudonymous from birth.
    pub submitter: Uuid,
    /// Written by the submit door only when the effective quorum is zero
    /// (P-D-68 arm 1); paired with its instant by a CHECK.
    pub author_override_ack: Option<String>,
    pub author_override_ack_at: Option<ChronoDateTimeUtc>,
    pub submitted_at: ChronoDateTimeUtc,
    /// Non-null exactly when the state is terminal.
    pub finalized_at: Option<ChronoDateTimeUtc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
