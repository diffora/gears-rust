//! Create `bss.pricing_plan_phase` — the phase chain of **one plan revision**
//! (`design/02-plan-definition.md` §6), keyed `(phase_id, plan_revision)`. The
//! first of Slice 2's three revision-scoped child tables, and the one the
//! canonical scope key points at.
//!
//! **The key is a pair, and only one half of it moves.** `plan_revision` is
//! there because a plan's shape versions with its revision: a new revision
//! **copies** the phase rows under its own number and the open draft edits its
//! own copies (D-83). `phase_id` is there unchanged, and its stability is the
//! whole point — the `phase` axis of the canonical scope key holds a **bare**
//! `phase_id` (D-19), and same-key supersession compares it (D-56). Re-minting
//! an id on a new revision would move every continuing price row onto a key
//! nothing else is filed under: the rows would stop superseding their
//! predecessors, phase coverage would judge a chain nobody authored, and the
//! damage would first be visible as a rating miss on a plan that published
//! cleanly.
//!
//! **`tenant_id` is not in §6's column list, and is here anyway.** §6's own
//! preamble calls these tables "tenant-scoped, `SecureORM`" and
//! `01-foundation.md` §3.7 says it of every physical table in this gear;
//! `Scopable` has nowhere else to read the tenant from. The omission is
//! reported rather than treated as a decision. The value is copied from the
//! parent revision by the repository and never taken from a request (Global
//! Constraint 9): the foreign key covers the plan key alone, so nothing here
//! stops a child carrying a different tenant from the row it points at, and
//! under `SecureORM` such a child is invisible to its true owner while still
//! joined by key to their plan.
//!
//! # At most one terminal phase, and the other half of the rule is not here
//!
//! `uq_pricing_plan_phase_terminal` is a partial `UNIQUE` over
//! `(plan_id, plan_revision) WHERE converts_to_phase_id IS NULL`: a revision
//! may not hold two phases with no successor. §6 is explicit that the
//! **existence** of exactly one terminal phase is the `PhaseGraph` pipeline's
//! rule (`inst-ph-graph`, `PHASE_GRAPH_INVALID`) — an index cannot enforce the
//! `>= 1` half, because a row is what an index sees and "there is no such row"
//! is not a row. Nobody should later try to "strengthen" this index into the
//! whole rule; what they would get is a weaker rule wearing the name of the
//! real one, and the zero-terminal chain — the one that leaves every phase
//! conversion resolving to nothing — would still publish.
//!
//! Wholesale replacement of a revision's phase set (`PlanShapeRepo`) is what
//! makes this index authorable at all: re-ordering a chain by moving one row at
//! a time transiently holds two terminals, and the index would refuse an edit
//! whose end state is legal.
//!
//! # Three constraints this table deliberately does NOT carry
//!
//! Each is a real rule, and each belongs to the validation pipeline, which
//! judges a shape the author assembled and owes them **one report enumerating
//! every finding**. A constraint here would answer with a driver error instead
//! — an internal fault where a line an operator can act on was owed — and would
//! refuse a half-authored draft besides, since a phase graph is authored in
//! successive `PATCH`es rather than all at once.
//!
//! 1. **No `CHECK` pairing `phase_duration_days` with terminality.** §6's "Key
//!    constraints" paragraph lists none, and `inst-ph-duration` assigns the
//!    rule — required `> 0` on every non-terminal phase, forbidden on the
//!    terminal one — to the pipeline as `PHASE_DURATION_INVALID`. An author who
//!    sets a successor in one request and its duration in the next passes
//!    through the state such a CHECK would forbid.
//! 2. **No `CHECK` that a terminal phase's `kind` is `evergreen`.** C-4 gives
//!    that rule its own code, `TERMINAL_PHASE_KIND_INVALID`, for the same
//!    authoring reason: terminality is structural
//!    (`converts_to_phase_id IS NULL`) and the `kind` column is independent of
//!    it, which is exactly the pair the rule exists to reject.
//! 3. **No foreign key on `converts_to_phase_id`.** "No dangling target" is
//!    `inst-ph-graph`'s rule. A self-referential FK would forbid authoring
//!    phase A's successor before phase B exists — the ordinary way a chain is
//!    written — and would answer a genuinely dangling edge with a constraint
//!    violation rather than a report line naming the phase.
//!
//! # `display_trial_days` may not drift from its source
//!
//! `chk_pricing_plan_phase_display_trial_days` is §6's, verbatim: a `trial`
//! phase publishes `displayTrialDays` as the PRD-named projection of its
//! `phaseDurationDays` (`inst-ph-trial`), one value under two persisted names,
//! and the two may never disagree (2026-07-28 review fix). Subscriptions reads
//! the published projection as its single source for trial runtime, so a drift
//! here is a trial that ends on a different day than the catalog says it does.
//!
//! The CHECK is satisfied when `display_trial_days` is NULL — an untaken
//! projection — and, by SQL's NULL propagation, **also when
//! `phase_duration_days` is NULL while `display_trial_days` is set**: the
//! comparison is then NULL, which both engines count as satisfied. That
//! remaining shape is a trial phase projecting a duration it does not have, and
//! it is refused at publish rather than here, by the two rules already named:
//! a non-terminal phase without a duration is `PHASE_DURATION_INVALID`, and a
//! terminal one carrying the `trial` kind is `TERMINAL_PHASE_KIND_INVALID`.
//! Closing it with an extra `phase_duration_days IS NOT NULL` conjunct would
//! make exactly the half-authored draft the paragraph above protects unsavable,
//! so the schema stands behind the pipeline here rather than in front of it.
//!
//! # Append-only with its revision (`01-foundation.md` §3.7, the L-2 fix)
//!
//! Every revision-scoped child table carries the same discipline as its parent:
//! child rows are physically immutable once **their** revision publishes, while
//! a draft revision's copies stay freely mutable and deletable. There is no
//! `lifecycle_state` on this table — the parent revision's is the referent —
//! so the predicate reads `pricing_plan.lifecycle_state = 'draft'` for
//! `(plan_id, plan_revision)`, exactly as `pricing_price_tier_band` reads its
//! parent price row. Without it, the phase set of a frozen revision could be
//! rewritten under an unchanged `pricing_plan`, and the projector's warm
//! re-drive — which reads truth rows (§4.4) — would quietly re-materialize a
//! frozen `CatalogVersion` at a different shape.
//!
//! INSERT is guarded and not only UPDATE and DELETE, because an INSERT is the
//! one verb that **adds** a phase to a frozen revision. An UPDATE is checked
//! against **both** ends: the `OLD` parent, the revision the row is bound to
//! now, whose freeze governs it; and the `NEW` parent, the revision the row
//! would land under, whose freeze forbids the append. That is not
//! belt-and-braces — re-pointing a child row's `plan_revision` is precisely how
//! one would otherwise append to a frozen revision without ever issuing an
//! INSERT.
//!
//! **`abandoned` is not `draft`**, and the ordering that follows from it is
//! mandatory rather than stylistic. D-145 drops a discarded revision's child
//! copies; the moment the revision row reads `abandoned` this trigger refuses
//! the DELETE, so `PlanRepo::abandon_draft` drops the phase rows **before** it
//! flips the revision, in one transaction. The mirror-image ordering holds on
//! the way in: `PlanRepo::open_revision` inserts the new revision row first,
//! because the INSERT arm reads the *new* parent and requires it to be `draft`.
//!
//! **Backend differences.** Postgres carries the rule as one PL/pgSQL trigger
//! function with the offending state interpolated; `SQLite` has no procedural
//! language and `RAISE(ABORT, ...)` takes a literal message, so the same rule
//! becomes three fixed-message triggers, one per DML verb, whose parent lookup
//! is a `WHERE NOT EXISTS` subquery in the trigger **body** — a `SQLite` `WHEN`
//! clause may not contain a subquery. `uuid` becomes `text` and the `bss.`
//! qualification is dropped, as elsewhere in this chain. Every CHECK, index, FK
//! and PK is preserved on both sides.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.pricing_plan_phase (
        phase_id             uuid   NOT NULL,
        plan_revision        bigint NOT NULL,
        tenant_id            uuid   NOT NULL,
        plan_id              uuid   NOT NULL,
        kind                 text   NOT NULL,
        ordinal              int    NOT NULL,
        converts_to_phase_id uuid,
        phase_duration_days  int,
        display_trial_days   int,
        PRIMARY KEY (phase_id, plan_revision),
        CONSTRAINT chk_pricing_plan_phase_kind CHECK (
            kind IN ('trial','intro','evergreen')),
        -- The persisted projection may never drift from its source. See the
        -- module doc for the NULL shape this deliberately leaves to the
        -- pipeline.
        CONSTRAINT chk_pricing_plan_phase_display_trial_days CHECK (
            display_trial_days IS NULL OR display_trial_days = phase_duration_days),
        CONSTRAINT fk_pricing_plan_phase_revision FOREIGN KEY (plan_id, plan_revision)
            REFERENCES bss.pricing_plan (plan_id, revision)
    )",
    // At most one terminal phase per revision. The `>= 1` half is the pipeline's
    // (`inst-ph-graph`) and cannot be expressed here; see the module doc.
    "CREATE UNIQUE INDEX uq_pricing_plan_phase_terminal
        ON bss.pricing_plan_phase (plan_id, plan_revision)
        WHERE converts_to_phase_id IS NULL",
    // The copy-forward, the drop-on-abandon and the projector all range over one
    // revision's phases under one tenant.
    "CREATE INDEX idx_pricing_plan_phase_revision
        ON bss.pricing_plan_phase (tenant_id, plan_id, plan_revision)",
    // The parent revision's `lifecycle_state` is the phase row's. UPDATE and
    // DELETE consult the OLD parent - the revision the row is bound to now;
    // INSERT and UPDATE consult the NEW parent - the revision it would land
    // under.
    "CREATE OR REPLACE FUNCTION bss.pricing_plan_phase_append_only() RETURNS trigger AS $$
        DECLARE
          parent_state text;
        BEGIN
          IF TG_OP <> 'INSERT' THEN
            SELECT lifecycle_state INTO parent_state
              FROM bss.pricing_plan
             WHERE plan_id = OLD.plan_id AND revision = OLD.plan_revision;
            IF parent_state IS DISTINCT FROM 'draft' THEN
              RAISE EXCEPTION
                'pricing_plan_phase: % of a phase under a % plan revision is not permitted',
                TG_OP, coalesce(parent_state, 'missing');
            END IF;
          END IF;

          IF TG_OP = 'DELETE' THEN
            RETURN OLD;
          END IF;

          SELECT lifecycle_state INTO parent_state
            FROM bss.pricing_plan
           WHERE plan_id = NEW.plan_id AND revision = NEW.plan_revision;
          IF parent_state IS DISTINCT FROM 'draft' THEN
            RAISE EXCEPTION
              'pricing_plan_phase: % of a phase under a % plan revision is not permitted',
              TG_OP, coalesce(parent_state, 'missing');
          END IF;

          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_pricing_plan_phase_append_only
        BEFORE INSERT OR UPDATE OR DELETE ON bss.pricing_plan_phase
        FOR EACH ROW EXECUTE FUNCTION bss.pricing_plan_phase_append_only()",
];

