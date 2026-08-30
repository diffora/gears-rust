//! Tests for the state-machine floor.
//!
//! Three refusals live in this module and each is measured separately,
//! because each reaches a different set of acts: terminality reaches **every**
//! head write (a save and a publish, not only a transition), the edge list
//! reaches only a `lifecycle_state` change, and a same-value write is not a
//! transition at all and must not be refused by either.
//!
//! The edge cases are enumerated over the full 5x5 product of states rather
//! than spot-checked, so an edge added to [`ADMITTED_EDGES`] without a
//! decision behind it fails a test instead of quietly widening the machine.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use bss_products_sdk::models::LifecycleState;

use super::{
    ADMITTED_EDGES, ApprovalInvalidation, ApprovalInvalidationHook, NoApprovalStoreHook,
    RevisionBump, TransitionDecision, check_head_write, guard, invalidation_for,
};
use crate::domain::error::DomainError;
use crate::domain::governance::EntityRef;

/// Every state, so a case can quantify over the machine rather than name a
/// sample of it.
const ALL_STATES: [LifecycleState; 5] = [
    LifecycleState::Draft,
    LifecycleState::Published,
    LifecycleState::Deprecated,
    LifecycleState::Retired,
    LifecycleState::Discarded,
];

/// A subject for the hook port; its identity is not what any case measures.
fn subject() -> EntityRef {
    EntityRef {
        tenant_id: uuid::Uuid::from_u128(0x11),
        entity_kind: bss_products_sdk::models::EntityKind::Product,
        entity_id: uuid::Uuid::from_u128(0x22),
    }
}

/// All five admitted edges are admitted, named one by one rather than looped
/// over [`ADMITTED_EDGES`] — a loop over the constant would pass against any
/// constant, including a wrong one.
#[test]
fn the_five_admitted_edges_are_admitted() {
    let edges = [
        (LifecycleState::Draft, LifecycleState::Published),
        (LifecycleState::Draft, LifecycleState::Discarded),
        (LifecycleState::Published, LifecycleState::Deprecated),
        (LifecycleState::Deprecated, LifecycleState::Published),
        (LifecycleState::Deprecated, LifecycleState::Retired),
    ];

    for (from, to) in edges {
        let decision = guard(from, to)
            .unwrap_or_else(|_| panic!("{} -> {} is admitted", from.as_str(), to.as_str()));
        assert!(
            matches!(decision, TransitionDecision::Transition(_)),
            "{} -> {} is a transition, not a same-value write",
            from.as_str(),
            to.as_str()
        );
    }
    assert_eq!(
        ADMITTED_EDGES.len(),
        edges.len(),
        "the admitted set is exactly these five; a sixth needs a decision behind it"
    );
    for edge in edges {
        assert!(
            ADMITTED_EDGES.contains(&edge),
            "{} -> {} is missing from ADMITTED_EDGES",
            edge.0.as_str(),
            edge.1.as_str()
        );
    }
}

/// Everything outside the five, and outside the same-value diagonal, is
/// refused — and the refusal names the pair, so a door can report which edge
/// was asked for.
///
/// Quantified over the whole 5x5 product: a widened edge list fails here.
#[test]
fn every_other_edge_is_refused_as_an_illegal_transition() {
    for from in ALL_STATES {
        for to in ALL_STATES {
            if from == to || ADMITTED_EDGES.contains(&(from, to)) {
                continue;
            }
            let error = guard(from, to)
                .expect_err("an unadmitted edge is refused")
                .clone();
            if from.is_terminal() {
                assert!(
                    matches!(error, DomainError::EntityTerminal(_)),
                    "a terminal head refuses before the edge list is consulted"
                );
                continue;
            }
            match error {
                DomainError::IllegalTransition { from: f, to: t } => {
                    assert_eq!(f, from.as_str());
                    assert_eq!(t, to.as_str());
                }
                other => panic!(
                    "{} -> {} must be ILLEGAL_TRANSITION, got {}",
                    from.as_str(),
                    to.as_str(),
                    other.code()
                ),
            }
        }
    }
}

