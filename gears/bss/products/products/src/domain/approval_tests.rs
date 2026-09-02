//! `domain::approval` — the ceremony's rules, each probed on the case whose
//! absence would ship the defect its instruction names.

use std::collections::BTreeSet;

use uuid::Uuid;

use super::{
    AckPlacement, ApproverDiff, ApproverRole, ApproverScopeVerdict, CastDecision,
    DEFAULT_APPROVER_COUNT, PLATFORM_QUORUM_FLOOR, QuorumOutcome, UnsatisfiablePredicate,
    ack_placement, approver_covers_subject, decision_admitted, describe_platform_quorum,
    describe_quorum, diff_basis_for, evaluate_quorum, render_diff,
};
use crate::domain::containment::{ResolvedScope, ScopeDimension, ScopePair};
use crate::domain::materiality::Materiality;

/// A restricted scope from its members, so a probe reads as the set it means.
fn restricted(values: &[&str]) -> ResolvedScope {
    ResolvedScope::Restricted(
        values
            .iter()
            .map(|v| (*v).to_owned())
            .collect::<BTreeSet<_>>(),
    )
}

/// A scope pair, region then brand.
fn pair(region: ResolvedScope, brand: ResolvedScope) -> ScopePair {
    ScopePair { region, brand }
}

/// One approval from `principal` holding `roles`.
fn approves(principal: u128, roles: &[ApproverRole]) -> CastDecision {
    CastDecision {
        principal: Uuid::from_u128(principal),
        approved: true,
        roles: roles.to_vec(),
    }
}

/// **`required = N` for a material change and `min(N, 1)` for a
/// non-material one** — `inst-gv-materiality`'s two arms, over the whole
/// interesting range of `N`.
#[test]
fn required_is_the_effective_count_on_both_arms() {
    for (n, material, non_material) in [(0_u32, 0_u32, 0_u32), (1, 1, 1), (2, 2, 1), (5, 5, 1)] {
        assert_eq!(
            describe_quorum(Materiality::Material, n, false).required(),
            material,
            "material at N = {n}"
        );
        assert_eq!(
            describe_quorum(Materiality::NonMaterial, n, false).required(),
            non_material,
            "non-material at N = {n}"
        );
    }
}

/// **The raw `N` survives beside the effective count.** `inst-gv-queue`
/// forbids the raw value standing in for `required`, and the only way a
/// surface can honour that is by having both.
#[test]
fn the_descriptor_carries_both_counts_apart() {
    let d = describe_quorum(Materiality::NonMaterial, 5, false);
    assert_eq!(d.required(), 1);
    assert_eq!(
        d.configured_quorum(),
        5,
        "the raw N is kept, not overwritten"
    );
}

/// **`quorumReduced` fires below the retained default of two** (P-D-13) —
/// including on a *material* change at `N = 1`, which is the case a
/// "reduced means non-material" shortcut would miss.
#[test]
fn quorum_reduced_tracks_the_effective_count_not_the_verdict() {
    assert!(describe_quorum(Materiality::Material, 1, false).quorum_reduced());
    assert!(describe_quorum(Materiality::Material, 0, false).quorum_reduced());
    assert!(
        describe_quorum(Materiality::NonMaterial, 5, false).quorum_reduced(),
        "a non-material change at N = 5 closes on one, and one is below two"
    );
    assert!(
        !describe_quorum(Materiality::Material, 2, false).quorum_reduced(),
        "the default itself is not reduced"
    );
}

/// **At `N = 0` the finance predicate is recorded absent rather than set.**
/// Without this arm the descriptor demands a role no principal could hold
/// and the gate raises `APPROVAL_REQUIRED` forever, re-blocking the tenant
/// P-D-11 unblocked.
#[test]
fn the_finance_predicate_is_unsatisfiable_at_zero_and_set_above_it() {
    let zero = describe_quorum(Materiality::Material, 0, true);
    assert!(!zero.finance_required(), "no approver can hold the role");
    assert_eq!(
        zero.predicate_unsatisfiable(),
        Some(UnsatisfiablePredicate::FinanceReviewer),
        "the control's absence is a stored fact, not an inference"
    );

    let one = describe_quorum(Materiality::Material, 1, true);
    assert!(
        one.finance_required(),
        "a lone approver must be a FinanceReviewer"
    );
    assert_eq!(one.predicate_unsatisfiable(), None);
}

