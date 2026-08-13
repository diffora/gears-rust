//! Bulk import, Phase 2 — the per-row optimistic commit
//! (`design/12-operator-efficiency.md` §3 `inst-bk-phase2`, §4 `inst-bi-commit`,
//! `inst-bk-lock`; D-141, D-260, D-262, D-290).
//!
//! # Every row is its own transaction, and that is the whole shape
//!
//! `inst-bk-phase2`: "each row commits under its own `ETag`; a conflict (concurrent
//! manual edit) fails **only that row**; committed rows stand". So this is the
//! opposite of the clone, which had to be one transaction: there the receipt was
//! worthless without every row, here a partial result is the **product**. The
//! repository methods open their own transactions and that is why they are used
//! rather than their runner-taking forms.
//!
//! The token is the price row's own version column (D-141). Until that decision
//! §3.7 carried an `ETag` on `pricing_plan` alone, under which a batch either
//! conflicts entirely or not at all and "fails only that row" has no referent.
//!
//! # The locks are taken after the run enters `committing`, because the store
//! says so
//!
//! `pricing_bulk_row_lock`'s trigger refuses a lock whose run is not
//! `committing`, and it is right: a lock held through Phase 1 would exclude every
//! interactive editor for the length of a *read*. The consequence is an ordering
//! this module cannot choose — advance first, then lock.
//!
//! # A run that cannot take its locks conflicts on every row
//!
//! If another run holds a row, `take_locks` refuses and this one is already
//! `committing` — from which §4's only edges are `completed` and
//! `completed_with_conflicts`. There is no failure state to reach, and inventing
//! one would be a state machine edit for an outcome that already has an honest
//! reading: **nothing committed, every row conflicted, the report lists them for
//! retry.** That is what `BULK_ROW_CONFLICT` means and what "committed rows
//! stand" says about a run where none did.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use toolkit_db::secure::AccessScope;
use toolkit_db::{DBProvider, DbError};
use uuid::Uuid;

use crate::domain::audit::AuditStamp;
use crate::domain::bulk::BulkState;
use crate::domain::error::DomainError;
use crate::domain::import::{BatchReport, ImportRow, RowOutcome, RowViolation};
use crate::domain::lifecycle::LifecycleState;
use crate::domain::price_record::PriceRecord;
use crate::domain::scope_key::{PlanId, ScopeKey};
use crate::infra::storage::repo::{NewPriceDraft, PriceRepo, bulk_repo, price_repo};
use crate::infra::storage::{RepoError, repo_failure};

/// §5's per-row conflict code, reported in the operation report.
///
/// Declared by the design set and raised here for the first time: an `ETag` that no
/// longer matches, or a row another run holds. Both are the same fact for the
/// operator — somebody else moved it — and the remedy is the same: re-read and
/// resubmit the conflicted subset as a new import.
pub const BULK_ROW_CONFLICT: &str = "BULK_ROW_CONFLICT";

/// One row that landed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct CommittedRow {
    /// Its position in the submitted batch.
    pub row: usize,
    /// The draft row it wrote — minted here for a new key, the existing draft's
    /// id for an edit.
    pub price_id: Uuid,
}

/// What Phase 2 did.
///
/// `{committed, conflicted}` is `inst-bi-commit`'s own shape. The conflicted half
/// carries [`RowOutcome`]s rather than bare indices so the report a retry reads is
/// the report Phase 1 produces — one type, whichever phase filled it.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct CommitReceipt {
    /// Rows that landed, in batch order.
    pub committed: Vec<CommittedRow>,
    /// Rows that did not, each carrying its violation.
    pub conflicted: Vec<RowOutcome>,
}

impl CommitReceipt {
    /// The terminal state this receipt puts the run in.
    ///
    /// `completed_with_conflicts` is a **success**: `inst-bk-phase2`'s "committed
    /// rows stand" is the whole posture, and a run that landed nine of ten rows
    /// has done nine rows of work the operator keeps.
    #[must_use]
    pub fn terminal_state(&self) -> BulkState {
        if self.conflicted.is_empty() {
            BulkState::Completed
        } else {
            BulkState::CompletedWithConflicts
        }
    }
}

