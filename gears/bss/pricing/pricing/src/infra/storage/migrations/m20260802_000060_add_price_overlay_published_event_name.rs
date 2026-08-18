//! `PriceOverlayPublished` joins the frozen event-name set — D-248.
//!
//! The set is frozen in **two** places that have to agree: `CatalogEvent` in the
//! domain, and `chk_pricing_outbox_event_name` here. They are pinned to each other
//! by `postgres_schema_stores::every_frozen_event_name_is_enqueueable`, which drives
//! every name in `CatalogEvent::ALL` into the table — so a name added to the enum
//! alone does not merely fail to be emittable, it fails that test.
//!
//! That pairing is why this migration exists at all, and it was found the way this
//! program looks for guards: the first run of the new case failed with a **driver**
//! refusal (`code: 275, CHECK constraint failed`) rather than an assertion. D-248 was
//! written without knowing the name would cost a table rebuild.
//!
//! # Why the whole list is restated rather than the one name appended
//!
//! Postgres can drop and re-add a named constraint, and does. `SQLite` cannot alter a
//! `CHECK` at all, so the table is rebuilt — the same four-statement shape
//! `m20260802_000042` uses for the taxonomy predicates. A rebuild restates the table
//! body by hand, which means the **indexes go with it**: `DROP TABLE` takes its
//! indexes, so all three are recreated after the rename. Leaving them out would drop
//! `uq_pricing_outbox_dedup_key` silently, and at-least-once delivery with no dedup
//! index is a duplicate-event bug that nothing in the fast tier would see.
//!
//! # The `down` restores thirteen, and that is not symmetric
//!
//! Rolling this back with a `PriceOverlayPublished` row already enqueued would fail
//! the restored `CHECK` on Postgres and fail the rebuild's `INSERT … SELECT` on
//! `SQLite`. That is correct rather than unfortunate: the row is a promise made to a
//! consumer, and a migration that quietly dropped it would break at-least-once
//! delivery. A rollback past this point is a decision about already-published events,
//! not a schema step.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres — the production schema.
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_outbox
        DROP CONSTRAINT chk_pricing_outbox_event_name",
    "ALTER TABLE bss.pricing_outbox
        ADD CONSTRAINT chk_pricing_outbox_event_name CHECK (event_name IN (
            'PlanCreated','PlanUpdated','PlanPublished','PlanRetired',
            'PlanMigrationScheduled','PlanPublishDegraded','BundleUpdated',
            'PriceCreated','PriceUpdated','PriceWindowScheduled',
            'PriceWindowActivated','PriceWindowExpired','PriceWindowCancelled',
            'PriceOverlayPublished'))",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_outbox
        DROP CONSTRAINT chk_pricing_outbox_event_name",
    "ALTER TABLE bss.pricing_outbox
        ADD CONSTRAINT chk_pricing_outbox_event_name CHECK (event_name IN (
            'PlanCreated','PlanUpdated','PlanPublished','PlanRetired',
            'PlanMigrationScheduled','PlanPublishDegraded','BundleUpdated',
            'PriceCreated','PriceUpdated','PriceWindowScheduled',
            'PriceWindowActivated','PriceWindowExpired','PriceWindowCancelled'))",
];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------
//
// One rebuild. The table text is `m20260802_000009`'s, restated by hand with the
// one name added, and all three indexes recreated because `DROP TABLE` took them.

macro_rules! sqlite_rebuild {
    ($names:literal) => {
        &[
            concat!(
                "CREATE TABLE pricing_outbox_rebuilt (
        outbox_id      text   NOT NULL PRIMARY KEY,
        tenant_id      text   NOT NULL,
        aggregate_id   text   NOT NULL,
        event_name     text   NOT NULL,
        seq            bigint NOT NULL,
        payload        text   NOT NULL,
        dedup_key      text   NOT NULL,
        correlation_id text   NOT NULL,
        enqueued_at    text   NOT NULL DEFAULT (CURRENT_TIMESTAMP),
        published_at   text,
        CONSTRAINT chk_pricing_outbox_sequence CHECK (seq >= 0),
        CONSTRAINT chk_pricing_outbox_event_name CHECK (event_name IN (",
                $names,
                "))
    )"
            ),
            "INSERT INTO pricing_outbox_rebuilt (
        outbox_id, tenant_id, aggregate_id, event_name, seq, payload,
        dedup_key, correlation_id, enqueued_at, published_at)
     SELECT
        outbox_id, tenant_id, aggregate_id, event_name, seq, payload,
        dedup_key, correlation_id, enqueued_at, published_at
     FROM pricing_outbox",
            "DROP TABLE pricing_outbox",
            "ALTER TABLE pricing_outbox_rebuilt RENAME TO pricing_outbox",
            "CREATE UNIQUE INDEX uq_pricing_outbox_sequence
        ON pricing_outbox (tenant_id, aggregate_id, seq)",
            "CREATE UNIQUE INDEX uq_pricing_outbox_dedup_key
        ON pricing_outbox (tenant_id, dedup_key)",
            "CREATE INDEX idx_pricing_outbox_undrained
        ON pricing_outbox (tenant_id, aggregate_id, seq)
        WHERE published_at IS NULL",
        ]
    };
}

const SQLITE_UP_STATEMENTS: &[&str] = sqlite_rebuild!(
    "'PlanCreated','PlanUpdated','PlanPublished','PlanRetired',
            'PlanMigrationScheduled','PlanPublishDegraded','BundleUpdated',
            'PriceCreated','PriceUpdated','PriceWindowScheduled',
            'PriceWindowActivated','PriceWindowExpired','PriceWindowCancelled',
            'PriceOverlayPublished'"
);

const SQLITE_DOWN_STATEMENTS: &[&str] = sqlite_rebuild!(
    "'PlanCreated','PlanUpdated','PlanPublished','PlanRetired',
            'PlanMigrationScheduled','PlanPublishDegraded','BundleUpdated',
            'PriceCreated','PriceUpdated','PriceWindowScheduled',
            'PriceWindowActivated','PriceWindowExpired','PriceWindowCancelled'"
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
