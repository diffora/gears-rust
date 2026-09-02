//! `domain::taxonomy` — the cycle rule on the shape a two-deep fixture
//! cannot reach, and the name rule's two paths.

use uuid::Uuid;

use super::{
    AssignmentRole, CategoryReferenced, DefinitionState, REGISTRY_SEEDED_BY, RetireCensus,
    TaxonomyLimitExceeded, TaxonomyLimits, TaxonomyMutation, WELL_KNOWN_SEEDS, ancestors_of,
    children_of, cycle_verdict, depth_of, is_removable, limit_verdict, retire_verdict,
};

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

// -- Depth, children and the limits that have no number. --

/// **Depth is derived from the same chain the cycle rule reads**, so the two
/// cannot disagree — including on a tree that already contains a cycle, where
/// the walk stops on the repeat and the depth is finite rather than a hang.
#[test]
fn depth_counts_edges_and_a_root_is_zero() {
    assert_eq!(depth_of(A, &parent_of), 0, "a root sits at depth 0");
    assert_eq!(depth_of(B, &parent_of), 1);
    assert_eq!(depth_of(C, &parent_of), 2);
    assert_eq!(
        depth_of(D, &parent_of),
        0,
        "a node the map does not know is a root, not an error"
    );

    let looped = |id: Uuid| match id {
        x if x == A => Some(B),
        x if x == B => Some(A),
        _ => None,
    };
    assert_eq!(
        depth_of(A, &looped),
        1,
        "a pre-existing cycle terminates rather than counting forever"
    );
}

/// **Children are counted off the same edge list**, and `None` counts the
/// roots — the case a `parent_id`-equality count silently skips, both engines
/// treating NULL as unequal to everything.
#[test]
fn children_are_counted_including_the_roots() {
    let edges = vec![(A, None), (B, Some(A)), (C, Some(B)), (D, None)];
    assert_eq!(children_of(Some(A), &edges), 1);
    assert_eq!(children_of(Some(B), &edges), 1);
    assert_eq!(children_of(Some(C), &edges), 0);
    assert_eq!(
        children_of(None, &edges),
        2,
        "the roots are a sibling set too, and the root-name index treats them as one"
    );
}

/// **An unstated limit judges nothing.**
///
/// `None` is not "unlimited as policy" — it is that §7 row 2's owner has
/// stated no threshold. The assertion that matters is that an absent limit
/// cannot refuse: a guard with no number that refused would close the taxonomy
/// entirely, which is the half of "either refuses everything or nothing" this
/// rules out.
#[test]
fn an_unstated_limit_refuses_nothing() {
    let unstated = TaxonomyLimits {
        max_depth: None,
        max_children: None,
    };
    for (depth, children) in [(0, 0), (1, 1), (u32::MAX, u32::MAX)] {
        limit_verdict(depth, children, unstated)
            .expect("no threshold is stated, so nothing can exceed one");
    }
}

/// **Each limit refuses only above itself, and names which it was.**
///
/// The boundary is the load-bearing case: a guard written `>=` would refuse
/// the last admitted node, and every test written with a value two past the
/// limit would still pass.
#[test]
fn each_limit_refuses_above_itself_and_names_which() {
    let limits = TaxonomyLimits {
        max_depth: Some(3),
        max_children: Some(2),
    };

    limit_verdict(3, 2, limits).expect("exactly at both limits is admitted");

    let deep = limit_verdict(4, 2, limits).expect_err("one past max_depth");
    assert_eq!(deep.limit, "max_depth");
    assert_eq!((deep.allowed, deep.measured), (3, 4));

    let wide = limit_verdict(3, 3, limits).expect_err("one past max_children");
    assert_eq!(wide.limit, "max_children");
    assert_eq!((wide.allowed, wide.measured), (2, 3));

    assert_eq!(TaxonomyLimitExceeded::CODE, "TAXONOMY_LIMIT");
}

/// **One dimension configured does not make the other judge.**
///
/// Without this, a verdict written over a single `Option` pair — or one whose
/// second arm read the first's threshold — would pass every case above.
#[test]
fn a_configured_depth_leaves_an_unstated_children_limit_alone() {
    let depth_only = TaxonomyLimits {
        max_depth: Some(1),
        max_children: None,
    };
    limit_verdict(1, 10_000, depth_only).expect("children are not judged at all");
    limit_verdict(2, 0, depth_only).expect_err("depth still is");

    let children_only = TaxonomyLimits {
        max_depth: None,
        max_children: Some(1),
    };
    limit_verdict(10_000, 1, children_only).expect("depth is not judged at all");
    limit_verdict(0, 2, children_only).expect_err("children still are");
}

