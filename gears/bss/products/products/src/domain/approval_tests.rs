//! `domain::approval` — the ceremony's rules, each probed on the case whose
//! absence would ship the defect its instruction names.

use std::collections::BTreeSet;

use uuid::Uuid;

use super::{
    AckPlacement, ApproverDiff, ApproverRole, ApproverScopeVerdict, BaseRoleSet, CastDecision,
    DEFAULT_APPROVER_COUNT, PLATFORM_QUORUM_FLOOR, QuorumOutcome, UnsatisfiablePredicate,
    ack_placement, approver_covers_subject, decision_admitted, describe_platform_quorum,
    describe_quorum, descriptor_from_stored, diff_basis_for, evaluate_quorum, render_diff,
};
use crate::domain::concurrency::InternalRevision;
use crate::domain::containment::{ResolvedScope, ScopeDimension, ScopePair};
use bss_products_sdk::models::EntityKind;

use crate::domain::governance::{
    ApprovalDisposition, ApprovalId, EntityRef, GateMode, GateSubject, GateVerdict,
    GovernanceGate as _, SubjectKind,
};
use crate::domain::materiality::Materiality;

use super::{ApprovalState, CandidateApproval, StoredApprovalGate};

const GATE_TENANT: Uuid = Uuid::from_u128(0x6a_11);
const GATE_ENTITY: Uuid = Uuid::from_u128(0x6a_e1);

/// The subject every host probe below asks about — the same shape all seven
/// production call sites build.
fn gate_subject() -> GateSubject {
    GateSubject::entity_publish(EntityRef {
        tenant_id: GATE_TENANT,
        entity_kind: EntityKind::Product,
        entity_id: GATE_ENTITY,
    })
}

/// One candidate on [`gate_subject`] at `revision` in `state`.
fn candidate(id: u128, revision: i64, state: ApprovalState) -> CandidateApproval {
    CandidateApproval {
        approval_id: ApprovalId::new(Uuid::from_u128(id)),
        subject: gate_subject(),
        internal_revision: revision,
        state,
        override_acknowledged: false,
    }
}

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

/// The author of every record these probes evaluate — distinct from every
/// approver, because C1's approvers are *"each distinct from the author"* and
/// a fixture reusing one id could not tell the two rules apart.
const SUBMITTER: Uuid = Uuid::from_u128(0x5b_11);

