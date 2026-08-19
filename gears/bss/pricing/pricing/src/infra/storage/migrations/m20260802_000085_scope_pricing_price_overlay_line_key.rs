//! `pricing_price_overlay_line`'s primary key gains `tenant_id`, and its amount
//! table's key and foreign key move with it — D-340's class, review A1-3.
//!
//! `m20260802_000033` keyed the line `(line_id, overlay_revision)` and argued the
//! pair at length: `overlay_revision` because a copy-on-new-revision needs the
//! revision in the key or D-92's stable line identity is unexpressible, and
//! `line_id` because §6's literal `PK line_id` cannot hold two copies of one line.
//! Both halves survive here untouched. What that argument never mentions — the
//! module doc never uses the word *tenant* — is that a line id therefore belonged
//! to one overlay per revision **number across the entire table, every tenant's
//! included**.
//!
//! # `line_id` is client-supplied, and supplying it is the documented usage
//!
//! `api/rest/overlays.rs` renders it `request.line_id.unwrap_or_else(Uuid::now_v7)`
//! and the route description names the read-modify-write round trip as the
//! intended flow, because that is what makes D-92's *"the identity is stable"* true
//! of an authored edit and not only of the server's copy-forward. It is one of
//! exactly three client-supplied ids in this schema; the other two were
//! `phase_id` (`m20260802_000081`) and `composite_id` (`m20260802_000084`).
//!
//! # Why this one was Medium and what remained of it
//!
//! Unlike the other two, the refusal here was already **typed**:
//! `overlay_repo::is_line_identity_collision` gates on `is_unique_violation()`
//! before matching the constraint, and the caller gets
//! `RepoError::ValueOutOfRange { field: "line_id", … }` rather than a `500`. So
//! the caller-fixable half was fixed. What remained is the isolation half, and a
//! 4xx that discriminates is still an oracle: the difference between the refusal
//! and the `200` answered *is this line id in use somewhere I cannot read*. The
//! permanent-lockout half applies identically — the first tenant to take a line id
//! at a revision number held it against every other tenant, at that number, for
//! good.
//!
//! # What this migration does **not** do, and the reason is a schema decision
//!
//! The review's stated fix is `(tenant_id, price_overlay_id, overlay_revision,
//! line_id)`. This migration widens by `tenant_id` alone, to
//! `(tenant_id, overlay_revision, line_id)`, and the `price_overlay_id` half is
//! deliberately left — recorded here rather than quietly dropped.
//!
//! The two halves close different things and only one of them is isolation.
//! `tenant_id` closes it completely: no id a caller supplies can collide with, or
//! be probed against, a row in another tenant. `price_overlay_id` would close a
//! second, **intra-tenant** question — that pasting a line id from one of a
//! tenant's own overlays into another at the same revision collides — which is the
//! caller-fixable half that is already typed, already legible, and already
//! remediable by the author who caused it.
//!
//! And it is not a mechanical widening. `pricing_price_overlay_line_amount` does
//! not carry `price_overlay_id`, so a parent key containing it could not be
//! referenced by the child at all: the column would have to be **added** to the
//! amount table and backfilled through the line, which is a change to what the
//! child row *is* and not to how the parent is keyed. That is worth deciding on
//! its own rather than as a rider here.
//!
//! # The child moves in the same migration, and A1-4 is why
//!
//! `pricing_price_overlay_line_amount` is keyed `(line_id, overlay_revision,
//! currency)` and its insert maps every failure to `RepoError::Db` → `500`. That
//! catch-all is unreachable today for a reason review A1-4 records exactly: the
//! line insert refuses the colliding `line_id` first, and a `line_id` past that
//! check was unique at that revision *globally*, so its amounts were too. A1-4
//! then names the condition that arms it — *"if A1-3's fix scopes the line key by
//! tenant, then two tenants can hold the same `line_id` at one revision, and the
//! amount table's key must widen in the same change or this catch-all becomes
//! reachable as a `500`"*. It widens in the same change.
//!
//! # The amount table's append-only trigger had to change, and it is the only
//! # trigger body this migration moves
//!
//! This is the half that is not a key. The amount guard resolves its parent by
//! `(line_id, overlay_revision)` and nothing else, which was unambiguous while the
//! line key was globally unique and stops being so here. Left alone, an amount
//! under a **published** line would find another tenant's **draft** line carrying
//! the same `(line_id, overlay_revision)`, read `draft`, and be admitted — D-92's
//! freeze defeated across a tenant boundary by a widening meant to close a leak.
//!
//! All three `SQLite` bodies and the Postgres function therefore gain
//! `l.tenant_id = NEW.tenant_id` (`OLD.tenant_id` on the arms that ask where the
//! row comes from). Those three digests move in `tests/sqlite_migrations.rs` and
//! no others: the **line's** own triggers resolve their parent through
//! `price_overlay_id`, whose own key `(price_overlay_id, revision)` is rooted in a
//! server-minted id and is globally unique, so nothing about them is ambiguous and
//! they are carried over character for character.
//!
//! # Both tables are rebuilt on `SQLite`, child-first on the way down
//!
//! Neither engine has an `ALTER` that reaches a `SQLite` `PRIMARY KEY`, so both
//! tables are rebuilt whole and every index and trigger they carry is recreated.
//! The order is what makes it safe with a foreign key between them, and the first
//! order tried was wrong in a way worth recording: building both replacements side
//! by side and copying into each fails `foreign key mismatch`, because
//! `foreign_keys` is ON in this gear's harness and the child's new reference names
//! columns that are not a key on the parent **yet**.
//!
//! So the child's rows are parked in a stash carrying **no constraints at all**,
//! the old pair is dropped child-first, the parent is renamed into place, and only
//! then is the child rebuilt and its rows copied back — where the foreign key is
//! checked for real against the key it now names, rather than skipped.
//!
//! The rows are copied under an explicit column list on both sides of each
//! `SELECT`. A bare `INSERT INTO … SELECT * FROM …` would bind by position, and
//! the position of a column is the one property of these tables nobody has
//! promised to keep.
//!
//! # `down` restores keys the data may no longer fit
//!
//! Both engines' `down` re-narrows, and on a database where two tenants have since
//! taken one line id at one revision it fails. That is correct: the narrow key
//! cannot represent those rows, so a `down` that appeared to succeed would have had
//! to drop some of them. `m20260802_000081` states the same property for the same
//! reason.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema.
// ---------------------------------------------------------------------------
//
// The child's foreign key has to be dropped before the parent's primary key can
// be, and re-added after: a `PRIMARY KEY` naming columns a foreign key depends on
// cannot be dropped while the dependency stands. Both constraints keep their
// names across the change, so every census that spells them is unmoved.

