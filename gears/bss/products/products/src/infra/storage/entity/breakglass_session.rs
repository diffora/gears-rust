//! `SeaORM` entity for `bss.products_breakglass_session` — the elevation
//! session (`design/05` §4, P-D-68 arms 2 and 3).
//!
//! **Scoped by `target_tenant`, not by an owning tenant.** The session is a
//! platform record — its principal is outside the tenant entirely — but the
//! thing it grants access TO is one tenant, so scoping on the target is what
//! keeps a tenant-scoped read from seeing another tenant's elevations.
//! `no_tenant` would make every session visible under every scope.

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_breakglass_session")]
#[secure(
    tenant_col = "target_tenant",
    resource_col = "session_id",
    no_owner,
    no_type
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub session_id: Uuid,
    /// Pseudonymous from birth, like every actor-bearing store here.
    pub principal: Uuid,
    pub target_tenant: Uuid,
    pub reason: String,
    /// The window is half-open: expiry gates admission, so an act admitted
    /// inside it finishes (P-D-68 arm 2).
    pub valid_from: ChronoDateTimeUtc,
    pub valid_until: ChronoDateTimeUtc,
    /// Exactly one of the two paths is taken, and a CHECK enforces the
    /// exclusivity. This one carries no FK: whether the referent is an
    /// `ApprovalRecord` is an open item P-D-68 arm 3 deliberately did not
    /// presuppose.
    pub two_person_approval_ref: Option<Uuid>,
    /// `pending` or `reviewed` (P-D-68 arm 3).
    pub posthoc_state: Option<String>,
    pub reviewed_by: Option<Uuid>,
    pub reviewed_at: Option<ChronoDateTimeUtc>,
    /// The CAS stamp `BreakGlassExpired`'s one emitter flips.
    pub expired_emitted: bool,
    pub opened_at: ChronoDateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
