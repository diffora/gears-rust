//! Create `bss.products_idempotency` — the at-most-once gate for every
//! mutating flow that carries an `Idempotency-Key`, keyed `(tenant_id,
//! endpoint, client_key)` (`design/01-foundation.md` §3.2, §4.4).
//!
//! # The claim `INSERT` is the gate, not a lookup — this migration lays down
//! only the storage half
//!
//! `features/foundation.md`'s `cpt-cf-bss-products-dod-idempotency-store` and
//! **P-D-42** make the claim `INSERT` itself the concurrency gate: it joins
//! the guarded mutation's own transaction, so a rollback frees the key with
//! no separate release step. **This migration writes no repository function
//! and no door wiring** — those are a later slice's — and lays down exactly
//! the table shape that claim, that CAS and that CHECK need.
//!
//! # No `in_flight_until` column — its absence is a decision, not an
//! oversight (P-D-42)
//!
//! The donor's `in_flight_until` existed only because a claim once committed
//! in its own transaction, ahead of the mutation it guarded, so an unanswered
//! claim needed a deadline of its own to be released by. **P-D-42** put the
//! claim `INSERT` inside the mutation's transaction instead: an unanswered
//! claim can no longer outlive its transaction, because a rollback frees the
//! key automatically. There is no state left for a separate in-flight
//! deadline to describe, so this table carries none.
//!
//! # `state`, the response pair, and the one `CHECK` that ties them
//!
//! `state` is `claimed` or `answered`, and `chk_products_idempotency_response_group`
//! ties it to the response columns exactly as §4.4 states: `claimed` implies
//! both response columns `NULL`, `answered` implies both `NOT NULL`. There is
//! no third shape and no nullable-for-internal carve-out.
//!
//! **An `internal:` lane stores a synthetic `200` and its own outcome record
//! as the body** (**P-D-42**): a non-HTTP caller has no wire response to
//! reproduce, so it manufactures one rather than widening the `CHECK` with a
//! second, internal-only shape. One `CHECK`, one shape, and absence keeps a
//! single meaning in these columns. The cost is named rather than hidden: a
//! status that never reached a wire is stored as though it had, and only a
//! replay of an internal lane ever reads it.
//!
//! **A refusal stores nothing** (**P-D-38**): the response columns carry a
//! success's answer only. The answer write joins the mutation's transaction
//! and rolls back with it, and — since **P-D-42** — the claim shares that
//! transaction too, so a refused request leaves no row behind at all and the
//! key is free for an immediate retry.
//!
//! The replay is self-contained (**P-D-29**): `response_status` and
//! `response_body` are what a replay reproduces, not a reference to some
//! other row the answer might point at, since a refusal — the case a
//! reference would have had nothing to name — never reaches storage under
//! P-D-38.
//!
//! # `expires_at` is both the retention deadline and P-D-49's claim stamp
//!
//! `expires_at` is stamped at the claim `INSERT` from the sweep's configured
//! value and is the retention window of the key. **P-D-49** also makes it the
//! operand of the expired-key takeover's compare-and-swap: nothing holds an
//! expired row between one transaction's conflict check and its takeover
//! `UPDATE`, so two duplicates on one expired key can both clear the check
//! and both read the same expired row. The takeover `UPDATE` therefore
//! carries `WHERE expires_at = <the value the reader saw>`; exactly one
//! matches, the other finds nothing left to update, and the loser is refused
//! `IDEMPOTENCY_KEY_IN_FLIGHT` having executed nothing. No second column is
//! needed to carry that stamp — `expires_at` is the row's own claim stamp,
//! addressed by name.
//!
//! Retention itself — at least 24 hours and at least the configured maximum
//! freeze timeout (C6) — is the sweep's policy read from configuration, not a
//! constraint this table can express; this migration stores the deadline and
//! leaves collecting past it to that later job. A background sweep runs only
//! to reclaim space — correctness never waits on it, since expiry is decided
//! at claim time.
//!
//! # `endpoint` is the concrete resource path, not the route template
//!
//! **P-D-42**: keying on the route template would let two publishes of
//! different entities under one client key share the whole key and an
//! identical empty-body hash, so the second would replay the first's `200`
//! without running. `endpoint` therefore holds the concrete path a wire
//! caller resolved, never the template it matched. Three reserved
//! `internal:` lane names (`internal:scheduled-activation`,
//! `internal:cascade-leg`, `internal:bulk-row`) occupy the same column for
//! non-HTTP callers. **This migration only holds the column** — which value a
//! door writes into it is that door's own rule, built in a later slice.
//!
//! # No append-only trigger — C5 exempts expiring operational stores
//!
//! Unlike `products_audit_log`, this table carries **no append-only guard**.
//! `design/01-foundation.md` C5 states the append-only posture applies to
//! head rows, history rows and `products_audit_log`, and is explicitly
//! **exempt** for "expiring operational stores (idempotency sweep)". This
//! table is expiring operational state, not a record: a row is claimed,
//! answered, taken over past its expiry, or eventually swept away, none of
//! which an append-only posture would admit. A later reader must not
//! "restore" a guard here by analogy with the audit log.
//!
//! # Backend differences
//!
//! `uuid` becomes `text` for `tenant_id`; `bytea` becomes `blob` for
//! `payload_hash`; `jsonb` becomes `text` for `response_body`; `timestamptz`
//! becomes `text` for `expires_at`; and the `bss.` qualification is dropped.
//! Both `CHECK`s and the primary key are preserved on both sides.
//!
//! # `entity_ref` — the composite act's parent handle (P-D-79)
//!
//! Nullable, `NULL` for every single-entity door, and outside both `CHECK`s
//! deliberately: the response pair's shape is the answer's contract, while
//! `entity_ref` is working state a composite act stamps in its **first**
//! transaction (claim `INSERT`, parent row, stamp — together or not at all)
//! and reads back on a same-key retry to resume from. The family clone is
//! the first such act: its claim stays `claimed` after the parent's
//! transaction commits — committed-but-unanswered means *in progress*
//! (P-D-72) — and `entity_ref` is how the re-entry finds the parent whose
//! children it scans, since several family acts over one source make
//! `cloned_from` alone ambiguous. The expired-claim takeover resets it
//! beside the response pair.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-idempotency-store:p1

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.products_idempotency (
            tenant_id       uuid        NOT NULL,
            endpoint        text        NOT NULL,
            client_key      text        NOT NULL,
            state           text        NOT NULL,
            payload_hash    bytea       NOT NULL,
            response_status integer,
            response_body   jsonb,
            expires_at      timestamptz NOT NULL,
            entity_ref      uuid,
            CONSTRAINT products_idempotency_pkey PRIMARY KEY (tenant_id, endpoint, client_key),
            CONSTRAINT chk_products_idempotency_state CHECK (state IN ('claimed', 'answered')),
            CONSTRAINT chk_products_idempotency_response_group CHECK (
                (state = 'claimed' AND response_status IS NULL AND response_body IS NULL)
                OR
                (state = 'answered' AND response_status IS NOT NULL AND response_body IS NOT NULL)
            )
        )",
    "CREATE INDEX idx_products_idempotency_expires ON bss.products_idempotency USING btree (tenant_id, expires_at)",
];

const PG_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS bss.products_idempotency"];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE products_idempotency (
            tenant_id       text    NOT NULL,
            endpoint        text    NOT NULL,
            client_key      text    NOT NULL,
            state           text    NOT NULL,
            payload_hash    blob    NOT NULL,
            response_status integer,
            response_body   text,
            expires_at      text    NOT NULL,
            entity_ref      text,
            PRIMARY KEY (tenant_id, endpoint, client_key),
            CONSTRAINT chk_products_idempotency_state CHECK (state IN ('claimed', 'answered')),
            CONSTRAINT chk_products_idempotency_response_group CHECK (
                (state = 'claimed' AND response_status IS NULL AND response_body IS NULL)
                OR
                (state = 'answered' AND response_status IS NOT NULL AND response_body IS NOT NULL)
            )
        )",
    "CREATE INDEX idx_products_idempotency_expires ON products_idempotency (tenant_id, expires_at)",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS products_idempotency"];

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
