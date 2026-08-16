//! `pricing_rounding_policy_taxonomy` — the rounding-policy references a tenant
//! declares (D-322).
//!
//! # Why a declared set, and why it is not this gear legislating
//!
//! `rounding_policy_ref` decides the last minor unit of every charge, and until
//! now it was **free text validated for presence alone** — the 2026-08-10 review
//! carried that as an open finding, and D-320 made it sharper by giving the
//! tenant default a writer: a typo that used to sit on one row now sits on the
//! whole tenant. The hazard is not hypothetical. On the stand today one plan's
//! seven rows carry `half_up` while the tenant default carries `half_up_2dp`;
//! both publish, both resolve, and nothing anywhere says they are two spellings
//! of what an operator meant once.
//!
//! **Both of those examples are also malformed, which the same day's correction
//! to D-321 records**: `bss/ledger` §6.8 fixes the scale to the currency's ISO
//! 4217 minor unit, so `_2dp` in an identifier is the catalog restating a
//! decision the currency already made — and getting it wrong on the first
//! currency with a different minor unit. A reference names a **mode**.
//!
//! The fix is the one Slice 4 already uses for regions: **the tenant declares
//! the vocabulary and the gear refuses references outside it**. That keeps
//! D-320's boundary intact — this gear still neither defines nor applies a
//! rounding policy, and it invents no semantics for `half_up`. It only refuses a
//! reference to something the tenant never declared, exactly as `REGION_UNKNOWN`
//! does.
//!
//! # The shape is the taxonomies', deliberately
//!
//! `(tenant_id, value)` primary key, a `display_name`, and `state IN ('active',
//! 'retired')` — `m20260802_000029`'s table with its name changed. A retired
//! value keeps resolving for rows that already name it and cannot be newly
//! authored, which is what retirement means on every other taxonomy here.
//!
//! **It is not a fifth `TaxonomyClass`.** That enum's `as_str` is the overlay
//! `scope_class` token (`ScopeClass::as_str`), so adding a member would declare
//! that an overlay may be scoped by rounding policy — which is false and would
//! reach the `pricing_price_overlay.scope_class` column. Same table shape, own
//! type, own route.
//!
//! # Empty means unconstrained, and that is a decision rather than an oversight
//!
//! A tenant that has declared nothing is not refused: the check binds only where
//! a vocabulary exists (D-322 clause 3). Declaring the set is how a tenant opts
//! into the constraint, and the alternative — an empty set refusing every row —
//! would make this migration break every catalog on the day it lands, including
//! the seven rows above, for a vocabulary nobody has had a chance to write.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &["CREATE TABLE bss.pricing_rounding_policy_taxonomy (
        tenant_id    uuid NOT NULL,
        value        text NOT NULL,
        display_name text NOT NULL,
        state        text NOT NULL DEFAULT 'active',
        PRIMARY KEY (tenant_id, value),
        CONSTRAINT chk_pricing_rounding_policy_taxonomy_state CHECK (
            state IN ('active', 'retired')),
        CONSTRAINT chk_pricing_rounding_policy_taxonomy_value_present CHECK (
            length(value) > 0)
    )"];

const PG_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS bss.pricing_rounding_policy_taxonomy"];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------

const SQLITE_UP_STATEMENTS: &[&str] = &["CREATE TABLE pricing_rounding_policy_taxonomy (
        tenant_id    text NOT NULL,
        value        text NOT NULL,
        display_name text NOT NULL,
        state        text NOT NULL DEFAULT 'active',
        PRIMARY KEY (tenant_id, value),
        CONSTRAINT chk_pricing_rounding_policy_taxonomy_state CHECK (
            state IN ('active', 'retired')),
        CONSTRAINT chk_pricing_rounding_policy_taxonomy_value_present CHECK (
            length(value) > 0)
    )"];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS pricing_rounding_policy_taxonomy"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
