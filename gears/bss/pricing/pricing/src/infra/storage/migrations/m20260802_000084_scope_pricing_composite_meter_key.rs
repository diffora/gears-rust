//! `pricing_composite_meter`'s primary key gains `tenant_id` and `plan_id` —
//! D-340's class, and the instance D-340 itself left standing.
//!
//! `m20260802_000046` keyed the table `(composite_id, plan_revision)` and its
//! module doc named the model outright: *"`PRIMARY KEY (composite_id,
//! plan_revision)` is `pricing_plan_phase`'s shape one table over"*. That sentence
//! was true when it was written and stopped being true on 2026-08-17, when
//! `m20260802_000081` widened `pricing_plan_phase` to
//! `(tenant_id, plan_id, plan_revision, phase_id)` **for this exact reason** and
//! did not move its acknowledged twin. The doc has since advertised a resemblance
//! to a key that no longer exists.
//!
//! Every word of `m20260802_000046`'s own argument survives here, because the
//! argument was never about scope: `plan_revision` is in the key because a
//! composite definition is plan-shape configuration versioned with the plan
//! revision (D-106, §6), so opening a draft copies the rows under the new number
//! with a **stable `composite_id`** and a published revision's rows are immutable
//! with it. What that argument never said, and what the key nevertheless asserted,
//! is that a composite id belongs to one plan **per revision number across the
//! whole table**, every tenant's included.
//!
//! # `composite_id` is client-supplied, which is what makes it reachable
//!
//! `api/rest/plans.rs` renders it `view.composite_id.unwrap_or_else(Uuid::now_v7)`
//! — the mint-if-absent idiom, and one of exactly three hits crate-wide, the other
//! two being `phase_id` (fixed by `m20260802_000081`) and `line_id`. Supplying it
//! is the intended usage and has to be: D-19's clone remap and D-83's copy-forward
//! both hand the server an id it did not mint.
//!
//! So any `plan × write` holder met this key simply by naming an id, through the
//! `composites` PATCH facet, and the two consequences are `m20260802_000081`'s
//! verbatim. A caller in one tenant naming an id another tenant holds at the same
//! `plan_revision` was refused; a caller naming a free one was not; and the
//! tenant-scoped reads on this path see nothing either way, so the whole
//! discrimination came from the key — an oracle over another tenant's composite
//! ids on a table this gear scopes by `tenant_id` everywhere else. And the *first*
//! tenant to take an id at revision `0` locked every other tenant out of it at
//! that number permanently, which is the half `m20260802_000081` measured as
//! unrecoverable on the stand.
//!
//! # What the widened key still refuses, and why that half matters
//!
//! `(tenant_id, plan_id, plan_revision, composite_id)` admits exactly one row per
//! `(plan, revision, composite)`, so **one revision may still not hold the same
//! composite id twice**. That is not a leftover. `list_composites` hands back a set
//! the self-reference walk and `COMPOSITE_TOO_FEW_CONSTITUENTS`'s arity rule both
//! quantify over, and both are written as though a composite id names at most one
//! row of a revision. A widening that also admitted the duplicate would have
//! satisfied the collision this migration is about and quietly made those rules
//! judge a set nobody can author. `tests/sqlite_plan_repo.rs` carries one probe per
//! direction for that reason, the negative control green before this change as
//! well as after.
//!
//! The new tuple is exactly what `idx_pricing_composite_meter_revision` already
//! ranges over, and what `uq_pricing_composite_meter_output` already leads with —
//! `(tenant_id, plan_id, plan_revision, output_unit)`, correctly scoped since
//! `m20260802_000046`. The primary key was the only tuple on this table that
//! omitted the tenant. No foreign key in this schema names it: the table has one FK
//! **outward** (`fk_pricing_composite_meter_revision` → `pricing_plan`) and none
//! **onto** it, verified across the whole chain, so the ripple is the key itself
//! plus two additional `primary_key` attributes on `entity/composite_meter.rs`.
//! `composite_meter::Entity::find_by_id` has no call site, so the entity's key
//! arity is not a signature anything spells out.
//!
//! # Postgres is two statements and `SQLite` is a whole-table rebuild
//!
//! `m20260802_000081`'s asymmetry exactly, and its arrangement: Postgres names its
//! primary key as a droppable constraint; `SQLite` spells `PRIMARY KEY` inside the
//! `CREATE TABLE` and offers no `ALTER` that reaches it, so the table is rebuilt
//! whole — and `DROP TABLE` takes both indexes and all three append-only triggers
//! with it, so every one of them is recreated after the rename.
//!
//! The rebuild is parameterised by the primary key alone, so `up` and `down`
//! cannot drift in any other respect: the columns, the `CHECK` and the composite
//! foreign key are written once. The index and trigger statements are
//! `m20260802_000046`'s, carried over **character for character** rather than
//! retyped from a reading of it — `m20260802_000065`'s doc records what retyping
//! did there (two triggers dropped outright and a third's refusal message
//! shortened), and `sqlite_migrations.rs` pins every trigger body by digest, so a
//! verbatim copy is the one form of this change that census can confirm lost
//! nothing. **No digest moves**, which is the property to check rather than an
//! expectation to hold.
//!
//! The rows are copied under an explicit column list on both sides of the
//! `SELECT`. A bare `INSERT INTO … SELECT * FROM …` would bind by position, and
//! the position of a column is the one property of this table nobody has promised
//! to keep.
//!
//! # `down` restores a key the data may no longer fit
//!
//! Both engines' `down` re-narrows to `(composite_id, plan_revision)`, and on a
//! database where two plans have since taken one composite id at one revision
//! number it fails — the Postgres `ADD PRIMARY KEY` on a duplicate, the `SQLite`
//! `INSERT … SELECT` on the rebuilt table's key. That is correct and not an
//! oversight: the narrow key cannot represent those rows, so a `down` that appeared
//! to succeed would have had to drop some of them.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema.
// ---------------------------------------------------------------------------
//
// `pricing_composite_meter_pkey` is the name the server assigns when a
// `CREATE TABLE` declares the key inline, which is how `m20260802_000046`
// declares it; re-added under the same name so the constraint keeps one spelling
// across the change.

