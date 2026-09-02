//! `domain::taxonomy` — the cycle rule on the shape a two-deep fixture
//! cannot reach, and the name rule's two paths.

use uuid::Uuid;

use super::{AssignmentRole, DefinitionState, TaxonomyMutation, ancestors_of, cycle_verdict};

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
    // And moving `C` under `D` -- a root with no chain of its own.
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

// -- The two stored rosters. --

/// **The role roster cannot drift from the `CHECK` that enforces it.**
///
/// `uq_products_product_category_primary` is a partial index keyed on the
/// literal `'primary'`, so a spelling this type renders and the DDL does not
/// admit would not merely fail a write — it would defeat the at-most-one
/// guarantee `dod-category-assignment-table` says must be an index. The
/// migration is read as text because its statement arrays are private, the
/// same reach `migrations_tests` takes for the head tables.
///
/// **Both engines, counted.** The clause appears twice in that file — once in
/// `PG_UP_STATEMENTS` and once in `SQLITE_UP_STATEMENTS` — and asserting
/// presence alone would pass on a file that had lost the `SQLite` half.
#[test]
fn the_role_roster_matches_the_check_on_both_engines() {
    const SOURCE: &str =
        include_str!("../infra/storage/migrations/m20260901_000018_create_products_category.rs");
    let clause = format!(
        "role IN ('{}', '{}')",
        AssignmentRole::Primary.as_str(),
        AssignmentRole::Secondary.as_str()
    );
    assert_eq!(
        SOURCE.matches(clause.as_str()).count(),
        2,
        "the role roster `{clause}` must be the CHECK on both engines"
    );
}

/// **The definition-state roster cannot drift from its `CHECK`**, and the
/// same both-engines count applies for the same reason.
#[test]
fn the_definition_state_roster_matches_the_check_on_both_engines() {
    const SOURCE: &str =
        include_str!("../infra/storage/migrations/m20260901_000019_create_products_attribute.rs");
    let clause = format!(
        "state IN ('{}', '{}', '{}')",
        DefinitionState::Active.as_str(),
        DefinitionState::Deprecated.as_str(),
        DefinitionState::Removed.as_str()
    );
    assert_eq!(
        SOURCE.matches(clause.as_str()).count(),
        2,
        "the definition-state roster `{clause}` must be the CHECK on both engines"
    );
}

/// **Both rosters round-trip, and refuse everything outside themselves.**
///
/// The negative half is the load-bearing one: a `parse` written as a
/// catch-all default would pass every positive case here and silently read a
/// corrupt row as `Secondary` or `Active`. The near-misses are the ones a
/// hand-written column would actually contain — a capitalised token, a
/// neighbouring roster's value, the empty string.
#[test]
fn the_rosters_round_trip_and_refuse_everything_else() {
    for role in [AssignmentRole::Primary, AssignmentRole::Secondary] {
        assert_eq!(AssignmentRole::parse(role.as_str()), Some(role));
    }
    for outside in ["", "Primary", "PRIMARY", "primary ", "active", "main"] {
        assert_eq!(
            AssignmentRole::parse(outside),
            None,
            "`{outside}` is outside the role roster"
        );
    }

    for state in [
        DefinitionState::Active,
        DefinitionState::Deprecated,
        DefinitionState::Removed,
    ] {
        assert_eq!(DefinitionState::parse(state.as_str()), Some(state));
    }
    for outside in ["", "Active", "archived", "retired", "primary", "deleted"] {
        assert_eq!(
            DefinitionState::parse(outside),
            None,
            "`{outside}` is outside the definition-state roster"
        );
    }
}

/// **`retired` is the category roster's and never a definition's.** The two
/// state columns sit two tables apart and both are `text`; a reader that
/// accepted either roster's tokens would let a category state parse as a
/// definition state. `retired` is the one token that makes the mistake
/// silent, since it is a real value of the sibling column.
#[test]
fn the_category_states_are_not_definition_states() {
    for category_state in ["active", "retired"] {
        let parsed = DefinitionState::parse(category_state);
        if category_state == "active" {
            assert_eq!(parsed, Some(DefinitionState::Active), "the shared token");
        } else {
            assert_eq!(parsed, None, "`retired` is the category column's alone");
        }
    }
}
