//! Create `bss.pricing_bundle_revshare` — one rev-share **party** row within one
//! group of one bundle revision (`design/08-bundles.md` §6, D-07 + D-92 +
//! D-105), keyed `(bundle_id, plan_revision, vendor_sku_id, party)`.
//!
//! # Two share columns, and the difference between them is the whole of D-07
//!
//! `share_bp` is what the operator **typed**; `effective_share_bp` is what the
//! publish **normalized**. Authoring accepts
//! `|Σ(share_bp) + platform_cut_bp − 10000| ≤ 1 bp`, because percentages arrive
//! from contracts as 33.33% and three of those are 9999 bp; publish then adjusts
//! the group's `residual_absorber_party`'s effective share so the published
//! shares sum to **exactly** 10000 bp per group. The typed values are retained
//! for audit — that is the entire reason there are two columns rather than one
//! rewritten in place — and downstream consumers (Tariffs, Marketplace) read
//! only the effective ones.
//!
//! `effective_share_bp` is therefore **nullable**: an unpublished draft has no
//! normalized answer yet, and defaulting it to the typed value would make "not
//! yet reconciled" indistinguishable from "reconciled to exactly what was
//! typed", which is the common case for every party that is not the absorber.
//!
//! # The group foreign key is what makes an implicit platform cut impossible
//!
//! `(bundle_id, plan_revision, vendor_sku_id)` references
//! `pricing_bundle_revshare_group`, so a party row cannot exist without its
//! group — and the group is where `platform_cut_bp` lives. `inst-rs-sum` requires
//! *"an explicit per-group platform cut"*, and this is the physical half of it:
//! there is no state in which shares are authored against a total whose platform
//! cut nobody stated. `REVSHARE_UNBALANCED` remains the report line for the
//! malformations a foreign key cannot see.
//!
//! There is deliberately **no** foreign key onto `pricing_bundle` here, though
//! `bundle_id` is a column: the group reference already covers it transitively,
//! and a second path to the same parent is a second thing that can be true.
//!
//! # Rev-share is a `sum_of_parts` property (D-55) and the schema does not say so
//!
//! An `own_price` bundle has one bundle amount and no per-vendor-SKU revenue to
//! allocate, so `own_price` + rev-share fails publish
//! (`REVSHARE_BASIS_UNSUPPORTED`, 422). That is a cross-table property —
//! `price_basis` lives on `pricing_bundle` — and a `CHECK` sees one row, so it is
//! `inst-rs-sum`'s and not this table's. Recorded here because the absence looks
//! like an oversight otherwise.
//!
//! **Backend differences and the append-only discipline** are
//! `m20260802_000025`'s, verbatim and for the same reasons.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.pricing_bundle_revshare (
        bundle_id          uuid   NOT NULL,
        plan_revision      bigint NOT NULL,
        vendor_sku_id      uuid   NOT NULL,
        party              text   NOT NULL,
        tenant_id          uuid   NOT NULL,
        share_bp           int    NOT NULL,
        effective_share_bp int,
        PRIMARY KEY (bundle_id, plan_revision, vendor_sku_id, party),
        CONSTRAINT chk_pricing_bundle_revshare_share_bp CHECK (
            share_bp >= 0 AND share_bp <= 10000),
        -- Null until publish normalizes it; bounded on the same scale once set.
        CONSTRAINT chk_pricing_bundle_revshare_effective_share_bp CHECK (
            effective_share_bp IS NULL
            OR (effective_share_bp >= 0 AND effective_share_bp <= 10000)),
        -- A party must name something, and must not be able to spell the
        -- group's `platform` sentinel: the absorber column is compared against
        -- this one, and a party literally named `platform` would make the
        -- comparison ambiguous in the one place D-07 says it cannot be.
        CONSTRAINT chk_pricing_bundle_revshare_party CHECK (
            length(party) > 0 AND party <> 'platform'),
        CONSTRAINT fk_pricing_bundle_revshare_group
            FOREIGN KEY (bundle_id, plan_revision, vendor_sku_id)
            REFERENCES bss.pricing_bundle_revshare_group
                       (bundle_id, plan_revision, vendor_sku_id)
    )",
    "CREATE INDEX idx_pricing_bundle_revshare_revision
        ON bss.pricing_bundle_revshare (tenant_id, bundle_id, plan_revision)",
    "CREATE OR REPLACE FUNCTION bss.pricing_bundle_revshare_append_only() RETURNS trigger AS $$
        DECLARE
          parent_state text;
        BEGIN
          IF TG_OP <> 'INSERT' THEN
            SELECT p.lifecycle_state INTO parent_state
              FROM bss.pricing_bundle b
              JOIN bss.pricing_plan p ON p.plan_id = b.plan_id
             WHERE b.bundle_id = OLD.bundle_id AND p.revision = OLD.plan_revision;
            IF parent_state IS DISTINCT FROM 'draft' THEN
              RAISE EXCEPTION
                'pricing_bundle_revshare: % of a rev-share party under a non-draft plan revision is not permitted (state %)',
                TG_OP, coalesce(parent_state, 'missing');
            END IF;
          END IF;

          IF TG_OP = 'DELETE' THEN
            RETURN OLD;
          END IF;

          SELECT p.lifecycle_state INTO parent_state
            FROM bss.pricing_bundle b
            JOIN bss.pricing_plan p ON p.plan_id = b.plan_id
           WHERE b.bundle_id = NEW.bundle_id AND p.revision = NEW.plan_revision;
          IF parent_state IS DISTINCT FROM 'draft' THEN
            RAISE EXCEPTION
              'pricing_bundle_revshare: % of a rev-share party under a non-draft plan revision is not permitted (state %)',
              TG_OP, coalesce(parent_state, 'missing');
          END IF;

          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_pricing_bundle_revshare_append_only
        BEFORE INSERT OR UPDATE OR DELETE ON bss.pricing_bundle_revshare
        FOR EACH ROW EXECUTE FUNCTION bss.pricing_bundle_revshare_append_only()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS bss.pricing_bundle_revshare",
    "DROP FUNCTION IF EXISTS bss.pricing_bundle_revshare_append_only()",
];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE pricing_bundle_revshare (
        bundle_id          text   NOT NULL,
        plan_revision      bigint NOT NULL,
        vendor_sku_id      text   NOT NULL,
        party              text   NOT NULL,
        tenant_id          text   NOT NULL,
        share_bp           int    NOT NULL,
        effective_share_bp int,
        PRIMARY KEY (bundle_id, plan_revision, vendor_sku_id, party),
        CONSTRAINT chk_pricing_bundle_revshare_share_bp CHECK (
            share_bp >= 0 AND share_bp <= 10000),
        CONSTRAINT chk_pricing_bundle_revshare_effective_share_bp CHECK (
            effective_share_bp IS NULL
            OR (effective_share_bp >= 0 AND effective_share_bp <= 10000)),
        CONSTRAINT chk_pricing_bundle_revshare_party CHECK (
            length(party) > 0 AND party <> 'platform'),
        CONSTRAINT fk_pricing_bundle_revshare_group
            FOREIGN KEY (bundle_id, plan_revision, vendor_sku_id)
            REFERENCES pricing_bundle_revshare_group
                       (bundle_id, plan_revision, vendor_sku_id)
    )",
    "CREATE INDEX idx_pricing_bundle_revshare_revision
        ON pricing_bundle_revshare (tenant_id, bundle_id, plan_revision)",
    "CREATE TRIGGER trg_pricing_bundle_revshare_no_insert
        BEFORE INSERT ON pricing_bundle_revshare
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_bundle_revshare: INSERT of a rev-share party under a non-draft plan revision is not permitted')
          WHERE NOT EXISTS (
            SELECT 1 FROM pricing_bundle b
              JOIN pricing_plan p ON p.plan_id = b.plan_id
             WHERE b.bundle_id = NEW.bundle_id AND p.revision = NEW.plan_revision
               AND p.lifecycle_state = 'draft');
        END",
    "CREATE TRIGGER trg_pricing_bundle_revshare_no_update
        BEFORE UPDATE ON pricing_bundle_revshare
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_bundle_revshare: UPDATE of a rev-share party under a non-draft plan revision is not permitted')
          WHERE NOT EXISTS (
            SELECT 1 FROM pricing_bundle b
              JOIN pricing_plan p ON p.plan_id = b.plan_id
             WHERE b.bundle_id = OLD.bundle_id AND p.revision = OLD.plan_revision
               AND p.lifecycle_state = 'draft')
             OR NOT EXISTS (
            SELECT 1 FROM pricing_bundle b
              JOIN pricing_plan p ON p.plan_id = b.plan_id
             WHERE b.bundle_id = NEW.bundle_id AND p.revision = NEW.plan_revision
               AND p.lifecycle_state = 'draft');
        END",
    "CREATE TRIGGER trg_pricing_bundle_revshare_no_delete
        BEFORE DELETE ON pricing_bundle_revshare
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_bundle_revshare: DELETE of a rev-share party under a non-draft plan revision is not permitted')
          WHERE NOT EXISTS (
            SELECT 1 FROM pricing_bundle b
              JOIN pricing_plan p ON p.plan_id = b.plan_id
             WHERE b.bundle_id = OLD.bundle_id AND p.revision = OLD.plan_revision
               AND p.lifecycle_state = 'draft');
        END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS pricing_bundle_revshare"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
