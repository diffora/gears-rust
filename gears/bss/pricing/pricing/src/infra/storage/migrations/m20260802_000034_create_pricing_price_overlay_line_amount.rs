//! Create `bss.pricing_price_overlay_line_amount` — the per-currency values of
//! an amount-based line (`design/09-price-overlays.md` §6, D-08, D-67).
//!
//! An **amount-based** magnitude is money and exists only per currency (D-08,
//! no-implicit-FX): the catalog never converts, so a line whose magnitude is
//! absolute carries one value per currency its resolved target scope sells, and
//! a missing one fails save and publish (`ADJUSTMENT_CURRENCY_NOT_COVERED`).
//! A **percent** magnitude is currency-neutral and lives on the line itself as
//! `adjustment_value`, so a percent line has no rows here at all — which is what
//! `chk_pricing_price_overlay_line_magnitude_pairing` makes physical one table
//! up.
//!
//! # The key is the pair, and the pair is the primary key
//!
//! §6 says `UNIQUE (line_id, currency)`. It is spelled as the **primary key**
//! rather than as a surrogate id plus a unique index, because the pair *is* the
//! row's identity: there is no such thing as two values of one line in one
//! currency, and a surrogate would invite a second row to exist and be
//! ignored. Nothing references this table, so nothing needs a narrower handle.
//!
//! # `value_minor >= 0` is D-67 and not the money rule
//!
//! `DESIGN` §2's `>= 0` binds authored **price rows**; D-67 states the overlay
//! line's own bound, and this `CHECK` is its physical half — *"amount magnitudes
//! `>= 0` at the currency's ISO 4217 minor unit"*. **Zero is admitted**, and
//! deliberately: a `fixed 0` line is how a market is priced at nothing, which is
//! a real authoring act, while the bp magnitudes one table up are refused at
//! zero because a `markup` of 0 bp adjusts nothing at all. The two bounds differ
//! because the two kinds mean different things by zero.
//!
//! **The minor-unit precision itself is not checkable here.** `value_minor` is
//! already an integer count of minor units, so every integer satisfies "at the
//! currency's ISO 4217 minor unit" at every precision — the same observation
//! `DomainError::ThresholdInvalid` records about §6's `absolute_minor` rule.
//! What *is* checkable is that the currency is a currency, and
//! `chk_..._currency` holds the ISO 4217 shape; the code's validity is
//! `domain::money::CurrencyCode`'s and is checked at the authoring edge.
//!
//! # The append-only trigger reaches its parent through the line
//!
//! An amount rides the same revision its line does (§6: *"the amount table below
//! rides the same revision through its line"*), so the freeze is the line's and
//! the lookup is one join longer than `m20260802_000033`'s. Without it the
//! line's freeze would be worth nothing — the line would be immutable and its
//! money still editable.
//!
//! **Backend differences.** As `m20260802_000032` and `m20260802_000033`.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.pricing_price_overlay_line_amount (
        line_id     uuid   NOT NULL,
        currency    text   NOT NULL,
        tenant_id   uuid   NOT NULL,
        value_minor bigint NOT NULL,
        PRIMARY KEY (line_id, currency),
        CONSTRAINT fk_pricing_price_overlay_line_amount_line FOREIGN KEY (line_id)
            REFERENCES bss.pricing_price_overlay_line (line_id),
        CONSTRAINT chk_pricing_price_overlay_line_amount_value_minor CHECK (
            value_minor >= 0),
        CONSTRAINT chk_pricing_price_overlay_line_amount_currency CHECK (
            length(currency) = 3)
    )",
    // The coverage walk reads one line's whole value set.
    "CREATE INDEX idx_pricing_price_overlay_line_amount_tenant
        ON bss.pricing_price_overlay_line_amount (tenant_id, line_id)",
    "CREATE OR REPLACE FUNCTION bss.pricing_price_overlay_line_amount_append_only()
        RETURNS trigger AS $$
        DECLARE
          parent_state text;
        BEGIN
          IF TG_OP <> 'INSERT' THEN
            SELECT o.lifecycle_state INTO parent_state
              FROM bss.pricing_price_overlay_line l
              JOIN bss.pricing_price_overlay o
                ON o.price_overlay_id = l.price_overlay_id
               AND o.revision = l.overlay_revision
             WHERE l.line_id = OLD.line_id;
            IF parent_state IS DISTINCT FROM 'draft' THEN
              RAISE EXCEPTION
                'pricing_price_overlay_line_amount: % of a value under a non-draft overlay revision is not permitted (state %)',
                TG_OP, coalesce(parent_state, 'missing');
            END IF;
          END IF;

          IF TG_OP = 'DELETE' THEN
            RETURN OLD;
          END IF;

          SELECT o.lifecycle_state INTO parent_state
            FROM bss.pricing_price_overlay_line l
            JOIN bss.pricing_price_overlay o
              ON o.price_overlay_id = l.price_overlay_id
             AND o.revision = l.overlay_revision
           WHERE l.line_id = NEW.line_id;
          IF parent_state IS DISTINCT FROM 'draft' THEN
            RAISE EXCEPTION
              'pricing_price_overlay_line_amount: % of a value under a non-draft overlay revision is not permitted (state %)',
              TG_OP, coalesce(parent_state, 'missing');
          END IF;

          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_pricing_price_overlay_line_amount_append_only
        BEFORE INSERT OR UPDATE OR DELETE ON bss.pricing_price_overlay_line_amount
        FOR EACH ROW EXECUTE FUNCTION bss.pricing_price_overlay_line_amount_append_only()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS bss.pricing_price_overlay_line_amount",
    "DROP FUNCTION IF EXISTS bss.pricing_price_overlay_line_amount_append_only()",
];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE pricing_price_overlay_line_amount (
        line_id     text   NOT NULL,
        currency    text   NOT NULL,
        tenant_id   text   NOT NULL,
        value_minor bigint NOT NULL,
        PRIMARY KEY (line_id, currency),
        CONSTRAINT fk_pricing_price_overlay_line_amount_line FOREIGN KEY (line_id)
            REFERENCES pricing_price_overlay_line (line_id),
        CONSTRAINT chk_pricing_price_overlay_line_amount_value_minor CHECK (
            value_minor >= 0),
        CONSTRAINT chk_pricing_price_overlay_line_amount_currency CHECK (
            length(currency) = 3)
    )",
    "CREATE INDEX idx_pricing_price_overlay_line_amount_tenant
        ON pricing_price_overlay_line_amount (tenant_id, line_id)",
    "CREATE TRIGGER trg_pricing_price_overlay_line_amount_no_insert
        BEFORE INSERT ON pricing_price_overlay_line_amount
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price_overlay_line_amount: INSERT of a value under a non-draft overlay revision is not permitted')
          WHERE NOT EXISTS (
            SELECT 1 FROM pricing_price_overlay_line l
              JOIN pricing_price_overlay o
                ON o.price_overlay_id = l.price_overlay_id
               AND o.revision = l.overlay_revision
             WHERE l.line_id = NEW.line_id AND o.lifecycle_state = 'draft');
        END",
    "CREATE TRIGGER trg_pricing_price_overlay_line_amount_no_update
        BEFORE UPDATE ON pricing_price_overlay_line_amount
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price_overlay_line_amount: UPDATE of a value under a non-draft overlay revision is not permitted')
          WHERE NOT EXISTS (
            SELECT 1 FROM pricing_price_overlay_line l
              JOIN pricing_price_overlay o
                ON o.price_overlay_id = l.price_overlay_id
               AND o.revision = l.overlay_revision
             WHERE l.line_id = OLD.line_id AND o.lifecycle_state = 'draft')
             OR NOT EXISTS (
            SELECT 1 FROM pricing_price_overlay_line l
              JOIN pricing_price_overlay o
                ON o.price_overlay_id = l.price_overlay_id
               AND o.revision = l.overlay_revision
             WHERE l.line_id = NEW.line_id AND o.lifecycle_state = 'draft');
        END",
    "CREATE TRIGGER trg_pricing_price_overlay_line_amount_no_delete
        BEFORE DELETE ON pricing_price_overlay_line_amount
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price_overlay_line_amount: DELETE of a value under a non-draft overlay revision is not permitted')
          WHERE NOT EXISTS (
            SELECT 1 FROM pricing_price_overlay_line l
              JOIN pricing_price_overlay o
                ON o.price_overlay_id = l.price_overlay_id
               AND o.revision = l.overlay_revision
             WHERE l.line_id = OLD.line_id AND o.lifecycle_state = 'draft');
        END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS pricing_price_overlay_line_amount"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