const PG_UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_composite_meter DROP CONSTRAINT pricing_composite_meter_pkey",
    "ALTER TABLE bss.pricing_composite_meter
        ADD CONSTRAINT pricing_composite_meter_pkey
        PRIMARY KEY (tenant_id, plan_id, plan_revision, composite_id)",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_composite_meter DROP CONSTRAINT pricing_composite_meter_pkey",
    "ALTER TABLE bss.pricing_composite_meter
        ADD CONSTRAINT pricing_composite_meter_pkey
        PRIMARY KEY (composite_id, plan_revision)",
];

// ---------------------------------------------------------------------------
// SQLite variant - the rebuild.
// ---------------------------------------------------------------------------

/// The rebuild, parameterised by the primary key so `up` and `down` differ in
/// nothing else.
///
/// `PRAGMA foreign_keys = off` is not available here — the runner has the
/// statement inside a transaction, where the pragma is a silent no-op — so the
/// order is what makes the swap safe instead: build beside the old table, copy,
/// drop (which takes the old table's indexes and triggers with it, and whose
/// implicit row removal fires no trigger), rename, then restore both indexes and
/// all three triggers. Nothing in this schema holds a foreign key onto
/// `pricing_composite_meter`, so the drop breaks no reference; the FK this table
/// holds **onto** `pricing_plan` is satisfied throughout, because every row copied
/// already named a stored revision.
macro_rules! sqlite_rebuild {
    ($pk:literal) => {
        &[
            concat!(
                "CREATE TABLE pricing_composite_meter_rebuilt (
        composite_id      text   NOT NULL,
        plan_revision     bigint NOT NULL,
        tenant_id         text   NOT NULL,
        plan_id           text   NOT NULL,
        output_unit       text   NOT NULL,
        constituent_units text   NOT NULL,
        formula           text   NOT NULL,
        PRIMARY KEY (",
                $pk,
                "),
        CONSTRAINT chk_pricing_composite_meter_output_unit CHECK (
            length(output_unit) > 0),
        CONSTRAINT fk_pricing_composite_meter_revision FOREIGN KEY (plan_id, plan_revision)
            REFERENCES pricing_plan (plan_id, revision)
    )"
            ),
            "INSERT INTO pricing_composite_meter_rebuilt (
        composite_id, plan_revision, tenant_id, plan_id, output_unit,
        constituent_units, formula)
     SELECT
        composite_id, plan_revision, tenant_id, plan_id, output_unit,
        constituent_units, formula
     FROM pricing_composite_meter",
            "DROP TABLE pricing_composite_meter",
            "ALTER TABLE pricing_composite_meter_rebuilt RENAME TO pricing_composite_meter",
            "CREATE UNIQUE INDEX uq_pricing_composite_meter_output
        ON pricing_composite_meter (tenant_id, plan_id, plan_revision, output_unit)",
            "CREATE INDEX idx_pricing_composite_meter_revision
        ON pricing_composite_meter (tenant_id, plan_id, plan_revision)",
            "CREATE TRIGGER trg_pricing_composite_meter_no_insert
        BEFORE INSERT ON pricing_composite_meter
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_composite_meter: INSERT of a composite under a non-draft plan revision is not permitted')
          WHERE NOT EXISTS (
            SELECT 1 FROM pricing_plan
             WHERE plan_id = NEW.plan_id AND revision = NEW.plan_revision
               AND lifecycle_state = 'draft');
        END",
            // Both ends: the revision the row leaves and the revision it lands under.
            "CREATE TRIGGER trg_pricing_composite_meter_no_update
        BEFORE UPDATE ON pricing_composite_meter
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_composite_meter: UPDATE of a composite under a non-draft plan revision is not permitted')
          WHERE NOT EXISTS (
            SELECT 1 FROM pricing_plan
             WHERE plan_id = OLD.plan_id AND revision = OLD.plan_revision
               AND lifecycle_state = 'draft')
             OR NOT EXISTS (
            SELECT 1 FROM pricing_plan
             WHERE plan_id = NEW.plan_id AND revision = NEW.plan_revision
               AND lifecycle_state = 'draft');
        END",
            "CREATE TRIGGER trg_pricing_composite_meter_no_delete
        BEFORE DELETE ON pricing_composite_meter
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_composite_meter: DELETE of a composite under a non-draft plan revision is not permitted')
          WHERE NOT EXISTS (
            SELECT 1 FROM pricing_plan
             WHERE plan_id = OLD.plan_id AND revision = OLD.plan_revision
               AND lifecycle_state = 'draft');
        END",
        ]
    };
}

const SQLITE_UP_STATEMENTS: &[&str] =
    sqlite_rebuild!("tenant_id, plan_id, plan_revision, composite_id");

const SQLITE_DOWN_STATEMENTS: &[&str] = sqlite_rebuild!("composite_id, plan_revision");

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
