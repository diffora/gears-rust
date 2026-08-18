//! What the sweep decides without a database.
//!
//! One thing, and it is the one the design set names by string: the two alarm
//! constants. §3.6 and §4.4 spell them, an operator's runbook greps for them,
//! and nothing else in this crate would notice a typo. They are on the
//! `PricingAlarm` roster since D-238, so a misspelling here would now disagree
//! with the enum rather than merely with the document — which is why the case
//! below asserts against **literals** rather than against the enum's own
//! rendering. Everything else the pass does is a statement about rows
//! in four tables and is proved in `tests/sqlite_read_model.rs` — including
//! D-237's, which is why the plan-only degraded mark is not asserted here.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::{ALARM_COMMIT_OVERDUE, ALARM_PIN_ELIGIBILITY_OVERDUE, SweepReport};

#[test]
fn the_two_alarm_names_are_the_ones_the_design_set_spells() {
    // Asserted against literals rather than derived, for `CatalogEvent::as_str`'s
    // reason: these are the strings an operator's runbook matches on, and
    // asserting them against `PricingAlarm::as_str` would be asserting the enum
    // against itself. D-238 put both names on that enum, so this case and the
    // roster now have to agree — which is the point of transcribing rather than
    // deriving.
    assert_eq!(
        ALARM_COMMIT_OVERDUE,
        "pricing.catalogversion.commit_overdue"
    );
    assert_eq!(
        ALARM_PIN_ELIGIBILITY_OVERDUE,
        "pricing.readmodel.pin_eligibility_overdue"
    );
}

// A second case used to be here: that a pass which did nothing reports nothing.
// It constructed no job and never called `run`, so what it asserted was
// `#[derive(Default)]` — and of eleven fields it named three, under a name that
// claimed a property of the pass. `jobs/window_activation_tests.rs:16-21` had
// already recorded deleting exactly this shape one file over; the correction had
// been applied to one of the two siblings (review Z4-7). The property it was
// named for belongs to `run` and is asserted of `run` in
// `tests/sqlite_read_model.rs`, which drives a pass over a store with no pending
// refs. Deleted rather than repointed, for that reason.

/// The three members of [`SweepReport`] that are **not** counters, asserted to
/// be independent of each other.
///
/// Each marks a signal that could not be delivered or could not be evaluated,
/// and each exists because *no counter of this pass moves when it fires* — which
/// is precisely what made the swallow untestable before the field existed.
/// `frontier_scan_failed` is Z10-5's; `degraded_emit_failed` and
/// `frontier_block_probe_failed` are Z4-2's and Z4-8's, added for the same
/// reason after review found the two arms that had not been given one.
///
/// What this asserts is the property the three share and the reason they are
/// three fields rather than one: a pass may hit any one of them without hitting
/// the others, so folding them into a single `something_failed` flag would tell
/// an operator that a signal was lost without saying which. The behaviour of
/// each arm is driven end to end in `tests/sqlite_read_model.rs`;
/// `module_tests::sweep_is_noteworthy` is where each is proved to reach an
/// operator.
#[test]
fn the_three_undeliverable_signal_flags_are_independent_of_one_another() {
    let mut report = SweepReport::default();
    assert!(!report.frontier_scan_failed);
    assert!(!report.degraded_emit_failed);
    assert!(!report.frontier_block_probe_failed);

    // Setting one moves neither of the others, and moves no counter — the
    // property that is the whole reason each is its own field.
    report.degraded_emit_failed = true;
    assert!(
        !report.frontier_scan_failed && !report.frontier_block_probe_failed,
        "a failed degraded emit is not a failed frontier read and not a failed block probe"
    );
    assert_eq!(
        report.degraded_emitted, 0,
        "the counter an operator would otherwise watch is unmoved by the failure, which is why \
         the flag has to exist"
    );

    report.frontier_block_probe_failed = true;
    assert!(
        !report.frontier_scan_failed,
        "the per-tenant committed-version probe and the cross-tenant frontier scan are two reads \
         and two flags"
    );
    assert_eq!(
        report.pin_eligibility_overdue, 0,
        "likewise: the Critical this probe feeds is not raised by its failure, and must not read \
         as healthy either"
    );
}