/// Run Phase 2 for an import whose Phase 1 passed.
///
/// The run must be `validating`; it is moved to `committing`, takes its locks,
/// commits row by row, releases, and lands on the terminal state
/// [`CommitReceipt::terminal_state`] names.
///
/// # Errors
/// [`DomainError`] for a storage or state-machine refusal that is about the
/// **run** rather than a row — a move that is not an edge, a run that does not
/// exist. A row's refusal is not an error: it is a conflicted entry in the
/// receipt.
#[allow(
    clippy::too_many_arguments,
    reason = "every argument is a fact only the caller holds: the provider the run's own \
              statements go through, the price repository whose methods each open the \
              transaction one row commits in, the scope, the tenant, the run, the batch, the \
              D-135 stamp and the instant. Bundling them would put values that are never \
              carried together anywhere else into a type existing only to satisfy a count"
)]
pub async fn commit_batch(
    db: &DBProvider<DbError>,
    prices: &PriceRepo,
    scope: &AccessScope,
    tenant_id: Uuid,
    operation_id: Uuid,
    rows: &[ImportRow],
    stamp: AuditStamp,
    now: DateTime<Utc>,
) -> Result<CommitReceipt, DomainError> {
    let conn = db
        .conn()
        .map_err(|e| DomainError::Internal(format!("bss-pricing: bulk commit: {e}")))?;

    // Resolved before the run moves: the read is Phase 2's own and tells it which
    // rows are edits (and therefore which rows there are to lock at all).
    let drafts = draft_rows(&conn, scope, tenant_id, rows).await?;
    let mut targets: Vec<Uuid> = rows
        .iter()
        .filter_map(|row| drafts.get(&row.scope_key).map(|found| found.price_id))
        .collect();
    // Deduped, or a batch naming one draft twice collides with **its own** lock and
    // conflicts every row against itself. Phase 1 refuses in-batch duplicates, but
    // this function is `pub` and a caller is not obliged to have run it.
    targets.sort_unstable();
    targets.dedup();

    bulk_repo::advance(
        &conn,
        scope,
        tenant_id,
        operation_id,
        // Phase 1 has just passed on a run this function's own contract says is
        // `validating`; the premise rides into the statement (Z8-7) so a second
        // caller on one run cannot re-enter `committing` over the first's work.
        BulkState::Validating,
        BulkState::Committing,
        // **Not a placeholder.** This column is what an abort reports from, and
        // overwriting it on entry left a run that died mid-flight with nothing
        // but `{"phase":"committing"}` — every committed row invisible. It now
        // carries how many rows the run is about, so an abort can say how many
        // were not attempted.
        serde_json::json!({ "phase": "committing", "rows": rows.len() }),
        now,
    )
    .await
    .map_err(|e| repo_failure(&e))?;

    // **Everything from here lands the run terminal and releases its locks, on
    // every path.** §4 offers no failure edge out of `committing`, so a run that
    // enters it and stops has no exit — holding its rows against every interactive
    // editor, with no operator remedy until D-37's lease takeover exists, which it
    // does not. `inst-bs-done` says the lock is released "either way"; the `?` that
    // used to sit on `commit_rows` made that false.
    let (receipt, failure) =
        match bulk_repo::take_locks(&conn, scope, tenant_id, operation_id, &targets, now).await {
            Ok(()) => {
                // **The receipt survives the failure, and D-300 is why.** Every row
                // is its own transaction, so on a run-level fault at row `k` the
                // rows before it are *in the store*. This arm used to discard the
                // receipt and substitute `not_attempted_all`, so the run's stored
                // report asserted that rows which had committed were never
                // attempted — and an operator resubmitting on that report meets a
                // stale `ETag` on every one of them.
                let (partial, failed) = commit_rows(
                    prices,
                    scope,
                    tenant_id,
                    operation_id,
                    rows,
                    &drafts,
                    stamp,
                    now,
                )
                .await;
                // **No `?` here either.** D-294 took the `?` off `commit_rows` and
                // left this one, which skipped the terminal transition below on a
                // failed release — the very state the block comment above says is
                // impossible, reached by the other of the two statements.
                let released = bulk_repo::release_locks(&conn, scope, tenant_id, operation_id)
                    .await
                    .map_err(|e| repo_failure(&e));
                (partial, failed.or_else(|| released.err()))
            }
            // Another run holds one of the rows, and `take_locks` released what this
            // one had taken. Nothing committed; every row is un-attempted.
            Err(held @ RepoError::BulkRowLocked { .. }) => (not_attempted(rows, &held), None),
            // The locks could not be taken at all, so no row was reached and
            // `not_attempted_all`'s claim is true of every one of them.
            Err(other) => (not_attempted_all(rows), Some(repo_failure(&other))),
        };

    // The run reaches a terminal state on every path and the caller still learns
    // the failure — both survive, which is what a state machine with no failure
    // edge forces.
    bulk_repo::advance(
        &conn,
        scope,
        tenant_id,
        operation_id,
        BulkState::Committing,
        receipt.terminal_state(),
        report_of(&receipt),
        now,
    )
    .await
    .map_err(|e| repo_failure(&e))?;
    failure.map_or(Ok(receipt), Err)
}

