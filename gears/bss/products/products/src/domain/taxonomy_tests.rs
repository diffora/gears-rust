//! `domain::taxonomy` — the cycle rule on the shape a two-deep fixture
//! cannot reach, and the name rule's two paths.

use uuid::Uuid;

use super::{
    AssignmentCandidate, AssignmentRole, AttributeDefinitionActiveRule,
    AttributeDefinitionKnownRule, AttributeScopeRule, AttributeValueTypeRule, CarriedDefinition,
    CategoryNotRetiredRule, CategoryReferenced, CategoryResolvableRule, CategoryRoleConflictRule,
    CategoryState, ContentPiiBlocked, ContentSaveSubject, DefaultLocaleRequired, DefinitionInUse,
    DefinitionState, FrozenAttributeValue, GLOBAL_COORDINATE, LocaleRequest, LocalizedValue,
    MetadataLimitExceeded, MetadataLimits, NO_PII_POLICY_REASON, NoPiiPolicyDetector, PiiDetector,
    PiiVerdict, PublishedContentSubject, REGISTRY_SEEDED_BY, ResolutionStep, ResolvedDefinition,
    RetireCensus, StaleCategoryToken, TAXONOMY_ERROR_CODES, TaxonomyLimitExceeded, TaxonomyLimits,
    TaxonomyMutation, ValueCandidate, ValueShape, WELL_KNOWN_SEEDS, ancestors_of,
    assignment_collection, children_of, content_pii_block, cycle_verdict, definition_edge,
    definition_in_use_verdict, depth_of, is_global, is_removable, limit_verdict,
    metadata_limit_verdict, resolve_localized, retire_verdict, seeded_edge, value_collection,
};
use crate::domain::validation::ValidationPipeline;

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

// -- The content-save validators (`inst-av-validate`, `inst-tx-assign`). --

/// The pipeline both save doors now run -- the real one, not a mirror.
///
/// Group A6 replaced the test-local copy that stood here. A mirror could pass
/// while the doors registered a different list, which is precisely the drift
/// `content_save_pipeline`'s own doc explains the single list exists to
/// remove; a test asserting the mirror would have proved nothing about either
/// door.
fn content_pipeline() -> ValidationPipeline<ContentSaveSubject> {
    super::content_save_pipeline()
}

/// Run the pipeline and answer the codes it raised, in order.
fn codes(subject: &ContentSaveSubject) -> Vec<&'static str> {
    content_pipeline()
        .run(subject)
        .map(|(_phase, report)| report.violations().iter().map(|v| v.code).collect())
        .unwrap_or_default()
}

fn assignment(
    id: Uuid,
    role: AssignmentRole,
    resolved: Option<CategoryState>,
) -> AssignmentCandidate {
    AssignmentCandidate {
        category_id: id,
        role,
        resolved,
    }
}

fn active_definition() -> ResolvedDefinition {
    ResolvedDefinition {
        state: DefinitionState::Active,
        value_type: "localized_string".to_owned(),
        localized: true,
        region_scope: String::new(),
        brand_scope: String::new(),
    }
}

fn value(resolved: Option<ResolvedDefinition>) -> ValueCandidate {
    ValueCandidate {
        definition_key: "displayName".to_owned(),
        locale: String::new(),
        region: String::new(),
        brand: String::new(),
        value: "Fibre 500".to_owned(),
        resolved,
    }
}

/// **The positive control for the whole pipeline.**
///
/// Every refusal below is measured against this: a payload naming a live
/// category and a live definition passes all seven rules. Without it, a rule
/// set that refused unconditionally would satisfy each refusal case alone.
#[test]
fn a_clean_content_payload_passes_every_rule() {
    let subject = ContentSaveSubject {
        assignments: vec![assignment(
            A,
            AssignmentRole::Primary,
            Some(CategoryState::Active),
        )],
        values: vec![value(Some(active_definition()))],
        entity_region_scope: String::new(),
        entity_brand_scope: String::new(),
    };
    assert!(
        content_pipeline().run(&subject).is_none(),
        "a clean payload must reach the store"
    );
}

/// **An unresolvable category is refused**, and the code is the Foundation's
/// generic because §7 row 17 records that this refusal has none of its own.
#[test]
fn an_unresolvable_category_is_refused_under_the_unassigned_code() {
    let subject = ContentSaveSubject {
        assignments: vec![assignment(A, AssignmentRole::Primary, None)],
        ..ContentSaveSubject::default()
    };
    assert_eq!(codes(&subject), vec![CategoryResolvableRule::CODE]);
    assert_eq!(
        CategoryResolvableRule::CODE,
        "VALIDATION",
        "row 17 owes this refusal a code; until then it rides the declared generic"
    );
}

/// **A retired category refuses a new assignment**, with its own declared
/// code -- and an active one does not, which is the paired control.
#[test]
fn a_retired_category_refuses_a_new_assignment() {
    let retired = ContentSaveSubject {
        assignments: vec![assignment(
            A,
            AssignmentRole::Primary,
            Some(CategoryState::Retired),
        )],
        ..ContentSaveSubject::default()
    };
    assert_eq!(codes(&retired), vec!["CATEGORY_RETIRED"]);

    let active = ContentSaveSubject {
        assignments: vec![assignment(
            A,
            AssignmentRole::Primary,
            Some(CategoryState::Active),
        )],
        ..ContentSaveSubject::default()
    };
    assert!(codes(&active).is_empty(), "the paired positive control");
}

/// **One category named twice is refused once**, not twice.
///
/// The rule compares forward only. A naive pairwise scan would raise a
/// violation from each side of the same pair and tell the operator about two
/// problems where there is one.
#[test]
fn one_category_in_two_roles_raises_exactly_one_violation() {
    let subject = ContentSaveSubject {
        assignments: vec![
            assignment(A, AssignmentRole::Primary, Some(CategoryState::Active)),
            assignment(A, AssignmentRole::Secondary, Some(CategoryState::Active)),
        ],
        ..ContentSaveSubject::default()
    };
    assert_eq!(codes(&subject), vec![CategoryRoleConflictRule::CODE]);

    let distinct = ContentSaveSubject {
        assignments: vec![
            assignment(A, AssignmentRole::Primary, Some(CategoryState::Active)),
            assignment(B, AssignmentRole::Secondary, Some(CategoryState::Active)),
        ],
        ..ContentSaveSubject::default()
    };
    assert!(
        codes(&distinct).is_empty(),
        "two categories, two roles, no conflict"
    );
}

