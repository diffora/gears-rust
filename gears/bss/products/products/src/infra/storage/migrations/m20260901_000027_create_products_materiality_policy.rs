//! Create `bss.products_materiality_policy` — the tenant's governed
//! materiality policy (**P-D-112** arm 1; `design/05-governance.md` C4,
//! `inst-mt-policy-material`) — and the two indexes **P-D-110** arm 3 and
//! **P-D-111** routed to this migration.
//!
//! # One row per tenant, and the absent row is not an absent policy
//!
//! P-D-112 arm 2 is the clause the schema has to make easy rather than merely
//! permit: *"an absent row resolves to the default; only a failed read is
//! unresolved"*. **P-D-11** already fixes `N` as *"reachable only by explicit
//! configuration, absent ⇒ default"*, interim 2 with a floor of 0. So a tenant
//! that has never configured anything has **no row**, and that is a resolved
//! policy carrying the default — not a lookup failure. The table therefore
//! carries no seed and no provisioning step: nothing writes a row until a
//! tenant configures one, and the read supplies the default for every tenant
//! that has not.
//!
//! Keyed on `(tenant_id)` alone for the same reason `products_read_stamp` is:
//! the row is per tenant and the table must be addressable with no row in
//! existence.
//!
//! # Why a fourth table rather than configuration
//!
//! C4: *"the policy's own mutation is material — the two-person rule's
//! foundation must not be single-person-editable"*. Configuration is
//! single-person-editable by construction, so a config home would put the
//! two-person rule's own foundation outside the two-person rule. `inst-mt-once`
//! compounds it: the evaluation reads *"the policy in force at the submission
//! instant"*, and a process's configuration has no historical value to
//! re-read. Two independent clauses, one answer (P-D-112).
//!
//! # What the columns pin
//!
//! `inst-mt-policy-material` makes the policy **field set + trigger + `N`**,
//! and `N` is inside the governed object because C1 and P-D-11 both require
//! every later change to it to be material under the then-current quorum,
//! which only holds if it is part of what is governed.
//!
//! - `field_set` is the tenant's addition to the bucket registry, stored as
//!   the canonical rendering of a string array so both engines hold identical
//!   bytes; empty is the default and is spelled `[]`, never `NULL`, because an
//!   absent *column* would be a third state beside "no row" and "no fields".
//! - `affected_entity_trigger` carries §17.1's interim 10.
//! - `approver_count` carries `N`. **No upper `CHECK` and a floor of zero**:
//!   P-D-11 made zero reachable, and a `CHECK (approver_count >= 1)` would
//!   silently restore the fixed count it retired.
//! - `updated_by` / `updated_at` are the governed mutation's own audit pair,
//!   pseudonymous like every actor-bearing column in this gear.
//!
//! # The two indexes that ride this migration, and why they ride it
//!
//! Both were routed here because this is the change that makes their reads
//! live (P-D-110 arm 3, P-D-111):
//!
//! - **`idx_products_approval_gate`** — `(tenant_id, subject_kind,
//!   subject_ref, submitted_at)` with **no state predicate**, for
//!   `repo::gate_candidates`. That read is deliberately stateless so
//!   `PreAuthorized` can see `consumed` rows, which is exactly why
//!   `uq_products_approval_open`'s `WHERE state IN ('pending','satisfied')`
//!   cannot serve it and `idx_products_approval_queue` offers only the
//!   `tenant_id` prefix and cannot serve its `ORDER BY submitted_at`. **Not a
//!   `LIMIT`**: bounding a read whose whole purpose is to find an arbitrarily
//!   old `consumed` record trades a performance cost for a correctness one,
//!   and any `k` makes some composite act unverifiable.
//! - **`idx_products_breakglass_two_person`** — on
//!   `two_person_approval_ref`. **P-D-111** put the elevation's authority on
//!   the session row rather than in the quorum descriptor, which leaves the
//!   approval-side question *"was this act an elevation?"* a **reverse**
//!   lookup on a column with no index. Partial on both engines, since the
//!   column is NULL for every post-hoc session and a NULL is never the answer
//!   to that question.
//!
//! # Backend differences
//!
//! `uuid` becomes `text`, `bigint`/`integer` become `integer`, `timestamptz`
//! becomes `text`, and the `bss.` qualification is dropped. Every `CHECK`, the
//! key and both indexes are preserved on both sides.
//!
//! **No marker.** `dod-materiality-policy` also obliges the door and the
//! `GovernedLiveOp` subject its mutation carries, and §7 row 38 — which
//! `subject_kind` a policy mutation records — is live. The table is what this
//! file ships.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.products_materiality_policy (
            tenant_id               uuid        NOT NULL,
            field_set               text        NOT NULL,
            affected_entity_trigger integer     NOT NULL,
            approver_count          integer     NOT NULL,
            updated_by              uuid        NOT NULL,
            updated_at              timestamptz NOT NULL,
            CONSTRAINT products_materiality_policy_pkey PRIMARY KEY (tenant_id),
            CONSTRAINT chk_products_materiality_policy_field_set CHECK (field_set <> ''),
            CONSTRAINT chk_products_materiality_policy_trigger CHECK (affected_entity_trigger >= 0),
            CONSTRAINT chk_products_materiality_policy_count CHECK (approver_count >= 0)
        )",
    "CREATE INDEX idx_products_approval_gate ON bss.products_approval USING btree (tenant_id, subject_kind, subject_ref, submitted_at)",
    "CREATE INDEX idx_products_breakglass_two_person ON bss.products_breakglass_session USING btree (two_person_approval_ref) WHERE two_person_approval_ref IS NOT NULL",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP INDEX IF EXISTS bss.idx_products_breakglass_two_person",
    "DROP INDEX IF EXISTS bss.idx_products_approval_gate",
    "DROP TABLE IF EXISTS bss.products_materiality_policy",
];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE products_materiality_policy (
            tenant_id               text    NOT NULL,
            field_set               text    NOT NULL,
            affected_entity_trigger integer NOT NULL,
            approver_count          integer NOT NULL,
            updated_by              text    NOT NULL,
            updated_at              text    NOT NULL,
            PRIMARY KEY (tenant_id),
            CONSTRAINT chk_products_materiality_policy_field_set CHECK (field_set <> ''),
            CONSTRAINT chk_products_materiality_policy_trigger CHECK (affected_entity_trigger >= 0),
            CONSTRAINT chk_products_materiality_policy_count CHECK (approver_count >= 0)
        )",
    "CREATE INDEX idx_products_approval_gate ON products_approval (tenant_id, subject_kind, subject_ref, submitted_at)",
    "CREATE INDEX idx_products_breakglass_two_person ON products_breakglass_session (two_person_approval_ref) WHERE two_person_approval_ref IS NOT NULL",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    "DROP INDEX IF EXISTS idx_products_breakglass_two_person",
    "DROP INDEX IF EXISTS idx_products_approval_gate",
    "DROP TABLE IF EXISTS products_materiality_policy",
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
