//! Create `bss.pricing_outbox` — the transactional event outbox
//! (`design/01-foundation.md` §3.7).
//!
//! Transactional is the point: an event exists if and only if its publish
//! commit happened, because the row is written in that same commit. Delivery is
//! **at-least-once** and carries dedup and correlation keys, so a consumer
//! dedups rather than assuming exactly-once, and ordering is per
//! `(tenant_id, aggregate_id)` — not global, because a global sequence would
//! serialize every tenant's publishing behind one counter and no consumer needs
//! cross-aggregate order.
//!
//! Four physical guards. `chk_pricing_outbox_event_name` pins the **frozen**
//! event-name set (the same thirteen names as `domain::events::CatalogEvent`):
//! a name here is a contract a consumer is entitled to keep receiving forever,
//! so a typo must fail at insert rather than become an event nobody is
//! subscribed to. `uq_pricing_outbox_sequence` makes the per-aggregate order a
//! total order — two rows at the same seq would leave the relay free to
//! pick either. `uq_pricing_outbox_dedup_key` makes the dedup key actually
//! dedup, at the writer rather than at every consumer.
//! `idx_pricing_outbox_undrained` is the relay's cursor: the drain scans only
//! rows still unpublished, so the index does not grow with delivered history.
//!
//! **Backend differences.** `jsonb` becomes `text` on `SQLite`; the partial
//! index and both unique constraints carry over unchanged.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.pricing_outbox (
        outbox_id      uuid        NOT NULL PRIMARY KEY,
        tenant_id      uuid        NOT NULL,
        aggregate_id   uuid        NOT NULL,
        event_name     text        NOT NULL,
        seq            bigint      NOT NULL,
        payload        jsonb       NOT NULL,
        dedup_key      text        NOT NULL,
        correlation_id uuid        NOT NULL,
        enqueued_at    timestamptz NOT NULL DEFAULT now(),
        published_at   timestamptz,
        CONSTRAINT chk_pricing_outbox_sequence CHECK (seq >= 0),
        CONSTRAINT chk_pricing_outbox_event_name CHECK (event_name IN (
            'PlanCreated','PlanUpdated','PlanPublished','PlanRetired',
            'PlanMigrationScheduled','PlanPublishDegraded','BundleUpdated',
            'PriceCreated','PriceUpdated','PriceWindowScheduled',
            'PriceWindowActivated','PriceWindowExpired','PriceWindowCancelled'))
    )",
    "CREATE UNIQUE INDEX uq_pricing_outbox_sequence
        ON bss.pricing_outbox (tenant_id, aggregate_id, seq)",
    "CREATE UNIQUE INDEX uq_pricing_outbox_dedup_key
        ON bss.pricing_outbox (tenant_id, dedup_key)",
    "CREATE INDEX idx_pricing_outbox_undrained
        ON bss.pricing_outbox (tenant_id, aggregate_id, seq)
        WHERE published_at IS NULL",
];

const PG_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS bss.pricing_outbox"];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE pricing_outbox (
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
        CONSTRAINT chk_pricing_outbox_event_name CHECK (event_name IN (
            'PlanCreated','PlanUpdated','PlanPublished','PlanRetired',
            'PlanMigrationScheduled','PlanPublishDegraded','BundleUpdated',
            'PriceCreated','PriceUpdated','PriceWindowScheduled',
            'PriceWindowActivated','PriceWindowExpired','PriceWindowCancelled'))
    )",
    "CREATE UNIQUE INDEX uq_pricing_outbox_sequence
        ON pricing_outbox (tenant_id, aggregate_id, seq)",
    "CREATE UNIQUE INDEX uq_pricing_outbox_dedup_key
        ON pricing_outbox (tenant_id, dedup_key)",
    "CREATE INDEX idx_pricing_outbox_undrained
        ON pricing_outbox (tenant_id, aggregate_id, seq)
        WHERE published_at IS NULL",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS pricing_outbox"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
