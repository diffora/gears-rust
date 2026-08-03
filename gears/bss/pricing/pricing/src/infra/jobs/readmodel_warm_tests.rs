//! What the sweep decides without a database.
//!
//! One thing, and it is the one the design set names by string: the two alarm
//! constants. §3.6 and §4.4 spell them, an operator's runbook greps for them,
//! and nothing else in this crate would notice a typo — the alarms have no
//! sink, so the string *is* the artifact. Everything else the pass does is a
//! statement about rows in four tables and is proved in
//! `tests/sqlite_read_model.rs`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::{ALARM_COMMIT_OVERDUE, ALARM_PIN_ELIGIBILITY_OVERDUE, SweepReport};

#[test]
fn the_two_alarm_names_are_the_ones_the_design_set_spells() {
    // Asserted against literals rather than derived, for `CatalogEvent::as_str`'s
    // reason: these are the strings an operator's runbook matches on, and this
    // gear has no alarm facility for a second spelling to fail against.
    assert_eq!(
        ALARM_COMMIT_OVERDUE,
        "pricing.catalogversion.commit_overdue"
    );
    assert_eq!(
        ALARM_PIN_ELIGIBILITY_OVERDUE,
        "pricing.readmodel.pin_eligibility_overdue"
    );
}

#[test]
fn a_pass_that_did_nothing_reports_nothing_rather_than_reporting_inert() {
    // `inert` means "no registry is wired", which is a deployment state a
    // caller may act on. A pass with no pending refs is the ordinary steady
    // state and must not claim it.
    let quiet = SweepReport::default();

    assert!(!quiet.inert);
    assert_eq!(quiet.pending_seen, 0);
    assert_eq!(quiet.versions_projected, 0);
}
