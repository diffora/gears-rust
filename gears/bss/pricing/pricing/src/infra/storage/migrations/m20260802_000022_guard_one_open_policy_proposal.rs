//! `pricing_approval` gains `uq_pricing_approval_policy_pending` — "one open
//! policy proposal per tenant" as an **index** rather than as a read-then-write
//! check (D-192 clause (2)).
//!
//! # What was unguarded, and it is the mint
//!
//! `pricing_approval_threshold`'s primary key is `(tenant_id, version, currency)`,
//! and that is deliberate: the store permits one mutation of an approved version —
//! an `INSERT` of a currency it did not have — and relies on the content pin to take
//! a widened version *out of effect* rather than let it quietly extend what an
//! approver signed. `rest_threshold_policy::a_version_widened_after_approval_stops_being_the_effective_policy`
//! is that behaviour's pin, and it is why the guard here is **not** a version-header
//! table keyed `(tenant_id, version)`: such a table would forbid the widening and
//! redden the case that pins it (D-192 clause (3), rejected option (a)).
//!
//! What is unguarded is the **mint**. `threshold_repo::open_version` does not check
//! whether a version number is already taken, and the rule that stops two proposals
//! reaching for one number — "one open policy proposal per tenant" — was a
//! read-then-write check: `infra::approval::open_policy_unit` reads
//! `approval_repo::find_pending_policy_unit` and then inserts. Under `READ
//! COMMITTED` both proposals read a store with no pending policy unit, both mint
//! version *n* off the same `latest_version`, and both commit — leaving one version
//! number holding the union of two disjoint currency sets, which is a row set no
//! approver ever saw and which the store then refuses to `UPDATE` or `DELETE`.
//!
//! # The subject is the **proposal**, not the version
//!
//! A policy proposal's open unit is a `pricing_approval` row with
//! `subject_kind = 'policy'` and `state = 'submitted'` — that is what
//! `find_pending_policy_unit` reads and what `PENDING_CHANGE_UNIT_EXISTS` names —
//! so the constraint that makes the rule physical belongs on the approval store, on
//! `(tenant_id)` under that predicate. The version store keeps the key it has.
//!
//! Both halves of the predicate are load-bearing:
//!
//! * **`subject_kind = 'policy'`** narrows it to the one plane where the rule is
//!   per **tenant**. Plan-revision and window units are per canonical scope key
//!   (`inst-co-single-pending`, and `pricing_approval_key` is where *that* rule is
//!   physical); one tenant holding several of those at once is the normal case, and
//!   an index without this conjunct would refuse it.
//! * **`state = 'submitted'`** is the rule itself, exactly as
//!   `uq_pricing_approval_key_pending`'s own predicate is: a decided or withdrawn
//!   unit holds nothing, which is what makes `inst-as-void`'s withdraw an escape
//!   from the pin rather than a second way to spell it. Without it the index would
//!   say "one policy proposal per tenant **ever**", and a tenant would be unable to
//!   author a second threshold version for the rest of time.
//!
//! Nothing beyond those two is wanted. `subject_ref` (the version number) is
//! deliberately *not* in the key: two proposals that lost this race disagree about
//! their currency sets and not necessarily about their number, and an index keyed on
//! the number would admit exactly the pair that produces the corrupt version.
//!
//! # It forbids no designed flow, checked rather than assumed
//!
//! `AuditSubjectKind::Policy` has exactly one writer in this gear —
//! `infra::approval::open_policy_unit`, reached from `ThresholdService::propose` and
//! `ThresholdService::retire` — and both go through the same refusal. No path opens
//! two policy units on purpose, and the decided-then-reopened flow the register's
//! own escape hatch exists for still works: a withdraw moves the holding unit to
//! `voided`, which leaves the predicate, so
//! `rest_threshold_policy::a_withdrawn_proposal_frees_the_tenant_to_propose_again`
//! is unaffected. A tombstone (D-185) is an ordinary appended version and rides the
//! same unit, so it is one proposal and not two.
//!
//! # The check stays, and the index is not a replacement for it
//!
//! D-148's arrangement, verbatim: the in-transaction read is the ordinary answer —
//! it is the only one that can **name the unit** holding the proposal open, which
//! is what an operator acts on — and the index is the invariant, which no reader
//! racing a writer can step through. Neither is the other's test.
//!
//! What the loser of the race is told is `PENDING_CHANGE_UNIT_EXISTS`, the same code
//! the check answers, because the caller's situation is identical whether they lost
//! a race or arrived second. Reaching it needed a classifier change, which
//! `approval_repo::open` carries and documents: `contention_or_db` would have
//! rendered this violation as `CONCURRENT_MUTATION` ("retry"), and a retry would
//! then be refused by the check — the right answer, one round trip late and under a
//! code that sends the caller to the wrong place first.
//!
//! # Why a migration of its own rather than an amendment to `000015`
//!
//! `m20260802_000018`'s reason, which is the chain's rule: `000015` is what the
//! approval store was when it was created, and a reader asking when this rule became
//! physical gets a dated answer rather than a `git blame`. The `down` is the exact
//! inverse.
//!
//! **A later rebuild of `pricing_approval` has to re-create this index.**
//! `m20260802_000019` rebuilds the table on `SQLite` (create-copy-drop-rename) and
//! its doc says the rebuild needs no index work, which was true when it was written
//! and is no longer: `DROP TABLE` takes every index with it. That migration sorts
//! **before** this one, so the chain is correct in both directions as it stands — a
//! `down` walks this file's `DROP INDEX` first, and a re-`up` re-creates it after the
//! rebuild — but a *new* rebuild appended after this file would lose the guard
//! silently, and `tests/sqlite_migrations.rs`'s index census is what would say so.
//!
//! **Backend differences.** The schema prefix, and nothing else: both engines take a
//! partial unique index over the same two-conjunct predicate, and `state` and
//! `subject_kind` are both columns of the indexed table, so no denormalisation and
//! no trigger is needed here (compare `m20260802_000017`, whose predicate needed a
//! `state` column copied onto a child table and a trigger to keep it).

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &["CREATE UNIQUE INDEX uq_pricing_approval_policy_pending
        ON bss.pricing_approval (tenant_id)
        WHERE subject_kind = 'policy' AND state = 'submitted'"];

const PG_DOWN_STATEMENTS: &[&str] =
    &["DROP INDEX IF EXISTS bss.uq_pricing_approval_policy_pending"];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------
//
// Systematic transform from the Postgres variant: the `bss.` prefix is dropped
// (single namespace). The predicate is identical - `SQLite` has supported partial
// indexes since 3.8.0 and both conjuncts reference only the indexed table's own
// columns, which is the whole of what it restricts.

const SQLITE_UP_STATEMENTS: &[&str] = &["CREATE UNIQUE INDEX uq_pricing_approval_policy_pending
        ON pricing_approval (tenant_id)
        WHERE subject_kind = 'policy' AND state = 'submitted'"];

const SQLITE_DOWN_STATEMENTS: &[&str] =
    &["DROP INDEX IF EXISTS uq_pricing_approval_policy_pending"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