/// Commit each row on its own, collecting what landed and what did not.
#[allow(
    clippy::too_many_arguments,
    reason = "the repository, the scope, the tenant, the run whose lock its own edits are \
              allowed through, the batch, the drafts already read, the stamp and the instant"
)]
async fn commit_rows(
    prices: &PriceRepo,
    scope: &AccessScope,
    tenant_id: Uuid,
    operation_id: Uuid,
    rows: &[ImportRow],
    drafts: &HashMap<ScopeKey, PriceRecord>,
    stamp: AuditStamp,
    now: DateTime<Utc>,
) -> (CommitReceipt, Option<DomainError>) {
    let mut receipt = CommitReceipt::default();
    for (index, row) in rows.iter().enumerate() {
        let outcome = if let Some(found) = drafts.get(&row.scope_key) {
            // An edit: the `ETag` the row asserted, against the draft it names.
            let Some(expected) = row.if_match else {
                receipt.conflicted.push(conflicted(
                    index,
                    format!(
                        "a draft row ({}) already holds this key and this row asserted no \
                         version; re-read it and resubmit with its `ETag`",
                        found.price_id
                    ),
                ));
                continue;
            };
            prices
                .update_draft(
                    scope,
                    tenant_id,
                    found.price_id,
                    expected,
                    row.content.clone(),
                    stamp,
                    // The run that locked this row is the one editing it, which is
                    // the whole distinction: the lock excludes somebody else, not
                    // its own holder.
                    Some(operation_id),
                )
                .await
                .map(|record| record.price_id)
        } else {
            // A new key. `if_match` on a key nothing holds is the mirror fault.
            if row.if_match.is_some() {
                receipt.conflicted.push(conflicted(
                    index,
                    "this row asserted a version and no draft holds its key; the row it meant \
                     to edit is gone, so re-read and resubmit"
                        .to_owned(),
                ));
                continue;
            }
            let price_id = Uuid::now_v7();
            prices
                .create_draft(
                    scope,
                    tenant_id,
                    NewPriceDraft {
                        price_id,
                        scope_key: row.scope_key.clone(),
                        content: row.content.clone(),
                        created_by: stamp.actor_principal_id,
                        created_at_utc: now,
                        correlation_id: stamp.correlation_id,
                    },
                )
                .await
                .map(|record| record.price_id)
        };

        match outcome {
            Ok(price_id) => receipt.committed.push(CommittedRow {
                row: index,
                price_id,
            }),
            // **Only a row's own refusal is a conflict.** A storage failure is the
            // run's, and swallowing it here would report a batch as conflicted
            // when the database was down.
            // **`DuplicateScopeKey` is per-row, and the design says so outright**:
            // "one racing a concurrent author fails at commit on the draft-plane
            // partial UNIQUE, reported per-row like every other row outcome".
            // `NotFound` is the same shape — a concurrent author deleted the draft
            // this row named. D-291 caught neither and took the whole run down for
            // a fact about one row.
            // **Four more joined the partition in D-300**, each a fact about one
            // row that Phase 1 does not screen and a bulk body can set: a
            // `grandfatherUntil` on a non-grandfathered key, a sub-millisecond
            // horizon, an authored quantity past its column, and content whose
            // usage line disagrees with its key. `inst-bk-phase1`'s "reported
            // per-row like every other row outcome" covers these too, and taking
            // the run down for one of them also erased what the run had committed.
            Err(
                e @ (RepoError::StaleRowVersion { .. }
                | RepoError::NotDraft { .. }
                | RepoError::DuplicateScopeKey(_)
                | RepoError::NotFound { .. }
                | RepoError::GrandfatherHorizonOffClass { .. }
                | RepoError::TimestampPrecisionExceeded { .. }
                | RepoError::ValueOutOfRange { .. }
                | RepoError::UsageLineDisagrees { .. }),
            ) => {
                receipt.conflicted.push(conflicted(index, e.to_string()));
            }
            // A run-level fault. **The rows already committed stay in the receipt**
            // — they are in the store — and the rows from here on are reported
            // un-attempted, which is true of them and of nothing else.
            Err(other) => {
                let failure = repo_failure(&other);
                for unreached in index..rows.len() {
                    receipt.conflicted.push(conflicted(
                        unreached,
                        format!("not attempted: the run failed at row {index}. {failure}"),
                    ));
                }
                return (receipt, Some(failure));
            }
        }
    }
    (receipt, None)
}

