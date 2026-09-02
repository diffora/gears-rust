//! `domain::taxonomy` — the cycle rule on the shape a two-deep fixture
//! cannot reach, and the name rule's two paths.

use uuid::Uuid;

use super::{TaxonomyMutation, ancestors_of, cycle_verdict};

const A: Uuid = Uuid::from_u128(0xa1);
const B: Uuid = Uuid::from_u128(0xb2);
const C: Uuid = Uuid::from_u128(0xc3);
const D: Uuid = Uuid::from_u128(0xd4);

/// A tree `A -> B -> C` read as a parent map.
fn parent_of(id: Uuid) -> Option<Uuid> {
    match id {
        x if x == C => Some(B),
        x if x == B => Some(A),
        _ => None,
    }
}

/// **The chain is root-last and complete.**
#[test]
fn the_walk_returns_the_whole_chain_root_last() {
    assert_eq!(ancestors_of(C, &parent_of), vec![C, B, A]);
    assert_eq!(
        ancestors_of(A, &parent_of),
        vec![A],
        "a root is its own chain"
    );
}

/// **A cycle already in the store terminates the walk** rather than looping.
/// This runs exactly when a lock was skipped or a row was corrupted, so
/// non-termination would take the process with it.
#[test]
fn the_walk_terminates_on_a_pre_existing_cycle() {
    let looped = |id: Uuid| match id {
        x if x == A => Some(B),
        x if x == B => Some(A),
        _ => None,
    };
    let chain = ancestors_of(A, &looped);
    assert_eq!(chain, vec![A, B], "the repeat ends the walk");
}

/// **The deep case: `A -> B -> C`, re-parenting `A` under `C`.** `A` is
/// `C`'s grandparent, so the new chain `[C, B, A]` contains it — and a guard
/// comparing only the immediate parent (`C != A`) admits it. A two-deep
/// fixture cannot tell the two guards apart.
#[test]
fn a_reparent_under_a_descendant_two_levels_down_is_refused() {
    let new_ancestors = ancestors_of(C, &parent_of);
    assert_eq!(new_ancestors, vec![C, B, A]);
    let err = cycle_verdict(A, &new_ancestors)
        .expect_err("A is C's grandparent: the re-parent would close a cycle");
    assert_eq!(err.code(), "TAXONOMY_CYCLE");
    assert!(
        err.to_string().contains("depth 2"),
        "the depth is named: {err}"
    );
}

/// The immediate case, which the physical `CHECK` also refuses — kept
/// because defence in depth is only defence if both layers are asserted.
#[test]
fn a_node_under_itself_is_refused() {
    let err = cycle_verdict(A, &[A]).expect_err("a node is not its own parent");
    assert_eq!(err.code(), "TAXONOMY_CYCLE");
    assert!(err.to_string().contains("depth 0"), "{err}");
}

/// **The paired positive control**: a re-parent that closes nothing is
/// admitted, so the refusals above cannot be passing because every chain is
/// refused.
#[test]
fn a_reparent_outside_the_subtree_is_admitted() {
    // `D` is unrelated: moving it under `C` closes no loop.
    cycle_verdict(D, &ancestors_of(C, &parent_of)).expect("an unrelated node may move anywhere");
    // And moving `C` under `D` — a root with no chain of its own.
    cycle_verdict(C, &ancestors_of(D, &parent_of)).expect("a root parent closes nothing");
}

/// **All three mutations re-check the name**, and the re-parent arm is the
/// one a rename-only guard misses: the node carries its existing name into a
/// new sibling set, so it collides without the name changing.
#[test]
fn every_mutation_rechecks_the_name_including_the_reparent() {
    for m in [
        TaxonomyMutation::Rename,
        TaxonomyMutation::Reparent,
        TaxonomyMutation::Create,
    ] {
        assert!(m.rechecks_name(), "{m:?} must re-check name-in-parent");
    }
}
