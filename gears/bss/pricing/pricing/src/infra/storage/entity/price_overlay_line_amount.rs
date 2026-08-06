//! `SeaORM` entity for `bss.pricing_price_overlay_line_amount` — one line's value
//! in one currency (`design/09-price-overlays.md` §6, D-08).
//!
//! An amount-based magnitude is money and exists **only per currency**: the
//! catalog performs no FX, so a line whose magnitude is absolute carries one
//! value per currency its resolved target scope sells, and a missing one fails
//! save and publish (`ADJUSTMENT_CURRENCY_NOT_COVERED`, naming the line). A
//! percent line is currency-neutral and has no rows here at all.
//!
//! The key is `(line_id, currency)` and it is spelled as the **primary key**
//! rather than as a surrogate plus a unique index, because the pair *is* the
//! row's identity — there is no such thing as two values of one line in one
//! currency, and a surrogate would invite a second row to exist and be ignored.
//!
//! There is no `lifecycle_state` here. A value is frozen when the revision its
//! **line** belongs to publishes, so the reference is the overlay revision and
//! the table's append-only trigger resolves it through
//! [`super::price_overlay_line`] — [`super::bundle_revshare`]'s arrangement one
//! slice over.

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "pricing_price_overlay_line_amount")]
#[secure(tenant_col = "tenant_id", resource_col = "line_id", no_owner, no_type)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub line_id: Uuid,
    /// ISO 4217.
    #[sea_orm(primary_key, auto_increment = false)]
    pub currency: String,
    /// Copied from the parent line by the repository, never taken from a
    /// request: the foreign key covers `line_id` alone, so nothing in the schema
    /// stops a child carrying a foreign tenant.
    pub tenant_id: Uuid,
    /// The magnitude, in the currency's ISO 4217 minor unit. `>= 0` (D-67), and
    /// **zero is admitted**: a `fixed 0` line is how a market is priced at
    /// nothing, which is a real authoring act — unlike a `markup` of 0 bp, which
    /// adjusts nothing and is refused one table up.
    pub value_minor: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
