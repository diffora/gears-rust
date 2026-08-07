//! `SeaORM` entity for `bss.pricing_policy_object` — the per-tenant policy
//! object (`design/01-foundation.md` §3.7).
//!
//! Absence is meaningful on every nullable column here, and in each case it is
//! the fail-safe or ratified reading: no default rounding policy means every
//! published row must carry its own `rounding_policy_ref` or fail publish, and an
//! absent cap (D-152) means the ratified launch value from the deployment
//! section — so a tenant that configures nothing is governed by the numbers PRD
//! §14 ratified rather than by whatever a `NOT NULL DEFAULT` happened to say.
//!
//! **The approval threshold is not here, and used to be.**
//! `approval_threshold_minor` / `approval_threshold_currency` were one currency's
//! absolute threshold; §6 requires per-currency `{absolute_minor | percent}`
//! entries and D-10 requires the policy to be versioned, neither of which a single
//! column pair can carry. `m20260802_000018` moved the fact to
//! [`super::approval_threshold`] and dropped the pair in the same migration, so
//! there is exactly one place to read a threshold from.
//!
//! The four caps and the descriptor required-set extension sit here **for now**:
//! they are per-tenant settings with no settings gear to live in, and D-152's
//! confirmation records that they are expected to move once one exists.

use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "pricing_policy_object")]
#[secure(
    tenant_col = "tenant_id",
    resource_col = "tenant_id",
    no_owner,
    no_type
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    /// `fail_closed` (default) | `warn` — C4's enforcement mode over the two
    /// incomplete-basis arms of `inst-td-policy` (§6, `m20260802_000038`).
    ///
    /// Not nullable: C4 makes fail-closed the rule for **all** tenants, so
    /// "unconfigured" and "fail-closed" are one state and giving them two
    /// spellings would invite a reader to treat absence as "no policy".
    pub tax_display_policy_mode: String,
    /// The tenant default named rounding-policy id; optional by design.
    pub default_rounding_policy_ref: Option<String>,
    /// Enforced-migration notice period in days; floor 60 (D-49).
    pub enforced_migration_notice_days: i32,
    /// Soft cap on tier bands per price row (D-152). `None` => the ratified
    /// launch value.
    pub max_tier_bands_per_row: Option<i32>,
    /// Soft cap on price rows per plan (D-152). `None` => the ratified launch
    /// value.
    pub max_price_rows_per_plan: Option<i32>,
    /// Largest `n` a `customEveryN Days(n)` frequency may carry (D-152).
    /// `None` => the ratified launch value.
    pub max_custom_interval_days: Option<i32>,
    /// Largest `n` a `customEveryN Months(n)` frequency may carry (D-152).
    /// `None` => the ratified launch value.
    pub max_custom_interval_months: Option<i32>,
    /// The descriptor keys this tenant requires **in addition** to D-48 v1's
    /// pinned three: a JSON array of names matched against
    /// `pricing_plan_descriptor_set.additional_fields` (`jsonb` on Postgres,
    /// `text` on `SQLite`). Additive-only — there is no column here that can
    /// drop a v1 element, because a tenant policy may not publish past a pinned
    /// element of the contract Billing countersigns.
    pub additional_required_descriptors: Json,
    pub updated_at_utc: DateTime<Utc>,
    pub updated_by: Uuid,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