/// Two named refusals from the never-admitted set, so the sweep above cannot
/// be the only thing asserting them: the forward-only rule the `PRD` states
/// (`published -> draft`) and the deprecation shortcut
/// (`published -> retired`, which must go through `deprecated`).
#[test]
fn published_to_draft_and_published_to_retired_are_both_illegal() {
    for (from, to) in [
        (LifecycleState::Published, LifecycleState::Draft),
        (LifecycleState::Published, LifecycleState::Retired),
    ] {
        match guard(from, to).expect_err("neither edge is admitted") {
            DomainError::IllegalTransition { from: f, to: t } => {
                assert_eq!(f, from.as_str());
                assert_eq!(t, to.as_str());
            }
            other => panic!("expected ILLEGAL_TRANSITION, got {}", other.code()),
        }
    }
}

/// A same-value write is not a transition and the guard must not refuse it:
/// the head row is the authoring surface in every non-terminal state, so a
/// save on a `published` head passes `published -> published` through this
/// guard and is not an edge (`inst-fd-transition-guard`).
#[test]
fn a_same_value_write_is_not_a_transition_and_is_not_refused() {
    for state in ALL_STATES {
        if state.is_terminal() {
            continue;
        }
        let decision = guard(state, state)
            .unwrap_or_else(|_| panic!("a save on a {} head is not an edge", state.as_str()));
        assert_eq!(
            decision,
            TransitionDecision::NotATransition,
            "{} -> {} carries no transition effects of its own",
            state.as_str(),
            state.as_str()
        );
    }
}

/// A head write on a terminal row is `ENTITY_TERMINAL`, and that check is
/// separate from the edge list because it reaches saves and publishes too,
/// not only transitions (P-D-25, widened by P-D-32).
#[test]
fn a_head_write_on_a_terminal_row_is_refused_as_entity_terminal() {
    for state in [LifecycleState::Retired, LifecycleState::Discarded] {
        match check_head_write(state).expect_err("a terminal row admits no head write") {
            DomainError::EntityTerminal(reason) => {
                assert!(
                    reason.contains(state.as_str()),
                    "the refusal names the state it refused from: {reason}"
                );
            }
            other => panic!("expected ENTITY_TERMINAL, got {}", other.code()),
        }
    }
}

/// The same check admits every non-terminal state, so it cannot be a blanket
/// refusal that happens to be right twice.
#[test]
fn a_head_write_on_a_non_terminal_row_is_admitted() {
    for state in [
        LifecycleState::Draft,
        LifecycleState::Published,
        LifecycleState::Deprecated,
    ] {
        assert!(
            check_head_write(state).is_ok(),
            "{} is an authoring state",
            state.as_str()
        );
    }
}

/// A same-value write on a terminal row is still refused: terminality is not
/// about whether the state changes, it is about whether the row may be
/// written at all.
#[test]
fn a_same_value_write_on_a_terminal_row_is_still_entity_terminal() {
    for state in [LifecycleState::Retired, LifecycleState::Discarded] {
        let error = guard(state, state).expect_err("a terminal row admits no head write");
        assert!(matches!(error, DomainError::EntityTerminal(_)));
    }
}

/// The gated edge the publish door owns bumps **once** and fires **no** hook:
/// the door's own head-row `UPDATE` carries the bump, and a hook firing
/// against the record the same transaction is consuming has no defined
/// ordering (P-D-26, extended by P-D-34).
#[test]
fn the_gated_draft_to_published_edge_bumps_once_with_no_hook() {
    let effects = match guard(LifecycleState::Draft, LifecycleState::Published)
        .expect("draft -> published is admitted")
    {
        TransitionDecision::Transition(effects) => effects,
        TransitionDecision::NotATransition => panic!("draft -> published is an edge"),
    };

    assert_eq!(
        effects.revision_bump,
        RevisionBump::CarriedByTheAuthorizedAct
    );
    assert_eq!(effects.bumps_the_guard_owns(), 0);
    assert_eq!(effects.bumps_on_the_row(), 1);
    assert_eq!(effects.invalidation, ApprovalInvalidation::Skip);
}