/// A non-material finance-material change at `N = 3` closes on **one**
/// approver, and that approver must still be a `FinanceReviewer` — so the
/// predicate keys off the effective count, not the raw `N`.
#[test]
fn the_predicate_keys_off_the_effective_count() {
    let d = describe_quorum(Materiality::NonMaterial, 3, true);
    assert_eq!(d.required(), 1);
    assert!(d.finance_required());
    assert_eq!(d.predicate_unsatisfiable(), None);
}

/// Nothing finance-material sets nothing.
#[test]
fn an_ordinary_change_carries_no_predicate() {
    let d = describe_quorum(Materiality::Material, 2, false);
    assert!(!d.finance_required());
    assert_eq!(d.predicate_unsatisfiable(), None);
}

/// The stored rendering carries §4's five names and is stable — the column
/// is compared byte-for-byte, so key order cannot be incidental.
#[test]
fn the_stored_descriptor_is_canonical_and_names_all_five_fields() {
    let d = describe_quorum(Materiality::NonMaterial, 3, true);
    let stored = d.stored();
    // **The full literal, not `contains`.** `contains` passes for any
    // ordering, and comparing a pure function's output to itself asserts
    // nothing — so swapping the canonical renderer for `serde_json` (which
    // under `preserve_order` emits insertion order) would pass both. The
    // column is compared byte-for-byte, so the bytes are the assertion.
    assert_eq!(
        stored,
        r#"{"configuredQuorum":3,"financeRequired":true,"predicateUnsatisfiable":null,"quorumReduced":true,"required":1}"#,
        "sorted keys, an explicit null, and the five names section 4 gives"
    );
}

/// **The author's own decision is refused at every `N >= 1`.** The whole
/// interesting range, because a guard written for the default only would let
/// a lone author approve themselves at `N = 1`.
#[test]
fn the_author_is_refused_at_every_n_of_one_or_more() {
    let author = Uuid::from_u128(1);
    for n in [1_u32, 2, 5] {
        let d = describe_quorum(Materiality::Material, n, false);
        let err = decision_admitted(author, author, &d)
            .expect_err("an author may never decide their own record");
        assert_eq!(err.code(), "SELF_APPROVAL_FORBIDDEN", "at N = {n}");
    }
}

/// **The paired positive control**: a different principal is admitted on the
/// same record, so the refusal above cannot be passing because every
/// decision is refused.
#[test]
fn a_different_principal_is_admitted() {
    let author = Uuid::from_u128(1);
    let approver = Uuid::from_u128(2);
    for n in [1_u32, 2, 5] {
        let d = describe_quorum(Materiality::Material, n, false);
        decision_admitted(author, approver, &d)
            .unwrap_or_else(|e| panic!("a distinct principal is admitted at N = {n}: {e}"));
    }
}

/// At `N = 0` there is no decision to refuse: the record closes with no
/// approver, so the guard has no `>= 1` to bite on and the author's
/// acknowledgment lives on the record instead.
#[test]
fn at_zero_there_is_no_decision_to_refuse() {
    let author = Uuid::from_u128(1);
    let d = describe_quorum(Materiality::NonMaterial, 0, false);
    assert_eq!(d.required(), 0);
    decision_admitted(author, author, &d)
        .expect("at N = 0 no decision row exists, so nothing is being self-approved");
    assert_eq!(ack_placement(&d), AckPlacement::OnRecord);
}

/// Above zero the acknowledgment rides the decision row — a synthetic
/// decision row naming the author would break C2's UNIQUE, which is why the
/// two homes are distinguished rather than merged.
#[test]
fn above_zero_the_acknowledgment_rides_the_decision() {
    for n in [1_u32, 2] {
        let d = describe_quorum(Materiality::Material, n, false);
        assert_eq!(ack_placement(&d), AckPlacement::OnDecision, "at N = {n}");
    }
}

