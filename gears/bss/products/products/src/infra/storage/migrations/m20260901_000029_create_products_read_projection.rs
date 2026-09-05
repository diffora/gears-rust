//! `08`'s projection plane beside its two serving tables (P-D-150): the
//! inbox the `ReadProjector` consumes, its per-tenant checkpoint, the poison
//! park, and the three polled dashboard tables of `inst-ps-dashboards`.
//!
//! # The inbox is the durable acceptance the projector reads
//!
//! Every event the projector consumes is written to `products_read_inbox`
//! **in the same transaction** as its outbox row, by the same `enqueue_*`
//! call — so `created_at` is the write's commit instant, P-D-124's origin for
//! the convergence meter, and ordering per tenant is the row id. The gear
//! cannot read the toolkit's outbox rows (no runner-level raw SQL, no read
//! API on the outbox), so the consumer side of the outbox pattern lives here:
//! a gear-owned copy, rebuildable and sweepable, never the audited truth.
//!
//! # Checkpoint per tenant
//!
//! `design/08` says per `(topic, partition)`; every inbox row carries its
//! tenant and the partition `partition_for` derives, and the projector walks
//! one tenant's rows in id order, so the checkpoint is `(tenant_id) ->
//! inbox_id` — per-aggregate order holds within the tenant. A checkpoint
//! behind the swept tail rebuilds from the latest catalog version into a
//! shadow generation and swaps (`inst-rp-bootstrap`).
//!
//! # These are rebuildable state, not records
//!
//! No append-only guard, no audit rows: `design/08` §4's exemption (L2).
//! The dashboards are polled projections from their owning surfaces and
//! carry a `polled_at` a reader can judge staleness by.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-dashboards:p1

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.products_read_inbox (
            inbox_id      bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
            tenant_id     uuid        NOT NULL,
            partition     integer     NOT NULL,
            aggregate_id  uuid        NOT NULL,
            payload_type  text        NOT NULL,
            payload       text        NOT NULL,
            actor_ref     uuid        NOT NULL,
            created_at    timestamptz NOT NULL
        )",
    "CREATE INDEX idx_products_read_inbox_tenant ON bss.products_read_inbox USING btree (tenant_id, inbox_id)",
    "CREATE TABLE bss.products_read_checkpoint (
            tenant_id     uuid        NOT NULL,
            inbox_id      bigint      NOT NULL,
            generation    integer     NOT NULL DEFAULT 0,
            updated_at    timestamptz NOT NULL,
            CONSTRAINT products_read_checkpoint_pkey PRIMARY KEY (tenant_id)
        )",
    "CREATE TABLE bss.products_read_poison (
            inbox_id      bigint      NOT NULL,
            tenant_id     uuid        NOT NULL,
            payload_type  text        NOT NULL,
            attempts      integer     NOT NULL DEFAULT 1,
            last_error    text        NOT NULL,
            parked_at     timestamptz NOT NULL,
            released_at   timestamptz,
            CONSTRAINT products_read_poison_pkey PRIMARY KEY (inbox_id)
        )",
    "CREATE TABLE bss.products_read_deferred_intent (
            tenant_id      uuid        NOT NULL,
            product_id     uuid        NOT NULL,
            cascade_ref    uuid        NOT NULL,
            children_count integer     NOT NULL,
            created_at     timestamptz NOT NULL,
            age_secs       bigint      NOT NULL,
            polled_at      timestamptz NOT NULL,
            CONSTRAINT products_read_deferred_intent_pkey PRIMARY KEY (tenant_id, product_id)
        )",
    "CREATE TABLE bss.products_read_freeze_status (
            tenant_id          uuid        NOT NULL,
            catalog_version_id bigint      NOT NULL,
            freeze_state       text        NOT NULL,
            pending            integer     NOT NULL,
            acked              integer     NOT NULL,
            released           integer     NOT NULL,
            forced             integer     NOT NULL,
            published_at       timestamptz NOT NULL,
            polled_at          timestamptz NOT NULL,
            CONSTRAINT products_read_freeze_status_pkey PRIMARY KEY (tenant_id, catalog_version_id)
        )",
    "CREATE TABLE bss.products_read_delivery_state (
            tenant_id               uuid        NOT NULL,
            inbox_pending           bigint      NOT NULL,
            parked                  bigint      NOT NULL,
            oldest_pending_age_secs bigint      NOT NULL,
            polled_at               timestamptz NOT NULL,
            CONSTRAINT products_read_delivery_state_pkey PRIMARY KEY (tenant_id)
        )",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS bss.products_read_delivery_state",
    "DROP TABLE IF EXISTS bss.products_read_freeze_status",
    "DROP TABLE IF EXISTS bss.products_read_deferred_intent",
    "DROP TABLE IF EXISTS bss.products_read_poison",
    "DROP TABLE IF EXISTS bss.products_read_checkpoint",
    "DROP TABLE IF EXISTS bss.products_read_inbox",
];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE products_read_inbox (
            inbox_id      integer PRIMARY KEY AUTOINCREMENT,
            tenant_id     text    NOT NULL,
            partition     integer NOT NULL,
            aggregate_id  text    NOT NULL,
            payload_type  text    NOT NULL,
            payload       text    NOT NULL,
            actor_ref     text    NOT NULL,
            created_at    text    NOT NULL
        )",
    "CREATE INDEX idx_products_read_inbox_tenant ON products_read_inbox (tenant_id, inbox_id)",
    "CREATE TABLE products_read_checkpoint (
            tenant_id     text    NOT NULL,
            inbox_id      integer NOT NULL,
            generation    integer NOT NULL DEFAULT 0,
            updated_at    text    NOT NULL,
            PRIMARY KEY (tenant_id)
        )",
    "CREATE TABLE products_read_poison (
            inbox_id      integer NOT NULL,
            tenant_id     text    NOT NULL,
            payload_type  text    NOT NULL,
            attempts      integer NOT NULL DEFAULT 1,
            last_error    text    NOT NULL,
            parked_at     text    NOT NULL,
            released_at   text,
            PRIMARY KEY (inbox_id)
        )",
    "CREATE TABLE products_read_deferred_intent (
            tenant_id      text    NOT NULL,
            product_id     text    NOT NULL,
            cascade_ref    text    NOT NULL,
            children_count integer NOT NULL,
            created_at     text    NOT NULL,
            age_secs       integer NOT NULL,
            polled_at      text    NOT NULL,
            PRIMARY KEY (tenant_id, product_id)
        )",
    "CREATE TABLE products_read_freeze_status (
            tenant_id          text    NOT NULL,
            catalog_version_id integer NOT NULL,
            freeze_state       text    NOT NULL,
            pending            integer NOT NULL,
            acked              integer NOT NULL,
            released           integer NOT NULL,
            forced             integer NOT NULL,
            published_at       text    NOT NULL,
            polled_at          text    NOT NULL,
            PRIMARY KEY (tenant_id, catalog_version_id)
        )",
    "CREATE TABLE products_read_delivery_state (
            tenant_id               text    NOT NULL,
            inbox_pending           integer NOT NULL,
            parked                  integer NOT NULL,
            oldest_pending_age_secs integer NOT NULL,
            polled_at               text    NOT NULL,
            PRIMARY KEY (tenant_id)
        )",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS products_read_delivery_state",
    "DROP TABLE IF EXISTS products_read_freeze_status",
    "DROP TABLE IF EXISTS products_read_deferred_intent",
    "DROP TABLE IF EXISTS products_read_poison",
    "DROP TABLE IF EXISTS products_read_checkpoint",
    "DROP TABLE IF EXISTS products_read_inbox",
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(self.name(), manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(
            self.name(),
            manager,
            PG_DOWN_STATEMENTS,
            SQLITE_DOWN_STATEMENTS,
        )
        .await
    }
}