/// The amount table's guard, with the tenant conjunct. See the module doc.
const PG_AMOUNT_FUNCTION_SCOPED: &str =
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
             WHERE l.line_id = OLD.line_id AND l.overlay_revision = OLD.overlay_revision
               AND l.tenant_id = OLD.tenant_id;
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
           WHERE l.line_id = NEW.line_id AND l.overlay_revision = NEW.overlay_revision
             AND l.tenant_id = NEW.tenant_id;
          IF parent_state IS DISTINCT FROM 'draft' THEN
            RAISE EXCEPTION
              'pricing_price_overlay_line_amount: % of a value under a non-draft overlay revision is not permitted (state %)',
              TG_OP, coalesce(parent_state, 'missing');
          END IF;

          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql";

/// `m20260802_000034`'s body, verbatim — what `down` restores.
const PG_AMOUNT_FUNCTION_UNSCOPED: &str =
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
             WHERE l.line_id = OLD.line_id AND l.overlay_revision = OLD.overlay_revision;
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
           WHERE l.line_id = NEW.line_id AND l.overlay_revision = NEW.overlay_revision;
          IF parent_state IS DISTINCT FROM 'draft' THEN
            RAISE EXCEPTION
              'pricing_price_overlay_line_amount: % of a value under a non-draft overlay revision is not permitted (state %)',
              TG_OP, coalesce(parent_state, 'missing');
          END IF;

          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql";

