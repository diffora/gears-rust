//! `uq_pricing_bundle_plan` gains `tenant_id` — D-340's class, one table over.
//!
//! `m20260802_000024` added the index and argued it: §6 names no uniqueness on
//! `plan_id`, and without one a plan carries two bundle rows, so "the bundle's
//! plan revisions" (D-92) names two composition chains inside one revision
//! sequence and the projector has no way to choose. That argument is entirely
//! about **one plan**, and every word of it survives here. What it never said, and
//! what the index nevertheless asserted, is that a `plan_id` belongs to one
//! bundle **across the whole table, every tenant's included**.
//!
//! # Why that is an isolation defect and not a modelling nit
//!
//! `plan_id` is the one column on this table that is a client-supplied reference
//! to another table: `POST /bss-pricing/v1/bundles` takes it from the request
//! body, and `pricing_bundle` holds no foreign key at all — a fact
//! `m20260802_000024`'s module doc states and explains, `pricing_plan` having no
//! total uniqueness on `plan_id` alone for a foreign key to name. So the index
//! was the *only* thing in the schema with an opinion about a plan id a caller
//! supplied, and its opinion spanned every tenant.
//!
//! Two consequences, measured on this branch 2026-08-17 and reproduced in
//! `tests/sqlite_bundle_repo.rs` before this migration existed:
//!
//! - **An existence oracle.** A caller naming another tenant's `plan_id` was told
//!   `BUNDLE_EXISTS_ON_PLAN` when that tenant had a bundle and `201 Created` when
//!   it did not. The repository's own pre-check is tenant-scoped and found nothing
//!   either way, so the whole discrimination came from this index.
//! - **A cross-tenant denial that is irreversible from the victim's side.** In the
//!   `201` case the row *lands*, occupying the owning tenant's bundle slot. That
//!   tenant is then refused `BUNDLE_EXISTS_ON_PLAN` against a row it cannot read,
//!   in a tenant it cannot see, and `pricing_bundle` has no `DELETE` path anywhere
//!   in the API.
//!
//! The oracle is closed one layer up, by `bundle_repo::create_on` reading the plan
//! in the caller's scope and answering `NotFound` — the two halves close different
//! things and both are wanted. This one is what stops the slot being *takeable*,
//! which is the half that matters on a database that already holds a squatted row
//! and the half that decides a race, a read being unable to.
//!
//! # The index is already read as if it carried the tenant
//!
//! `bundle_repo::bundle_of_plan` filters `tenant_id.eq(…)` **and**
//! `plan_id.eq(…)`, and `list_page`'s doc says the filtered page answers *"does
//! this plan carry a bundle"* — a question that is only well posed per tenant. The
//! index was therefore **narrower than its only reader assumes**, which is the
//! precise inversion of the usual hazard and the reason the widening removes no
//! guarantee anything relied on: `(tenant_id, plan_id)` still admits exactly one
//! bundle per plan for the tenant that owns it, and D-92's ambiguity cannot arise.
//!
//! # No rebuild on either engine
//!
//! Unlike `m20260802_000081`, which reached a `PRIMARY KEY` and therefore had to
//! rebuild the `SQLite` table whole, this is a standalone index on both arms —
//! `m20260802_000024` writes it as its own `CREATE UNIQUE INDEX` statement, not
//! inline in the `CREATE TABLE`. `DROP INDEX` then reaches it on `SQLite` exactly
//! as on Postgres, so no table is dropped, no trigger or `CHECK` is recreated, and
//! no trigger-body digest in `tests/sqlite_migrations.rs` moves. The name is kept
//! across the change so the constraint keeps one spelling, and so
//! `bundle_repo::names_the_bundle_plan_slot`'s Postgres arm still matches.
//!
//! The `SQLite` refusal message moves and that is the observable half: the engine
//! names the offending **columns**, so `pricing_bundle.plan_id` becomes
//! `pricing_bundle.tenant_id, pricing_bundle.plan_id`. `names_the_bundle_plan_slot`
//! matches the old spelling as a substring of the new one and keeps working
//! unchanged; `tests/sqlite_bundle_store.rs` asserts the new one in full, because a
//! fragment naming one column would pass against an index that had lost the other.
//!
//! # `down` re-narrows and can fail, correctly
//!
//! On a database where two tenants have since taken one `plan_id` the narrow index
//! cannot be created. That is the right behaviour: the narrow key cannot represent
//! those rows, so a `down` that appeared to succeed would have had to drop one of
//! them — `m20260802_000081`'s `down` states the same property for the same reason.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema.
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &[
    "DROP INDEX bss.uq_pricing_bundle_plan",
    "CREATE UNIQUE INDEX uq_pricing_bundle_plan ON bss.pricing_bundle (tenant_id, plan_id)",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP INDEX bss.uq_pricing_bundle_plan",
    "CREATE UNIQUE INDEX uq_pricing_bundle_plan ON bss.pricing_bundle (plan_id)",
];

// ---------------------------------------------------------------------------
// SQLite variant - the same two statements, unqualified.
// ---------------------------------------------------------------------------

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "DROP INDEX uq_pricing_bundle_plan",
    "CREATE UNIQUE INDEX uq_pricing_bundle_plan ON pricing_bundle (tenant_id, plan_id)",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    "DROP INDEX uq_pricing_bundle_plan",
    "CREATE UNIQUE INDEX uq_pricing_bundle_plan ON pricing_bundle (plan_id)",
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
