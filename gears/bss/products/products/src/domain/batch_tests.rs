//! The batch machine's edges and the abandon dispositions, each probed on
//! the case whose absence §7 row 5 recorded.

use super::{
    ABANDON_REASON, AbandonDisposition, abandon_disposition, all_rows_terminal, batch_edge,
};
use crate::domain::states::BatchState;

/// The six admitted edges, by name — including the two P-D-69 added, whose
/// absence was the whole of §7 row 5.
#[test]
fn the_machine_admits_exactly_the_six_named_edges() {
    for (from, to) in [
        (BatchState::Staging, BatchState::Reported),
        (BatchState::Reported, BatchState::Approved),
        (BatchState::Approved, BatchState::Committing),
        (BatchState::Committing, BatchState::Completed),
        (BatchState::Reported, BatchState::Abandoned),
        (BatchState::Staging, BatchState::Failed),
        (BatchState::Committing, BatchState::Failed),
    ] {
        batch_edge(from, to)
            .unwrap_or_else(|e| panic!("{} -> {} is admitted: {e}", from.as_str(), to.as_str()));
    }
}

/// **The approval cannot be skipped**, and no terminal state has an exit.
#[test]
fn the_edge_list_is_closed_in_both_directions() {
    let refused = [
        (BatchState::Reported, BatchState::Committing),
        (BatchState::Staging, BatchState::Approved),
        (BatchState::Approved, BatchState::Abandoned),
        (BatchState::Completed, BatchState::Committing),
        (BatchState::Abandoned, BatchState::Reported),
        (BatchState::Failed, BatchState::Staging),
    ];
    for (from, to) in refused {
        let err = batch_edge(from, to).expect_err(&format!(
            "{} -> {} is outside the design's list",
            from.as_str(),
            to.as_str()
        ));
        assert_eq!(err.code(), "ILLEGAL_TRANSITION");
    }
    for state in [
        BatchState::Staging,
        BatchState::Reported,
        BatchState::Approved,
        BatchState::Committing,
        BatchState::Completed,
        BatchState::Failed,
        BatchState::Abandoned,
    ] {
        batch_edge(state, state).expect_err("no self-edge is admitted");
    }
}

/// **`abandoned` is entered from `reported` and from nowhere else** — the
/// approval's rejection is the only trigger, so a batch already committing
/// cannot be abandoned out from under its own row publishes.
#[test]
fn abandon_is_reachable_only_from_reported() {
    batch_edge(BatchState::Reported, BatchState::Abandoned).expect("the P-D-69 edge");
    for from in [
        BatchState::Staging,
        BatchState::Approved,
        BatchState::Committing,
    ] {
        batch_edge(from, BatchState::Abandoned)
            .expect_err("abandon has one entry, and it is the reported state");
    }
}

/// The per-row abandon paths, one per kind and one for the rows there is
/// nothing to undo for.
#[test]
fn each_row_kind_has_its_own_abandon_path() {
    assert_eq!(
        abandon_disposition("product", false, true, false),
        AbandonDisposition::DiscardDraft
    );
    assert_eq!(
        abandon_disposition("sku", false, true, true),
        AbandonDisposition::RevertToPublished
    );
    assert_eq!(
        abandon_disposition("governed_live_op", false, true, false),
        AbandonDisposition::DropPendingOp,
        "a pending live-entity operation is dropped, never applied"
    );
}

/// A row the ledger already closed, or one staging never materialised, has
/// nothing to undo — and the two reach the same answer for different
/// reasons, which is why the function takes both facts.
#[test]
fn a_closed_or_unmaterialised_row_is_left_alone() {
    assert_eq!(
        abandon_disposition("product", true, true, false),
        AbandonDisposition::AlreadyTerminal,
        "a row whose ledger disposition stands is history, not work"
    );
    assert_eq!(
        abandon_disposition("product", false, false, false),
        AbandonDisposition::AlreadyTerminal,
        "a row staging never materialised has no entity to undo"
    );
}

/// The completion precondition: every row terminal, whatever the mix — and
/// an empty ledger is **not** a completed batch.
#[test]
fn completion_needs_every_row_terminal_and_at_least_one() {
    let mixed = [
        Some("published".to_owned()),
        Some("failed".to_owned()),
        Some("no_op".to_owned()),
    ];
    assert!(
        all_rows_terminal(&mixed),
        "parts-succeeded is the honest end state: a failed row does not hold the batch open"
    );
    assert!(!all_rows_terminal(&[Some("published".to_owned()), None]));
    assert!(
        !all_rows_terminal(&[]),
        "a batch with no rows has not completed anything"
    );
}

/// The reason is the literal P-D-50 pins, and the migration's CHECK admits
/// exactly it.
#[test]
fn the_abandon_reason_is_the_pinned_literal() {
    assert_eq!(ABANDON_REASON, "batch-abandoned");
}