/// **An unknown definition raises one violation and not four.**
///
/// All seven rules share `Phase::RegisteredValidators`, so they all run over
/// the same payload and each must skip what it cannot judge. A type rule or a
/// scope rule that read an unresolved definition would either panic or invent
/// a verdict, and the operator would be told four things about one mistake.
#[test]
fn an_unresolved_definition_raises_one_violation_and_not_four() {
    let subject = ContentSaveSubject {
        values: vec![value(None)],
        ..ContentSaveSubject::default()
    };
    assert_eq!(codes(&subject), vec!["ATTRIBUTE_DEFINITION_UNKNOWN"]);
}

/// **A `removed` definition is unknown, not deprecated.**
///
/// The tombstone exists as a row and is outside the set -- `repo::recognized`
/// states the same rule for the sibling roster. It keeps a terminal head's
/// value resolving and admits no new write, and the code says *not in the
/// set* rather than *on its way out*.
#[test]
fn a_removed_definition_is_refused_as_unknown_and_a_deprecated_one_as_deprecated() {
    for (state, expected) in [
        (DefinitionState::Removed, "ATTRIBUTE_DEFINITION_UNKNOWN"),
        (
            DefinitionState::Deprecated,
            "ATTRIBUTE_DEFINITION_DEPRECATED",
        ),
    ] {
        let subject = ContentSaveSubject {
            values: vec![value(Some(ResolvedDefinition {
                state,
                ..active_definition()
            }))],
            ..ContentSaveSubject::default()
        };
        assert_eq!(codes(&subject), vec![expected], "{state:?}");
    }

    let live = ContentSaveSubject {
        values: vec![value(Some(active_definition()))],
        ..ContentSaveSubject::default()
    };
    assert!(codes(&live).is_empty(), "the paired positive control");
}

/// **The three shapes, each refused and each admitted.**
///
/// `LocalizedString` constrains nothing, which is stated rather than left to
/// be inferred from a missing case.
#[test]
fn each_known_value_shape_refuses_and_admits() {
    for (value_type, good, bad) in [
        ("uri_string", "https://cdn.example/a.png", "just a name"),
        (
            "localized_string_list",
            r#"["fast","cheap"]"#,
            "fast, cheap",
        ),
    ] {
        for (candidate, should_pass) in [(good, true), (bad, false)] {
            let subject = ContentSaveSubject {
                values: vec![ValueCandidate {
                    value: candidate.to_owned(),
                    resolved: Some(ResolvedDefinition {
                        value_type: value_type.to_owned(),
                        ..active_definition()
                    }),
                    ..value(None)
                }],
                ..ContentSaveSubject::default()
            };
            assert_eq!(
                codes(&subject).is_empty(),
                should_pass,
                "`{candidate}` against {value_type}"
            );
        }
    }

    assert!(
        ValueShape::LocalizedString.admits(""),
        "a localized string constrains nothing, including emptiness"
    );
}

/// **An unmapped type token is not judged**, which is the honest reading of a
/// roster `design/02` §6 has not decided.
///
/// The load-bearing half is that it is not *refused*: a fail-closed reading
/// would make every operator-defined type unwritable, closing the feature to
/// everything outside the five seeds.
#[test]
fn a_type_token_the_gear_does_not_know_is_not_judged() {
    assert_eq!(ValueShape::of("something_nobody_declared"), None);
    let subject = ContentSaveSubject {
        assignments: Vec::new(),
        values: vec![ValueCandidate {
            value: "anything at all".to_owned(),
            resolved: Some(ResolvedDefinition {
                value_type: "something_nobody_declared".to_owned(),
                ..active_definition()
            }),
            ..value(None)
        }],
        ..ContentSaveSubject::default()
    };
    assert!(codes(&subject).is_empty());
}

/// **An empty scope column is unrestricted, not empty** (P-D-39), on both the
/// definition's side and the entity's.
///
/// This is the trap the handoff names: a containment predicate written as set
/// membership alone hides every unrestricted row -- here it would refuse every
/// coordinate under nearly every definition in the gear.
#[test]
fn an_empty_scope_column_admits_every_named_coordinate() {
    let subject = ContentSaveSubject {
        assignments: Vec::new(),
        values: vec![ValueCandidate {
            region: "apac".to_owned(),
            brand: "acme".to_owned(),
            resolved: Some(active_definition()),
            ..value(None)
        }],
        entity_region_scope: String::new(),
        entity_brand_scope: String::new(),
    };
    assert!(
        codes(&subject).is_empty(),
        "both scopes are unrestricted, so any coordinate is inside them"
    );
}

/// **A named coordinate outside either scope is refused**, and the refusal
/// says which side it failed.
#[test]
fn a_coordinate_outside_either_scope_is_refused() {
    let outside_definition = ContentSaveSubject {
        assignments: Vec::new(),
        values: vec![ValueCandidate {
            region: "apac".to_owned(),
            resolved: Some(ResolvedDefinition {
                region_scope: "eu".to_owned(),
                ..active_definition()
            }),
            ..value(None)
        }],
        entity_region_scope: String::new(),
        entity_brand_scope: String::new(),
    };
    assert_eq!(
        codes(&outside_definition),
        vec!["ATTRIBUTE_SCOPE_VIOLATION"]
    );

    let outside_entity = ContentSaveSubject {
        assignments: Vec::new(),
        values: vec![ValueCandidate {
            region: "apac".to_owned(),
            resolved: Some(active_definition()),
            ..value(None)
        }],
        entity_region_scope: "eu".to_owned(),
        entity_brand_scope: String::new(),
    };
    let report = content_pipeline()
        .run(&outside_entity)
        .expect("the entity's own scope refuses it");
    assert_eq!(report.1.violations()[0].code, "ATTRIBUTE_SCOPE_VIOLATION");
    assert!(
        report.1.violations()[0]
            .detail
            .contains("entity's own scope"),
        "the two sides are told apart: {:?}",
        report.1.violations()[0]
    );

    let inside = ContentSaveSubject {
        assignments: Vec::new(),
        values: vec![ValueCandidate {
            region: "eu".to_owned(),
            resolved: Some(ResolvedDefinition {
                region_scope: "eu,apac".to_owned(),
                ..active_definition()
            }),
            ..value(None)
        }],
        entity_region_scope: "eu".to_owned(),
        entity_brand_scope: String::new(),
    };
    assert!(codes(&inside).is_empty(), "the paired positive control");
}