/// **A first publish pins no basis.** The arm is explicit because filling
/// the gap by convention would diff the draft against the head, which is the
/// re-derivation the rule forbids.
#[test]
fn a_first_publish_pins_no_diff_basis() {
    assert_eq!(diff_basis_for(None), None);
    assert_eq!(diff_basis_for(Some(3)), Some(3));
}

/// The renderer's half of `dod-stored-snapshot`: what
/// [`render_diff`] does with the snapshot it is handed.
///
/// **This is not the flagship probe, and calling it one was a defect in this
/// comment.** §5's flagship is stateful — *"submit, edit the head, and the
/// **superseded record's** diff still renders the original submission against
/// the published version"* — and there is no head, no store and no
/// supersession here. What this case does prove is real and narrow: a
/// renderer that stopped reading its snapshot argument fails it. What it
/// cannot see is a **store** that failed to preserve the submitted bytes, or
/// a **caller** that handed the live head over in the snapshot's place; both
/// need a record read back after a head edit, which is
/// `infra::storage::repo::governance::governance_tests::the_superseded_records_diff_renders_the_submission_not_the_edited_head`.
/// The split was measured by perturbing each half in turn.
///
/// The `edited_head` local below is passed to nothing, so the third
/// assertion — that its bytes are absent from the diff — compares two
/// literals this test wrote and cannot fail. It is kept as the statement of
/// intent it is, with the load-bearing version in the store probe.
#[test]
fn the_diff_renders_the_stored_submission_not_the_edited_head() {
    let submitted = r#"{"name":"as submitted"}"#;
    let published = r#"{"name":"as published"}"#;
    // The head moves after submission — this is the edit the pricing defect
    // silently showed the approver.
    let edited_head = r#"{"name":"edited after submission"}"#;

    let diff = render_diff(submitted, diff_basis_for(Some(7)), Some(published));
    match diff {
        ApproverDiff::Against {
            basis,
            submitted: shown,
            basis_content,
        } => {
            assert_eq!(basis, 7);
            assert_eq!(shown, submitted, "the approver sees what was submitted");
            assert_eq!(basis_content, published);
            assert!(
                !shown.contains("edited after submission"),
                "the edited head reached the diff: {edited_head}"
            );
        }
        other => panic!("expected a diff against the published version, got {other:?}"),
    }
}

/// The first-publish rendering is the whole submission, against no basis.
#[test]
fn a_first_publish_renders_a_whole_content_addition() {
    let submitted = r#"{"name":"first"}"#;
    let diff = render_diff(submitted, diff_basis_for(None), None);
    assert_eq!(
        diff,
        ApproverDiff::WholeContentAddition {
            submitted: submitted.to_owned()
        }
    );
}

