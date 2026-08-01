//! Create `bss.pricing_policy_object` — the tenant policy objects
//! (`design/01-foundation.md` §3.7): the approval threshold, the tax-display
//! policy, the optional tenant **default rounding policy**, and the
//! enforced-migration notice period.
//!
//! Every default here is the **fail-safe** one, and the nullability encodes it.
//! `approval_threshold_minor` is nullable because absence must mean "the
//! two-person rule applies", not "everything is below threshold": the rule
//! applies unless a threshold is explicitly configured and the change is below
//! it and it is not a first publish (§4.2 step 3). `default_rounding_policy_ref`
//! is nullable for the mirror-image reason — a tenant without one simply
//! requires every published row to carry its own `rounding_policy_ref`, and an
//! unresolved policy fails publish with `ROUNDING_POLICY_UNRESOLVED` rather than
//! picking a rounding mode quietly (PRD §17.4).
//!
//! The physical guards are the notice-period floor
//! (`enforced_migration_notice_days >= 60`, D-49 — Slice 11 validates it again
//! at scheduling, but a floor that lives only in application code is one
//! migration script away from being bypassed) and the amount/currency
//! co-nullability `CHECK`, since a threshold amount with no currency is not a
//! threshold at all.
//!
//! **Backend differences.** None beyond the systematic type mirror.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &["CREATE TABLE bss.pricing_policy_object (
        tenant_id                      uuid        NOT NULL PRIMARY KEY,
        approval_threshold_minor       bigint,
        approval_threshold_currency    varchar(3),
        tax_display_mode               text        NOT NULL DEFAULT 'tax_exclusive',
        default_rounding_policy_ref    text,
        enforced_migration_notice_days integer     NOT NULL DEFAULT 60,
        updated_at_utc                 timestamptz NOT NULL DEFAULT now(),
        updated_by                     uuid        NOT NULL,
        CONSTRAINT chk_pricing_policy_object_tax_display CHECK (
            tax_display_mode IN ('tax_inclusive','tax_exclusive')),
        CONSTRAINT chk_pricing_policy_object_threshold CHECK (
            (approval_threshold_minor IS NULL) = (approval_threshold_currency IS NULL)),
        CONSTRAINT chk_pricing_policy_object_threshold_non_negative CHECK (
            approval_threshold_minor IS NULL OR approval_threshold_minor >= 0),
        CONSTRAINT chk_pricing_policy_object_notice_floor CHECK (
            enforced_migration_notice_days >= 60)
    )"];

const PG_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS bss.pricing_policy_object"];

const SQLITE_UP_STATEMENTS: &[&str] = &["CREATE TABLE pricing_policy_object (
        tenant_id                      text       NOT NULL PRIMARY KEY,
        approval_threshold_minor       bigint,
        approval_threshold_currency    varchar(3),
        tax_display_mode               text       NOT NULL DEFAULT 'tax_exclusive',
        default_rounding_policy_ref    text,
        enforced_migration_notice_days integer    NOT NULL DEFAULT 60,
        updated_at_utc                 text       NOT NULL DEFAULT (CURRENT_TIMESTAMP),
        updated_by                     text       NOT NULL,
        CONSTRAINT chk_pricing_policy_object_tax_display CHECK (
            tax_display_mode IN ('tax_inclusive','tax_exclusive')),
        CONSTRAINT chk_pricing_policy_object_threshold CHECK (
            (approval_threshold_minor IS NULL) = (approval_threshold_currency IS NULL)),
        CONSTRAINT chk_pricing_policy_object_threshold_non_negative CHECK (
            approval_threshold_minor IS NULL OR approval_threshold_minor >= 0),
        CONSTRAINT chk_pricing_policy_object_notice_floor CHECK (
            enforced_migration_notice_days >= 60)
    )"];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS pricing_policy_object"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
