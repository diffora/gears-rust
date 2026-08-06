//! Tests for the grandfathering cutover's compose-time refusals.

use chrono::{DateTime, TimeZone, Utc};

use super::{CUTOVER_INSTANT_PASSED, check_cutover_instant};
use crate::domain::error::DomainError;
use crate::domain::supersession::{
    ChangeoverMoment, MAX_BATCHING_DELAY, SUPERSESSION_INSTANT_PASSED, changeover_floor,
    check_changeover_instant,
};

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 6, 12, 0, 0)
        .single()
        .expect("a fixed instant")
}

fn message(err: &DomainError) -> String {
    err.to_string()
}

#[test]
fn a_cutover_instant_in_the_past_is_refused_at_submit() {
    let err = check_cutover_instant(now() - MAX_BATCHING_DELAY, now(), ChangeoverMoment::Submit)
        .expect_err("a past cutover instant must be refused");

    assert!(
        matches!(err, DomainError::CutoverInstantPassed(_)),
        "the cutover's own code, not the supersession's: {err:?}"
    );
    assert!(
        message(&err).contains("submit"),
        "the refusal names which floor was missed: {}",
        message(&err)
    );
}

#[test]
fn an_instant_inside_the_batching_delay_is_refused_at_commit() {
    // The whole reason the commit floor exists: an instant inside the batching
    // and warm lag activates the successor's window while its row is not yet
    // addressable at any completed `CatalogVersion`, so renewals and arrears on
    // the key just closed fail transiently.
    let inside = now() + MAX_BATCHING_DELAY - chrono::Duration::seconds(1);

    let err = check_cutover_instant(inside, now(), ChangeoverMoment::Commit)
        .expect_err("an instant inside the batching delay must be refused at commit");

    assert!(matches!(err, DomainError::CutoverInstantPassed(_)));
    assert!(
        message(&err).contains(&MAX_BATCHING_DELAY.num_seconds().to_string()),
        "the remedy formats the constant rather than restating the number: {}",
        message(&err)
    );
}

#[test]
fn an_instant_clear_of_the_delay_commits() {
    assert!(
        check_cutover_instant(
            now() + MAX_BATCHING_DELAY + chrono::Duration::seconds(1),
            now(),
            ChangeoverMoment::Commit,
        )
        .is_ok()
    );
}

#[test]
fn the_two_units_share_one_floor_and_answer_two_codes() {
    // **§5 says this in as many words**: `SUPERSESSION_INSTANT_PASSED` is "the same
    // floor `inst-gc-compose` gives cutovers, applied to the everyday mechanism".
    // So the bound is one spelling — a second copy is how two mechanisms come to
    // disagree about one SLO — while the code stays each unit's own, because an
    // operator reading a refusal is told which act they were performing.
    let inside = now() + MAX_BATCHING_DELAY - chrono::Duration::seconds(1);

    let cutover = check_cutover_instant(inside, now(), ChangeoverMoment::Commit)
        .expect_err("the cutover refuses");
    let supersession = check_changeover_instant(inside, now(), ChangeoverMoment::Commit)
        .expect_err("the supersession refuses the same instant");

    assert!(matches!(cutover, DomainError::CutoverInstantPassed(_)));
    assert!(matches!(
        supersession,
        DomainError::SupersessionInstantPassed(_)
    ));
    assert_ne!(
        CUTOVER_INSTANT_PASSED, SUPERSESSION_INSTANT_PASSED,
        "two codes, and the design set declares both"
    );

    // And the sentence is one sentence, so a drift in the bound cannot show up in
    // one unit and not the other. Asserted on the **shared floor** rather than on
    // the two envelopes: each variant carries its own `#[error]` prefix, which is
    // exactly the part that is supposed to differ, so comparing whole messages
    // would be comparing the thing the test is not about.
    let as_cutover = changeover_floor(inside, now(), ChangeoverMoment::Commit, "cutover")
        .expect_err("the floor refuses");
    let as_changeover = changeover_floor(inside, now(), ChangeoverMoment::Commit, "changeover")
        .expect_err("the same floor, the same instant");
    assert_eq!(
        as_cutover.replacen("cutover", "changeover", 1),
        as_changeover,
        "one floor, one sentence: only the noun is the caller's"
    );

    // And both envelopes carry that sentence verbatim, so the shared floor is what
    // an operator actually reads in either unit.
    assert!(message(&cutover).contains(&as_cutover));
    assert!(message(&supersession).contains(&as_changeover));

    // **The noun reaches the sentence**, asserted separately because the equality
    // above cannot see it: `"changeover"` does not contain `"cutover"` as a
    // substring, so the normalisation is a no-op when the noun is ignored and the
    // comparison passes either way. Found by a probe that hard-coded the noun and
    // reddened nothing — the probe ran and compiled, so the empty failure list was
    // a fact about this test rather than about the probe.
    assert!(
        as_cutover.starts_with("cutover instant "),
        "the caller's noun is what the operator reads: {as_cutover}"
    );
    assert!(
        as_changeover.starts_with("changeover instant "),
        "and the supersession keeps its own: {as_changeover}"
    );
}
