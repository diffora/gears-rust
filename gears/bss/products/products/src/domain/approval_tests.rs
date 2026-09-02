//! `domain::approval` — the ceremony's rules, each probed on the case whose
//! absence would ship the defect its instruction names.

use uuid::Uuid;

use super::{
    AckPlacement, ApproverDiff, UnsatisfiablePredicate, ack_placement, decision_admitted,
    describe_quorum, diff_basis_for, render_diff,
};
use crate::domain::materiality::Materiality;

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

/// **The flagship probe**: submit, edit the head, and the diff still renders
/// the ORIGINAL submission against the published version.
///
/// The head's later content is passed to nothing — [`render_diff`] takes the
/// stored snapshot and the basis content, and there is no third argument a
/// re-derivation could arrive through. So the assertion is that the edited
/// head's bytes appear nowhere in the rendered diff.
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
