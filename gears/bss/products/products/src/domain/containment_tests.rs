//! Coverage for the containment rule's three clauses and the three payload
//! input states, and for the interaction between them a bare pass/fail count
//! would hide.

use std::collections::BTreeSet;

use super::{
    EmptyScopeToken, ResolvedScope, ScopeContainment, ScopeDimension, ScopeInput, ScopePair,
    contains,
};

/// Build a `BTreeSet<String>` from string literals, for readable test bodies.
fn set(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| String::from(*value)).collect()
}

#[test]
fn clause_1_an_unrestricted_parent_contains_a_restricted_child() {
    // Clause 1: nothing about the child's own set matters once the parent
    // names no restriction at all.
    let parent = ResolvedScope::Unrestricted;
    let child = ResolvedScope::Restricted(set(&["eu"]));
    assert_eq!(
        contains(ScopeDimension::Region, &parent, &child),
        ScopeContainment::Contained
    );
}

#[test]
fn clause_1_an_unrestricted_parent_contains_an_unrestricted_child() {
    // Still clause 1 (the parent side is what decides it here), but exercised
    // with both sides unrestricted so clause 2's "only" is not mistaken for
    // ruling this case out too.
    let parent = ResolvedScope::Unrestricted;
    let child = ResolvedScope::Unrestricted;
    assert_eq!(
        contains(ScopeDimension::Region, &parent, &child),
        ScopeContainment::Contained
    );
}

#[test]
fn clause_2_a_restricted_parent_does_not_contain_an_unrestricted_child() {
    // Clause 2, the one that reads backwards and is the point of the whole
    // rule: an unrestricted child reaches everywhere, so a parent that does
    // not cannot contain it, even though the child names no set for a raw
    // subset check to reject. A suite that omits this case has not tested
    // the rule at all.
    let parent = ResolvedScope::Restricted(set(&["eu"]));
    let child = ResolvedScope::Unrestricted;
    assert_eq!(
        contains(ScopeDimension::Brand, &parent, &child),
        ScopeContainment::NotContained {
            dimension: ScopeDimension::Brand,
            parent: ResolvedScope::Restricted(set(&["eu"])),
            child: ResolvedScope::Unrestricted,
        }
    );
}

#[test]
fn clause_3_a_restricted_parent_contains_a_proper_subset_child() {
    // Ordinary subset, the strict case: the child names fewer values than
    // the parent.
    let parent = ResolvedScope::Restricted(set(&["eu", "apac"]));
    let child = ResolvedScope::Restricted(set(&["eu"]));
    assert_eq!(
        contains(ScopeDimension::Region, &parent, &child),
        ScopeContainment::Contained
    );
}

#[test]
fn clause_3_a_restricted_parent_contains_a_child_with_the_identical_set() {
    // Ordinary subset, the equal case: subset includes equality, and a
    // containment rule that quietly required a *proper* subset would refuse
    // a child cloned verbatim from its parent's own scope.
    let parent = ResolvedScope::Restricted(set(&["eu", "apac"]));
    let child = ResolvedScope::Restricted(set(&["eu", "apac"]));
    assert_eq!(
        contains(ScopeDimension::Region, &parent, &child),
        ScopeContainment::Contained
    );
}

#[test]
fn clause_3_a_restricted_parent_does_not_contain_a_child_naming_a_value_it_lacks() {
    // Ordinary subset, the refusal case: the child asks for a value the
    // parent never granted.
    let parent = ResolvedScope::Restricted(set(&["eu"]));
    let child = ResolvedScope::Restricted(set(&["eu", "apac"]));
    assert_eq!(
        contains(ScopeDimension::Region, &parent, &child),
        ScopeContainment::NotContained {
            dimension: ScopeDimension::Region,
            parent: ResolvedScope::Restricted(set(&["eu"])),
            child: ResolvedScope::Restricted(set(&["eu", "apac"])),
        }
    );
}

#[test]
fn an_omitted_child_input_inherits_an_unrestricted_parent_exactly() {
    // "Omitted" must resolve to the parent's own value, not to some default —
    // here the parent happens to be unrestricted, so the child must come out
    // unrestricted too, not restricted-to-nothing.
    let parent = ResolvedScope::Unrestricted;
    let resolved = ScopeInput::Omitted.resolve(&parent);
    assert_eq!(resolved, ResolvedScope::Unrestricted);
}

#[test]
fn an_omitted_child_input_inherits_a_restricted_parents_set_verbatim() {
    // Same inheritance rule, exercised against a restricted parent: the
    // child's resolved set must equal the parent's, value for value.
    let parent = ResolvedScope::Restricted(set(&["eu", "apac"]));
    let resolved = ScopeInput::Omitted.resolve(&parent);
    assert_eq!(resolved, ResolvedScope::Restricted(set(&["eu", "apac"])));
}