/// **A brand-less coordinate survives a brand-scoped entity** -- `design/02`
/// §6's open item, deferred in the only direction that leaves both `DoD`s
/// satisfiable.
///
/// The item records that a containment-only reading makes *"the write the
/// publish validator demands the write the save validator refuses"*, so a
/// brand-scoped entity could never publish. This case pins the reading taken:
/// `brand: ""` is P-D-88 arm 2's **absence**, not a brand named empty-string,
/// and there is nothing to contain. If the owner decides otherwise, this is
/// the test that has to change and the report says so.
#[test]
fn a_brand_less_global_value_survives_a_brand_scoped_entity() {
    let subject = ContentSaveSubject {
        assignments: Vec::new(),
        values: vec![ValueCandidate {
            brand: String::new(),
            resolved: Some(ResolvedDefinition {
                brand_scope: "acme".to_owned(),
                ..active_definition()
            }),
            ..value(None)
        }],
        entity_region_scope: String::new(),
        entity_brand_scope: "acme".to_owned(),
    };
    assert!(
        codes(&subject).is_empty(),
        "the global coordinate `dod-default-locale` makes mandatory must not be \
         the one `dod-value-validators` refuses"
    );
}

/// **A scope column that will not parse refuses rather than admits.**
///
/// `ResolvedScope::parse` rejects an empty token between separators, and a
/// corrupt column is not an admission -- fail-closed is the gear's principle
/// and this is the one branch where the two readings differ.
#[test]
fn an_unparseable_scope_column_refuses() {
    let subject = ContentSaveSubject {
        values: vec![ValueCandidate {
            region: "eu".to_owned(),
            resolved: Some(ResolvedDefinition {
                region_scope: "eu,,apac".to_owned(),
                ..active_definition()
            }),
            ..value(None)
        }],
        ..ContentSaveSubject::default()
    };
    assert_eq!(codes(&subject), vec!["ATTRIBUTE_SCOPE_VIOLATION"]);
}

// -- The definition state machine. --

/// **Exactly four edges, and `active -> removed` is not one of them.**
///
/// `inst-de-deprecate-then-remove` puts deprecation between them so the
/// destructive step cannot be reached in one act; both re-listings are
/// declared because §4 declares them.
#[test]
fn the_definition_machine_admits_exactly_its_four_edges() {
    use DefinitionState::{Active, Deprecated, Removed};
    let admitted = [
        (Active, Deprecated),
        (Deprecated, Removed),
        (Deprecated, Active),
        (Removed, Active),
    ];
    for (from, to) in admitted {
        definition_edge(from, to).unwrap_or_else(|e| panic!("{from:?} -> {to:?}: {e}"));
    }
    for from in [Active, Deprecated, Removed] {
        for to in [Active, Deprecated, Removed] {
            if admitted.contains(&(from, to)) {
                continue;
            }
            let err = definition_edge(from, to)
                .expect_err("every other pair, self-edges included, is refused");
            assert_eq!(err.code(), "ILLEGAL_TRANSITION", "{from:?} -> {to:?}");
        }
    }
    definition_edge(Active, Removed)
        .expect_err("the destructive step is never reachable in one act");
}

/// **A seed is deprecatable and never removable** -- both halves, so a guard
/// refusing every act on a seed fails as surely as one refusing none.
#[test]
fn a_seed_may_be_deprecated_and_never_removed() {
    seeded_edge(Some(REGISTRY_SEEDED_BY), DefinitionState::Deprecated)
        .expect("a seed is deprecatable");
    seeded_edge(Some(REGISTRY_SEEDED_BY), DefinitionState::Active).expect("and re-listable");
    let err = seeded_edge(Some(REGISTRY_SEEDED_BY), DefinitionState::Removed)
        .expect_err("and never removable");
    assert_eq!(err.code(), "ILLEGAL_FIELD_MUTATION");
    seeded_edge(None, DefinitionState::Removed).expect("an operator-added definition IS removable");
}

/// **`DEFINITION_IN_USE` names its carriers and bounds the sample**, the same
/// contract the retire guard's verdict carries.
#[test]
fn the_in_use_verdict_names_its_carriers() {
    definition_in_use_verdict(&[], 3).expect("nothing carries it");

    let held = definition_in_use_verdict(&["Fibre 500".to_owned()], 3).expect_err("held");
    assert!(held.detail.contains("Fibre 500"), "{held:?}");
    assert_eq!(DefinitionInUse::CODE, "DEFINITION_IN_USE");

    let many = definition_in_use_verdict(
        &[
            "a".to_owned(),
            "b".to_owned(),
            "c".to_owned(),
            "d".to_owned(),
        ],
        3,
    )
    .expect_err("held");
    assert!(many.detail.contains("at least 3"), "{many:?}");
}

// -- The locale fallback chain. --

const TENANT_DEFAULT: &str = "en-GB";

fn coordinate(locale: &str, region: &str, brand: &str, value: &str) -> LocalizedValue {
    LocalizedValue {
        locale: locale.to_owned(),
        region: region.to_owned(),
        brand: brand.to_owned(),
        value: value.to_owned(),
    }
}

/// The matrix fixture: one value at each step of the chain, plus a
/// brand-A-only default so the brand-B case has something to miss.
fn matrix() -> Vec<LocalizedValue> {
    vec![
        coordinate("fr-FR", "eu", "acme", "exact"),
        coordinate("fr-FR", "", "acme", "locale+brand"),
        coordinate(TENANT_DEFAULT, "", "acme", "default+brandA"),
        coordinate("", "", "", "global"),
    ]
}

