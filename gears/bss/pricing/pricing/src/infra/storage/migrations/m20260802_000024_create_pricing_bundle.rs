//! Create `bss.pricing_bundle` — a bundle's identity, its price basis and its
//! invoice itemization (`design/08-bundles.md` §6, D-105). The first of Slice
//! 8's four tables and the parent the other three key on.
//!
//! §6's column list is `price_basis`, `invoice_itemization` and "lifecycle
//! refs", over a primary key of `bundle_id` and the `plan_id` D-105 added. The
//! lifecycle refs are the `plan_id`: §4 of that document states the bundle has
//! **no slice-owned state machine** and rides the plan lifecycle of Slices 2/11
//! on its `bundle`-type SKU, so the reference *is* the lifecycle ref and a
//! second `lifecycle_state` column here would be a second answer to what state a
//! bundle is in.
//!
//! # `plan_id` carries no foreign key, and D-105 assumed it could
//!
//! D-105 gives this table *"the `plan_id` FK the revision keying presupposes"*.
//! A foreign key is the one thing that cannot be declared on it. `pricing_plan`
//! is keyed `(plan_id, revision)` (D-56), and the only uniqueness it has on
//! `plan_id` **alone** is in two *partial* indexes — `uq_pricing_plan_current`
//! (`WHERE lifecycle_state IN ('published','retired')`) and
//! `uq_pricing_plan_open_draft` (`WHERE lifecycle_state = 'draft'`). Postgres
//! refuses a partial index as a foreign key's referent, so the constraint is not
//! expressible against the plan table as it stands, and a `(plan_id, revision)`
//! pair is not available either: this row is the bundle's identity and does not
//! belong to one revision.
//!
//! The reference is nonetheless enforced, one table down and at full strength:
//! the three composition tables carry `bundle_id` with a **real** foreign key
//! onto this table, and their append-only triggers resolve `plan_id` through it
//! to read the owning revision's `lifecycle_state`. So an orphan bundle row is
//! possible and an orphan *composition* is not, which is where the integrity
//! actually has to hold. The divergence from D-105 is reported rather than
//! smoothed over, and is in the owed register.
//!
//! # One bundle per plan (`uq_pricing_bundle_plan`), an addition
//!
//! §6 names no uniqueness on `plan_id`. Without it a plan carries two bundle
//! rows, and then "the bundle's plan revisions" (D-92) names two composition
//! chains inside one revision sequence: a component set frozen at revision *N*
//! would have two parents and the projector no way to choose. It is a total
//! `UNIQUE` rather than a partial one because it constrains identity, not
//! lifecycle — the bundle row does not have a lifecycle of its own.
//!
//! # `price_basis` and `invoice_itemization` are **not** revision-scoped, and
//! # that is D-92's own defect arriving one row up
//!
//! Stated here because building is where it becomes visible. D-92 puts the
//! revision discipline on *"the three composition tables below"* and this row is
//! not one of them, so `price_basis` and `invoice_itemization` are mutated **in
//! place**. Both are D-104 always-material triggers — a basis change and an
//! itemization change each route through the two-person workflow — and an
//! in-place column means the published revision reads the new value from the
//! moment it is authored, before any approver has seen it. That is precisely the
//! defect D-92 was written to close ("a draft recomposition of a published
//! bundle mutates the published truth state"), one table above where it looked.
//!
//! Nothing is invented here to close it: the shape is §6's and D-105's as
//! written, the hazard is in the owed register with a proposed decision, and the
//! publish path re-reads this row inside its commit so the value it validates is
//! the value it publishes. A reader who reaches for this table's columns as
//! frozen truth should read that entry first.
//!
//! **Backend differences.** `uuid` becomes `text` and the `bss.` qualification
//! is dropped, as elsewhere in this chain. Every `CHECK`, index and the primary
//! key are preserved on both sides. No trigger: this table has no revision to be
//! frozen with, which is the subject of the paragraph above.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.pricing_bundle (
        bundle_id           uuid NOT NULL,
        tenant_id           uuid NOT NULL,
        plan_id             uuid NOT NULL,
        price_basis         text NOT NULL,
        invoice_itemization text NOT NULL,
        PRIMARY KEY (bundle_id),
        -- `inst-bb-declared`: the basis MUST be declared, and these are the two
        -- values that declare it.
        CONSTRAINT chk_pricing_bundle_price_basis CHECK (
            price_basis IN ('sum_of_parts', 'own_price')),
        -- B3. Persisted either way, and either way preserving per-SKU rev-share.
        CONSTRAINT chk_pricing_bundle_invoice_itemization CHECK (
            invoice_itemization IN ('aggregate', 'itemize'))
    )",
    // See the module doc: two bundles on one plan give one revision sequence two
    // composition chains.
    "CREATE UNIQUE INDEX uq_pricing_bundle_plan ON bss.pricing_bundle (plan_id)",
    // The tenant-scoped read every authoring surface makes.
    "CREATE INDEX idx_pricing_bundle_tenant ON bss.pricing_bundle (tenant_id, bundle_id)",
];

const PG_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS bss.pricing_bundle"];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE pricing_bundle (
        bundle_id           text NOT NULL,
        tenant_id           text NOT NULL,
        plan_id             text NOT NULL,
        price_basis         text NOT NULL,
        invoice_itemization text NOT NULL,
        PRIMARY KEY (bundle_id),
        CONSTRAINT chk_pricing_bundle_price_basis CHECK (
            price_basis IN ('sum_of_parts', 'own_price')),
        CONSTRAINT chk_pricing_bundle_invoice_itemization CHECK (
            invoice_itemization IN ('aggregate', 'itemize'))
    )",
    "CREATE UNIQUE INDEX uq_pricing_bundle_plan ON pricing_bundle (plan_id)",
    "CREATE INDEX idx_pricing_bundle_tenant ON pricing_bundle (tenant_id, bundle_id)",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS pricing_bundle"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
