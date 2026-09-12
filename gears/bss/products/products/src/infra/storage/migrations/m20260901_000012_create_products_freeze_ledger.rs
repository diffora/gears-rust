//! Create `bss.products_freeze_participant` and `bss.products_freeze_ack` —
//! the freeze ledger (`design/06-catalog-version.md` §4, P-D-60, P-D-67).
//!
//! # The registered set is live; the ledger is per version
//!
//! `products_freeze_participant` is the governed **live** set membership ops
//! write (`freeze_participant × write`, P-D-67's door). `products_freeze_ack`
//! is AC #44's liveness source, keyed `(tenant_id, catalog_version_id,
//! participant)`, **seeded by the increment transaction** — one `pending` row
//! per `participant_set_snapshot` member (P-D-67), which is what makes the
//! ack door an UPDATE whose row-existence is the membership check.
//!
//! # The six edges are the trigger's WHEN list — P-D-60
//!
//! `pending → acked` (the ack door), `pending → released` and
//! `acked → released` (the participant's own release door),
//! `pending → not_frozen(forced)` (force-completion, missing participants
//! only), and `not_frozen(forced) → acked | released` (a recovered
//! participant). `released` is terminal and **no other transition is
//! admitted** — the guard refuses the edge by name, the same idiom as the
//! head tables' lifecycle guard.
//!
//! # `released_at` is the ceremony's alone, and write-once — P-D-67
//!
//! The participant's own release door does **not** stamp it: a door-released
//! row is `state = released`, `released_at` NULL, and the retention gate's
//! two arms read exactly that pair. Force-completion stamps it in the same
//! transaction as `not_frozen(forced)`, and a recovered participant's later
//! ack does **not** clear it — the state moving is what makes the stamp
//! inert. The guard refuses any change to a non-NULL `released_at`, and the
//! shape CHECK ties the stamp's *first* write to the forced state.
//!
//! # Backend differences
//!
//! `uuid` becomes `text` on `SQLite`, `timestamptz` becomes `text`, and the
//! `bss.` qualification is dropped. The edge list, the write-once guard, the
//! CHECKs and both keys are preserved on both sides; Postgres carries them in
//! one `TG_OP` function, `SQLite` in `WHEN`-guarded triggers.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-freeze-ledger-tables:p1

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.products_freeze_participant (
            tenant_id      uuid        NOT NULL,
            participant    text        NOT NULL,
            registered_at  timestamptz NOT NULL,
            CONSTRAINT products_freeze_participant_pkey PRIMARY KEY (tenant_id, participant),
            CONSTRAINT chk_products_freeze_participant_name CHECK (participant <> '')
        )",
    "CREATE TABLE bss.products_freeze_ack (
            tenant_id           uuid        NOT NULL,
            catalog_version_id  bigint      NOT NULL,
            participant         text        NOT NULL,
            state               text        NOT NULL,
            acked_at            timestamptz,
            released_at         timestamptz,
            forced_at           timestamptz,
            ceremony_ref        uuid,
            CONSTRAINT products_freeze_ack_pkey PRIMARY KEY (tenant_id, catalog_version_id, participant),
            CONSTRAINT chk_products_freeze_ack_state CHECK (state IN ('pending', 'acked', 'released', 'not_frozen(forced)')),
            CONSTRAINT chk_products_freeze_ack_forced_shape CHECK (
                (state <> 'not_frozen(forced)' AND forced_at IS NULL AND ceremony_ref IS NULL)
                OR (state = 'not_frozen(forced)' AND forced_at IS NOT NULL AND ceremony_ref IS NOT NULL AND released_at IS NOT NULL)
            ),
            CONSTRAINT chk_products_freeze_ack_acked CHECK (state <> 'acked' OR acked_at IS NOT NULL),
            CONSTRAINT fk_products_freeze_ack_version FOREIGN KEY (tenant_id, catalog_version_id)
                REFERENCES bss.products_catalog_version (tenant_id, catalog_version_id)
        )",
    "CREATE OR REPLACE FUNCTION bss.products_freeze_ack_edges() RETURNS trigger AS $$
        BEGIN
          IF TG_OP = 'DELETE' THEN
            RAISE EXCEPTION 'products_freeze_ack rows are never deleted while their version exists (AC #44)';
          END IF;
          IF NEW.tenant_id IS DISTINCT FROM OLD.tenant_id
             OR NEW.catalog_version_id IS DISTINCT FROM OLD.catalog_version_id
             OR NEW.participant IS DISTINCT FROM OLD.participant THEN
            RAISE EXCEPTION 'products_freeze_ack: the key columns are immutable';
          END IF;
          IF OLD.released_at IS NOT NULL AND NEW.released_at IS DISTINCT FROM OLD.released_at THEN
            RAISE EXCEPTION 'products_freeze_ack: released_at is write-once (P-D-67)';
          END IF;
          IF NEW.state IS DISTINCT FROM OLD.state THEN
            IF NOT (
                 (OLD.state = 'pending' AND NEW.state IN ('acked', 'released', 'not_frozen(forced)'))
              OR (OLD.state = 'acked' AND NEW.state = 'released')
              OR (OLD.state = 'not_frozen(forced)' AND NEW.state IN ('acked', 'released'))
            ) THEN
              RAISE EXCEPTION 'products_freeze_ack: % -> % is not one of the six admitted edges (P-D-60)', OLD.state, NEW.state;
            END IF;
          END IF;
          RETURN NEW;
        END;
        $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_products_freeze_ack_edges
        BEFORE DELETE OR UPDATE ON bss.products_freeze_ack
        FOR EACH ROW EXECUTE FUNCTION bss.products_freeze_ack_edges()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TRIGGER IF EXISTS trg_products_freeze_ack_edges ON bss.products_freeze_ack",
    "DROP FUNCTION IF EXISTS bss.products_freeze_ack_edges",
    "DROP TABLE IF EXISTS bss.products_freeze_ack",
    "DROP TABLE IF EXISTS bss.products_freeze_participant",
];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE products_freeze_participant (
            tenant_id      text NOT NULL,
            participant    text NOT NULL,
            registered_at  text NOT NULL,
            PRIMARY KEY (tenant_id, participant),
            CONSTRAINT chk_products_freeze_participant_name CHECK (participant <> '')
        )",
    "CREATE TABLE products_freeze_ack (
            tenant_id           text    NOT NULL,
            catalog_version_id  integer NOT NULL,
            participant         text    NOT NULL,
            state               text    NOT NULL,
            acked_at            text,
            released_at         text,
            forced_at           text,
            ceremony_ref        text,
            PRIMARY KEY (tenant_id, catalog_version_id, participant),
            CONSTRAINT chk_products_freeze_ack_state CHECK (state IN ('pending', 'acked', 'released', 'not_frozen(forced)')),
            CONSTRAINT chk_products_freeze_ack_forced_shape CHECK (
                (state <> 'not_frozen(forced)' AND forced_at IS NULL AND ceremony_ref IS NULL)
                OR (state = 'not_frozen(forced)' AND forced_at IS NOT NULL AND ceremony_ref IS NOT NULL AND released_at IS NOT NULL)
            ),
            CONSTRAINT chk_products_freeze_ack_acked CHECK (state <> 'acked' OR acked_at IS NOT NULL),
            CONSTRAINT fk_products_freeze_ack_version FOREIGN KEY (tenant_id, catalog_version_id)
                REFERENCES products_catalog_version (tenant_id, catalog_version_id)
        )",
    "CREATE TRIGGER trg_products_freeze_ack_no_delete
        BEFORE DELETE ON products_freeze_ack
        BEGIN
          SELECT RAISE(ABORT, 'products_freeze_ack rows are never deleted while their version exists (AC #44)');
        END",
    "CREATE TRIGGER trg_products_freeze_ack_frozen_key
        BEFORE UPDATE ON products_freeze_ack
        WHEN NEW.tenant_id IS NOT OLD.tenant_id
          OR NEW.catalog_version_id IS NOT OLD.catalog_version_id
          OR NEW.participant IS NOT OLD.participant
        BEGIN
          SELECT RAISE(ABORT, 'products_freeze_ack: the key columns are immutable');
        END",
    "CREATE TRIGGER trg_products_freeze_ack_released_once
        BEFORE UPDATE ON products_freeze_ack
        WHEN OLD.released_at IS NOT NULL AND NEW.released_at IS NOT OLD.released_at
        BEGIN
          SELECT RAISE(ABORT, 'products_freeze_ack: released_at is write-once (P-D-67)');
        END",
    "CREATE TRIGGER trg_products_freeze_ack_edges
        BEFORE UPDATE ON products_freeze_ack
        WHEN NEW.state IS NOT OLD.state AND NOT (
             (OLD.state = 'pending' AND NEW.state IN ('acked', 'released', 'not_frozen(forced)'))
          OR (OLD.state = 'acked' AND NEW.state = 'released')
          OR (OLD.state = 'not_frozen(forced)' AND NEW.state IN ('acked', 'released'))
        )
        BEGIN
          SELECT RAISE(ABORT, 'products_freeze_ack: not one of the six admitted edges (P-D-60)');
        END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    "DROP TRIGGER IF EXISTS trg_products_freeze_ack_edges",
    "DROP TRIGGER IF EXISTS trg_products_freeze_ack_released_once",
    "DROP TRIGGER IF EXISTS trg_products_freeze_ack_frozen_key",
    "DROP TRIGGER IF EXISTS trg_products_freeze_ack_no_delete",
    "DROP TABLE IF EXISTS products_freeze_ack",
    "DROP TABLE IF EXISTS products_freeze_participant",
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
