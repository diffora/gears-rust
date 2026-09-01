//! `domain::deprecation`'s three rules, each probed on the case that would
//! ship the defect the design names rather than on the happy path.
//!
//! # Why the disposition table is probed exhaustively
//!
//! [`disposition_for`] is a total function over `LifecycleState`, and the
//! defect the design records is a **population** error: an earlier revision
//! keyed the cascade on *"non-terminal children"*, which folded `draft` in
//! with `published` and made deprecating a Product with one draft SKU fail
//! `ILLEGAL_TRANSITION` with no remedy. A test that checked only the
//! `published` arm would have passed against that revision, so every one of
//! the five states is asserted by name.

use bss_products_sdk::models::LifecycleState;

use super::{
    ChildDisposition, Provenance, disposition_for, no_orphan_at_flip, reversal_admits, stamp_for,
};
use crate::domain::error::DomainError;

/// Every state's disposition, by name — the population rule the design
/// states, not a "non-terminal" shorthand for it.
#[test]
fn the_cascade_disposition_is_stated_for_all_five_states() {
    assert_eq!(
        disposition_for(LifecycleState::Published),
        ChildDisposition::Deprecate,
        "a published child is deprecated cascaded"
    );
    assert_eq!(
        disposition_for(LifecycleState::Deprecated),
        ChildDisposition::LeaveUntouched,
        "an already-deprecated child is left untouched, never re-stamped"
    );
    assert_eq!(
        disposition_for(LifecycleState::Draft),
        ChildDisposition::SkipAndList,
        "a draft child is skipped and listed, not transitioned: the floor admits no edge"
    );
    for terminal in [LifecycleState::Retired, LifecycleState::Discarded] {
        assert_eq!(
            disposition_for(terminal),
            ChildDisposition::OutsidePopulation,
            "{} is outside the population rather than skipped inside it",
            terminal.as_str()
        );
    }
}

/// The draft arm is the one that had a defect, and it is asserted as its own
/// case: `SkipAndList` is not `OutsidePopulation`, because the operator sees
/// the listing.
#[test]
fn a_draft_child_is_listed_and_a_terminal_one_is_not() {
    assert_ne!(
        disposition_for(LifecycleState::Draft),
        disposition_for(LifecycleState::Retired),
        "skipped-and-listed and outside-the-population are two answers, not one"
    );
}

/// An operator act on a `published` head stamps `direct`; a parent-driven one
/// stamps `cascaded`. Same edge, different provenance, and the act says which.
#[test]
fn the_stamp_is_the_acts_own_provenance() {
    assert_eq!(
        stamp_for(LifecycleState::Published, Provenance::Direct).expect("admitted"),
        Some(Provenance::Direct)
    );
    assert_eq!(
        stamp_for(LifecycleState::Published, Provenance::Cascaded).expect("admitted"),
        Some(Provenance::Cascaded)
    );
}

/// **The re-stamp refusal**, which is the rule with a consequence rather than
/// a preference: a `direct` child re-stamped `cascaded` would be revived by
/// its parent's un-deprecation.
#[test]
fn an_already_deprecated_entity_is_never_re_stamped() {
    for act in [Provenance::Direct, Provenance::Cascaded] {
        assert_eq!(
            stamp_for(LifecycleState::Deprecated, act).expect("not an error, a no-op"),
            None,
            "an already-deprecated entity takes no transition and no stamp"
        );
    }

    // And the property that refusal buys: the stored value survives, so the
    // reversal rule still reads `direct` and still leaves the child alone.
    assert!(
        !reversal_admits(Some(Provenance::Direct)),
        "a directly deprecated child is not revived by its parent's reversal"
    );
}

/// The reversal operand, all three stored values.
#[test]
fn a_reversal_admits_cascaded_children_only() {
    assert!(reversal_admits(Some(Provenance::Cascaded)));
    assert!(!reversal_admits(Some(Provenance::Direct)));
    assert!(
        !reversal_admits(None),
        "a deprecated row naming no cause is not revived: that would invent a reversal for an \
         act with no record"
    );
}

/// A terminal head refuses the whole question rather than answering `None` —
/// which would read as the admitted no-op case.
#[test]
fn a_terminal_head_refuses_rather_than_reporting_a_no_op() {
    for terminal in [LifecycleState::Retired, LifecycleState::Discarded] {
        let err = stamp_for(terminal, Provenance::Direct)
            .expect_err("no head write is admitted on a terminal entity");
        assert!(
            matches!(err, DomainError::EntityTerminal(ref detail) if detail.contains(terminal.as_str())),
            "got {err:?}"
        );
    }
}

/// A `draft` reaching the stamp is a caller that skipped the disposition
/// table; the floor's own refusal is the answer, not a silent no-op.
#[test]
fn a_draft_reaching_the_stamp_is_an_illegal_transition() {
    let err = stamp_for(LifecycleState::Draft, Provenance::Cascaded)
        .expect_err("the floor admits no draft to deprecated edge");
    assert!(
        matches!(err, DomainError::IllegalTransition { ref from, ref to } if from == "draft" && to == "deprecated"),
        "got {err:?}"
    );
    assert_eq!(err.code(), "ILLEGAL_TRANSITION");
}

/// The provenance roster round-trips and refuses everything outside it.
#[test]
fn provenance_parses_its_two_values_and_no_others() {
    for p in [Provenance::Direct, Provenance::Cascaded] {
        assert_eq!(Provenance::parse(p.as_str()), Some(p));
    }
    for outside in ["", "DIRECT", "cascade", "parent", "null"] {
        assert_eq!(
            Provenance::parse(outside),
            None,
            "{outside} is outside the pair and must not default"
        );
    }
}

/// The no-orphan invariant, on the population that violates it and the two
/// that do not.
#[test]
fn no_published_child_may_remain_under_a_retiring_product() {
    let err = no_orphan_at_flip(&[
        LifecycleState::Deprecated,
        LifecycleState::Published,
        LifecycleState::Published,
    ])
    .expect_err("two published children must refuse the flip");
    assert!(
        matches!(err, DomainError::ParentTerminal(ref d) if d.contains('2')),
        "the refusal names how many remain; got {err:?}"
    );
    assert_eq!(err.code(), "PARENT_TERMINAL");

    no_orphan_at_flip(&[
        LifecycleState::Deprecated,
        LifecycleState::Draft,
        LifecycleState::Retired,
        LifecycleState::Discarded,
    ])
    .expect("no published child remains, whatever else does");
    no_orphan_at_flip(&[]).expect("a childless Product retires");
}

/// A `deprecated` child does **not** block the flip, and that is the
/// invariant's whole point: deprecation is the path to retirement, so a rule
/// that refused deprecated children would refuse every ordinary retirement.
#[test]
fn a_deprecated_child_does_not_block_the_flip() {
    no_orphan_at_flip(&[LifecycleState::Deprecated; 5])
        .expect("deprecated children are the expected state at a flip");
}
