//! `SeaORM` entity for `bss.pricing_price_overlay_line` — one adjustment line of
//! one overlay revision (`design/09-price-overlays.md` §6, D-42, D-67, D-78,
//! D-138).
//!
//! D-42 made an overlay a container of lines rather than a single adjustment;
//! this is the line. Its key is `(plan_id?, target_sku?, cohort?)` inside a
//! revision, enforced null-safely by `uq_pricing_price_overlay_line_key` — see
//! `m20260802_000033` for why a plain `UNIQUE` would enforce none of it.
//!
//! # `cohort` is an eligibility **filter**, not a specificity level (D-78)
//!
//! NULL means the line applies only to rows whose `priceEligibility` is
//! `all_subscriptions` or `new_subscriptions_only`; a value means it applies
//! **only** to `existing_grandfathered` rows of that generation. It is part of
//! the line's uniqueness key and of `OVERLAY_INTERVAL_OVERLAP`'s key, and it is
//! *not* an input to the most-specific rule — that rule runs unchanged inside
//! the eligible set.
//!
//! What it closes is worth restating, because the field looks optional and is
//! not: before D-78 every line applied to every resolved base row, so a single
//! `+2000 bp` markup repriced a grandfathered cohort whose price the whole
//! ADR-0002 machinery exists to guarantee — the row immutable, its window live,
//! its generation selected by the pinned price id, and the effective charge moved
//! anyway without touching a single row.
//!
//! # `adjustment_value` is the **percent** magnitude and nothing else
//!
//! Amount-based magnitudes are money and live per currency in
//! [`super::price_overlay_line_amount`] (D-08, no-implicit-FX). The pairing is a
//! biconditional `CHECK`, not a convention: the value type is **declared** via
//! `magnitude_kind` and never inferred from the presence of amount rows, because
//! implicit-absence semantics are forbidden by the Foundation and a bp value read
//! as minor units mis-prices by orders of magnitude.

use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "pricing_price_overlay_line")]
#[secure(
    tenant_col = "tenant_id",
    resource_col = "price_overlay_id",
    no_owner,
    no_type
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub line_id: Uuid,
    /// The overlay this line belongs to.
    pub price_overlay_id: Uuid,
    /// **The revision it is frozen with** (D-92, copy-on-new-revision). Without
    /// this half a line would belong to the overlay rather than to one of its
    /// revisions, and there would be nothing to copy forward.
    pub overlay_revision: i64,
    /// Copied from the parent overlay by the repository, never taken from a
    /// request (Global Constraint 9).
    pub tenant_id: Uuid,
    /// `None` is the **list-default line** — it applies to every target of the
    /// overlay's `target_ref`. A value must be a published plan inside that
    /// scope.
    pub plan_id: Option<Uuid>,
    /// Optional narrowing; requires `plan_id`, because a bare SKU is ambiguous
    /// per `(currency, region)`.
    pub target_sku: Option<String>,
    /// The grandfathered generation's cutover instant. See the module doc — this
    /// is a filter, not a level.
    pub cohort: Option<DateTime<Utc>>,
    /// `markup` | `discount` | `fixed`. **D-138 is normative about what each
    /// does to the running amount**: `markup` adds, `discount` subtracts, and
    /// `fixed` **replaces** — an absolute price at that stack layer. An additive
    /// `fixed` would duplicate `markup`, which already expresses absolute money
    /// through `magnitude_kind = amount`.
    pub adjustment_kind: String,
    /// `percent_bp` | `amount`, declared and never inferred (D-08).
    pub magnitude_kind: String,
    /// Basis points, on `percent_bp` lines only. Range-bounded by D-67:
    /// `0 < v <= 10000` on a discount, `v > 0` on a markup.
    pub adjustment_value: Option<i64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
