//! `SeaORM` entity for `bss.products_product` — a Product's identity, its
//! lifecycle and its two version counters.
//!
//! The capability columns a Product will carry are not here yet; they arrive
//! with the features that own their rules. What is here is what the Foundation
//! owns: identity, lifecycle, the counters and the scope sets.
//!
//! Category assignments live **only** in the taxonomy feature's assignment
//! table. A second inline representation here would be a divergence channel
//! with no authority rule.
//!
//! @cpt-cf-bss-products-fr-identifier-contract
//! @cpt-cf-bss-products-nfr-scale-extensibility
//! @cpt-dod:cpt-cf-bss-products-dod-entity-tables:p1

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_product")]
#[secure(
    tenant_col = "tenant_id",
    resource_col = "product_id",
    no_owner,
    no_type
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub product_id: Uuid,
    pub tenant_id: Uuid,
    /// An operand of the uniqueness index and a bucket-i column: re-branding
    /// moves the row into a different uniqueness scope, so it is refused after
    /// first publish.
    pub brand_id: Uuid,
    /// The operator-facing name, as authored.
    pub name: String,
    /// NFKC, full casefold, whitespace-collapsed. Computed application-side so
    /// both engines store identical bytes.
    pub name_normalized: String,
    /// The optional external mapping code, reserved under the same rules as a
    /// SKU's.
    pub product_code: Option<String>,
    /// `draft | published | deprecated | retired | discarded`, constrained by
    /// `chk_products_product_lifecycle_state`.
    pub lifecycle_state: String,
    /// Moves on every admitted write.
    pub internal_revision: i64,
    /// Moves only on publish.
    pub published_version: i64,
    /// A flat value set. `NOT NULL`, default empty, where **empty means
    /// unrestricted** rather than nothing.
    pub region_scope: String,
    /// The same shape and the same reading as `region_scope`.
    pub brand_scope: String,
    /// The pseudonymous ref of whoever created the row. Outside the bucket
    /// scheme entirely: admitted in no update at all.
    pub created_by: String,
    pub created_at: ChronoDateTimeUtc,
    pub updated_at: ChronoDateTimeUtc,
    /// The clone's immediate source (P-D-72: for a SKU child, its own source
    /// SKU) — create-only, guarded immutable by the head trigger (P-D-76).
    pub cloned_from: Option<Uuid>,
    /// The frozen version the source's content was read at; `NULL` under a
    /// non-`NULL` `cloned_from` means the source was read at its head — a
    /// draft (P-D-76's representable sentinel).
    pub cloned_from_version: Option<i64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
