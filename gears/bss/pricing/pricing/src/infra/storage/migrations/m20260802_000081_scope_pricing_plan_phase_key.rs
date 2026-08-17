//! `pricing_plan_phase`'s primary key gains `tenant_id` and `plan_id` — D-340.
//!
//! `m20260802_000012` keyed the table `(phase_id, plan_revision)` and its module
//! doc argued the pair at length: `plan_revision` because a plan's shape versions
//! with its revision (D-83), `phase_id` because the `phase` axis of the canonical
//! scope key holds a **bare** id (D-19) and same-key supersession compares it
//! (D-56). Both halves of that argument survive here untouched. What neither half
//! ever said, and what the key nevertheless asserted, is that a phase id belongs
//! to **one plan per revision number across the whole table** — every tenant's
//! included.
//!
//! # The state it produced is unrecoverable, which is why this is a migration
//!
//! Measured on the stand 2026-08-17: five drafts carry price rows keyed on the one
//! phase id `1e256059-2956-4b3e-8acd-898eb43a1ff7`, because whoever seeded them
//! reused it across plans. Exactly one of the five can attach it — the other four
//! collide on the key at every revision number they will ever hold, and a scope
//! key **is** a price row's identity (`01-foundation.md` §3.7), so the rows cannot
//! be re-pointed at a different phase either. Of the two remediations D-337's
//! refusal names — attach that id, or delete the rows — four plans out of five had
//! only the second. After this migration each of the five may hold the id and all
//! five repair through the ordinary API.
//!
//! The reach is not a script's, either. The `phases` PATCH facet takes `phase_id`
//! from the client, which is what makes D-19's clone remap and D-83's copy-forward
//! expressible at all, so any `plan × write` holder met this constraint simply by
//! naming an id — and the difference between the `500` it answered and a `200`
//! answered *is this id in use somewhere I cannot read*, a probe of another
//! tenant's phase ids on a table this gear scopes by `tenant_id` everywhere else.
//!
//! # What the widened key still refuses, and why that half matters
//!
//! `(tenant_id, plan_id, plan_revision, phase_id)` admits exactly one row per
//! `(plan, revision, phase)`, so **one revision may still not hold the same phase
//! id twice**. That is not a leftover: `inst-ph-graph` walks a chain and
//! `inst-ph-default` speaks of *the* terminal phase, and both are written as
//! though a phase id names at most one row of a revision. A widening that also
//! admitted the duplicate would satisfy the collision this migration is about and
//! quietly make the graph rules judge a chain nobody can author. The paired probes
//! in `tests/sqlite_plan_phase.rs` are one per direction for that reason.
//!
//! The new tuple is exactly what `idx_pricing_plan_phase_revision` already ranges
//! over, and no foreign key in this schema names `pricing_plan_phase` — verified
//! across every migration of the chain — so the ripple is the key itself plus
//! `entity/plan_phase.rs`'s two additional `primary_key` attributes. Nothing reads
//! this table by its key: `plan_phase::Entity::find_by_id` has no call sites, so
//! the entity's key arity is not a signature anything spells out.
//!
//! `uq_pricing_plan_phase_terminal` needs no change and gets none. It was already
//! partial over `(plan_id, plan_revision) WHERE converts_to_phase_id IS NULL` —
//! per plan, not per id — so the "at most one terminal phase" rule was never
//! carried by the primary key and does not move with it.
//!
//! # Postgres is two statements and `SQLite` is a whole-table rebuild
//!
//! `m20260802_000063`'s and `m20260802_000065`'s asymmetry exactly. Postgres names
//! its primary key as a droppable constraint; `SQLite` spells `PRIMARY KEY` inside
//! the `CREATE TABLE` and offers no `ALTER` that reaches it, so the table is
//! rebuilt whole — and `DROP TABLE` takes the two indexes and all three
//! append-only triggers with it, so every one of them is recreated after the
//! rename. The `INSERT … SELECT` in between is what carries the rows: a rebuild
//! that silently dropped them would look identical to a clean install, which is
//! why `tests/sqlite_plan_phase.rs` applies this migration to a **populated**
//! schema and reads the rows back rather than trusting a fresh-database run.
//!
//! The rebuild is parameterised by the primary key alone, so `up` and `down`
//! cannot drift in any other respect — the columns, both `CHECK`s and the
//! composite foreign key are written once. The index and trigger statements are
//! `m20260802_000012`'s, carried over character for character rather than retyped
//! from a reading of it: `m20260802_000065`'s own doc records what retyping did
//! there (two triggers dropped outright and a third's refusal message shortened),
//! and `sqlite_migrations.rs` pins every trigger body by digest, so a verbatim
//! copy is the one form of this change that census can confirm lost nothing.
//!
//! The rows are copied under an explicit column list on both sides of the
//! `SELECT`. A bare `INSERT INTO … SELECT * FROM …` would bind by position, and
//! the position of a column is the one property of this table nobody has promised
//! to keep.
//!
//! # `down` restores a key the data may no longer fit
//!
//! Both engines' `down` re-narrows to `(phase_id, plan_revision)`, and on a
//! database where two plans have since taken one phase id it fails — the Postgres
//! `ADD PRIMARY KEY` on a duplicate, the `SQLite` `INSERT … SELECT` on the rebuilt
//! table's key. That is the correct behaviour and not an oversight: the narrow key
//! cannot represent those rows, so a `down` that appeared to succeed would have
//! had to drop some of them.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema.
// ---------------------------------------------------------------------------
//
// `pricing_plan_phase_pkey` is the name the server assigned when
// `m20260802_000012` declared the key inline, read off the live stand
// 2026-08-17; re-added under the same name so the constraint keeps one spelling
// across the change.

