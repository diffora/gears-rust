//! The bulk batch's state machine and the abandon procedure's per-row
//! dispositions (`design/09-bulk-promotion.md` §2 `inst-bb-edge-*`,
//! `features/bulk-promotion.md`'s `algo-batch`, **P-D-54** and **P-D-69**).
//!
//! # Seven states, and the two edges P-D-69 added
//!
//! `staging → reported → approved → committing → completed`, with `failed`
//! entered from `staging` or `committing` on the worker's attempt-budget
//! exhaustion, and `abandoned` entered from `reported` on the batch
//! approval's rejection or explicit withdrawal. Both of those were absent
//! from the original machine — §7 row 5 recorded that `reported` had one
//! exit while the approval it waits on could be rejected, and that nothing
//! entered `failed` — and P-D-69 arm 1 answered both. **Row failures enter
//! neither**: a failed row is row-local and its siblings still commit, which
//! is why `completed` admits any mix of dispositions.
//!
//! # `completed` is not "everything succeeded"
//!
//! The completion edge fires when every row has reached a **terminal ledger
//! state**, whatever the mix — *"parts-succeeded is the honest end state,
//! not an error"*. A machine that refused to complete a batch with failed
//! rows would leave it in `committing` forever, holding the tenant's slot.
//!
//! # What the abandon procedure does per row, and what it does not
//!
//! Each row kind has its own path and none of them is a new door: created
//! drafts **discard**, update-as-draft rows **revert** through the ordinary
//! save with the last frozen version's content, and pending live-entity
//! operations are **dropped** — never applied. The reason recorded is the
//! literal `batch-abandoned` (**P-D-50**): this feature writes no operator
//! free-text reason anywhere, which is why `02`'s content-PII enumeration
//! no longer names it.

use super::error::DomainError;
use super::states::BatchState;
use bss_products_sdk::models::EntityKind;

/// The batch machine's admitted edges, as `design/09` §2 names them.
///
/// # Errors
///
/// [`DomainError::IllegalTransition`] for every pair outside the list —
/// including `reported → committing`, which would skip the approval, and
/// every edge out of a terminal state.
pub fn batch_edge(from: BatchState, to: BatchState) -> Result<(), DomainError> {
    // A `match` per source state rather than one flat alternation: it groups
    // each state's exits the way the design states them, and it is what
    // makes `reported`'s two — approve or abandon — visible as a pair.
    let admitted = match from {
        BatchState::Staging => matches!(to, BatchState::Reported | BatchState::Failed),
        BatchState::Reported => matches!(to, BatchState::Approved | BatchState::Abandoned),
        BatchState::Approved => to == BatchState::Committing,
        BatchState::Committing => matches!(to, BatchState::Completed | BatchState::Failed),
        // Terminal: no edge leaves `completed`, `failed` or `abandoned`.
        BatchState::Completed | BatchState::Failed | BatchState::Abandoned => false,
    };
    if admitted {
        Ok(())
    } else {
        Err(DomainError::IllegalTransition {
            from: from.as_str().to_owned(),
            to: to.as_str().to_owned(),
        })
    }
}

/// What the abandon procedure does with one staged row, by the row's kind
/// and how far it got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbandonDisposition {
    /// A created draft the staging phase minted: **discarded**, releasing
    /// its code reservation by the same write.
    DiscardDraft,
    /// A row that edited an existing entity as a draft: **reverted** to the
    /// last frozen version's content through the ordinary save, so the head
    /// returns to its published content with a revision bump.
    RevertToPublished,
    /// A pending live-entity operation: **dropped**, never applied.
    DropPendingOp,
    /// The row already reached a terminal ledger state before the abandon —
    /// nothing to undo, and its disposition stands as the history.
    AlreadyTerminal,
}

/// What the abandon procedure knows about one row before disposing of it.
///
/// Named fields rather than a run of booleans: the call site reads as the
/// row it describes, and a caller cannot swap two flags without the compiler
/// noticing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbandonRow {
    /// The row's staged kind, or `None` where the stored `entity_kind` is
    /// outside the roster staging admits. The roster is closed
    /// ([`EntityKind`] has two members), so `None` is a row no head door
    /// ever materialised; its pending operation is dropped, never applied.
    pub kind: Option<EntityKind>,
    /// Where the row stands in the ledger.
    pub standing: RowStanding,
    /// Whether the row's governed op edits an existing head — the one case
    /// where abandoning means **reverting** rather than discarding.
    pub edits_existing: bool,
}

/// Where one staged row stands when the abandon reaches it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowStanding {
    /// The ledger already carries a disposition for it.
    Terminal,
    /// Staging never minted or resolved the row's entity, so there is
    /// nothing to undo; it counts as terminal for this procedure's purposes.
    NeverMaterialised,
    /// A materialised row with no disposition yet — the one the abandon
    /// actually disposes of.
    Live,
}

/// The disposition for one row.
#[must_use]
pub fn abandon_disposition(row: AbandonRow) -> AbandonDisposition {
    match row.standing {
        RowStanding::Terminal | RowStanding::NeverMaterialised => {
            return AbandonDisposition::AlreadyTerminal;
        }
        RowStanding::Live => {}
    }
    match (row.kind, row.edits_existing) {
        (Some(EntityKind::Product | EntityKind::Sku), true) => {
            AbandonDisposition::RevertToPublished
        }
        (Some(EntityKind::Product | EntityKind::Sku), false) => AbandonDisposition::DiscardDraft,
        (None, _) => AbandonDisposition::DropPendingOp,
    }
}

/// The literal the abandon procedure records on every row it touches
/// (**P-D-50**): a constant from a closed set, never operator text.
pub const ABANDON_REASON: &str = "batch-abandoned";

/// Whether every row of a batch has reached a terminal ledger state — the
/// completion edge's own precondition.
#[must_use]
pub fn all_rows_terminal(dispositions: &[Option<String>]) -> bool {
    !dispositions.is_empty() && dispositions.iter().all(Option::is_some)
}

#[cfg(test)]
#[path = "batch_tests.rs"]
mod batch_tests;
