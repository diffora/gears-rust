//! Create `bss.pricing_price_overlay_line` — the adjustment lines of one
//! overlay revision (`design/09-price-overlays.md` §6, D-42, D-67, D-78, D-92,
//! D-138).
//!
//! D-42 made an overlay a **container of one or more adjustment lines**, each
//! keyed `(planId?, targetSku?)` with its own kind and magnitude, and D-78
//! extended that key with `cohort`. This table is that key.
//!
//! # The uniqueness is **null-safe**, and the naive form is silently wrong
//!
//! §6 states `UNIQUE (price_overlay_id, overlay_revision, plan_id, target_sku,
//! cohort)` and adds *"(null-safe — one default line, one line per plan, one per
//! `(plan, sku)`…)"*. A plain `UNIQUE` over those five columns delivers none of
//! that: inside a `UNIQUE` index NULLs are **distinct** on both engines, and all
//! three of `plan_id`, `target_sku` and `cohort` are nullable — so the very case
//! §6 spells *"one default line"* (all three NULL) would admit any number of
//! rows, and so would "one line per plan" (two of the three NULL). The rule
//! would be stated, indexed, and enforcing nothing.
//!
//! The index therefore keys over `COALESCE`d sentinels, which is
//! `m20260802_000023`'s treatment of `meter` applied to three columns at once.
//! Each sentinel is chosen so nothing else can render it:
//!
//! * `target_sku` -> `''`. A SKU may not be blank; `overlay_repo` refuses one.
//! * `cohort` -> `'-infinity'` on Postgres, `''` on `SQLite`. A cohort is a
//!   cutover instant and no cutover is at negative infinity. The two backends
//!   differ here because each sentinel is type-native to its column, which is
//!   what keeps the expression from silently coercing.
//! * `plan_id` -> the **nil uuid**. A plan id is a v4/v7 uuid and never nil, but
//!   unlike the other two this sentinel is *forgeable from the wire*: a request
//!   naming `00000000-…-0000` as its plan would key as the list-default line and
//!   collide with it. `overlay_repo::check_line_targets` refuses a nil plan id
//!   for that reason, and `sqlite_overlay_repo` proves it — the constraint is
//!   the guarantee and the check is what makes the refusal legible.
//!
//! # The `CHECK`s, and which decision each is
//!
//! * `cohort_needs_plan` (§6, 2026-07-31 review fix) — a `cohort` is validated
//!   against *"the line's target plan"*, which the list-default line has none of.
//!   Targeting a generation across every plan of a scope is authored as per-plan
//!   lines, never as a cohort-carrying default line.
//! * `sku_needs_plan` (§3 step 2a) — a bare SKU id is ambiguous per
//!   `(currency, region)`, so a sku-only line names no resolvable target.
//! * `magnitude_pairing` (D-08) — `(magnitude_kind = 'percent_bp') =
//!   (adjustment_value IS NOT NULL)`. The value type is **declared, never
//!   inferred from the presence of amount rows**: implicit-absence semantics are
//!   forbidden by the Foundation, and a bp value read as minor units mis-prices
//!   by orders of magnitude.
//! * `fixed_is_amount` (D-138) — `fixed` **replaces** the running line amount
//!   with an absolute price, so a percentage of the amount it is replacing
//!   evaluates to nothing.
//! * `magnitude_positive` + `discount_ceiling` (D-67) — `0 < v` on both kinds and
//!   `v <= 10000` on a discount. Before D-67 the only checks were duplicate line
//!   keys, out-of-scope targets, per-currency coverage and tax-basis
//!   declaration, so `discount / percent_bp = 15000` — the "150% of list"
//!   data-entry inversion — passed every stated validation. Two `CHECK`s rather
//!   than one compound arm because they are two rules and a compound refusal
//!   would not tell an author which bound they crossed.
//!
//! # The resolution order is **not** here, and that is D-42's own line
//!
//! `(plan, sku) > (plan) > default` is a **pipeline rule** (`inst-plv-lines`),
//! not an index: it selects which of several legal lines applies to a priced
//! row, and every one of them is legal. It lives in `domain::overlay_rules`, and
//! it is never `precedence` — precedence orders **lists** against each other and
//! never lines inside one.
//!
//! # The append-only trigger
//!
//! Lines are frozen with the revision that published them (D-92,
//! copy-on-new-revision). INSERT is guarded as well as UPDATE and DELETE,
//! because an INSERT is the one verb that **adds** a line to a frozen revision;
//! an UPDATE is checked against both the `OLD` parent and the `NEW` one, since
//! re-pointing `overlay_revision` is how one would otherwise append to a frozen
//! revision without ever issuing an INSERT. This is
//! `m20260802_000025`'s arrangement one slice over, one join shorter.
//!
//! **The trigger answers ahead of the composite foreign key**, and the suite
//! says so rather than asserting a message it will not get: an INSERT naming an
//! absent `(price_overlay_id, overlay_revision)` trips the `NOT EXISTS` in the
//! `BEFORE INSERT` trigger before `SQLite` evaluates the foreign key. The
//! foreign key is proved on its own by a DELETE of a **draft** revision that
//! still carries lines, which the header's own trigger permits and this table's
//! reference refuses.
//!
//! **Backend differences.** As `m20260802_000032`, plus: the expression index is
//! written per backend because the `cohort` sentinel is type-native to each. Note
//! that an expression index changes `SQLite`'s uniqueness message from the column
//! list to `index '<name>'`, while Postgres names the index either way — which is
//! why both suites assert the **index name**.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.pricing_price_overlay_line (
        line_id          uuid   NOT NULL,
        price_overlay_id uuid   NOT NULL,
        overlay_revision bigint NOT NULL,
        tenant_id        uuid   NOT NULL,
        plan_id          uuid,
        target_sku       text,
        cohort           timestamptz,
        adjustment_kind  text   NOT NULL,
        magnitude_kind   text   NOT NULL,
        adjustment_value bigint,
        PRIMARY KEY (line_id),
        CONSTRAINT fk_pricing_price_overlay_line_overlay
            FOREIGN KEY (price_overlay_id, overlay_revision)
            REFERENCES bss.pricing_price_overlay (price_overlay_id, revision),
        CONSTRAINT chk_pricing_price_overlay_line_adjustment_kind CHECK (
            adjustment_kind IN ('markup', 'discount', 'fixed')),
        CONSTRAINT chk_pricing_price_overlay_line_magnitude_kind CHECK (
            magnitude_kind IN ('percent_bp', 'amount')),
        CONSTRAINT chk_pricing_price_overlay_line_magnitude_pairing CHECK (
            (magnitude_kind = 'percent_bp') = (adjustment_value IS NOT NULL)),
        CONSTRAINT chk_pricing_price_overlay_line_fixed_is_amount CHECK (
            adjustment_kind <> 'fixed' OR magnitude_kind = 'amount'),
        CONSTRAINT chk_pricing_price_overlay_line_cohort_needs_plan CHECK (
            cohort IS NULL OR plan_id IS NOT NULL),
        CONSTRAINT chk_pricing_price_overlay_line_sku_needs_plan CHECK (
            target_sku IS NULL OR plan_id IS NOT NULL),
        CONSTRAINT chk_pricing_price_overlay_line_magnitude_positive CHECK (
            adjustment_value IS NULL OR adjustment_value > 0),
        CONSTRAINT chk_pricing_price_overlay_line_discount_ceiling CHECK (
            adjustment_kind <> 'discount' OR adjustment_value IS NULL
            OR adjustment_value <= 10000)
    )",
    // The null-safe line key. See the module doc for each sentinel's argument.
    "CREATE UNIQUE INDEX uq_pricing_price_overlay_line_key
        ON bss.pricing_price_overlay_line (
            price_overlay_id,
            overlay_revision,
            COALESCE(plan_id, '00000000-0000-0000-0000-000000000000'::uuid),
            COALESCE(target_sku, ''),
            COALESCE(cohort, '-infinity'::timestamptz))",
    // The copy-forward, the drop-on-discard and the whole-set read all range
    // over one revision's lines under one tenant.
    "CREATE INDEX idx_pricing_price_overlay_line_revision
        ON bss.pricing_price_overlay_line (tenant_id, price_overlay_id, overlay_revision)",
    // D-31's mirror question - which overlay lines target this plan - and
    // `inst-plv-dating`'s per-line-key collision walk both read this way.
    "CREATE INDEX idx_pricing_price_overlay_line_plan
        ON bss.pricing_price_overlay_line (tenant_id, plan_id)",
    "CREATE OR REPLACE FUNCTION bss.pricing_price_overlay_line_append_only() RETURNS trigger AS $$
        DECLARE
          parent_state text;
        BEGIN
          IF TG_OP <> 'INSERT' THEN
            SELECT o.lifecycle_state INTO parent_state
              FROM bss.pricing_price_overlay o
             WHERE o.price_overlay_id = OLD.price_overlay_id
               AND o.revision = OLD.overlay_revision;
            IF parent_state IS DISTINCT FROM 'draft' THEN
              RAISE EXCEPTION
                'pricing_price_overlay_line: % of a line under a non-draft overlay revision is not permitted (state %)',
                TG_OP, coalesce(parent_state, 'missing');
            END IF;
          END IF;

          IF TG_OP = 'DELETE' THEN
            RETURN OLD;
          END IF;

          SELECT o.lifecycle_state INTO parent_state
            FROM bss.pricing_price_overlay o
           WHERE o.price_overlay_id = NEW.price_overlay_id
             AND o.revision = NEW.overlay_revision;
          IF parent_state IS DISTINCT FROM 'draft' THEN
            RAISE EXCEPTION
              'pricing_price_overlay_line: % of a line under a non-draft overlay revision is not permitted (state %)',
              TG_OP, coalesce(parent_state, 'missing');
          END IF;

          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_pricing_price_overlay_line_append_only
        BEFORE INSERT OR UPDATE OR DELETE ON bss.pricing_price_overlay_line
        FOR EACH ROW EXECUTE FUNCTION bss.pricing_price_overlay_line_append_only()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS bss.pricing_price_overlay_line",
    "DROP FUNCTION IF EXISTS bss.pricing_price_overlay_line_append_only()",
];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE pricing_price_overlay_line (
        line_id          text   NOT NULL,
        price_overlay_id text   NOT NULL,
        overlay_revision bigint NOT NULL,
        tenant_id        text   NOT NULL,
        plan_id          text,
        target_sku       text,
        cohort           text,
        adjustment_kind  text   NOT NULL,
        magnitude_kind   text   NOT NULL,
        adjustment_value bigint,
        PRIMARY KEY (line_id),
        CONSTRAINT fk_pricing_price_overlay_line_overlay
            FOREIGN KEY (price_overlay_id, overlay_revision)
            REFERENCES pricing_price_overlay (price_overlay_id, revision),
        CONSTRAINT chk_pricing_price_overlay_line_adjustment_kind CHECK (
            adjustment_kind IN ('markup', 'discount', 'fixed')),
        CONSTRAINT chk_pricing_price_overlay_line_magnitude_kind CHECK (
            magnitude_kind IN ('percent_bp', 'amount')),
        CONSTRAINT chk_pricing_price_overlay_line_magnitude_pairing CHECK (
            (magnitude_kind = 'percent_bp') = (adjustment_value IS NOT NULL)),
        CONSTRAINT chk_pricing_price_overlay_line_fixed_is_amount CHECK (
            adjustment_kind <> 'fixed' OR magnitude_kind = 'amount'),
        CONSTRAINT chk_pricing_price_overlay_line_cohort_needs_plan CHECK (
            cohort IS NULL OR plan_id IS NOT NULL),
        CONSTRAINT chk_pricing_price_overlay_line_sku_needs_plan CHECK (
            target_sku IS NULL OR plan_id IS NOT NULL),
        CONSTRAINT chk_pricing_price_overlay_line_magnitude_positive CHECK (
            adjustment_value IS NULL OR adjustment_value > 0),
        CONSTRAINT chk_pricing_price_overlay_line_discount_ceiling CHECK (
            adjustment_kind <> 'discount' OR adjustment_value IS NULL
            OR adjustment_value <= 10000)
    )",
    "CREATE UNIQUE INDEX uq_pricing_price_overlay_line_key
        ON pricing_price_overlay_line (
            price_overlay_id,
            overlay_revision,
            COALESCE(plan_id, '00000000-0000-0000-0000-000000000000'),
            COALESCE(target_sku, ''),
            COALESCE(cohort, ''))",
    "CREATE INDEX idx_pricing_price_overlay_line_revision
        ON pricing_price_overlay_line (tenant_id, price_overlay_id, overlay_revision)",
    "CREATE INDEX idx_pricing_price_overlay_line_plan
        ON pricing_price_overlay_line (tenant_id, plan_id)",
    "CREATE TRIGGER trg_pricing_price_overlay_line_no_insert
        BEFORE INSERT ON pricing_price_overlay_line
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price_overlay_line: INSERT of a line under a non-draft overlay revision is not permitted')
          WHERE NOT EXISTS (
            SELECT 1 FROM pricing_price_overlay o
             WHERE o.price_overlay_id = NEW.price_overlay_id
               AND o.revision = NEW.overlay_revision
               AND o.lifecycle_state = 'draft');
        END",
    // Both ends: the revision the row leaves and the revision it lands under.
    "CREATE TRIGGER trg_pricing_price_overlay_line_no_update
        BEFORE UPDATE ON pricing_price_overlay_line
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price_overlay_line: UPDATE of a line under a non-draft overlay revision is not permitted')
          WHERE NOT EXISTS (
            SELECT 1 FROM pricing_price_overlay o
             WHERE o.price_overlay_id = OLD.price_overlay_id
               AND o.revision = OLD.overlay_revision
               AND o.lifecycle_state = 'draft')
             OR NOT EXISTS (
            SELECT 1 FROM pricing_price_overlay o
             WHERE o.price_overlay_id = NEW.price_overlay_id
               AND o.revision = NEW.overlay_revision
               AND o.lifecycle_state = 'draft');
        END",
    "CREATE TRIGGER trg_pricing_price_overlay_line_no_delete
        BEFORE DELETE ON pricing_price_overlay_line
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price_overlay_line: DELETE of a line under a non-draft overlay revision is not permitted')
          WHERE NOT EXISTS (
            SELECT 1 FROM pricing_price_overlay o
             WHERE o.price_overlay_id = OLD.price_overlay_id
               AND o.revision = OLD.overlay_revision
               AND o.lifecycle_state = 'draft');
        END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS pricing_price_overlay_line"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
