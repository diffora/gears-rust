//! `SeaORM` entity for `bss.products_materiality_policy` — the tenant's
//! governed materiality policy (**P-D-112** arm 1; `design/05` C4,
//! `inst-mt-policy-material`).
//!
//! **One row per tenant, and the absence of a row is a value.** P-D-112 arm 2:
//! *"an absent row resolves to the default; only a failed read is
//! unresolved"*. So there is no seed, no provisioning insert, and nothing here
//! that a reader could mistake for a missing lookup — a tenant that has never
//! configured a policy simply has no row, and
//! [`crate::infra::storage::repo::resolve_materiality_policy`] is where that
//! becomes `MaterialityPolicy::default()` rather than a refusal.
//!
//! Scoped `resource_col = "tenant_id"` like `products_read_stamp`, for the
//! same reason: the row **is** the tenant's, so the resource and the tenant
//! are one column.

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_materiality_policy")]
#[secure(
    tenant_col = "tenant_id",
    resource_col = "tenant_id",
    no_owner,
    no_type
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    /// The tenant's addition to the bucket registry, as the canonical
    /// rendering of a string array. Empty is `[]` and never `NULL`: an absent
    /// column would be a third state beside "no row" and "no fields", and
    /// only two are meant.
    pub field_set: String,
    /// §17.1's affected-entity trigger for batch acts.
    pub affected_entity_trigger: i32,
    /// `N`. **Zero is reachable** (P-D-11) and the `CHECK` floors at zero
    /// rather than one, because a floor of one would silently restore the
    /// fixed count that decision retired.
    pub approver_count: i32,
    /// The principal whose governed mutation last wrote this row,
    /// pseudonymous like every actor-bearing column here.
    pub updated_by: Uuid,
    pub updated_at: ChronoDateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