fn ask<'a>(locale: &'a str, region: &'a str, brand: &'a str) -> LocaleRequest<'a> {
    LocaleRequest {
        locale,
        region,
        brand,
        tenant_default_locale: TENANT_DEFAULT,
    }
}

/// **Every step of `inst-av-resolve`'s chain, each reached by a reader that
/// misses the ones above it.**
///
/// The step is asserted, not only the value: a resolver whose first step
/// matched everything would satisfy a value-only assertion at every row here,
/// and a chain with two steps collapsed would satisfy it at most of them.
#[test]
fn the_resolution_matrix_reaches_every_step() {
    let values = matrix();
    for (locale, region, brand, expected_value, expected_step) in [
        ("fr-FR", "eu", "acme", "exact", ResolutionStep::Exact),
        (
            "fr-FR",
            "apac",
            "acme",
            "locale+brand",
            ResolutionStep::LocaleAndBrand,
        ),
        (
            "de-DE",
            "eu",
            "acme",
            "default+brandA",
            ResolutionStep::DefaultLocaleAndBrand,
        ),
        ("de-DE", "eu", "beta", "global", ResolutionStep::Global),
    ] {
        let hit = resolve_localized(&ask(locale, region, brand), &values)
            .unwrap_or_else(|| panic!("({locale}, {region}, {brand}) resolved to nothing"));
        assert_eq!(
            (hit.value, hit.step),
            (expected_value, expected_step),
            "({locale}, {region}, {brand})"
        );
    }
}

/// **The `DoD`'s named case: a brand-B reader against a value present only at
/// `(default-locale, brand A)` resolves only through the global coordinate.**
///
/// This is why the per-brand default is an *override* and the global value is
/// what makes the chain total. Both halves are asserted -- brand A does reach
/// its own default, and brand B does not reach A's.
#[test]
fn a_brand_b_reader_never_reaches_brand_as_default_and_falls_to_global() {
    let values = matrix();

    let brand_a = resolve_localized(&ask("de-DE", "", "acme"), &values).expect("resolved");
    assert_eq!(brand_a.step, ResolutionStep::DefaultLocaleAndBrand);
    assert_eq!(brand_a.value, "default+brandA");

    let brand_b = resolve_localized(&ask("de-DE", "", "beta"), &values).expect("resolved");
    assert_eq!(
        brand_b.step,
        ResolutionStep::Global,
        "brand B never visits brand A's default"
    );
    assert_eq!(brand_b.value, "global");
}

/// **A tenant-default change is non-retroactive.**
///
/// `inst-av-resolve`'s item-37 note is the claim: totality is anchored on the
/// resolution path, not on the config value, *"so anchoring on it would
/// un-total the chain for every already-published entity the moment it
/// changed"*. Here the tenant default moves to a locale nothing is stored
/// under. The reader that had been resolving at step 3 moves to step 4 -- the
/// **step** changes, the resolution does not fail. That is the whole property,
/// and it holds because the last step is the global coordinate rather than the
/// config value.
#[test]
fn a_tenant_default_change_moves_a_step_and_never_un_resolves() {
    let values = matrix();
    let before = resolve_localized(&ask("de-DE", "", "acme"), &values).expect("resolved");
    assert_eq!(before.step, ResolutionStep::DefaultLocaleAndBrand);

    let after = resolve_localized(
        &LocaleRequest {
            locale: "de-DE",
            region: "",
            brand: "acme",
            tenant_default_locale: "ja-JP",
        },
        &values,
    )
    .expect("still resolves");
    assert_eq!(
        after.step,
        ResolutionStep::Global,
        "the chain falls through to global rather than running out"
    );
}

/// **A region-specific value is not widened to a neighbouring region.**
///
/// Steps 2 and 3 name only a locale and a brand, so both look for a value
/// whose region is **absent**. A step 2 written region-insensitively would
/// hand an `eu` value to an `apac` reader, which is the one silent way this
/// chain can be wrong.
#[test]
fn a_regional_value_is_never_handed_to_another_region() {
    let values = vec![
        coordinate("fr-FR", "eu", "acme", "eu only"),
        coordinate("", "", "", "global"),
    ];
    let hit = resolve_localized(&ask("fr-FR", "apac", "acme"), &values).expect("resolved");
    assert_eq!(
        (hit.value, hit.step),
        ("global", ResolutionStep::Global),
        "the eu value is reachable only by an eu reader"
    );
}

/// **An empty set resolves to nothing** rather than to an invented value --
/// the gap `dod-default-locale`'s validator exists to keep unreachable.
#[test]
fn a_definition_with_no_values_resolves_to_nothing() {
    assert_eq!(resolve_localized(&ask("fr-FR", "eu", "acme"), &[]), None);
}

// -- The default-locale publish validator. --

fn carried(localized: bool, values: Vec<LocalizedValue>) -> PublishedContentSubject {
    PublishedContentSubject {
        carried: vec![CarriedDefinition {
            key: "displayName".to_owned(),
            localized,
            values,
        }],
    }
}

fn publish_codes(subject: &PublishedContentSubject) -> Vec<&'static str> {
    super::published_content_pipeline()
        .run(subject)
        .map(|(_phase, report)| report.violations().iter().map(|v| v.code).collect())
        .unwrap_or_default()
}

/// **A localized definition with values but none global is refused**, and one
/// with a global value is admitted.
#[test]
fn a_localized_definition_needs_its_global_value_to_publish() {
    let missing = carried(true, vec![coordinate("fr-FR", "", "", "Fibre")]);
    assert_eq!(publish_codes(&missing), vec!["DEFAULT_LOCALE_MISSING"]);

    let present = carried(
        true,
        vec![
            coordinate("fr-FR", "", "", "Fibre"),
            coordinate("", "", "", "Fibre"),
        ],
    );
    assert!(publish_codes(&present).is_empty(), "the paired control");
}

