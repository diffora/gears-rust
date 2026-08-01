//! Tests for the row lifecycle state machine.

use super::LifecycleState;
use crate::domain::error::DomainError;

fn refused(from: LifecycleState, to: LifecycleState) {
    assert!(
        matches!(from.transition(to), Err(DomainError::LifecycleForbidden(_))),
        "expected {from} -> {to} to be refused"
    );
}

#[test]
fn the_three_legal_edges_are_the_only_legal_edges() {
    // Enumerating the whole 4x4 product is the point: a new variant, or a
    // widened `matches!` arm, changes this count and has to be argued for.
    let legal: Vec<(LifecycleState, LifecycleState)> = LifecycleState::ALL
        .iter()
        .flat_map(|from| {
            LifecycleState::ALL
                .iter()
                .filter(move |to| from.can_transition(**to))
                .map(move |to| (*from, *to))
        })
        .collect();

    assert_eq!(
        legal,
        vec![
            (LifecycleState::Draft, LifecycleState::Published),
            (LifecycleState::Published, LifecycleState::Superseded),
            (LifecycleState::Published, LifecycleState::Retired),
        ]
    );
}

#[test]
fn a_draft_publishes() {
    assert!(
        LifecycleState::Draft
            .transition(LifecycleState::Published)
            .is_ok()
    );
}

#[test]
fn a_published_row_supersedes_or_retires() {
    // The two sanctioned producers of the supersession flip are the
    // supersession unit and the grandfathering cutover commit; retirement is
    // its own publish unit (D-128).
    assert!(
        LifecycleState::Published
            .transition(LifecycleState::Superseded)
            .is_ok()
    );
    assert!(
        LifecycleState::Published
            .transition(LifecycleState::Retired)
            .is_ok()
    );
}

#[test]
fn retirement_is_terminal() {
    // Nothing leaves `retired`. This is why retirement had to become a publish
    // unit: no later publish could ever re-project the plan to correct a read
    // model that still advertised it as sellable.
    refused(LifecycleState::Retired, LifecycleState::Draft);
    refused(LifecycleState::Retired, LifecycleState::Published);
    refused(LifecycleState::Retired, LifecycleState::Superseded);
}

#[test]
fn a_superseded_row_never_returns_to_published() {
    // Its key belongs to the successor; re-publishing it would put two current
    // rows on one canonical scope key.
    refused(LifecycleState::Superseded, LifecycleState::Published);
    refused(LifecycleState::Superseded, LifecycleState::Draft);
    refused(LifecycleState::Superseded, LifecycleState::Retired);
}

#[test]
fn a_published_row_never_returns_to_draft() {
    // The append-only discipline is the whole immutability guarantee: an edit
    // of a published row is a new row, never a walk backwards.
    refused(LifecycleState::Published, LifecycleState::Draft);
}

#[test]
fn no_state_transitions_to_itself() {
    // A re-publish is not a no-op; treating it as one would let a retry pass
    // the same guard twice.
    for state in LifecycleState::ALL {
        refused(*state, *state);
    }
}

#[test]
fn a_draft_can_only_go_to_published() {
    refused(LifecycleState::Draft, LifecycleState::Superseded);
    refused(LifecycleState::Draft, LifecycleState::Retired);
}

#[test]
fn only_a_draft_has_mutable_content() {
    assert!(LifecycleState::Draft.is_content_mutable());
    assert!(!LifecycleState::Published.is_content_mutable());
    assert!(!LifecycleState::Superseded.is_content_mutable());
    assert!(!LifecycleState::Retired.is_content_mutable());
}

#[test]
fn a_retired_revision_is_still_the_current_one() {
    // D-128 widened the predicate for exactly this: the projector needs a
    // referent after retirement, so a retired plan keeps a resolvable delta
    // for its in-flight subscribers.
    assert!(LifecycleState::Retired.is_current_revision());
    assert!(LifecycleState::Published.is_current_revision());
    assert!(!LifecycleState::Draft.is_current_revision());
    assert!(!LifecycleState::Superseded.is_current_revision());
}

#[test]
fn the_refusal_names_the_edge_it_refused() {
    // An operator reading the log needs to know which move was attempted, not
    // just that one was.
    let err = LifecycleState::Retired
        .transition(LifecycleState::Published)
        .expect_err("retired is terminal");

    assert!(err.to_string().contains("retired -> published"));
}