const PG_UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_plan_phase DROP CONSTRAINT pricing_plan_phase_pkey",
    "ALTER TABLE bss.pricing_plan_phase
        ADD CONSTRAINT pricing_plan_phase_pkey
        PRIMARY KEY (tenant_id, plan_id, plan_revision, phase_id)",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_plan_phase DROP CONSTRAINT pricing_plan_phase_pkey",
    "ALTER TABLE bss.pricing_plan_phase
        ADD CONSTRAINT pricing_plan_phase_pkey PRIMARY KEY (phase_id, plan_revision)",
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
/// implicit row removal fires no trigger), rename, then restore the two indexes
/// and all three triggers. Nothing in this schema holds a foreign key onto
/// `pricing_plan_phase`, so the drop breaks no reference; the FK this table holds
/// **onto** `pricing_plan` is satisfied throughout, because every row copied
/// already named a stored revision.
macro_rules! sqlite_rebuild {
    ($pk:literal) => {
        &[
            concat!(
                "CREATE TABLE pricing_plan_phase_rebuilt (
        phase_id             text   NOT NULL,
        plan_revision        bigint NOT NULL,
        tenant_id            text   NOT NULL,
        plan_id              text   NOT NULL,
        kind                 text   NOT NULL,
        ordinal              int    NOT NULL,
        converts_to_phase_id text,
        phase_duration_days  int,
        display_trial_days   int,
        PRIMARY KEY (",
                $pk,
                "),
        CONSTRAINT chk_pricing_plan_phase_kind CHECK (
            kind IN ('trial','intro','evergreen')),
        CONSTRAINT chk_pricing_plan_phase_display_trial_days CHECK (
            display_trial_days IS NULL OR display_trial_days = phase_duration_days),
        CONSTRAINT fk_pricing_plan_phase_revision FOREIGN KEY (plan_id, plan_revision)
            REFERENCES pricing_plan (plan_id, revision)
    )"
            ),
            "INSERT INTO pricing_plan_phase_rebuilt (
        phase_id, plan_revision, tenant_id, plan_id, kind, ordinal,
        converts_to_phase_id, phase_duration_days, display_trial_days)
     SELECT
        phase_id, plan_revision, tenant_id, plan_id, kind, ordinal,
        converts_to_phase_id, phase_duration_days, display_trial_days
     FROM pricing_plan_phase",
            "DROP TABLE pricing_plan_phase",
            "ALTER TABLE pricing_plan_phase_rebuilt RENAME TO pricing_plan_phase",
            "CREATE UNIQUE INDEX uq_pricing_plan_phase_terminal
        ON pricing_plan_phase (plan_id, plan_revision)
        WHERE converts_to_phase_id IS NULL",
            "CREATE INDEX idx_pricing_plan_phase_revision
        ON pricing_plan_phase (tenant_id, plan_id, plan_revision)",
            "CREATE TRIGGER trg_pricing_plan_phase_no_insert
        BEFORE INSERT ON pricing_plan_phase
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_plan_phase: INSERT of a phase under a non-draft plan revision is not permitted')
          WHERE NOT EXISTS (
            SELECT 1 FROM pricing_plan
             WHERE plan_id = NEW.plan_id AND revision = NEW.plan_revision
               AND lifecycle_state = 'draft');
        END",
            // Both ends: the revision the row leaves and the revision it lands under.
            "CREATE TRIGGER trg_pricing_plan_phase_no_update
        BEFORE UPDATE ON pricing_plan_phase
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_plan_phase: UPDATE of a phase under a non-draft plan revision is not permitted')
          WHERE NOT EXISTS (
            SELECT 1 FROM pricing_plan
             WHERE plan_id = OLD.plan_id AND revision = OLD.plan_revision
               AND lifecycle_state = 'draft')
             OR NOT EXISTS (
            SELECT 1 FROM pricing_plan
             WHERE plan_id = NEW.plan_id AND revision = NEW.plan_revision
               AND lifecycle_state = 'draft');
        END",
            "CREATE TRIGGER trg_pricing_plan_phase_no_delete
        BEFORE DELETE ON pricing_plan_phase
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_plan_phase: DELETE of a phase under a non-draft plan revision is not permitted')
          WHERE NOT EXISTS (
            SELECT 1 FROM pricing_plan
             WHERE plan_id = OLD.plan_id AND revision = OLD.plan_revision
               AND lifecycle_state = 'draft');
        END",
        ]
    };
}

const SQLITE_UP_STATEMENTS: &[&str] =
    sqlite_rebuild!("tenant_id, plan_id, plan_revision, phase_id");

const SQLITE_DOWN_STATEMENTS: &[&str] = sqlite_rebuild!("phase_id, plan_revision");

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