/// **A per-brand default is an override and satisfies nothing.**
///
/// The rule's whole point: a value at `(default-locale, brand A)` leaves every
/// other brand's chain able to run out, which the resolver matrix above
/// measures from the reading side. A rule checking "any value with an empty
/// region" would pass this and be wrong for every brand but A.
#[test]
fn a_per_brand_default_does_not_satisfy_the_global_requirement() {
    let brand_only = carried(
        true,
        vec![coordinate(TENANT_DEFAULT, "", "acme", "brand A's default")],
    );
    assert_eq!(publish_codes(&brand_only), vec!["DEFAULT_LOCALE_MISSING"]);
}

/// **A non-localized definition is not judged, and neither is one carrying no
/// values.**
///
/// `imageUri` has no locale chain to make total, and a definition the entity
/// carries nothing for cannot have a gap. Without these two arms the rule
/// would refuse every publish of an entity holding an image or holding a
/// definition it has not authored yet.
#[test]
fn the_rule_skips_what_has_no_chain_to_make_total() {
    assert!(
        publish_codes(&carried(false, vec![coordinate("fr-FR", "", "", "x")])).is_empty(),
        "a non-localized definition has no locale chain"
    );
    assert!(
        publish_codes(&carried(true, Vec::new())).is_empty(),
        "no values, no gap"
    );
}

/// **`is_global` is the coordinate P-D-88 arm 2 spells, and nothing near it.**
///
/// A brand-scoped or region-scoped value is not the global one however few
/// coordinates it names, which is the distinction the whole chain rests on.
#[test]
fn only_three_absent_coordinates_are_the_global_one() {
    assert!(is_global(&coordinate("", "", "", "v")));
    for near in [
        coordinate("en-GB", "", "", "v"),
        coordinate("", "eu", "", "v"),
        coordinate("", "", "acme", "v"),
    ] {
        assert!(!is_global(&near), "{near:?} is not the global coordinate");
    }
    assert_eq!(GLOBAL_COORDINATE, ("", "", ""));
    assert_eq!(StaleCategoryToken::CODE, "STALE_CATEGORY_TOKEN");
}

// -- Frozen version content. --

fn frozen(
    definition: Uuid,
    locale: &str,
    region: &str,
    brand: &str,
    value: &str,
) -> FrozenAttributeValue {
    FrozenAttributeValue {
        definition_id: definition,
        coordinate: coordinate(locale, region, brand, value),
    }
}

fn render(collection: &serde_json::Value) -> String {
    crate::domain::canonical::canonical_rendering(
        collection,
        crate::domain::canonical::Absence::Omit,
    )
}

/// **The assignment set sorts by category id whatever order it arrives in.**
///
/// The input is deliberately reversed against the expected output, so a
/// function that returned its argument untouched would fail rather than
/// coincide.
#[test]
fn the_assignment_collection_sorts_by_category_id() {
    let forward =
        assignment_collection(&[(A, AssignmentRole::Primary), (B, AssignmentRole::Secondary)]);
    let reversed =
        assignment_collection(&[(B, AssignmentRole::Secondary), (A, AssignmentRole::Primary)]);
    assert_eq!(
        render(&forward),
        render(&reversed),
        "input order is not carried"
    );
    assert!(
        render(&forward).find(&A.to_string()) < render(&forward).find(&B.to_string()),
        "sorted by category id: {}",
        render(&forward)
    );
}

/// **The value set's order is total over the whole coordinate, which an
/// identifier sort is not.**
///
/// This is §7 row 9's own case: four rows of **one** definition. Sorting by
/// the definition id orders groups and leaves these four to whatever the
/// driver returned, so two engines can serialize one content two ways -- the
/// failure P-D-29 exists to prevent. Every permutation below must render to
/// the same bytes, and a sort keyed on the definition alone fails all of them
/// while passing any fixture that used four *different* definitions.
#[test]
fn the_value_collection_is_total_over_four_rows_of_one_definition() {
    let d = Uuid::from_u128(0xde_01);
    let rows = [
        frozen(d, "", "", "", "global"),
        frozen(d, "fr-FR", "", "", "locale"),
        frozen(d, "fr-FR", "eu", "", "locale+region"),
        frozen(d, "fr-FR", "eu", "acme", "all three"),
    ];
    let canonical = render(&value_collection(&rows));

    // Every rotation of the same four rows.
    for shift in 1..rows.len() {
        let mut shuffled = rows.to_vec();
        shuffled.rotate_left(shift);
        assert_eq!(
            render(&value_collection(&shuffled)),
            canonical,
            "rotation by {shift} rendered differently"
        );
    }
    // And the reverse, which no rotation reaches.
    let mut backwards = rows.to_vec();
    backwards.reverse();
    assert_eq!(render(&value_collection(&backwards)), canonical);
}

/// **Two definitions interleave by definition first, then by coordinate.**
///
/// Without this, a sort keyed only on the coordinate -- the mirror mistake of
/// the one row 9 names -- would pass the case above and scatter each
/// definition's rows through the other's.
#[test]
fn the_value_collection_groups_by_definition_before_coordinate() {
    let first = Uuid::from_u128(0x01);
    let second = Uuid::from_u128(0x02);
    let rendered = render(&value_collection(&[
        frozen(second, "", "", "", "b-global"),
        frozen(first, "fr-FR", "", "", "a-locale"),
        frozen(second, "fr-FR", "", "", "b-locale"),
        frozen(first, "", "", "", "a-global"),
    ]));
    let order: Vec<&str> = ["a-global", "a-locale", "b-global", "b-locale"]
        .into_iter()
        .collect();
    let mut last = 0;
    for value in order {
        let at = rendered
            .find(value)
            .unwrap_or_else(|| panic!("{value} missing from {rendered}"));
        assert!(at >= last, "{value} out of order in {rendered}");
        last = at;
    }
}