/// The draft rows occupying any key the batch aims at, by key.
///
/// One read per plan, `infra::import`'s arrangement and its reason.
async fn draft_rows(
    runner: &impl toolkit_db::secure::DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    rows: &[ImportRow],
) -> Result<HashMap<ScopeKey, PriceRecord>, DomainError> {
    let mut plans: Vec<PlanId> = rows.iter().map(|row| row.scope_key.plan_id()).collect();
    plans.sort_by_key(|plan| plan.get());
    plans.dedup();

    let mut occupied = HashMap::new();
    for plan in plans {
        let found =
            price_repo::load_for_plan(runner, scope, tenant_id, plan, &[LifecycleState::Draft])
                .await
                .map_err(|e| repo_failure(&e))?;
        for record in found {
            occupied.insert(record.scope_key.clone(), record);
        }
    }
    Ok(occupied)
}

/// One conflicted row.
fn conflicted(row: usize, detail: String) -> RowOutcome {
    RowOutcome {
        row,
        violations: vec![RowViolation {
            code: BULK_ROW_CONFLICT.to_owned(),
            detail,
        }],
    }
}

/// Nothing committed, every row un-attempted, because the run could not take its
/// locks.
///
/// **Each row's sentence is true of that row.** The first version put the holder's
/// `price_id` on all of them, so row 3's violation asserted something about row
/// 0's price. §4's reading of a run that committed nothing is "uncommitted rows
/// reported as not-attempted"; the contended row is named once, as context.
fn not_attempted(rows: &[ImportRow], held: &RepoError) -> CommitReceipt {
    let detail = format!(
        "not attempted: the run could not take its row locks and committed nothing. {held}"
    );
    CommitReceipt {
        committed: Vec::new(),
        conflicted: (0..rows.len())
            .map(|row| conflicted(row, detail.clone()))
            .collect(),
    }
}

/// The same, for a failure that is the run's rather than any row's.
fn not_attempted_all(rows: &[ImportRow]) -> CommitReceipt {
    CommitReceipt {
        committed: Vec::new(),
        conflicted: (0..rows.len())
            .map(|row| {
                conflicted(
                    row,
                    "not attempted: the run failed before this row was reached".to_owned(),
                )
            })
            .collect(),
    }
}

/// The receipt as the run's stored report.
///
/// `serde_json::to_value` of the receipt itself, not a JSON object assembled
/// beside it. **That was the second spelling D-285 named and left standing**: a
/// hand-built object and the type it describes are two things that can disagree,
/// and `inst-bk-idem` replays this column to a retry, so the shape is a contract.
/// The conflicted half is normalised through [`BatchReport`] first, so a report
/// written by two phases still holds one entry per row.
fn report_of(receipt: &CommitReceipt) -> serde_json::Value {
    let mut conflicts = BatchReport::default();
    for outcome in &receipt.conflicted {
        for violation in &outcome.violations {
            conflicts.add(outcome.row, violation.clone());
        }
    }
    let normalised = CommitReceipt {
        committed: receipt.committed.clone(),
        conflicted: conflicts.rows().to_vec(),
    };
    serde_json::to_value(&normalised).unwrap_or_else(|_| {
        // Unreachable: every field is a `Uuid`, a `usize` or a `String`. A JSON
        // failure here would be a serde bug, and losing the report to it would be
        // worse than storing an empty one the operator can see is empty.
        serde_json::json!({ "committed": [], "conflicted": [] })
    })
}