#[test]
fn an_omitted_input_and_an_explicitly_unrestricted_input_reach_opposite_outcomes_against_the_same_restricted_parent()
 {
    // The load-bearing case: two payload states that could easily be
    // collapsed into "no set was given" are run against the identical
    // restricted parent and must land on opposite sides of the containment
    // check. "Omitted" inherits the parent's own restriction and is
    // contained by construction; "explicitly unrestricted" asks to reach
    // everywhere and clause 2 refuses it. Conflating the two states — in
    // either direction — makes one of these two assertions false.
    let parent_resolved = ResolvedScope::Restricted(set(&["eu"]));

    let omitted_child = ScopeInput::Omitted.resolve(&parent_resolved);
    assert_eq!(
        contains(ScopeDimension::Region, &parent_resolved, &omitted_child),
        ScopeContainment::Contained,
        "an omitted set must inherit the parent's and therefore be contained"
    );

    let unrestricted_child = ScopeInput::Unrestricted.resolve(&parent_resolved);
    assert_eq!(
        contains(
            ScopeDimension::Region,
            &parent_resolved,
            &unrestricted_child
        ),
        ScopeContainment::NotContained {
            dimension: ScopeDimension::Region,
            parent: ResolvedScope::Restricted(set(&["eu"])),
            child: ResolvedScope::Unrestricted,
        },
        "an explicitly unrestricted set must not be silently narrowed to the parent's"
    );
}

#[test]
fn both_dimensions_are_decided_independently_and_the_failure_names_the_offending_one() {
    // A child can be contained on one dimension and refused on the other;
    // the combined check must not report success just because one of the two
    // passed, and its failure must name the dimension that actually failed
    // rather than the one that happened to be checked first.
    let parent = ScopePair {
        region: ResolvedScope::Restricted(set(&["eu"])),
        brand: ResolvedScope::Restricted(set(&["acme"])),
    };
    // Contained on region (subset), refused on brand (unrestricted child
    // against a restricted parent, clause 2).
    let child = ScopePair {
        region: ResolvedScope::Restricted(set(&["eu"])),
        brand: ResolvedScope::Unrestricted,
    };

    let verdict = parent.check_containment(&child);
    assert_eq!(
        verdict,
        Err(ScopeContainment::NotContained {
            dimension: ScopeDimension::Brand,
            parent: ResolvedScope::Restricted(set(&["acme"])),
            child: ResolvedScope::Unrestricted,
        })
    );
}

#[test]
fn a_child_contained_on_both_dimensions_passes_the_combined_check() {
    // The passing counterpart to the previous test: both dimensions must
    // independently agree before the combined check reports success.
    let parent = ScopePair {
        region: ResolvedScope::Restricted(set(&["eu", "apac"])),
        brand: ResolvedScope::Unrestricted,
    };
    let child = ScopePair {
        region: ResolvedScope::Restricted(set(&["eu"])),
        brand: ResolvedScope::Restricted(set(&["acme"])),
    };

    assert_eq!(parent.check_containment(&child), Ok(()));
}

#[test]
fn the_stored_form_round_trips_through_parse_and_render() {
    // The storage boundary this module owns: a resolved scope rendered to
    // the column's string form and parsed back must be the value it started
    // as, for both the unrestricted and the restricted case.
    assert_eq!(ResolvedScope::Unrestricted.render(), "");
    assert_eq!(ResolvedScope::parse(""), Ok(ResolvedScope::Unrestricted));

    let restricted = ResolvedScope::Restricted(set(&["apac", "eu"]));
    let rendered = restricted.render();
    assert_eq!(ResolvedScope::parse(&rendered), Ok(restricted));
}

/// F-5: a leading separator (`",eu"`) produces an empty first token and is
/// rejected, not silently read as `{"eu"}`.
#[test]
fn parse_rejects_a_leading_separator() {
    assert_eq!(ResolvedScope::parse(",eu"), Err(EmptyScopeToken));
}

/// F-5: a trailing separator (`"eu,"`) produces an empty last token and is
/// rejected, not silently read as `{"eu"}`.
#[test]
fn parse_rejects_a_trailing_separator() {
    assert_eq!(ResolvedScope::parse("eu,"), Err(EmptyScopeToken));
}

/// F-5: a doubled separator (`"eu,,us"`) produces an empty token between the
/// two real ones and is rejected, not silently read as `{"eu", "us"}`.
#[test]
fn parse_rejects_a_doubled_separator() {
    assert_eq!(ResolvedScope::parse("eu,,us"), Err(EmptyScopeToken));
}

/// F-5: a bare separator (`","`) is two empty tokens and neither `Unrestricted`
/// (that reading is reserved for the literal empty string) nor a
/// `Restricted` set containing the empty string.
#[test]
fn parse_rejects_a_bare_separator() {
    assert_eq!(ResolvedScope::parse(","), Err(EmptyScopeToken));
}
