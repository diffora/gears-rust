//! Two reads that run on every commit path get an index (review P-1, P-3).
//!
//! # What was wrong
//!
//! Neither table was missing an index by oversight in one engine's arm — this is
//! not `m20260802_000087`'s shape. Both arms agreed, both rosters matched the
//! chain, and the per-engine index census was green. What no census in this suite
//! ranges over is the **shape of the reads**, and these two are served by nothing.
//!
//! | table | the read | what it filters on | what existed |
//! |---|---|---|---|
//! | `pricing_approval` | `find_approved_for_content`, `find_pending_for_subject`, `find_pending_for_plan` | `(tenant_id, state, subject_ref[, …])` | `PRIMARY KEY (approval_id)` and one partial index scoped `WHERE subject_kind = 'policy' AND state = 'submitted'` |
//! | `pricing_group_membership` | `memberships_in_group` | `(tenant_id, group_value)`, ordered and resumed on `(effective_from, membership_id)` | `idx_pricing_group_membership_payer (tenant_id, payer_tenant_id, effective_from)` |
//!
//! Neither predicate begins with its table's primary key, and the approval
//! table's partial index has a `WHERE` neither of the three reads implies.
//!
//! # Why this is worth a migration rather than a note
//!
//! Both tables are append-only over a >= 7-year retention and neither has a purge
//! job anywhere in the crate: `pricing_approval` is `DELETE`-refused by
//! `trg_pricing_approval_append_only` / `trg_pricing_approval_no_delete`, and the
//! membership walk's own doc calls its table "never pruned". So the cost of the
//! missing index grows with the **retention horizon and with every other tenant's
//! history**, not with the plan being published.
//!
//! And one of the scans holds locks. `infra::retirement` calls
//! `approval::authorizing_unit` *inside* the retirement transaction, which is
//! `find_approved_for_content`; `find_pending_for_subject` follows it on the same
//! path. `infra::cutover`, `infra::supersession`, `infra::window` and
//! `infra::grandfather` each run the same pair on their own commit paths.
//!
//! # The columns, and why not more of them
//!
//! `idx_pricing_approval_subject` is `(tenant_id, state, subject_ref)` and stops
//! there. It serves the first two reads whole. The third —
//! `approval_repo::find_pending_for_plan` — adds `subject_kind` and a **prefix
//! match** on `subject_ref` (`.starts_with(format!("{plan_id}/"))`, so
//! `LIKE 'plan/%'`), and this index does **not** serve that as a range over the
//! third column, which this comment claimed until 2026-08-19.
//!
//! A plain b-tree cannot answer a `LIKE` prefix as a range under a non-`C`
//! collation: Postgres needs the column indexed with `text_pattern_ops` (or the
//! database created with `C` collation) before the planner will turn `LIKE 'x%'`
//! into `>= 'x' AND < 'y'`. What the index does buy that read is real and
//! smaller than advertised — the two leading equality columns narrow it to one
//! tenant's submitted units, and `subject_ref` is then a filter inside that, not
//! a range.
//!
//! Making the claim true is a `text_pattern_ops` opclass on the third column,
//! which changes how the *other* two reads use it and has no `SQLite`
//! counterpart at all. That is a decision to measure, not one to assert in a
//! comment, so the comment is corrected and the index is left as it stands.
//!
//! `content_hash` is deliberately **not** a fourth column: it is a blob,
//! only one of the three reads names it, and widening an index for a heap fetch
//! that one caller avoids is a cost every writer pays.
//!
//! `idx_pricing_group_membership_walk` is `(tenant_id, group_value,
//! effective_from, membership_id)` — the filter's two columns followed by the
//! walk's sort key **in the order the walk sorts**. The pair matters: the cursor
//! resumes on `(effective_from, membership_id)`, so an index stopping at
//! `effective_from` would still sort the tail of every equal-instant run.
//!
//! # `down` is symmetric here
//!
//! Unlike `m20260802_000087`, both indexes are created by this migration on both
//! engines, so both are this migration's to drop.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE INDEX idx_pricing_approval_subject
        ON bss.pricing_approval (tenant_id, state, subject_ref)",
    "CREATE INDEX idx_pricing_group_membership_walk
        ON bss.pricing_group_membership (tenant_id, group_value, effective_from, membership_id)",
];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE INDEX idx_pricing_approval_subject
        ON pricing_approval (tenant_id, state, subject_ref)",
    "CREATE INDEX idx_pricing_group_membership_walk
        ON pricing_group_membership (tenant_id, group_value, effective_from, membership_id)",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP INDEX bss.idx_pricing_group_membership_walk",
    "DROP INDEX bss.idx_pricing_approval_subject",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    "DROP INDEX idx_pricing_group_membership_walk",
    "DROP INDEX idx_pricing_approval_subject",
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
