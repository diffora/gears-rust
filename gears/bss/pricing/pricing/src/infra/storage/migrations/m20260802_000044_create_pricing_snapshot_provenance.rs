//! Create `bss.pricing_snapshot_provenance` — the `migrated-origin` record
//! (`design/11-lifecycle.md` §6, `inst-sy-provenance`, `inst-sy-payload`, D-76,
//! D-81, D-87, D-102).
//!
//! One row is one **frozen** snapshot for one subscription that never had a
//! `pricingSnapshotRef`: what was resolved, as of which instant, from which
//! selection tier, and the complete evaluable payload rating charges from and
//! Billing posts from.
//!
//! # This table is the marking, and there is no `origin` column
//!
//! §6 calls the record "marked **`migrated-origin`**". A column holding one
//! permitted value is not a marking, it is a tautology with a maintenance cost:
//! nothing else is ever stored here, so membership *is* the mark. The name says
//! so and `inst-sy-surface`'s endpoint is spelled after it.
//!
//! # Append-only in the strongest sense: no `UPDATE` at all
//!
//! Every other guarded table in this chain permits a whitelist of moves. This one
//! permits none, and the reason is the word §3 uses: the snapshot is **frozen**.
//! A `migrated-origin` ref resolves through **no** `CatalogVersion` by
//! construction (D-87, Foundation §4.4 names it the one deliberately
//! non-version-pinned reference), so the immutability that a frozen
//! `CatalogVersion` gives every other consumer contract has to come from
//! somewhere else — and the only place left is this row. If it could be edited,
//! a disputed legacy charge could be re-explained after the fact by the party
//! being disputed with.
//!
//! `DELETE` is refused for the same reason, one step further: an auditor
//! reconstructing a charge needs the record to still exist, and a subscription's
//! snapshot outlives the migration that synthesized it.
//!
//! # Idempotency is the unique index, and D-81 is why it is keyed this way
//!
//! §9 requires a second synthesis attempt to be idempotent — *the same frozen
//! ref*. `uq_pricing_snapshot_provenance_subscription` over
//! `(tenant_id, subscription_ref)` **is** that rule: a subscription has at most
//! one `migrated-origin` snapshot, ever, so a re-run finds the row rather than
//! freezing a second one at a second instant. Keying it on
//! `(subscription_ref, trigger)` instead would have let the `migration` and
//! `first-rating` triggers each freeze their own — and D-81 gives those two
//! *different* instants `t`, so the subscription would have two different frozen
//! prices with no rule saying which one rating reads.
//!
//! # `source_revision` is nullable, and that nullability is D-76's tier 2
//!
//! A fully-legacy key may have **no plan revision at all**: tier 2 resolves a
//! `pricing_historical_price` reference row that exists in no `CatalogVersion` by
//! construction, and D-87 states the case plainly — "a tier-2 (fully legacy) key
//! may have no plan revision at all". So the column admits `NULL` and the
//! payload, not the revision, is what makes the row evaluable. `source_plan_id`
//! stays `NOT NULL` because synthesis is always *about* a plan even when that
//! plan has no revision covering `t`.
//!
//! # What is deliberately **not** constrained
//!
//! **No relationship between `snapshot_instant` and `created_at`.** The obvious
//! rule — an instant frozen at execution cannot be in the future — is not written
//! because it is not true of both triggers: D-81 makes `t` the *migration
//! effective timestamp* for the `migration` trigger, and a migration is
//! synthesized in the run-up to a date that has not arrived. Writing the
//! plausible constraint would refuse the ordinary case of the more common
//! trigger. The `first-rating` half (`t` = earliest unrated usage) is genuinely
//! past, but a `CHECK` cannot be conditional on a fact this row states about
//! itself without becoming two rules that disagree at the boundary.
//!
//! **No foreign key on `source_plan_id`**, for `m20260802_000024`'s reason:
//! `pricing_plan` is keyed `(plan_id, revision)` and its uniqueness on `plan_id`
//! alone lives in two *partial* indexes, which Postgres refuses as an FK
//! referent. And none on `subscription_ref` at all — subscriptions are another
//! gear's, which is the whole premise of this slice.
//!
//! **Backend differences.** The systematic mirror of this chain: `bss.` dropped,
//! `uuid` -> `text`, `timestamptz` -> `text`, `jsonb` -> `text`, `now()` ->
//! `(CURRENT_TIMESTAMP)`, and the single PL/pgSQL trigger split into two
//! `RAISE(ABORT, ...)` triggers, since `SQLite` has no procedural language and no
//! `BEFORE UPDATE OR DELETE`. Both arms are unconditional here, so no arm needs
//! to repeat another's exclusion — this is the one guarded table in the chain
//! whose mirror is a straight transliteration. Every `CHECK`, both indexes and the
//! primary key are preserved name for name.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.pricing_snapshot_provenance (
        provenance_id    uuid        NOT NULL PRIMARY KEY,
        tenant_id        uuid        NOT NULL,
        subscription_ref uuid        NOT NULL,
        source_plan_id   uuid        NOT NULL,
        source_revision  integer,
        snapshot_instant timestamptz NOT NULL,
        trigger_kind     text        NOT NULL,
        acting_principal uuid        NOT NULL,
        resolved         jsonb       NOT NULL,
        payload          jsonb       NOT NULL,
        created_at       timestamptz NOT NULL DEFAULT now(),
        -- D-81's two triggers and no third. `first_rating` is spelled with an
        -- underscore because the design set's hyphenated `first-rating` is prose;
        -- the stored token follows this chain's convention.
        CONSTRAINT chk_pricing_snapshot_provenance_trigger CHECK (
            trigger_kind IN ('migration', 'first_rating')),
        -- A revision number is an ordinal. NULL is tier 2's fully-legacy key.
        CONSTRAINT chk_pricing_snapshot_provenance_revision CHECK (
            source_revision IS NULL OR source_revision >= 0),
        -- `inst-sy-select` clause (3) is fail-closed: synthesis never freezes a
        -- snapshot it could not resolve a row for. An empty resolved set is that
        -- refusal having been ignored.
        CONSTRAINT chk_pricing_snapshot_provenance_resolved CHECK (
            jsonb_typeof(resolved) = 'array' AND jsonb_array_length(resolved) > 0),
        CONSTRAINT chk_pricing_snapshot_provenance_payload CHECK (
            jsonb_typeof(payload) = 'object')
    )",
    // Section 9's idempotency rule, as an index rather than as a convention: one
    // subscription, one frozen snapshot, ever. See the module doc for why the key
    // deliberately excludes the trigger.
    "CREATE UNIQUE INDEX uq_pricing_snapshot_provenance_subscription
        ON bss.pricing_snapshot_provenance (tenant_id, subscription_ref)",
    // The audit read: every legacy snapshot synthesized off one plan.
    "CREATE INDEX idx_pricing_snapshot_provenance_plan
        ON bss.pricing_snapshot_provenance (tenant_id, source_plan_id)",
    "CREATE OR REPLACE FUNCTION bss.pricing_snapshot_provenance_frozen() RETURNS trigger AS $$
        BEGIN
          IF TG_OP = 'DELETE' THEN
            RAISE EXCEPTION
              'pricing_snapshot_provenance: DELETE of the migrated-origin record of subscription % is not permitted; an auditor reconstructing a legacy charge needs it to still exist',
              OLD.subscription_ref;
          END IF;

          RAISE EXCEPTION
            'pricing_snapshot_provenance: the migrated-origin snapshot of subscription % is frozen; it resolves through no CatalogVersion, so this row is the only thing making it immutable',
            OLD.subscription_ref;
        END;
     $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_pricing_snapshot_provenance_frozen
        BEFORE UPDATE OR DELETE ON bss.pricing_snapshot_provenance
        FOR EACH ROW EXECUTE FUNCTION bss.pricing_snapshot_provenance_frozen()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS bss.pricing_snapshot_provenance",
    "DROP FUNCTION IF EXISTS bss.pricing_snapshot_provenance_frozen()",
];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------
//
// Systematic transforms from the Postgres variant, plus one that is this table's
// own: `jsonb_typeof`/`jsonb_array_length` have no SQLite equivalent, so the two
// shape CHECKs become `json_valid` plus a `json_type` comparison, which SQLite's
// JSON1 extension provides and which asserts the same two facts - `resolved` is a
// non-empty array, `payload` is an object. Named identically so the roster reads
// as one set across both engines.

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE pricing_snapshot_provenance (
        provenance_id    text NOT NULL PRIMARY KEY,
        tenant_id        text NOT NULL,
        subscription_ref text NOT NULL,
        source_plan_id   text NOT NULL,
        source_revision  integer,
        snapshot_instant text NOT NULL,
        trigger_kind     text NOT NULL,
        acting_principal text NOT NULL,
        resolved         text NOT NULL,
        payload          text NOT NULL,
        created_at       text NOT NULL DEFAULT (CURRENT_TIMESTAMP),
        CONSTRAINT chk_pricing_snapshot_provenance_trigger CHECK (
            trigger_kind IN ('migration', 'first_rating')),
        CONSTRAINT chk_pricing_snapshot_provenance_revision CHECK (
            source_revision IS NULL OR source_revision >= 0),
        CONSTRAINT chk_pricing_snapshot_provenance_resolved CHECK (
            json_valid(resolved) AND json_type(resolved) = 'array'
            AND json_array_length(resolved) > 0),
        CONSTRAINT chk_pricing_snapshot_provenance_payload CHECK (
            json_valid(payload) AND json_type(payload) = 'object')
    )",
    "CREATE UNIQUE INDEX uq_pricing_snapshot_provenance_subscription
        ON pricing_snapshot_provenance (tenant_id, subscription_ref)",
    "CREATE INDEX idx_pricing_snapshot_provenance_plan
        ON pricing_snapshot_provenance (tenant_id, source_plan_id)",
    "CREATE TRIGGER trg_pricing_snapshot_provenance_no_delete
        BEFORE DELETE ON pricing_snapshot_provenance
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_snapshot_provenance: DELETE of a migrated-origin record is not permitted; an auditor reconstructing a legacy charge needs it to still exist');
        END",
    "CREATE TRIGGER trg_pricing_snapshot_provenance_no_update
        BEFORE UPDATE ON pricing_snapshot_provenance
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_snapshot_provenance: a migrated-origin snapshot is frozen; it resolves through no CatalogVersion, so this row is the only thing making it immutable');
        END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS pricing_snapshot_provenance"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