// -- The retire and delete guard's verdict. --

fn census(products: &[&str], children: &[&str], bound: usize) -> RetireCensus {
    RetireCensus {
        referencing_products: products.iter().map(|s| (*s).to_owned()).collect(),
        active_children: children.iter().map(|s| (*s).to_owned()).collect(),
        sample_bound: bound,
    }
}

/// **An empty census admits the retire** — the positive control without which
/// every refusal below could be a guard that refuses unconditionally.
#[test]
fn an_unreferenced_childless_category_may_retire() {
    retire_verdict(&census(&[], &[], 3)).expect("nothing holds it");
}

/// **Either half alone refuses, and the refusal names the holders.**
#[test]
fn products_and_children_each_refuse_on_their_own() {
    let by_products = retire_verdict(&census(&["Fibre 500"], &[], 3))
        .expect_err("a non-terminal product holds it");
    assert!(by_products.detail.contains("Fibre 500"), "{by_products:?}");
    assert!(by_products.detail.contains("product"), "{by_products:?}");
    assert!(
        !by_products.detail.contains("child"),
        "a clean children half must not be mentioned: {by_products:?}"
    );

    let by_children =
        retire_verdict(&census(&[], &["Fibre"], 3)).expect_err("an active child holds it");
    assert!(by_children.detail.contains("Fibre"), "{by_children:?}");
    assert!(by_children.detail.contains("child"), "{by_children:?}");
}

/// **Both halves are reported in one refusal.**
///
/// An operator who clears the products only to meet the children next has
/// been told half the truth twice. A guard returning the first blocker found
/// would pass both cases above and fail this one.
#[test]
fn both_halves_are_named_in_one_refusal() {
    let both = retire_verdict(&census(&["Fibre 500"], &["Fibre"], 3)).expect_err("both hold it");
    assert!(both.detail.contains("Fibre 500"), "{both:?}");
    assert!(both.detail.contains("child"), "{both:?}");
    assert!(both.detail.contains("product"), "{both:?}");
    assert_eq!(CategoryReferenced::CODE, "CATEGORY_REFERENCED");
}

/// **A sample that hit its bound says "at least".**
///
/// The caller reads `bound + 1` rows, so a sample longer than the bound is
/// the signal that more exist. Reporting a bare count there would name a
/// total the read never established.
#[test]
fn a_sample_at_its_bound_is_reported_as_at_least() {
    let exact = retire_verdict(&census(&["a", "b"], &[], 3)).expect_err("two hold it");
    assert!(
        exact.detail.contains("2 non-terminal"),
        "under the bound, the count is exact: {exact:?}"
    );

    let hit = retire_verdict(&census(&["a", "b", "c", "d"], &[], 3)).expect_err("four hold it");
    assert!(
        hit.detail.contains("at least 3"),
        "the read is bound + 1, so four rows means more than three exist: {hit:?}"
    );
}

// -- The well-known seeds. --

/// **The roster is `dod-well-known-seeds`' five keys, with its localized
/// flags.**
///
/// `imageUri` is the one non-localized seed, and it is the case a roster built
/// by copying one entry four times would get wrong.
#[test]
fn the_seed_roster_is_the_dods_five_with_its_localized_flags() {
    assert_eq!(
        WELL_KNOWN_SEEDS.map(|s| s.key),
        [
            "displayName",
            "description",
            "imageUri",
            "unitDisplayLabel",
            "marketingFeatures"
        ]
    );
    for seed in WELL_KNOWN_SEEDS {
        assert_eq!(
            seed.localized,
            seed.key != "imageUri",
            "{} is the DoD's only non-localized seed",
            seed.key
        );
        assert!(
            !seed.value_type.is_empty(),
            "chk_products_attribute_definition_value_type pins non-emptiness"
        );
    }
    assert_eq!(REGISTRY_SEEDED_BY, "registry");
}

/// **A seeded definition is not removable and an operator-added one is.**
///
/// The complement is the half that matters: §5's own trap is that a rule
/// worded as a whitelist gets implemented as one, and a guard that refused
/// every removal would satisfy the `DoD`'s sentence while breaking the roster
/// it is not about.
#[test]
fn only_an_operator_added_definition_is_removable() {
    assert!(
        !is_removable(Some(REGISTRY_SEEDED_BY)),
        "a registry seed must not be removable"
    );
    assert!(
        !is_removable(Some("some-other-seeder")),
        "the operand is the marker's presence, not its value"
    );
    assert!(
        is_removable(None),
        "an operator-added definition must be removable, or the rule refuses everything"
    );
}