const PG_UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_price_overlay_line_amount
        DROP CONSTRAINT fk_pricing_price_overlay_line_amount_line",
    "ALTER TABLE bss.pricing_price_overlay_line DROP CONSTRAINT pricing_price_overlay_line_pkey",
    "ALTER TABLE bss.pricing_price_overlay_line
        ADD CONSTRAINT pricing_price_overlay_line_pkey
        PRIMARY KEY (tenant_id, overlay_revision, line_id)",
    "ALTER TABLE bss.pricing_price_overlay_line_amount
        DROP CONSTRAINT pricing_price_overlay_line_amount_pkey",
    "ALTER TABLE bss.pricing_price_overlay_line_amount
        ADD CONSTRAINT pricing_price_overlay_line_amount_pkey
        PRIMARY KEY (tenant_id, overlay_revision, line_id, currency)",
    "ALTER TABLE bss.pricing_price_overlay_line_amount
        ADD CONSTRAINT fk_pricing_price_overlay_line_amount_line
        FOREIGN KEY (tenant_id, overlay_revision, line_id)
        REFERENCES bss.pricing_price_overlay_line (tenant_id, overlay_revision, line_id)",
    PG_AMOUNT_FUNCTION_SCOPED,
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_price_overlay_line_amount
        DROP CONSTRAINT fk_pricing_price_overlay_line_amount_line",
    "ALTER TABLE bss.pricing_price_overlay_line_amount
        DROP CONSTRAINT pricing_price_overlay_line_amount_pkey",
    "ALTER TABLE bss.pricing_price_overlay_line_amount
        ADD CONSTRAINT pricing_price_overlay_line_amount_pkey
        PRIMARY KEY (line_id, overlay_revision, currency)",
    "ALTER TABLE bss.pricing_price_overlay_line DROP CONSTRAINT pricing_price_overlay_line_pkey",
    "ALTER TABLE bss.pricing_price_overlay_line
        ADD CONSTRAINT pricing_price_overlay_line_pkey
        PRIMARY KEY (line_id, overlay_revision)",
    "ALTER TABLE bss.pricing_price_overlay_line_amount
        ADD CONSTRAINT fk_pricing_price_overlay_line_amount_line
        FOREIGN KEY (line_id, overlay_revision)
        REFERENCES bss.pricing_price_overlay_line (line_id, overlay_revision)",
    PG_AMOUNT_FUNCTION_UNSCOPED,
];

// ---------------------------------------------------------------------------
// SQLite variant - two rebuilds, child dropped first and renamed last.
// ---------------------------------------------------------------------------

