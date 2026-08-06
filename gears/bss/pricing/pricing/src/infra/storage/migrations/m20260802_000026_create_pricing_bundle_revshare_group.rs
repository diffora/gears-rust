//! Create `bss.pricing_bundle_revshare_group` — one rev-share group per
//! **included vendor SKU** within one bundle revision (`design/08-bundles.md`
//! §6, D-07 + D-55 + D-92 + D-105), keyed
//! `(bundle_id, plan_revision, vendor_sku_id)`.
//!
//! This table lands **before** `pricing_bundle_revshare`, and the order is a
//! foreign key: a party row belongs to a group, and the group is what carries the
//! platform cut and the absorber. Under the reverse order there is no referent.
//!
//! # Why the group exists at all
//!
//! D-55's 2026-07-28 correction. The tolerance and exact-sum rule of D-07 is
//! **per `(bundle, vendor SKU)` group** — *"sum to 100% per included vendor
//! SKU"* — so the values it ranges over have to live on a group row rather than
//! be smeared across the party rows or hoisted to the bundle. Before the
//! correction `platform_cut_bp` was a per-party column used once per group (so
//! nothing stopped two parties disagreeing about one group's cut) and the
//! absorber was a bundle-level column typed as *"a `vendor_sku_id`"*, which names
//! a **group** and not a resolvable party — matching the PRD's *"nominated
//! primary party"* only for the platform case.
//!
//! # `residual_absorber_party` is `text`, and so is a party everywhere else
//!
//! The column holds **either** a party of this group **or** the literal
//! `platform` sentinel (D-07: the default, so an "unnominated" state cannot
//! exist). One column, two inhabitants, so the type has to admit both — and a
//! `uuid` party beside a text sentinel would need a cast at every comparison,
//! in a comparison the reconciler makes for every group it normalizes.
//! `pricing_bundle_revshare.party` is therefore `text` as well: the two are
//! compared to each other, and a column pair that is compared should not have
//! two types.
//!
//! **`platform` is safe as a sentinel only because a party may not *be* named
//! `platform`** — the same argument `m20260802_000023` makes for the empty
//! string as the `meter` sentinel. `domain::bundle::Party::new` is where that is
//! enforced; the constraint here cannot see it.
//!
//! # What the schema does **not** enforce, and why
//!
//! That the absorber is a party **of this group** is a membership property of a
//! set the table cannot see one row at a time, so it is the reconciler's
//! (`RESIDUAL_ABSORBER_UNKNOWN` is not a code the design set declares — the
//! refusal renders under `REVSHARE_UNBALANCED`, and the gap is in the owed
//! register). A foreign key would not do it either: it would have to point at
//! `pricing_bundle_revshare`, which points **here**, and the `platform` sentinel
//! has no party row to point at by construction.
//!
//! That `SUM(share_bp) + platform_cut_bp` lands within 1 bp of 10000 is likewise
//! a set property and is `inst-rs-residual`'s. A `CHECK` sees one row.
//!
//! **Backend differences and the append-only discipline** are
//! `m20260802_000025`'s, verbatim and for the same reasons; see that module's
//! doc. `uuid` becomes `text`, `bss.` is dropped, and the single PL/pgSQL
//! function becomes three fixed-message `SQLite` triggers.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.pricing_bundle_revshare_group (
        bundle_id               uuid   NOT NULL,
        plan_revision           bigint NOT NULL,
        vendor_sku_id           uuid   NOT NULL,
        tenant_id               uuid   NOT NULL,
        platform_cut_bp         int    NOT NULL,
        residual_absorber_party text   NOT NULL DEFAULT 'platform',
        PRIMARY KEY (bundle_id, plan_revision, vendor_sku_id),
        -- Basis points are a bounded scale: 10000 bp is 100%. A cut outside it
        -- is not a share of anything, and `REVSHARE_UNBALANCED` is the report
        -- line for the structural malformation this refuses physically.
        CONSTRAINT chk_pricing_bundle_revshare_group_platform_cut_bp CHECK (
            platform_cut_bp >= 0 AND platform_cut_bp <= 10000),
        -- An absorber must name something. The blank string is not the sentinel
        -- and is not a party; admitting it would give 'unnominated' a spelling,
        -- which D-07 says cannot exist.
        CONSTRAINT chk_pricing_bundle_revshare_group_absorber CHECK (
            length(residual_absorber_party) > 0),
        CONSTRAINT fk_pricing_bundle_revshare_group_bundle FOREIGN KEY (bundle_id)
            REFERENCES bss.pricing_bundle (bundle_id)
    )",
    "CREATE INDEX idx_pricing_bundle_revshare_group_revision
        ON bss.pricing_bundle_revshare_group (tenant_id, bundle_id, plan_revision)",
    "CREATE OR REPLACE FUNCTION bss.pricing_bundle_revshare_group_append_only() RETURNS trigger AS $$
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
                'pricing_bundle_revshare_group: % of a rev-share group under a non-draft plan revision is not permitted (state %)',
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
              'pricing_bundle_revshare_group: % of a rev-share group under a non-draft plan revision is not permitted (state %)',
              TG_OP, coalesce(parent_state, 'missing');
          END IF;

          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_pricing_bundle_revshare_group_append_only
        BEFORE INSERT OR UPDATE OR DELETE ON bss.pricing_bundle_revshare_group
        FOR EACH ROW EXECUTE FUNCTION bss.pricing_bundle_revshare_group_append_only()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS bss.pricing_bundle_revshare_group",
    "DROP FUNCTION IF EXISTS bss.pricing_bundle_revshare_group_append_only()",
];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE pricing_bundle_revshare_group (
        bundle_id               text   NOT NULL,
        plan_revision           bigint NOT NULL,
        vendor_sku_id           text   NOT NULL,
        tenant_id               text   NOT NULL,
        platform_cut_bp         int    NOT NULL,
        residual_absorber_party text   NOT NULL DEFAULT 'platform',
        PRIMARY KEY (bundle_id, plan_revision, vendor_sku_id),
        CONSTRAINT chk_pricing_bundle_revshare_group_platform_cut_bp CHECK (
            platform_cut_bp >= 0 AND platform_cut_bp <= 10000),
        CONSTRAINT chk_pricing_bundle_revshare_group_absorber CHECK (
            length(residual_absorber_party) > 0),
        CONSTRAINT fk_pricing_bundle_revshare_group_bundle FOREIGN KEY (bundle_id)
            REFERENCES pricing_bundle (bundle_id)
    )",
    "CREATE INDEX idx_pricing_bundle_revshare_group_revision
        ON pricing_bundle_revshare_group (tenant_id, bundle_id, plan_revision)",
    "CREATE TRIGGER trg_pricing_bundle_revshare_group_no_insert
        BEFORE INSERT ON pricing_bundle_revshare_group
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_bundle_revshare_group: INSERT of a rev-share group under a non-draft plan revision is not permitted')
          WHERE NOT EXISTS (
            SELECT 1 FROM pricing_bundle b
              JOIN pricing_plan p ON p.plan_id = b.plan_id
             WHERE b.bundle_id = NEW.bundle_id AND p.revision = NEW.plan_revision
               AND p.lifecycle_state = 'draft');
        END",
    "CREATE TRIGGER trg_pricing_bundle_revshare_group_no_update
        BEFORE UPDATE ON pricing_bundle_revshare_group
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_bundle_revshare_group: UPDATE of a rev-share group under a non-draft plan revision is not permitted')
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
    "CREATE TRIGGER trg_pricing_bundle_revshare_group_no_delete
        BEFORE DELETE ON pricing_bundle_revshare_group
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_bundle_revshare_group: DELETE of a rev-share group under a non-draft plan revision is not permitted')
          WHERE NOT EXISTS (
            SELECT 1 FROM pricing_bundle b
              JOIN pricing_plan p ON p.plan_id = b.plan_id
             WHERE b.bundle_id = OLD.bundle_id AND p.revision = OLD.plan_revision
               AND p.lifecycle_state = 'draft');
        END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS pricing_bundle_revshare_group"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