/// C1's own base set, which is what binds a material change.
const C1: BaseRoleSet = BaseRoleSet::CatalogAdminOrFinanceReviewer;

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
/// An earlier revision also asserted that an `edited_head` local's bytes were
/// absent from the diff. That local was passed to nothing, so the assertion
/// compared two literals this test wrote and **could not fail**; it is
/// removed rather than kept as a statement of intent, because a
/// non-falsifiable assertion in a probe is indistinguishable from a passing
/// one. The claim it reached for lives in the store probe, where a head
/// really moves. (An earlier revision of this comment also called the body's
/// assertions three; there were four.)
#[test]
fn the_diff_renders_the_snapshot_it_was_handed() {
    let submitted = r#"{"name":"as submitted"}"#;
    let published = r#"{"name":"as published"}"#;

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
    // No third assertion. An earlier revision repeated the first one after
    // the `describe_quorum` call, under a message about tenant configuration
    // having no standing — which that comparison cannot measure, both being
    // pure functions over separate values. The standing is a property of the
    // signature: `describe_platform_quorum` takes no tenant operand at all.
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
        evaluate_quorum(&descriptor, SUBMITTER, &both, C1),
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
        evaluate_quorum(&descriptor, SUBMITTER, &two, C1),
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
        evaluate_quorum(&descriptor, SUBMITTER, &lens_only, C1),
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
        evaluate_quorum(&descriptor, SUBMITTER, &lens_and_body, C1),
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
        evaluate_quorum(&descriptor, SUBMITTER, &no_lens, C1),
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
        evaluate_quorum(&at_zero, SUBMITTER, &[], C1),
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

/// An approver holding none of C1's roles is not counted, and the permissive
/// reading has to be **named**.
///
/// §7 row 16 is still left open rather than answered — but it is now named
/// rather than defaulted, which is the fix. While the operand was a slice the
/// empty one meant "anyone counts", and the empty slice is the only value a
/// caller can supply today (§7 row 25), so a material change closed on two
/// principals holding neither C1 role.
#[test]
fn an_ineligible_approver_is_not_counted_and_any_decider_must_be_named() {
    let descriptor = describe_quorum(Materiality::Material, 1, false);
    let ineligible = [approves(0xd1, &[])];
    assert_eq!(
        evaluate_quorum(&descriptor, SUBMITTER, &ineligible, C1),
        QuorumOutcome::CountUnmet {
            counted: 0,
            required: 1
        },
        "holding no named role is not an eligible approver, so it moves the count by nothing"
    );
    assert_eq!(
        evaluate_quorum(&descriptor, SUBMITTER, &ineligible, BaseRoleSet::AnyDecider),
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
        evaluate_quorum(&descriptor, SUBMITTER, &rejected, C1),
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
    // by an unrestricted claim set. Transpose the mapping and `contains`
    // answers `Contained`, so the panic arm below fires and this case goes
    // red — the only place in the suite that catches the inversion.
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

// ---------------------------------------------------------------------------
// The store-backed host (`dod-gate-host`, `dod-preauthorized-mode`).
// ---------------------------------------------------------------------------

/// **A save and a discard are authorized naming no record**, which is the
/// arm §7 row 26 says a store-backed host cannot have.
///
/// Its reason is not `NoMaterialityPolicyGate`'s: that host records a
/// deviation it cannot avoid, while this one states a fact — no ceremony
/// applies to a save. The strings are asserted apart, because an operator
/// reading an audit row is being told two different things.
#[test]
fn an_ungoverned_act_is_authorized_with_nothing_to_spend() {
    let host = StoredApprovalGate::ungoverned();
    let verdict = host
        .evaluate(gate_subject(), InternalRevision::new(3), GateMode::Gate)
        .expect("this host reads nothing and cannot fail to reach an answer");
    match verdict {
        GateVerdict::Authorized(authorization) => {
            assert_eq!(authorization.disposition, ApprovalDisposition::NoRecord);
            assert_eq!(
                authorization.approval_to_consume(),
                None,
                "an ungoverned act spends nothing"
            );
            assert!(!authorization.uncomposed_bundle_override);
            assert!(
                authorization
                    .reason
                    .contains("no approval record is required"),
                "{}",
                authorization.reason
            );
            assert!(
                !authorization.reason.contains("deviation"),
                "this is not the no-policy host's deviation sentence: {}",
                authorization.reason
            );
        }
        GateVerdict::Refused { reason } => panic!(
            "a store-backed host that refuses here refuses every save and discard in the gear \
             (S7 row 26): {reason}"
        ),
    }
}

/// **A governed act with no record is refused** — the answer
/// `inst-fd-gate-mode-gate` requires and `NoMaterialityPolicyGate`
/// deliberately does not give.
///
/// Paired with the case above, this is the whole of what the construction
/// operand buys: the same subject, the same revision, the same mode, two
/// different correct answers.
#[test]
fn a_governed_act_with_no_record_is_refused_on_the_same_triple() {
    let refused = StoredApprovalGate::governed(Vec::new())
        .evaluate(gate_subject(), InternalRevision::new(3), GateMode::Gate)
        .expect("a verdict, not a host failure");
    assert!(
        matches!(refused, GateVerdict::Refused { .. }),
        "{refused:?}"
    );

    // The identical triple, the other construction: authorized.
    assert!(matches!(
        StoredApprovalGate::ungoverned()
            .evaluate(gate_subject(), InternalRevision::new(3), GateMode::Gate)
            .expect("a verdict"),
        GateVerdict::Authorized(_)
    ));
}

/// A `satisfied` record pinned to the door's expected revision authorizes and
/// names the record **to consume**.
#[test]
fn a_satisfied_record_at_the_pinned_revision_is_spent() {
    let host = StoredApprovalGate::governed(vec![candidate(0xf1, 3, ApprovalState::Satisfied)]);
    let verdict = host
        .evaluate(gate_subject(), InternalRevision::new(3), GateMode::Gate)
        .expect("a verdict");
    match verdict {
        GateVerdict::Authorized(authorization) => assert_eq!(
            authorization.approval_to_consume(),
            Some(ApprovalId::new(Uuid::from_u128(0xf1))),
            "Gate mode spends the record it found"
        ),
        GateVerdict::Refused { reason } => panic!("{reason}"),
    }
}

/// **The revision is matched, not merely reported.** A record pinned to
/// another revision is no record at all.
///
/// `inst-fd-publish-pin`: *"an approval is only usable against the exact
/// revision it pinned"*, and the trait's own doc says a host with a record
/// store *"matches on it rather than merely reporting it"*. Without this case
/// a host ignoring the revision passes every other probe here.
#[test]
fn a_record_pinned_to_another_revision_does_not_authorize() {
    let host = StoredApprovalGate::governed(vec![candidate(0xf2, 2, ApprovalState::Satisfied)]);
    assert!(matches!(
        host.evaluate(gate_subject(), InternalRevision::new(3), GateMode::Gate)
            .expect("a verdict"),
        GateVerdict::Refused { .. }
    ));
}

/// Only a **`satisfied`** record authorizes under `Gate`. The other four
/// states are swept, so a host matching on "any record" fails here rather
/// than in production.
#[test]
fn no_state_but_satisfied_authorizes_under_gate() {
    for state in [
        ApprovalState::Pending,
        ApprovalState::Consumed,
        ApprovalState::Rejected,
        ApprovalState::Superseded,
    ] {
        let host = StoredApprovalGate::governed(vec![candidate(0xf3, 3, state)]);
        assert!(
            matches!(
                host.evaluate(gate_subject(), InternalRevision::new(3), GateMode::Gate)
                    .expect("a verdict"),
                GateVerdict::Refused { .. }
            ),
            "state {} must not authorize a Gate-mode act",
            state.as_str()
        );
    }
}

/// **`PreAuthorized` verifies a `consumed` record and spends nothing**
/// (`dod-preauthorized-mode`, `inst-gv-one-shot`).
///
/// The disposition is `Verified`, and `approval_to_consume()` answering
/// `None` is what makes "nothing is consumed under `PreAuthorized`" a
/// property of the type rather than a rule a door must remember.
#[test]
fn preauthorized_verifies_a_consumed_record_and_spends_nothing() {
    let id = ApprovalId::new(Uuid::from_u128(0xf4));
    let host = StoredApprovalGate::governed(vec![candidate(0xf4, 3, ApprovalState::Consumed)]);
    let verdict = host
        .evaluate(
            gate_subject(),
            InternalRevision::new(3),
            GateMode::PreAuthorized(id),
        )
        .expect("a verdict");
    match verdict {
        GateVerdict::Authorized(authorization) => {
            assert_eq!(authorization.disposition, ApprovalDisposition::Verified(id));
            assert_eq!(
                authorization.approval_to_consume(),
                None,
                "a PreAuthorized stage cannot spend a record even by accident"
            );
            assert_eq!(
                authorization.approval_ref(),
                Some(id),
                "the column still records which approval stands behind the frozen version"
            );
        }
        GateVerdict::Refused { reason } => panic!("{reason}"),
    }
}

/// **`PreAuthorized` matches the named id, not just the shape.**
///
/// A subject may accumulate any number of `consumed` records — the partial
/// UNIQUE bounds only the open one — so a stage naming some *other* consumed
/// record of the same subject at the same revision must be refused. Weakening
/// this to "names a consumed record" is what turns a terminal record into an
/// unbounded bearer token (§7 row 27's own words).
#[test]
fn preauthorized_refuses_a_consumed_record_it_did_not_name() {
    let host = StoredApprovalGate::governed(vec![candidate(0xf5, 3, ApprovalState::Consumed)]);
    let refused = host
        .evaluate(
            gate_subject(),
            InternalRevision::new(3),
            GateMode::PreAuthorized(ApprovalId::new(Uuid::from_u128(0xf6))),
        )
        .expect("a verdict");
    assert!(
        matches!(refused, GateVerdict::Refused { .. }),
        "{refused:?}"
    );
}

/// A `satisfied` record does not answer a `PreAuthorized` stage, and a
/// `consumed` one does not answer a `Gate` act. The two modes read disjoint
/// states, and asserting both directions is what stops a host that ignores
/// the mode.
#[test]
fn the_two_modes_read_disjoint_states() {
    let id = ApprovalId::new(Uuid::from_u128(0xf7));
    let satisfied =
        StoredApprovalGate::governed(vec![candidate(0xf7, 3, ApprovalState::Satisfied)]);
    assert!(matches!(
        satisfied
            .evaluate(
                gate_subject(),
                InternalRevision::new(3),
                GateMode::PreAuthorized(id)
            )
            .expect("a verdict"),
        GateVerdict::Refused { .. }
    ));

    let consumed = StoredApprovalGate::governed(vec![candidate(0xf7, 3, ApprovalState::Consumed)]);
    assert!(matches!(
        consumed
            .evaluate(gate_subject(), InternalRevision::new(3), GateMode::Gate)
            .expect("a verdict"),
        GateVerdict::Refused { .. }
    ));
}

/// The override acknowledgment crosses the seam and nothing else does.
///
/// `inst-fd-gate-verdict` fixes the payload at the record's id plus that one
/// flag, so the flag is asserted to travel and to default `false` where no
/// record authorized the act.
#[test]
fn the_override_acknowledgment_travels_and_defaults_false_with_no_record() {
    let mut acked = candidate(0xf8, 3, ApprovalState::Satisfied);
    acked.override_acknowledged = true;
    match StoredApprovalGate::governed(vec![acked])
        .evaluate(gate_subject(), InternalRevision::new(3), GateMode::Gate)
        .expect("a verdict")
    {
        GateVerdict::Authorized(authorization) => {
            assert!(authorization.uncomposed_bundle_override);
        }
        GateVerdict::Refused { reason } => panic!("{reason}"),
    }

    match StoredApprovalGate::ungoverned()
        .evaluate(gate_subject(), InternalRevision::new(3), GateMode::Gate)
        .expect("a verdict")
    {
        GateVerdict::Authorized(authorization) => assert!(
            !authorization.uncomposed_bundle_override,
            "an override nobody granted is not one the door may apply"
        ),
        GateVerdict::Refused { reason } => panic!("{reason}"),
    }
}

/// Every stored state round-trips, and a token outside the roster is named
/// rather than defaulted.
#[test]
fn the_state_roster_round_trips_and_refuses_an_unknown_token() {
    for state in [
        ApprovalState::Pending,
        ApprovalState::Satisfied,
        ApprovalState::Consumed,
        ApprovalState::Rejected,
        ApprovalState::Superseded,
    ] {
        assert_eq!(ApprovalState::parse(state.as_str()), Ok(state));
    }
    assert_eq!(
        ApprovalState::parse("approved"),
        Err("approved".to_owned()),
        "a token outside chk_products_approval_state's roster is a row this gear wrote wrong, \
         and the refusal names it"
    );
}

// ---------------------------------------------------------------------------
// Regressions for the four HIGH findings of the 2026-09-02 four-lens review,
// plus the arms it measured as unexercised.
// ---------------------------------------------------------------------------

/// **The named record is found however the list is ordered** — the regression
/// for the review's HIGH on `PreAuthorized`.
///
/// The named record is deliberately **not** first: `gate_candidates` orders
/// newest-submission-first, so a subject with a history presents its most
/// recent consumed record ahead of the one a mechanical stage names. While the
/// id was a filter over `find`'s answer rather than part of its predicate, the
/// newer record shadowed the named one and the stage was refused — which 04
/// `inst-ar-failure` wraps into a terminal `SCHEDULE_STALE_APPROVAL`.
#[test]
fn preauthorized_finds_the_named_record_behind_a_newer_consumed_one() {
    let named = ApprovalId::new(Uuid::from_u128(0x9b));
    let host = StoredApprovalGate::governed(vec![
        // The shadow: same subject, same revision, same state, different id.
        candidate(0x9a, 3, ApprovalState::Consumed),
        candidate(0x9b, 3, ApprovalState::Consumed),
    ]);
    match host
        .evaluate(
            gate_subject(),
            InternalRevision::new(3),
            GateMode::PreAuthorized(named),
        )
        .expect("a verdict")
    {
        GateVerdict::Authorized(authorization) => {
            assert_eq!(
                authorization.disposition,
                ApprovalDisposition::Verified(named)
            );
            assert_eq!(authorization.approval_to_consume(), None);
        }
        GateVerdict::Refused { reason } => panic!(
            "the named record is present, consumed, on this subject and at this revision, and \
             was shadowed by a newer one: {reason}"
        ),
    }
}

/// **A record on another subject never authorizes**, in either mode — the
/// guard `gate_candidates` had made a tautology by stamping the queried
/// subject onto every candidate.
///
/// All three axes of `GateSubject` are perturbed one at a time, because a
/// guard comparing only the reference would pass a cross-tenant record.
#[test]
fn a_candidate_on_another_subject_authorizes_nothing() {
    let id = ApprovalId::new(Uuid::from_u128(0x9c));
    let others = [
        GateSubject {
            tenant_id: Uuid::from_u128(0x6a_99),
            kind: SubjectKind::EntityPublish,
            reference: gate_subject().reference,
        },
        GateSubject {
            tenant_id: GATE_TENANT,
            kind: SubjectKind::BulkBatch,
            reference: gate_subject().reference,
        },
        GateSubject {
            tenant_id: GATE_TENANT,
            kind: SubjectKind::EntityPublish,
            reference: "product/00000000-0000-0000-0000-0000000000ff".to_owned(),
        },
    ];
    for other in others {
        for (state, mode) in [
            (ApprovalState::Satisfied, GateMode::Gate),
            (ApprovalState::Consumed, GateMode::PreAuthorized(id)),
        ] {
            let mut foreign = candidate(0x9c, 3, state);
            foreign.subject = other.clone();
            let host = StoredApprovalGate::governed(vec![foreign]);
            assert!(
                matches!(
                    host.evaluate(gate_subject(), InternalRevision::new(3), mode)
                        .expect("a verdict"),
                    GateVerdict::Refused { .. }
                ),
                "a candidate on {other:?} must not authorize an act on {:?}",
                gate_subject()
            );
        }
    }
}

/// **`PreAuthorized` on an ungoverned host is refused, not authorized.**
///
/// The ungoverned arm used to return before the mode was read, so a stage
/// naming any id at all — nonexistent, another tenant's — was authorized with
/// `NoRecord`, verifying nothing and writing a NULL `approval_ref` for an act
/// its caller declared pre-authorized. Paired with the `Gate` control, so the
/// refusal is about the mode and not about the construction.
#[test]
fn an_ungoverned_host_refuses_a_preauthorized_stage() {
    let host = StoredApprovalGate::ungoverned();
    let refused = host
        .evaluate(
            gate_subject(),
            InternalRevision::new(3),
            GateMode::PreAuthorized(ApprovalId::new(Uuid::from_u128(0xdead))),
        )
        .expect("a verdict");
    assert!(
        matches!(refused, GateVerdict::Refused { .. }),
        "{refused:?}"
    );

    assert!(matches!(
        StoredApprovalGate::ungoverned()
            .evaluate(gate_subject(), InternalRevision::new(3), GateMode::Gate)
            .expect("a verdict"),
        GateVerdict::Authorized(_)
    ));
}

/// `PreAuthorized` over an empty candidate list refuses rather than panicking
/// or authorizing.
#[test]
fn preauthorized_over_no_candidates_refuses() {
    assert!(matches!(
        StoredApprovalGate::governed(Vec::new())
            .evaluate(
                gate_subject(),
                InternalRevision::new(3),
                GateMode::PreAuthorized(ApprovalId::new(Uuid::from_u128(0x9d)))
            )
            .expect("a verdict"),
        GateVerdict::Refused { .. }
    ));
}

/// The override acknowledgment crosses the seam under `PreAuthorized` too.
///
/// Only the `Gate` arm was asserted, so a `PreAuthorized` arm that dropped
/// the flag would have published a composite act's later stage without the
/// `composition_pending` operand its initiating act carried.
#[test]
fn the_override_acknowledgment_travels_under_preauthorized() {
    let id = ApprovalId::new(Uuid::from_u128(0x9e));
    let mut acked = candidate(0x9e, 3, ApprovalState::Consumed);
    acked.override_acknowledged = true;
    match StoredApprovalGate::governed(vec![acked])
        .evaluate(
            gate_subject(),
            InternalRevision::new(3),
            GateMode::PreAuthorized(id),
        )
        .expect("a verdict")
    {
        GateVerdict::Authorized(authorization) => {
            assert!(authorization.uncomposed_bundle_override);
        }
        GateVerdict::Refused { reason } => panic!("{reason}"),
    }
}

/// **The author is not one of their own approvers** — C1's other half, at the
/// evaluator.
///
/// `decision_admitted` refuses this at the write door, and the store's UNIQUE
/// is `(approval_id, approver_principal)`, which does not exclude the
/// submitter. So a caller assembling the list itself — the threat model the
/// deduplication below already assumes — could close a record on its author
/// alone. The paired control is that the same list with a different principal
/// closes it.
#[test]
fn the_author_is_not_counted_among_their_own_approvers() {
    let descriptor = describe_quorum(Materiality::Material, 1, false);
    let authors_own = [CastDecision {
        principal: SUBMITTER,
        approved: true,
        roles: vec![ApproverRole::CatalogAdmin],
    }];
    assert_eq!(
        evaluate_quorum(&descriptor, SUBMITTER, &authors_own, C1),
        QuorumOutcome::CountUnmet {
            counted: 0,
            required: 1
        },
        "C1's approvers are each distinct from the author, and this is where a caller-built \
         list is held to it"
    );
    assert_eq!(
        evaluate_quorum(
            &descriptor,
            SUBMITTER,
            &[approves(0xaa, &[ApproverRole::CatalogAdmin])],
            C1
        ),
        QuorumOutcome::Satisfied,
        "the control: a different principal closes it"
    );
}

/// **A `FinanceReviewer` counts as an ordinary approver under C1's pair.**
///
/// C1 reads "`CatalogAdmin` **or** `FinanceReviewer`", so a finance-only
/// principal is a full approver and not merely the lens. While the operand was
/// a slice, six probes passed `[CatalogAdmin]` alone — a narrowing C8 says v1
/// does not register — and that dropped this principal together with the lens
/// `inst-gv-finance-predicate` needs.
#[test]
fn a_finance_reviewer_is_a_full_approver_and_also_the_lens() {
    let descriptor = describe_quorum(Materiality::Material, 2, true);
    let pair = [
        approves(0xba, &[ApproverRole::CatalogAdmin]),
        approves(0xbb, &[ApproverRole::FinanceReviewer]),
    ];
    assert_eq!(
        evaluate_quorum(&descriptor, SUBMITTER, &pair, C1),
        QuorumOutcome::Satisfied,
        "two eligible principals, one of them the mandated FinanceReviewer"
    );
}

/// A **rejecting** `FinanceReviewer` supplies no lens.
///
/// The rejection guard sits before the lens read, and nothing exercised that
/// order: the existing rejection probe used a `CatalogAdmin` on a descriptor
/// with no finance predicate, so the lens read was never reached at all.
#[test]
fn a_rejecting_finance_reviewer_supplies_no_lens() {
    let descriptor = describe_quorum(Materiality::Material, 1, true);
    let decisions = [
        CastDecision {
            principal: Uuid::from_u128(0xca),
            approved: false,
            roles: vec![ApproverRole::FinanceReviewer],
        },
        approves(0xcb, &[ApproverRole::CatalogAdmin]),
    ];
    assert_eq!(
        evaluate_quorum(&descriptor, SUBMITTER, &decisions, C1),
        QuorumOutcome::RolePredicateUnmet { counted: 1 },
        "a refused verdict is not a lens: the count is met by the CatalogAdmin and the \
         predicate is not"
    );
}

/// **`predicateUnsatisfiable`'s `Some` arm round-trips**, which nothing
/// measured end to end.
///
/// Every `stored()` and round-trip probe used a descriptor whose predicate is
/// `None`, so the one field P-D-11 minted was never rendered or decoded. Its
/// stored spelling is asserted literally, because that string crosses a
/// storage boundary and two engines.
#[test]
fn the_unsatisfiable_marker_round_trips_through_its_stored_spelling() {
    let at_zero = describe_quorum(Materiality::Material, 0, true);
    let stored = at_zero.stored();
    assert!(
        stored.contains(r#""predicateUnsatisfiable":"finance_reviewer""#),
        "the marker's stored spelling is part of the contract: {stored}"
    );
    assert_eq!(
        descriptor_from_stored(&stored).expect("a descriptor this gear wrote decodes"),
        at_zero,
        "the Some arm survives the round trip, not just the None one"
    );
}

/// **The decode refuses a descriptor whose stored fields contradict**, which
/// is what makes `evaluate_quorum`'s branch on `finance_required` alone sound.
///
/// Both invariants are derivable from the stored fields with no extra operand,
/// and the second is the one that matters: a row recording the marker at a
/// non-zero `required` would discharge the finance predicate at `N >= 1`,
/// exactly the discharge `inst-gv-quorum` forbids.
#[test]
fn the_decode_refuses_a_contradictory_descriptor() {
    let marker_above_zero = r#"{"configuredQuorum":2,"financeRequired":false,"predicateUnsatisfiable":"finance_reviewer","quorumReduced":false,"required":2}"#;
    let err = descriptor_from_stored(marker_above_zero)
        .expect_err("the marker is admitted only where the predicate has no subject");
    assert!(err.contains("predicateUnsatisfiable"), "{err}");

    let reduced_disagrees = r#"{"configuredQuorum":2,"financeRequired":false,"predicateUnsatisfiable":null,"quorumReduced":true,"required":2}"#;
    let err = descriptor_from_stored(reduced_disagrees)
        .expect_err("quorumReduced is set exactly below the retained-name default");
    assert!(err.contains("quorumReduced"), "{err}");

    // The control: a descriptor this gear actually writes decodes.
    let honest = describe_quorum(Materiality::Material, 2, true);
    assert_eq!(
        descriptor_from_stored(&honest.stored()).expect("decodes"),
        honest
    );
}

/// When **both** dimensions fail the verdict names the region, and the
/// ordering is pinned rather than incidental.
#[test]
fn both_dimensions_failing_reports_the_region() {
    assert!(matches!(
        approver_covers_subject(
            &pair(restricted(&["eu"]), restricted(&["acme"])),
            &pair(restricted(&["us"]), restricted(&["globex"])),
        ),
        ApproverScopeVerdict::Exceeded {
            dimension: ScopeDimension::Region,
            ..
        }
    ));
}