/// **The golden vector**: the exact bytes, pinned.
///
/// `dod-version-content-rendering` requires a golden vector proving the
/// rendering byte-identical across both engines. The rendering is a pure
/// function of the rows, so what an engine can change is the **order** they
/// arrive in -- which the two cases above hold against every permutation of
/// the hard case. This pins the bytes themselves, so a change to the element
/// shape, the key names or the Foundation's field ordering reddens here
/// rather than in a restore drill.
#[test]
fn the_frozen_content_golden_vector_is_pinned() {
    let d = Uuid::from_u128(0xde_01);
    assert_eq!(
        render(&value_collection(&[
            frozen(d, "fr-FR", "", "", "Fibre"),
            frozen(d, "", "", "", "Fibre 500"),
        ])),
        concat!(
            r#"[{"brand":"","definitionId":"00000000-0000-0000-0000-00000000de01","#,
            r#""locale":"","region":"","value":"Fibre 500"},"#,
            r#"{"brand":"","definitionId":"00000000-0000-0000-0000-00000000de01","#,
            r#""locale":"fr-FR","region":"","value":"Fibre"}]"#
        ),
        "the element keys are field-ordered by the Foundation's rule and the \
         array order is the collection's"
    );
    assert_eq!(
        render(&assignment_collection(&[(A, AssignmentRole::Primary)])),
        r#"[{"categoryId":"00000000-0000-0000-0000-0000000000a1","role":"primary"}]"#
    );
}

/// **An empty collection renders as an empty array**, not as `null` and not
/// as an omitted field. A frozen row whose entity carries no assignments must
/// still be distinguishable from one whose collection was never rendered.
#[test]
fn an_empty_collection_renders_as_an_empty_array() {
    assert_eq!(render(&assignment_collection(&[])), "[]");
    assert_eq!(render(&value_collection(&[])), "[]");
}

// -- The sixteen codes. --

/// **The roster is the design's sixteen, distinct, and every entry that has a
/// raiser is reachable through it.**
#[test]
fn the_sixteen_codes_are_a_distinct_roster() {
    assert_eq!(TAXONOMY_ERROR_CODES.len(), 16);
    let distinct: std::collections::HashSet<&str> = TAXONOMY_ERROR_CODES.into_iter().collect();
    assert_eq!(distinct.len(), 16, "a repeat would satisfy the count");

    // Every `CODE` constant this feature declares is one of the sixteen. A
    // constant naming a code outside the roster is a seventeenth minted by a
    // rule, which §7 row 17 is what stops.
    for code in [
        CategoryNotRetiredRule::CODE,
        AttributeDefinitionKnownRule::CODE,
        AttributeDefinitionActiveRule::CODE,
        AttributeValueTypeRule::CODE,
        AttributeScopeRule::CODE,
        DefaultLocaleRequired::CODE,
        CategoryReferenced::CODE,
        TaxonomyLimitExceeded::CODE,
        DefinitionInUse::CODE,
        StaleCategoryToken::CODE,
    ] {
        assert!(
            TAXONOMY_ERROR_CODES.contains(&code),
            "`{code}` is not one of the design's sixteen"
        );
    }
}

/// **The counted gate on the registration gap.**
///
/// Twelve of the sixteen have no `DomainError` variant at this commit, so no
/// door can raise them as themselves and every one would fall back to
/// `INCOMPLETE_ENTITY` through `transition_refusal`'s ladder. That is
/// `dod-taxonomy-errors`' remaining work and it lands as a patch to
/// `domain::error` and `infra::error_mapping`, neither of which is this
/// strand's file.
///
/// The literal is the gate: **when the patch lands this test reddens**, and
/// the number is what the applier updates -- to `0`, in the same edit. A
/// silent gap is what this exists to prevent, so it is deliberately not
/// written as an inequality.
#[test]
fn ten_of_the_sixteen_codes_have_no_domain_error_variant_yet() {
    let raiseable = raiseable_codes();
    assert_eq!(
        raiseable,
        vec![
            "DUPLICATE_CATEGORY_NAME",
            "TAXONOMY_CYCLE",
            "PRIMARY_CATEGORY_REQUIRED",
            "CONTENT_PII_BLOCKED",
            "METADATA_LIMIT",
            "STALE_LIVE_OP",
        ],
        "exactly these six are raiseable as themselves, in TAXONOMY_ERROR_CODES' \
         own order"
    );
    assert_eq!(
        TAXONOMY_ERROR_CODES.len() - raiseable.len(),
        10,
        "ten still need a variant and a mapping arm"
    );
}

/// The taxonomy codes `DomainError::code` can answer, **derived from the
/// enum's own source** rather than listed.
///
/// # This was a literal, its doc denied being one, and it drifted twice
///
/// The first version claimed to be *"read off the enum rather than listed --
/// so this cannot drift"* and was a hand-written `&[&str]`. It went stale when
/// `b844b2632` landed `DomainError::ContentPiiBlocked`, and the test above
/// **passed anyway**, because its filter and its subtraction were both
/// computed from that one array: the two halves drift together and cancel.
///
/// The repair on 2026-09-03 corrected the contents and the doc but left it a
/// literal with a written-down re-derivation discipline -- and the very next
/// commit, `METADATA_LIMIT`'s, walked past it the same way. **A discipline in
/// a doc comment is not a guard.** So the filter now reads `error.rs` for the
/// `code()` arm itself, which is what the original doc always claimed and what
/// breaks the shared operand: the expected list in the test stays a hand
/// written claim, and nothing computes it from the same place.
fn raiseable_codes() -> Vec<&'static str> {
    let source = include_str!("error.rs");
    TAXONOMY_ERROR_CODES
        .into_iter()
        .filter(|code| source.contains(&format!("=> \"{code}\"")))
        .collect()
}