// This migration puts no trigger on a table it does not own, so dropping the
// table takes every trigger of this migration with it; the function is named
// separately because a Postgres function outlives the table it guarded.
const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS bss.pricing_plan_phase",
    "DROP FUNCTION IF EXISTS bss.pricing_plan_phase_append_only()",
];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------
//
// Systematic transforms: `bss.` dropped, `uuid` -> `text`, and the single
// PL/pgSQL trigger function split into three fixed-message `RAISE(ABORT, ...)`
// triggers, one per DML verb, whose parent lookup is a `WHERE NOT EXISTS`
// subquery in the trigger *body*. Every CHECK, the partial UNIQUE, the FK, the
// index and the PK are preserved.

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE pricing_plan_phase (
        phase_id             text   NOT NULL,
        plan_revision        bigint NOT NULL,
        tenant_id            text   NOT NULL,
        plan_id              text   NOT NULL,
        kind                 text   NOT NULL,
        ordinal              int    NOT NULL,
        converts_to_phase_id text,
        phase_duration_days  int,
        display_trial_days   int,
        PRIMARY KEY (phase_id, plan_revision),
        CONSTRAINT chk_pricing_plan_phase_kind CHECK (
            kind IN ('trial','intro','evergreen')),
        CONSTRAINT chk_pricing_plan_phase_display_trial_days CHECK (
            display_trial_days IS NULL OR display_trial_days = phase_duration_days),
        CONSTRAINT fk_pricing_plan_phase_revision FOREIGN KEY (plan_id, plan_revision)
            REFERENCES pricing_plan (plan_id, revision)
    )",
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
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS pricing_plan_phase"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
