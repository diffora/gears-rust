//! Create `bss.pricing_charge_tier` — shared tier geometry of a line version.
//!
//! Rates live on `pricing_price_tier_band` and must name both the price's
//! structure version and a band ordinal of this table.
//!
//! **Two keys, because the table this geometry came from had one that did two
//! jobs.** `pricing_price_tier_band` was keyed `(price_id, from_qty)` with
//! deliberately no ordinal -- *a band's identity is where it starts* -- and that one
//! index bought both the read order and the refusal of two bands on one lower
//! bound. Rates now join geometry on `band_ordinal`, so the ordinal is the primary
//! key; `uq_pricing_charge_tier_lower_bound` keeps the other half, which the split
//! first dropped without anyone deciding to. The band set is still not judged as a
//! sequence here -- order, gaplessness and the open top stay the
//! `TierBandValidator`'s -- but a second band on one lower bound is a fact about a
//! row and its twin, and an index can see that. The writer replaces a version's
//! geometry wholesale (delete, then insert), so no in-place move can collide with
//! it transiently.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.pricing_charge_tier (
            tenant_id       uuid    NOT NULL,
            line_version_id uuid    NOT NULL,
            band_ordinal    integer NOT NULL,
            from_qty        bigint  NOT NULL,
            to_qty          bigint,
            CONSTRAINT chk_pricing_charge_tier_from_qty CHECK (from_qty >= 0),
            CONSTRAINT chk_pricing_charge_tier_ordinal CHECK (band_ordinal >= 0),
            CONSTRAINT chk_pricing_charge_tier_width CHECK (to_qty IS NULL OR to_qty > from_qty),
            CONSTRAINT fk_pricing_charge_tier_version FOREIGN KEY (tenant_id, line_version_id)
                REFERENCES bss.pricing_charge_line_version (tenant_id, line_version_id),
            CONSTRAINT uq_pricing_charge_tier_lower_bound UNIQUE (tenant_id, line_version_id, from_qty),
            CONSTRAINT pricing_charge_tier_pkey PRIMARY KEY (tenant_id, line_version_id, band_ordinal)
        )",
    "CREATE INDEX idx_pricing_charge_tier_version ON bss.pricing_charge_tier USING btree (tenant_id, line_version_id)",
    "CREATE OR REPLACE FUNCTION bss.pricing_charge_tier_append_only() RETURNS trigger AS $$
        DECLARE
          parent_state text;
        BEGIN
          IF TG_OP <> 'INSERT' THEN
            SELECT lifecycle_state INTO parent_state
              FROM bss.pricing_charge_line_version
             WHERE tenant_id = OLD.tenant_id AND line_version_id = OLD.line_version_id;
            IF parent_state IS DISTINCT FROM 'draft' THEN
              RAISE EXCEPTION
                'pricing_charge_tier: % of a band under a % line version is not permitted',
                TG_OP, coalesce(parent_state, 'missing');
            END IF;
          END IF;

          IF TG_OP = 'DELETE' THEN
            RETURN OLD;
          END IF;

          SELECT lifecycle_state INTO parent_state
            FROM bss.pricing_charge_line_version
           WHERE tenant_id = NEW.tenant_id AND line_version_id = NEW.line_version_id;
          IF parent_state IS DISTINCT FROM 'draft' THEN
            RAISE EXCEPTION
              'pricing_charge_tier: % of a band under a % line version is not permitted',
              TG_OP, coalesce(parent_state, 'missing');
          END IF;

          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql",
    "CREATE OR REPLACE FUNCTION bss.pricing_charge_tier_kind() RETURNS trigger AS $$
        DECLARE
          parent_kind text;
        BEGIN
          SELECT model_kind INTO parent_kind
            FROM bss.pricing_charge_line_version
           WHERE tenant_id = NEW.tenant_id AND line_version_id = NEW.line_version_id;
          IF parent_kind IS NULL OR parent_kind NOT IN ('graduated','volume') THEN
            RAISE EXCEPTION
              'pricing_charge_tier: band rows are forbidden on a % line version',
              coalesce(parent_kind, 'kindless');
          END IF;
          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_pricing_charge_tier_append_only BEFORE INSERT OR DELETE OR UPDATE ON bss.pricing_charge_tier FOR EACH ROW EXECUTE FUNCTION bss.pricing_charge_tier_append_only()",
    "CREATE TRIGGER trg_pricing_charge_tier_kind BEFORE INSERT OR UPDATE ON bss.pricing_charge_tier FOR EACH ROW EXECUTE FUNCTION bss.pricing_charge_tier_kind()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS bss.pricing_charge_tier",
    "DROP FUNCTION IF EXISTS bss.pricing_charge_tier_append_only()",
    "DROP FUNCTION IF EXISTS bss.pricing_charge_tier_kind()",
];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE pricing_charge_tier (
            tenant_id       text    NOT NULL,
            line_version_id text    NOT NULL,
            band_ordinal    integer NOT NULL,
            from_qty        bigint  NOT NULL,
            to_qty          bigint,
            PRIMARY KEY (tenant_id, line_version_id, band_ordinal),
            CONSTRAINT chk_pricing_charge_tier_from_qty CHECK (from_qty >= 0),
            CONSTRAINT chk_pricing_charge_tier_ordinal CHECK (band_ordinal >= 0),
            CONSTRAINT chk_pricing_charge_tier_width CHECK (to_qty IS NULL OR to_qty > from_qty),
            CONSTRAINT uq_pricing_charge_tier_lower_bound UNIQUE (tenant_id, line_version_id, from_qty),
            CONSTRAINT fk_pricing_charge_tier_version FOREIGN KEY (tenant_id, line_version_id)
                REFERENCES pricing_charge_line_version (tenant_id, line_version_id)
        )",
    "CREATE INDEX idx_pricing_charge_tier_version ON pricing_charge_tier (tenant_id, line_version_id)",
    "CREATE TRIGGER trg_pricing_charge_tier_kind_insert BEFORE INSERT ON pricing_charge_tier FOR EACH ROW BEGIN SELECT RAISE(ABORT, 'pricing_charge_tier: band rows are permitted only on a graduated or volume line version') WHERE NOT EXISTS (SELECT 1 FROM pricing_charge_line_version WHERE tenant_id = NEW.tenant_id AND line_version_id = NEW.line_version_id AND model_kind IN ('graduated','volume')); END",
    "CREATE TRIGGER trg_pricing_charge_tier_kind_update BEFORE UPDATE ON pricing_charge_tier FOR EACH ROW BEGIN SELECT RAISE(ABORT, 'pricing_charge_tier: band rows are permitted only on a graduated or volume line version') WHERE NOT EXISTS (SELECT 1 FROM pricing_charge_line_version WHERE tenant_id = NEW.tenant_id AND line_version_id = NEW.line_version_id AND model_kind IN ('graduated','volume')); END",
    "CREATE TRIGGER trg_pricing_charge_tier_no_delete BEFORE DELETE ON pricing_charge_tier FOR EACH ROW BEGIN SELECT RAISE(ABORT, 'pricing_charge_tier: DELETE of a band under a non-draft line version is not permitted') WHERE NOT EXISTS (SELECT 1 FROM pricing_charge_line_version WHERE tenant_id = OLD.tenant_id AND line_version_id = OLD.line_version_id AND lifecycle_state = 'draft'); END",
    "CREATE TRIGGER trg_pricing_charge_tier_no_insert BEFORE INSERT ON pricing_charge_tier FOR EACH ROW BEGIN SELECT RAISE(ABORT, 'pricing_charge_tier: INSERT of a band under a non-draft line version is not permitted') WHERE NOT EXISTS (SELECT 1 FROM pricing_charge_line_version WHERE tenant_id = NEW.tenant_id AND line_version_id = NEW.line_version_id AND lifecycle_state = 'draft'); END",
    "CREATE TRIGGER trg_pricing_charge_tier_no_update BEFORE UPDATE ON pricing_charge_tier FOR EACH ROW BEGIN SELECT RAISE(ABORT, 'pricing_charge_tier: UPDATE of a band under a non-draft line version is not permitted') WHERE NOT EXISTS (SELECT 1 FROM pricing_charge_line_version WHERE tenant_id = OLD.tenant_id AND line_version_id = OLD.line_version_id AND lifecycle_state = 'draft') OR NOT EXISTS (SELECT 1 FROM pricing_charge_line_version WHERE tenant_id = NEW.tenant_id AND line_version_id = NEW.line_version_id AND lifecycle_state = 'draft'); END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS pricing_charge_tier"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(self.name(), manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(
            self.name(),
            manager,
            PG_DOWN_STATEMENTS,
            SQLITE_DOWN_STATEMENTS,
        )
        .await
    }
}