/// Every other admitted edge bumps on its own account and fires the
/// approval-invalidation hook, exactly as a save does
/// (`inst-fd-transition-bump`).
#[test]
fn an_ordinary_transition_bumps_and_fires_the_hook() {
    for (from, to) in [
        (LifecycleState::Draft, LifecycleState::Discarded),
        (LifecycleState::Published, LifecycleState::Deprecated),
        (LifecycleState::Deprecated, LifecycleState::Published),
        (LifecycleState::Deprecated, LifecycleState::Retired),
    ] {
        let effects = match guard(from, to)
            .unwrap_or_else(|_| panic!("{} -> {} is admitted", from.as_str(), to.as_str()))
        {
            TransitionDecision::Transition(effects) => effects,
            TransitionDecision::NotATransition => {
                panic!("{} -> {} is an edge", from.as_str(), to.as_str())
            }
        };
        assert_eq!(effects.revision_bump, RevisionBump::Own);
        assert_eq!(effects.bumps_the_guard_owns(), 1);
        assert_eq!(effects.bumps_on_the_row(), 1);
        assert_eq!(
            effects.invalidation,
            ApprovalInvalidation::Fire,
            "{} -> {} consumes no approval, so the hook fires",
            from.as_str(),
            to.as_str()
        );
    }
}

/// The row bumps exactly once on every admitted edge, gated or not — the
/// property `inst-fd-publish-bump` states as "once" and the reason the gated
/// edge asks the guard for no bump of its own rather than for a second one.
#[test]
fn every_admitted_edge_bumps_the_row_exactly_once() {
    for (from, to) in ADMITTED_EDGES {
        let TransitionDecision::Transition(effects) = guard(from, to)
            .unwrap_or_else(|_| panic!("{} -> {} is admitted", from.as_str(), to.as_str()))
        else {
            panic!("{} -> {} is an edge", from.as_str(), to.as_str())
        };
        assert_eq!(effects.bumps_on_the_row(), 1);
    }
}

/// [`invalidation_for`] answers `Skip` on the diagonal, and the edge's own
/// answer on an edge.
///
/// # Why this case is here and not at a door
///
/// No door can pin the [`TransitionDecision::NotATransition`] arm today. The
/// only hook in the gear is [`NoApprovalStoreHook`], a no-op that always
/// succeeds, so at a door `Fire` and `Skip` produce byte-identical outcomes:
/// a door-level test could assert nothing that would go red if the arm were
/// flipped. The answer becomes observable when slice 05 supplies a record
/// store and the hook starts doing something, and until then the only place
/// the arm is measurable is here, against the function that decides it.
///
/// The `Skip` is not the diagonal's own property but the re-publish's: the
/// transaction that writes version `N + 1` consumes the approval
/// `inst-gv-materiality` gave it, and `inst-fd-transition-bump`'s exception —
/// *"a transition that consumes an approval in the same transaction"* — is
/// therefore satisfied (05 C3, P-D-30). See [`invalidation_for`]'s doc for why
/// a save reaching the same arm is not a counter-example.
#[test]
fn a_re_publish_skips_the_hook_and_an_edge_keeps_its_own_answer() {
    assert_eq!(
        invalidation_for(TransitionDecision::NotATransition),
        ApprovalInvalidation::Skip,
        "a re-publish consumes the approval it is publishing under"
    );

    for (from, to) in ADMITTED_EDGES {
        let TransitionDecision::Transition(effects) = guard(from, to)
            .unwrap_or_else(|_| panic!("{} -> {} is admitted", from.as_str(), to.as_str()))
        else {
            panic!("{} -> {} is an edge", from.as_str(), to.as_str())
        };
        assert_eq!(
            invalidation_for(TransitionDecision::Transition(effects)),
            effects.invalidation,
            "{} -> {} keeps the effects' own answer, unaltered",
            from.as_str(),
            to.as_str()
        );
    }

    assert_eq!(
        invalidation_for(TransitionDecision::Transition(
            match guard(LifecycleState::Draft, LifecycleState::Published)
                .expect("draft -> published is admitted")
            {
                TransitionDecision::Transition(effects) => effects,
                TransitionDecision::NotATransition => panic!("draft -> published is an edge"),
            }
        )),
        ApprovalInvalidation::Skip,
        "the gated edge is the other act that consumes in the same transaction"
    );
}

/// The default hook does nothing and says so by succeeding: there is no
/// approval store at this commit, and a hook that failed closed here would
/// refuse every ordinary transition the gear can currently take.
#[test]
fn the_default_invalidation_hook_is_a_no_op_that_succeeds() {
    assert!(NoApprovalStoreHook.invalidate(subject()).is_ok());
}
