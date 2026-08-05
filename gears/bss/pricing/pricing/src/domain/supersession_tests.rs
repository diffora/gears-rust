//! The changeover instant's two floors.
//!
//! Both cases are driven from **fixed** instants rather than `Utc::now()`, and the
//! reason is the one D-194's case records one plane over: a rule about the distance
//! between two instants, tested against a clock that moves, passes or fails by how
//! long the test took. `now` is a parameter here precisely so it can be a fact.

use chrono::{TimeZone, Utc};

use super::{ChangeoverMoment, MAX_BATCHING_DELAY, check_changeover_instant};
use crate::domain::error::DomainError;

/// A fixed reading of "now" for every case below.
fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2099, 3, 1, 12, 0, 0).unwrap()
}

#[test]
fn at_submit_the_instant_must_only_be_strictly_future() {
    // `inst-su-instant`'s first floor. A submit is not a commit: the batching delay
    // has not started running, and holding a submit to the commit floor would
    // refuse a unit that will be perfectly legal by the time it is approved.
    check_changeover_instant(now() + MAX_BATCHING_DELAY, now(), ChangeoverMoment::Submit)
        .expect("well clear of both floors");
    check_changeover_instant(
        now() + chrono::Duration::milliseconds(1),
        now(),
        ChangeoverMoment::Submit,
    )
    .expect("one quantum into the future is strictly future, which is all submit asks");
}

#[test]
fn at_submit_now_itself_and_anything_behind_it_is_refused() {
    // Strictly future, so the boundary is exclusive: an instant equal to `now` is
    // an instant that has arrived.
    for behind in [
        chrono::Duration::zero(),
        chrono::Duration::milliseconds(1),
        chrono::Duration::days(30),
    ] {
        let err = check_changeover_instant(now() - behind, now(), ChangeoverMoment::Submit)
            .expect_err("an instant at or behind now has passed");
        let DomainError::SupersessionInstantPassed(detail) = err else {
            panic!("the refusal is SUPERSESSION_INSTANT_PASSED, got: {err:?}");
        };
        assert!(
            detail.contains("changeover"),
            "the refusal names the field, got: {detail}"
        );
    }
}

#[test]
fn at_commit_the_instant_must_clear_the_whole_batching_delay() {
    // `inst-su-instant`'s second floor, and the money reason for it: an instant
    // inside the batching lag activates the successor's window while its row is not
    // yet addressable at any completed `CatalogVersion`, so renewals and arrears
    // fail closed for up to the whole delay.
    check_changeover_instant(
        now() + MAX_BATCHING_DELAY + chrono::Duration::milliseconds(1),
        now(),
        ChangeoverMoment::Commit,
    )
    .expect("clear of the delay");

    // The instant a submit would have accepted, refused at commit — which is the
    // whole point of there being two moments.
    let err = check_changeover_instant(
        now() + chrono::Duration::milliseconds(1),
        now(),
        ChangeoverMoment::Commit,
    )
    .expect_err("strictly future is not enough at commit");
    let DomainError::SupersessionInstantPassed(detail) = err else {
        panic!("the refusal is SUPERSESSION_INSTANT_PASSED, got: {err:?}");
    };
    assert!(
        detail.contains("recompose"),
        "the refusal names the remedy the design set gives it, got: {detail}"
    );
}

#[test]
fn the_commit_floor_is_exclusive_at_exactly_the_batching_delay() {
    // "**at least** the max batching delay in the future" against a delay that is
    // itself a maximum: an instant exactly one delay away is one the last batch of
    // that window can still miss. Refused, and the boundary is pinned because an
    // off-by-one here is a fail-closed renewal rather than a refused request.
    let err = check_changeover_instant(now() + MAX_BATCHING_DELAY, now(), ChangeoverMoment::Commit)
        .expect_err("exactly one delay away does not clear the delay");
    assert!(matches!(err, DomainError::SupersessionInstantPassed(_)));
}

#[test]
fn the_delay_is_the_ratified_five_minutes_and_the_refusal_says_so() {
    // D-47's ratified maximum. The number is asserted rather than left to the
    // constant's own definition because the two floors are the only place it is a
    // *rule* rather than an alarm threshold, and a silent change to it moves a
    // money boundary.
    assert_eq!(MAX_BATCHING_DELAY, chrono::Duration::minutes(5));

    let err = check_changeover_instant(now(), now(), ChangeoverMoment::Commit)
        .expect_err("now is not five minutes from now");
    let DomainError::SupersessionInstantPassed(detail) = err else {
        panic!("got: {err:?}");
    };
    assert!(
        detail.contains("300s") || detail.contains("5 min"),
        "an operator has to be told how far ahead is far enough, got: {detail}"
    );
}