/// **An unset tenant default skips step 3 rather than keying it on `""`.**
///
/// `ProductsConfig::default_locale`'s own doc states the behaviour — *"an
/// unset default locale skips step 3 and resolution still succeeds"* — and
/// running the step with `""` would key it on `("", "", brand)`, which is a
/// brand-scoped locale-less value a caller can genuinely write. A reader with
/// no configured default would then be handed that value **ahead of the
/// global one**: a different answer, not a shorter path.
///
/// The paired control is the same fixture with a default configured, which
/// must still reach step 3 — without it a resolver that skipped step 3
/// unconditionally would pass this case.
#[test]
fn an_unset_tenant_default_skips_step_three() {
    let values = vec![
        coordinate("", "", "acme", "brand-scoped, locale-less"),
        coordinate("", "", "", "global"),
        coordinate(
            TENANT_DEFAULT,
            "",
            "acme",
            "the brand's default-locale value",
        ),
    ];

    let unset = resolve_localized(
        &LocaleRequest {
            locale: "de-DE",
            region: "",
            brand: "acme",
            tenant_default_locale: "",
        },
        &values,
    )
    .expect("resolution still succeeds");
    assert_eq!(
        (unset.value, unset.step),
        ("global", ResolutionStep::Global),
        "with no default configured the chain falls past step 3 to the global value"
    );

    let configured = resolve_localized(&ask("de-DE", "", "acme"), &values).expect("resolved");
    assert_eq!(
        (configured.value, configured.step),
        (
            "the brand's default-locale value",
            ResolutionStep::DefaultLocaleAndBrand
        ),
        "the paired control: a configured default still reaches step 3"
    );
}

// -- The content-PII write block. --

/// A detector double: blocks any text containing `needle`, and is uncertain
/// about text containing `maybe`.
struct Doubles {
    needle: &'static str,
    maybe: &'static str,
}

impl PiiDetector for Doubles {
    fn inspect(&self, _subject: &str, text: &str) -> PiiVerdict {
        if text.contains(self.needle) {
            PiiVerdict::Blocked("matches a personal-data pattern".to_owned())
        } else if text.contains(self.maybe) {
            PiiVerdict::Uncertain("the allow-list does not cover this shape".to_owned())
        } else {
            PiiVerdict::Clean
        }
    }
}

fn doubles() -> Doubles {
    Doubles {
        needle: "ssn",
        maybe: "perhaps",
    }
}

/// **The block refuses personal data and admits clean text**, and the second
/// half is not an extra.
///
/// `design/02` §6 says so about this very seam: *"a stub that refuses every
/// string satisfies both `dod-pii-write-block` and acceptance criterion 22.
/// A clean-text positive control is therefore part of the criterion."* A hook
/// that refused everything would pass the first assertion here and fail the
/// second, which is the whole point of having both.
#[test]
fn the_block_refuses_personal_data_and_admits_clean_text() {
    let blocked = content_pii_block(&doubles(), "attributes.description", "ssn 000-00-0000")
        .expect_err("personal data is refused");
    assert!(
        blocked.detail().contains("attributes.description"),
        "{blocked:?}"
    );
    assert_eq!(ContentPiiBlocked::CODE, "CONTENT_PII_BLOCKED");

    content_pii_block(&doubles(), "attributes.description", "a fast fibre line")
        .expect("clean text is admitted -- the positive control section 6 asks for");
}

/// **Uncertainty blocks, and the rule lives in the hook.**
///
/// `dod-pii-write-block` puts *"failing closed on uncertainty"* on the hook
/// rather than on each detector, and that placement is the assertion: a
/// detector cannot opt out by answering its own doubt as `Clean`, because the
/// hook never sees `Clean` in that case. The refusal says which of the two
/// reasons it was, so an operator can tell "we found something" from "we could
/// not tell".
#[test]
fn an_undecided_verdict_fails_closed() {
    let refused = content_pii_block(&doubles(), "metadata.owner", "perhaps a name")
        .expect_err("uncertainty is refused");
    assert!(
        refused.detail().contains("could not decide"),
        "the two refusals are told apart: {refused:?}"
    );
    assert!(refused.detail().contains("fails closed"), "{refused:?}");
}

/// **The default host admits everything and names the deviation.**
///
/// It is not a claim that the text is clean. `NoMaterialityPolicyGate`'s own
/// reason constant takes the same shape and for the same reason: the string is
/// what an operator sees, so it states what is missing rather than justifying
/// the pass.
#[test]
fn the_default_host_admits_and_says_it_inspected_nothing() {
    for text in ["ssn 000-00-0000", "perhaps a name", ""] {
        content_pii_block(&NoPiiPolicyDetector, "attributes.description", text)
            .expect("no detector is registered, so nothing is inspected");
    }
    assert!(
        NO_PII_POLICY_REASON.contains("deviation owed to slice 10"),
        "the reason names what is missing, not a finding that the text is clean"
    );
    assert!(
        !NO_PII_POLICY_REASON.contains("clean"),
        "and does not claim the text was found clean"
    );
    // -- The one property no cargo gate can see. This constant is joined from
    // wrapped source lines, and a `\`-continuation dropped in the join leaves
    // the continuation's indentation inside the value -- which is what shipped
    // here, twice, as six-space runs in the middle of a sentence an operator
    // reads. `fmt` does not reformat string literals and `clippy` does not read
    // their contents, so the assertion is the only place this can be caught. --
    assert!(
        !NO_PII_POLICY_REASON.contains("  "),
        "no run of two spaces: the operator reads this string, and a doubled \
         space here means a line-continuation's indentation was baked into the \
         literal rather than stripped"
    );
}

/// **`CONTENT_PII_BLOCKED` is one of the sixteen**, and the hook is its single
/// raiser — there is no second construction of the code anywhere in the
/// module.
#[test]
fn the_block_is_the_single_raiser_of_its_code() {
    assert!(TAXONOMY_ERROR_CODES.contains(&ContentPiiBlocked::CODE));
    let source = include_str!("taxonomy.rs");
    assert_eq!(
        source.matches("\"CONTENT_PII_BLOCKED\"").count(),
        2,
        "exactly two: the roster entry and the constant on the refusal. A third \
         would be a second raiser, which `dod-pii-write-block` forbids"
    );
    // -- The blind spot of the count above, closed structurally. A raiser that
    // builds `ContentPiiBlocked` with a struct literal carries no code literal,
    // so it moves the count by nothing and the assertion stays green while the
    // hook's fail-closed-on-uncertainty rule has been bypassed. A private field
    // makes the compiler refuse that construction outside this module; these
    // two assertions are what keep the field private and the construction
    // singular. --
    // Scoped to this struct's own body on purpose: a bare scan for a public
    // `detail` field matches `CategoryReferenced` and `DefinitionInUse` too,
    // which carry one legitimately -- their codes reach the wire through a
    // violation's own `code`, so neither has a single-raiser rule to bypass.
    let body = source
        .split_once("pub struct ContentPiiBlocked {")
        .expect("the refusal is declared in this file")
        .1
        .split_once('}')
        .expect("and its body closes")
        .0;
    assert!(
        !body.contains("pub "),
        "`ContentPiiBlocked` carries no public field: a public one is a second \
         raiser the code-literal count cannot see, a struct literal needing no \
         code literal. Body was: {body}"
    );
    assert_eq!(
        source.matches("ContentPiiBlocked { detail }").count(),
        1,
        "one construction, and it is the hook's"
    );
}