/// The pair of rebuilds, parameterised by the two primary keys and by the amount
/// guard's tenant conjunct, so `up` and `down` differ in nothing else.
///
/// `PRAGMA foreign_keys = off` is not available here — the runner has the
/// statement inside a transaction, where the pragma is a silent no-op — so the
/// order is what makes the swap safe: build both replacements beside the
/// originals, copy both, drop the child before the parent so no reference
/// outlives its referent, then rename parent before child. The drops take the old
/// tables' indexes and triggers with them, so every one is recreated at the end;
/// the implicit row removal of a `DROP TABLE` fires no trigger.
macro_rules! sqlite_rebuild {
    ($line_pk:literal, $amount_pk:literal, $fk_cols:literal, $scope_old:literal, $scope_new:literal) => {
        &[
            concat!(
                "CREATE TABLE pricing_price_overlay_line_rebuilt (
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
        PRIMARY KEY (",
                $line_pk,
                "),
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
            OR adjustment_value <= 10000),
        CONSTRAINT chk_pricing_price_overlay_line_plan_id_not_nil CHECK (
            plan_id IS NULL OR plan_id <> '00000000-0000-0000-0000-000000000000'),
        CONSTRAINT chk_pricing_price_overlay_line_target_sku_present CHECK (
            target_sku IS NULL OR length(target_sku) > 0)
    )"
            ),
            "INSERT INTO pricing_price_overlay_line_rebuilt (
        line_id, price_overlay_id, overlay_revision, tenant_id, plan_id, target_sku,
        cohort, adjustment_kind, magnitude_kind, adjustment_value)
     SELECT
        line_id, price_overlay_id, overlay_revision, tenant_id, plan_id, target_sku,
        cohort, adjustment_kind, magnitude_kind, adjustment_value
     FROM pricing_price_overlay_line",
            // The amounts are parked in a table with **no constraints at all**,
            // and that is the whole reason this is a stash rather than a second
            // `_rebuilt` beside the first. `foreign_keys` is ON in this gear's
            // test harness, so a child created with the *new* reference and filled
            // while the parent still holds the *old* key is a `foreign key
            // mismatch` — the reference names columns that are not yet a key
            // anywhere. Parking the rows outside any reference, and rebuilding the
            // child only once the parent has been renamed into place, is what makes
            // the copy back a real foreign-key check rather than a skipped one.
            "CREATE TABLE pricing_price_overlay_line_amount_stash (
        line_id          text   NOT NULL,
        overlay_revision bigint NOT NULL,
        currency         text   NOT NULL,
        tenant_id        text   NOT NULL,
        value_minor      bigint NOT NULL
    )",
            "INSERT INTO pricing_price_overlay_line_amount_stash (
        line_id, overlay_revision, currency, tenant_id, value_minor)
     SELECT
        line_id, overlay_revision, currency, tenant_id, value_minor
     FROM pricing_price_overlay_line_amount",
            "DROP TABLE pricing_price_overlay_line_amount",
            "DROP TABLE pricing_price_overlay_line",
            "ALTER TABLE pricing_price_overlay_line_rebuilt RENAME TO pricing_price_overlay_line",
            concat!(
                "CREATE TABLE pricing_price_overlay_line_amount (
        line_id          text   NOT NULL,
        overlay_revision bigint NOT NULL,
        currency         text   NOT NULL,
        tenant_id        text   NOT NULL,
        value_minor      bigint NOT NULL,
        PRIMARY KEY (",
                $amount_pk,
                "),
        CONSTRAINT fk_pricing_price_overlay_line_amount_line
            FOREIGN KEY (",
                $fk_cols,
                ")
            REFERENCES pricing_price_overlay_line (",
                $fk_cols,
                "),
        CONSTRAINT chk_pricing_price_overlay_line_amount_value_minor CHECK (
            value_minor >= 0),
        CONSTRAINT chk_pricing_price_overlay_line_amount_currency CHECK (
            length(currency) = 3)
    )"
            ),
            "INSERT INTO pricing_price_overlay_line_amount (
        line_id, overlay_revision, currency, tenant_id, value_minor)
     SELECT
        line_id, overlay_revision, currency, tenant_id, value_minor
     FROM pricing_price_overlay_line_amount_stash",
            "DROP TABLE pricing_price_overlay_line_amount_stash",
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
            "CREATE INDEX idx_pricing_price_overlay_line_amount_tenant
        ON pricing_price_overlay_line_amount (tenant_id, line_id)",
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
            concat!(
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
             WHERE l.line_id = NEW.line_id AND l.overlay_revision = NEW.overlay_revision",
                $scope_new,
                "
               AND o.lifecycle_state = 'draft');
        END"
            ),
            concat!(
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
             WHERE l.line_id = OLD.line_id AND l.overlay_revision = OLD.overlay_revision",
                $scope_old,
                "
               AND o.lifecycle_state = 'draft')
             OR NOT EXISTS (
            SELECT 1 FROM pricing_price_overlay_line l
              JOIN pricing_price_overlay o
                ON o.price_overlay_id = l.price_overlay_id
               AND o.revision = l.overlay_revision
             WHERE l.line_id = NEW.line_id AND l.overlay_revision = NEW.overlay_revision",
                $scope_new,
                "
               AND o.lifecycle_state = 'draft');
        END"
            ),
            concat!(
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
             WHERE l.line_id = OLD.line_id AND l.overlay_revision = OLD.overlay_revision",
                $scope_old,
                "
               AND o.lifecycle_state = 'draft');
        END"
            ),
        ]
    };
}

const SQLITE_UP_STATEMENTS: &[&str] = sqlite_rebuild!(
    "tenant_id, overlay_revision, line_id",
    "tenant_id, overlay_revision, line_id, currency",
    "tenant_id, overlay_revision, line_id",
    "\n               AND l.tenant_id = OLD.tenant_id",
    "\n               AND l.tenant_id = NEW.tenant_id"
);

const SQLITE_DOWN_STATEMENTS: &[&str] = sqlite_rebuild!(
    "line_id, overlay_revision",
    "line_id, overlay_revision, currency",
    "line_id, overlay_revision",
    "",
    ""
);

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