/// **A pinned basis whose content could not be read is NOT a first
/// publish.** Its own arm, because collapsing the two shows an approver a
/// whole-content addition for a change that has a predecessor — they approve
/// a diff they were never shown.
#[test]
fn an_unreadable_basis_is_its_own_answer_not_a_first_publish() {
    let submitted = r#"{"name":"x"}"#;
    match render_diff(submitted, Some(4), None) {
        ApproverDiff::BasisUnreadable {
            basis,
            submitted: s,
        } => {
            assert_eq!(basis, 4, "the basis is named, not discarded");
            assert_eq!(s, submitted);
        }
        other => panic!("a pinned basis must not render as a first publish: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// The platform floor (P-D-13), the evaluator (inst-gv-quorum) and the
// approver-scope rule (inst-gv-scope).
// ---------------------------------------------------------------------------

/// **The writer §7 row 9 says does not exist.** A fixed 2 that no tenant
/// configuration reaches.
///
/// Armed at `N = 0` on the other side: `describe_quorum` at zero answers a
/// descriptor that closes on nobody, and the floor answers 2 for the same
/// tenant. A probe at `N = 2` could not tell a fixed floor from a
/// configurable one, which is why the comparison is drawn at zero.
#[test]
fn the_platform_floor_is_fixed_where_the_tenant_count_is_not() {
    let floor = describe_platform_quorum();
    assert_eq!(floor.required(), PLATFORM_QUORUM_FLOOR);
    assert_eq!(
        floor.required(),
        2,
        "P-D-13's two distinct platform principals"
    );

    // The same tenant, at the floor P-D-11 made reachable.
    let tenant_at_zero = describe_quorum(Materiality::Material, 0, false);
    assert_eq!(
        tenant_at_zero.required(),
        0,
        "P-D-11: a tenant at N = 0 publishes approver-less by policy"
    );
    assert_eq!(
        describe_platform_quorum().required(),
        PLATFORM_QUORUM_FLOOR,
        "and no tenant configuration reaches the platform ceremony: that is the whole of \
         P-D-13's 'no tenant's configured N has standing'"
    );
}

/// `quorumReduced` is **false** at the platform floor, and true is what a
/// tenant ceremony below the default gets.
///
/// P-D-13 sets the marker *"when the effective count is below the
/// retained-name default of 2"*. The floor is exactly that default, so the
/// trail that says "two-person" is telling the truth and the marker is
/// correctly silent — which is only meaningful next to a case where it fires.
#[test]
fn the_floor_is_not_a_reduced_quorum_and_a_one_person_tenant_is() {
    assert!(
        !describe_platform_quorum().quorum_reduced(),
        "the floor IS the retained-name default, so nothing was reduced"
    );
    assert_eq!(
        PLATFORM_QUORUM_FLOOR, DEFAULT_APPROVER_COUNT,
        "same value, two facts"
    );
    assert!(
        describe_quorum(Materiality::Material, 1, false).quorum_reduced(),
        "a tenant ceremony at N = 1 is below the default and must say so"
    );
}

/// The floor carries no finance predicate and records none unsatisfiable —
/// "not applicable", not "could not be carried".
#[test]
fn the_floor_carries_no_finance_predicate_at_all() {
    let floor = describe_platform_quorum();
    assert!(!floor.finance_required());
    assert_eq!(
        floor.predicate_unsatisfiable(),
        None,
        "an elevation touches no finance-material field, so the predicate has no subject to \
         be absent from, which is distinct from the N = 0 case that records the absence"
    );
    assert_eq!(
        describe_quorum(Materiality::Material, 0, true).predicate_unsatisfiable(),
        Some(UnsatisfiablePredicate::FinanceReviewer),
        "the paired case that makes the assertion above mean something"
    );
}

/// **One human holding both roles counts once** — C2's floor, at the
/// evaluator rather than at the index, and the probe `dod-quorum-evaluator`
/// names.
///
/// The same principal appears twice, once under each role, which is exactly
/// the shape the index would refuse and a caller assembling the list itself
/// would not.
#[test]
fn one_human_holding_both_roles_counts_once() {
    let descriptor = describe_quorum(Materiality::Material, 2, false);
    let both = [
        approves(0xa1, &[ApproverRole::CatalogAdmin]),
        approves(0xa1, &[ApproverRole::FinanceReviewer]),
    ];
    assert_eq!(
        evaluate_quorum(
            &descriptor,
            &both,
            &[ApproverRole::CatalogAdmin, ApproverRole::FinanceReviewer]
        ),
        QuorumOutcome::CountUnmet {
            counted: 1,
            required: 2
        },
        "two rows, one principal, one approver (C2)"
    );

    // The positive control: a second, different principal closes it.
    let two = [
        approves(0xa1, &[ApproverRole::CatalogAdmin]),
        approves(0xa2, &[ApproverRole::CatalogAdmin]),
    ];
    assert_eq!(
        evaluate_quorum(
            &descriptor,
            &two,
            &[ApproverRole::CatalogAdmin, ApproverRole::FinanceReviewer]
        ),
        QuorumOutcome::Satisfied
    );
}

/// **The finance predicate is satisfiable by that one principal only as one
/// of the two** (`design/05` §5's second bullet).
///
/// The dual-role human supplies the lens; the count still needs a second
/// body. Both halves are asserted, because a rule that let the lens stand in
/// for the body would pass the first assertion alone.
#[test]
fn the_dual_role_human_supplies_the_lens_but_not_the_second_body() {
    let descriptor = describe_quorum(Materiality::Material, 2, true);
    assert!(descriptor.finance_required());

    let lens_only = [approves(
        0xb1,
        &[ApproverRole::CatalogAdmin, ApproverRole::FinanceReviewer],
    )];
    assert_eq!(
        evaluate_quorum(&descriptor, &lens_only, &[ApproverRole::CatalogAdmin]),
        QuorumOutcome::CountUnmet {
            counted: 1,
            required: 2
        },
        "holding both roles does not buy the second signature"
    );

    let lens_and_body = [
        approves(
            0xb1,
            &[ApproverRole::CatalogAdmin, ApproverRole::FinanceReviewer],
        ),
        approves(0xb2, &[ApproverRole::CatalogAdmin]),
    ];
    assert_eq!(
        evaluate_quorum(&descriptor, &lens_and_body, &[ApproverRole::CatalogAdmin]),
        QuorumOutcome::Satisfied,
        "one of the two being the FinanceReviewer is what the predicate asks"
    );
}

/// **Numerically met with the predicate unmet is its own answer** — L-2's
/// `APPROVER_ROLE_REQUIRED` case, distinguished from a short count.
#[test]
fn a_met_count_with_no_finance_lens_is_the_role_refusal_not_a_short_count() {
    let descriptor = describe_quorum(Materiality::Material, 2, true);
    let no_lens = [
        approves(0xc1, &[ApproverRole::CatalogAdmin]),
        approves(0xc2, &[ApproverRole::CatalogAdmin]),
    ];
    assert_eq!(
        evaluate_quorum(&descriptor, &no_lens, &[ApproverRole::CatalogAdmin]),
        QuorumOutcome::RolePredicateUnmet { counted: 2 },
        "the count is met and the lens is missing: a caller told 'not enough approvers' would \
         add a third CatalogAdmin and fail again"
    );
}

/// **A recorded `predicateUnsatisfiable` counts as met, and only at the count
/// that records it.**
///
/// Armed at `N = 0` per the rule that a probe at `N = 2` cannot distinguish
/// the arms: at zero the marker is set and no decision exists, and the
/// descriptor is met by nobody. At `N >= 1` the marker is never recorded, so
/// the discharge `inst-gv-quorum` forbids there is unreachable rather than
/// merely unused — which is what the second half asserts.
#[test]
fn an_unsatisfiable_predicate_is_met_at_zero_and_unreachable_above_it() {
    let at_zero = describe_quorum(Materiality::Material, 0, true);
    assert_eq!(
        at_zero.predicate_unsatisfiable(),
        Some(UnsatisfiablePredicate::FinanceReviewer)
    );
    assert!(
        !at_zero.finance_required(),
        "the predicate is not SET at zero"
    );
    assert_eq!(
        evaluate_quorum(&at_zero, &[], &[ApproverRole::CatalogAdmin]),
        QuorumOutcome::Satisfied,
        "the marker is the only discharge, and the descriptor closes on nobody"
    );

    // Above zero the marker cannot be recorded at all, so no evaluation can
    // reach the discharge.
    for n in 1_u32..=4 {
        let above = describe_quorum(Materiality::Material, n, true);
        assert_eq!(
            above.predicate_unsatisfiable(),
            None,
            "at N = {n} the predicate binds and is never recorded unsatisfiable"
        );
        assert!(above.finance_required(), "at N = {n} the predicate is SET");
    }
}

/// An approver holding none of the binding roles is not counted, and an empty
/// binding set counts anyone.
///
/// The second half is §7 row 16 left open rather than answered: the caller
/// names the set that binds, so a non-material change whose base set is
/// undecided is expressed by passing none, not by this function guessing.
#[test]
fn an_ineligible_approver_is_not_counted_and_an_empty_base_set_counts_anyone() {
    let descriptor = describe_quorum(Materiality::Material, 1, false);
    let ineligible = [approves(0xd1, &[])];
    assert_eq!(
        evaluate_quorum(&descriptor, &ineligible, &[ApproverRole::CatalogAdmin]),
        QuorumOutcome::CountUnmet {
            counted: 0,
            required: 1
        },
        "holding no named role is not an eligible approver, so it moves the count by nothing"
    );
    assert_eq!(
        evaluate_quorum(&descriptor, &ineligible, &[]),
        QuorumOutcome::Satisfied,
        "an empty base set is 'any holder of approval x decide', row 16's other reading, \
         expressed by the caller rather than chosen here"
    );
}

/// A rejection never counts toward satisfaction.
#[test]
fn a_rejection_moves_no_count() {
    let descriptor = describe_quorum(Materiality::Material, 1, false);
    let rejected = [CastDecision {
        principal: Uuid::from_u128(0xe1),
        approved: false,
        roles: vec![ApproverRole::CatalogAdmin],
    }];
    assert_eq!(
        evaluate_quorum(&descriptor, &rejected, &[ApproverRole::CatalogAdmin]),
        QuorumOutcome::CountUnmet {
            counted: 0,
            required: 1
        }
    );
}

/// **The claim set is the parent and the subject is the child.** All three of
/// P-D-39's clauses, in the direction `inst-gv-scope` words them.
///
/// The transposed mapping is the defect this case exists to catch, and clause
/// 2 is the only asymmetric one — so it is the assertion that would flip:
/// under a transposition a restricted approver would cover an unrestricted
/// subject.
#[test]
fn an_unrestricted_claim_set_covers_everything_and_the_reverse_does_not() {
    // Clause 1: unrestricted claims cover every subject.
    assert_eq!(
        approver_covers_subject(
            &pair(ResolvedScope::Unrestricted, ResolvedScope::Unrestricted),
            &pair(restricted(&["eu"]), restricted(&["acme"])),
        ),
        ApproverScopeVerdict::Covered
    );

    // Clause 2, the asymmetric one: an unrestricted SUBJECT is covered only
    // by an unrestricted claim set. Transpose the mapping and this passes.
    match approver_covers_subject(
        &pair(restricted(&["eu"]), ResolvedScope::Unrestricted),
        &pair(ResolvedScope::Unrestricted, ResolvedScope::Unrestricted),
    ) {
        ApproverScopeVerdict::Exceeded {
            dimension,
            claimed,
            subject,
        } => {
            assert_eq!(dimension, ScopeDimension::Region);
            assert_eq!(claimed, restricted(&["eu"]), "the claims are the parent");
            assert_eq!(
                subject,
                ResolvedScope::Unrestricted,
                "the subject is the child, and an unrestricted child needs an unrestricted \
                 parent"
            );
        }
        ApproverScopeVerdict::Covered => panic!(
            "a region-restricted approver covered a tenant-wide subject: the claim set and the \
             subject scope are transposed"
        ),
    }

    // Clause 3: ordinary subset between two non-empty sets, both ways.
    assert_eq!(
        approver_covers_subject(
            &pair(restricted(&["eu", "apac"]), restricted(&["acme"])),
            &pair(restricted(&["eu"]), restricted(&["acme"])),
        ),
        ApproverScopeVerdict::Covered,
        "the in-scope control"
    );
    assert!(matches!(
        approver_covers_subject(
            &pair(restricted(&["eu"]), restricted(&["acme"])),
            &pair(restricted(&["eu", "us"]), restricted(&["acme"])),
        ),
        ApproverScopeVerdict::Exceeded {
            dimension: ScopeDimension::Region,
            ..
        }
    ));
}

/// The brand dimension is judged independently, and the verdict names which
/// one failed.
///
/// Without this case a rule that checked region twice would pass every
/// assertion above.
#[test]
fn the_brand_dimension_is_judged_on_its_own_and_is_named() {
    assert!(matches!(
        approver_covers_subject(
            &pair(ResolvedScope::Unrestricted, restricted(&["acme"])),
            &pair(restricted(&["eu"]), restricted(&["globex"])),
        ),
        ApproverScopeVerdict::Exceeded {
            dimension: ScopeDimension::Brand,
            ..
        }
    ));
}