// ── `METADATA_LIMIT`'s judge (P-D-107 arm 1) ─────────────────────────────────
//
// The door does not exist — P-D-106 gave it a route and nothing has built it —
// so these drive `metadata_limit_verdict` directly. That is the whole rule:
// the caps are enforced at the door, and this is what the door will ask.

fn caps() -> MetadataLimits {
    MetadataLimits {
        keys: 3,
        key_bytes: 8,
        value_bytes: 10,
    }
}

fn entry(key: &str, value: &str) -> (String, String) {
    (key.to_owned(), value.to_owned())
}

/// **A map standing at the cap can be reduced** — the rule's half of what
/// `dod-metadata-door` requires a test to prove.
///
/// **And only the rule's half.** The operand is chosen by the caller, not
/// here: this function is handed `resulting_keys` and can only be right about
/// the number it is given. The failure mode the `DoD` is guarding against —
/// counting the payload, or the map *before* the merge, and so refusing the one
/// act that fixes an over-full map — is a **door** defect, and no test of this
/// function can catch it. What is pinned here is the contract the door must
/// satisfy: a resulting count at or under the ceiling is admitted, including
/// when the act writes nothing at all.
///
/// The `DoD`'s own required test therefore stays owed until the door exists,
/// and saying so is better than a green that reads as if it covered both.
#[test]
fn a_map_at_the_cap_can_be_reduced() {
    // Three keys stored, cap three, and the payload deletes two: the write
    // sets nothing and leaves one.
    metadata_limit_verdict(1, &[], caps()).expect("a reduction from the cap is admitted");
    // And the same map merely re-written at the cap is still admitted.
    metadata_limit_verdict(3, &[entry("a", "v")], caps())
        .expect("a write that leaves the map exactly at the cap is admitted");
}

/// One key over the ceiling is refused, and the refusal names the map.
#[test]
fn a_resulting_map_over_the_key_cap_is_refused() {
    let refused = metadata_limit_verdict(4, &[entry("d", "v")], caps())
        .expect_err("four keys against a cap of three");
    assert_eq!(refused.limit, "max_keys");
    assert_eq!(refused.allowed, 3);
    assert_eq!(refused.measured, 4);
    assert_eq!(
        refused.key, None,
        "`max_keys` is about the map, so no key is named"
    );
    assert_eq!(MetadataLimitExceeded::CODE, "METADATA_LIMIT");
}

/// The two byte ceilings are separate operands, and each names its key.
///
/// Asserted apart because one arm covering both passes if the implementation
/// checks the same length twice.
#[test]
fn each_byte_ceiling_is_its_own_refusal() {
    let long_key = metadata_limit_verdict(1, &[entry("nine-char", "v")], caps())
        .expect_err("a nine-byte key against a cap of eight");
    assert_eq!(long_key.limit, "max_key_bytes");
    assert_eq!(long_key.measured, 9);
    assert_eq!(long_key.key.as_deref(), Some("nine-char"));

    let long_value = metadata_limit_verdict(1, &[entry("k", "eleven-char")], caps())
        .expect_err("an eleven-byte value against a cap of ten");
    assert_eq!(long_value.limit, "max_value_bytes");
    assert_eq!(long_value.measured, 11);
    assert_eq!(
        long_value.key.as_deref(),
        Some("k"),
        "a value refusal names the key that carries it, or an operator cannot find it"
    );
}

/// **The count is checked before either length, and lengths in sorted order.**
///
/// Not a precedence claim — `design/02` states none among this feature's
/// sixteen codes and its own §7 records that. It is reproducibility: two
/// writes of the same payload must refuse identically, and a `HashMap`'s
/// iteration order would make the named key arbitrary.
#[test]
fn the_order_of_checks_is_reproducible() {
    // Over on all three at once: the count wins.
    let both = metadata_limit_verdict(9, &[entry("nine-char", "eleven-char")], caps())
        .expect_err("over on every ceiling");
    assert_eq!(both.limit, "max_keys", "the count is judged first");

    // Two keys both over the key ceiling: the sorted-first one is named.
    let sorted = metadata_limit_verdict(
        1,
        &[entry("zebra-key", "v"), entry("alpha-key", "v")],
        caps(),
    )
    .expect_err("two over-long keys");
    assert_eq!(
        sorted.key.as_deref(),
        Some("alpha-key"),
        "sorted order, so the refusal does not depend on the payload's order"
    );

    // And the reverse payload order refuses identically.
    let reversed = metadata_limit_verdict(
        1,
        &[entry("alpha-key", "v"), entry("zebra-key", "v")],
        caps(),
    )
    .expect_err("the same two, the other way round");
    assert_eq!(sorted, reversed, "the same payload refuses the same way");
}

/// Bytes, not characters — a multi-byte value can exceed a ceiling its
/// character count is under.
#[test]
fn the_ceilings_count_bytes() {
    // Four characters, twelve bytes. Escaped because `clippy::non_ascii_literal`
    // is denied here: these are four euro signs, three bytes each.
    let value = "\u{20ac}\u{20ac}\u{20ac}\u{20ac}";
    assert_eq!(value.chars().count(), 4);
    assert_eq!(
        value.len(),
        12,
        "the premise: four characters, twelve bytes"
    );
    let refused = metadata_limit_verdict(1, &[entry("k", value)], caps())
        .expect_err("twelve bytes against a cap of ten");
    assert_eq!(refused.measured, 12, "the ceiling is a byte length");
}
