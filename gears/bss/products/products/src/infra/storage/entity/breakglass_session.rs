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
    /// The first of the two **platform** principals who approved the
    /// elevation (**P-D-133** row 9), set on the two-person path and `NULL`
    /// on the post-hoc one — `chk_products_breakglass_approvers` binds each
    /// to `two_person_approval_ref`'s own nullity.
    ///
    /// **Why the pair lives here and not on an `ApprovalRecord`.** The
    /// store's `required` is `N` or `min(N, 1)` and its row is tenant-scoped,
    /// while an elevation needs exactly **two** principals from outside the
    /// tenant; no writer of `products_approval` can produce that fixed floor,
    /// and the approver-scope rule would refuse a platform approver on
    /// another tenant's subject. P-D-111 already made
    /// `two_person_approval_ref` the authority; these two make it legible.
    pub approver_a: Option<Uuid>,
    /// The second platform principal. Distinct from
    /// [`Model::approver_a`] by `chk_products_breakglass_approvers_distinct`
    /// — two-person means two humans, and a `CHECK` is the only place that
    /// cannot be forgotten by a caller.
    pub approver_b: Option<Uuid>,
    /// `pending` or `reviewed` (P-D-68 arm 3).
    pub posthoc_state: Option<String>,
    pub reviewed_by: Option<Uuid>,
    pub reviewed_at: Option<ChronoDateTimeUtc>,
    /// When the post-hoc review's SLA lapse was alerted (P-D-133) — a CAS
    /// stamp the lifecycle tick sets once; `NULL` until then.
    pub posthoc_overdue_alerted_at: Option<ChronoDateTimeUtc>,
    /// The CAS stamp `BreakGlassExpired`'s one emitter flips.
    pub expired_emitted: bool,
    pub opened_at: ChronoDateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
