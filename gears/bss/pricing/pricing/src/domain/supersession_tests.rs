//! The changeover instant's two floors, and the two window operations composed
//! against it.
//!
//! Every case is driven from **fixed** instants rather than `Utc::now()`, and the
//! reason is the one D-194's case records one plane over: a rule about the distance
//! between two instants, tested against a clock that moves, passes or fails by how
//! long the test took. `now` is a parameter here precisely so it can be a fact.
//!
//! The compose cases below carry more assertions than their line count suggests in
//! one place: the **adjacency** case reconstructs the shortened predecessor and asks
//! `covers` of both intervals on either side of the changeover. Asserting only the
//! two instants — that the end equals the successor's start — would pass equally
//! under an inclusive end, which is exactly the false positive §9 names and would
//! double-cover the changeover instant.

use chrono::{Duration, TimeZone, Utc};
use uuid::Uuid;

use super::{
    ChangeoverMoment, MAX_BATCHING_DELAY, NamedWindow, check_changeover_instant, compose_windows,
};
use crate::domain::error::DomainError;
use crate::domain::window::{WindowInterval, WindowState};

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

// ---------------------------------------------------------------------------
// Composing the two window operations (`inst-su-compose`).
// ---------------------------------------------------------------------------

fn window_id(n: u128) -> Uuid {
    Uuid::from_u128(0x00d0_0000 ^ n)
}

/// The changeover every case below composes against: well clear of both floors.
fn changeover() -> chrono::DateTime<Utc> {
    now() + Duration::days(30)
}

fn named(n: u128, interval: WindowInterval) -> NamedWindow {
    NamedWindow {
        window_id: window_id(n),
        interval,
    }
}

/// The ordinary world: one active open-ended window carrying the key's coverage.
fn live_open_ended() -> NamedWindow {
    named(
        1,
        WindowInterval::new(now() - Duration::days(90), None, WindowState::Active),
    )
}

#[test]
fn compose_shortens_the_covering_window_and_opens_the_successor_at_the_changeover() {
    let composed = compose_windows(&[live_open_ended()], changeover())
        .expect("an open-ended active window is the ordinary case");

    assert_eq!(composed.shorten.window_id, window_id(1));
    assert_eq!(composed.shorten.effective_to, changeover());
    assert_eq!(composed.successor.effective_from, changeover());
    assert_eq!(
        composed.successor.effective_to, None,
        "`inst-su-compose` schedules the successor as open-ended from the changeover"
    );
    assert_eq!(composed.successor.state, WindowState::Scheduled);
}

#[test]
fn the_two_intervals_meet_exactly_so_no_instant_is_covered_twice_or_not_at_all() {
    // The half-open interval is the whole of the adjacency rule: a window does not
    // cover its own `effectiveTo`, so `effectiveTo = successor.effectiveFrom` leaves
    // the changeover instant covered exactly once. This is the property
    // `inst-su-compose` promises — neither WINDOW_OVERLAP nor WINDOW_TRAILING_VOID
    // can arise from a committed unit — asserted rather than assumed.
    let composed = compose_windows(&[live_open_ended()], changeover()).expect("compose");
    let predecessor = WindowInterval::new(
        live_open_ended().interval.effective_from,
        Some(composed.shorten.effective_to),
        WindowState::Active,
    );

    assert!(!predecessor.covers(changeover()), "the old row stops here");
    assert!(
        composed.successor.covers(changeover()),
        "the new one starts"
    );
    let one_before = changeover() - Duration::milliseconds(1);
    assert!(predecessor.covers(one_before));
    assert!(!composed.successor.covers(one_before));
}

#[test]
fn a_window_with_a_future_end_is_shortened_to_the_changeover_and_keeps_its_start() {
    let bounded = named(
        1,
        WindowInterval::new(
            now() - Duration::days(90),
            Some(now() + Duration::days(365)),
            WindowState::Active,
        ),
    );
    let composed = compose_windows(&[bounded], changeover()).expect("compose");

    assert_eq!(composed.shorten.effective_to, changeover());
    assert_eq!(
        composed.successor.effective_to, None,
        "the successor is open-ended even where its predecessor was not: the \
         predecessor's end was a fact about the old price, not about the key"
    );
}

#[test]
fn a_key_whose_coverage_has_already_ended_is_not_supersedable() {
    // `inst-su-compose`: a key with no window covering the changeover is dormant,
    // and the unit presupposes current coverage. The design set's remedy is a plain
    // publish plus a window schedule — a revival, not a supersession.
    let expired = named(
        1,
        WindowInterval::new(
            now() - Duration::days(90),
            Some(now() - Duration::days(1)),
            WindowState::Expired,
        ),
    );

    let err = compose_windows(&[expired], changeover())
        .expect_err("coverage ended yesterday; there is nothing to shorten");
    let DomainError::LifecycleForbidden(detail) = err else {
        panic!("a dormant key is a refused state, got: {err:?}");
    };
    assert!(detail.contains("dormant"), "got: {detail}");
    assert!(
        detail.contains("publish"),
        "the refusal names the revival path the design set gives it, got: {detail}"
    );
}

#[test]
fn a_key_with_no_windows_at_all_is_dormant_too_and_says_the_same_thing() {
    let err = compose_windows(&[], changeover()).expect_err("no coverage at all");
    assert!(matches!(err, DomainError::LifecycleForbidden(_)));
}

#[test]
fn a_cancelled_window_is_not_coverage_even_where_its_interval_would_cover() {
    // A cancelled window is a schedule that never happened, which is why D-121
    // keeps it out of the read model and why `OCCUPYING_STATES` excludes it. Its
    // interval still spans the changeover, so a compose reading intervals without
    // states would shorten a window that carries nothing.
    let cancelled = named(
        1,
        WindowInterval::new(now() - Duration::days(90), None, WindowState::Cancelled),
    );

    let err = compose_windows(&[cancelled], changeover())
        .expect_err("a cancelled window is not this key's coverage");
    assert!(matches!(err, DomainError::LifecycleForbidden(_)));
}

#[test]
fn a_scheduled_window_that_covers_the_changeover_is_coverage() {
    // The key's coverage has not started yet and the changeover falls inside it.
    // Repricing before the window opens is a supersession like any other: the
    // covering window is `scheduled`, its start is untouched, its end moves.
    let ahead = named(
        1,
        WindowInterval::new(now() + Duration::days(7), None, WindowState::Scheduled),
    );
    let composed = compose_windows(&[ahead], changeover()).expect("scheduled coverage counts");

    assert_eq!(composed.shorten.window_id, window_id(1));
    assert_eq!(composed.shorten.effective_to, changeover());
}

#[test]
fn a_window_beginning_at_or_after_the_changeover_would_collide_with_the_successor() {
    // The successor is open-ended, so anything on the key starting at or after the
    // changeover is inside it. `inst-su-compose` promises a committed unit can
    // never produce WINDOW_OVERLAP, which means compose is where it is produced.
    let later = named(
        2,
        WindowInterval::new(
            changeover() + Duration::days(1),
            None,
            WindowState::Scheduled,
        ),
    );

    let err = compose_windows(&[live_open_ended(), later], changeover())
        .expect_err("the successor's open-ended interval would swallow the later window");
    let DomainError::WindowOverlap(detail) = err else {
        panic!("a collision is WINDOW_OVERLAP, got: {err:?}");
    };
    assert!(
        detail.contains(&window_id(2).to_string()),
        "the refusal names the window it would collide with, got: {detail}"
    );
}

#[test]
fn a_window_beginning_exactly_at_the_changeover_collides_too() {
    // The boundary case of the one above, and the direction matters: the successor
    // covers its own `effectiveFrom`, so a sibling starting on that instant is a
    // genuine double cover rather than an adjacency.
    let coincident = named(
        2,
        WindowInterval::new(changeover(), None, WindowState::Scheduled),
    );

    let err = compose_windows(&[live_open_ended(), coincident], changeover())
        .expect_err("two windows claiming the changeover instant");
    assert!(matches!(err, DomainError::WindowOverlap(_)));
}

#[test]
fn history_behind_the_changeover_is_left_alone() {
    // Expired windows earlier on the key are the key's history and are immutable;
    // they neither supply coverage at the changeover nor collide with the
    // successor, so compose must ignore them rather than refuse over them.
    let history = named(
        3,
        WindowInterval::new(
            now() - Duration::days(400),
            Some(now() - Duration::days(90)),
            WindowState::Expired,
        ),
    );
    let composed = compose_windows(&[history, live_open_ended()], changeover())
        .expect("history does not block a supersession");

    assert_eq!(composed.shorten.window_id, window_id(1));
}

#[test]
fn a_cancelled_window_after_the_changeover_does_not_collide_with_the_successor() {
    // The state filter's *other* load-bearing case, and the one a removal probe over
    // the covering half does not reach. A cancelled window is a schedule that never
    // happened, so a cancelled future interval is not coverage this supersession
    // would be replacing — refusing over it would make a key unsupersedable because
    // of an act somebody already took back.
    let cancelled_ahead = named(
        2,
        WindowInterval::new(
            changeover() + Duration::days(1),
            None,
            WindowState::Cancelled,
        ),
    );
    let composed = compose_windows(&[live_open_ended(), cancelled_ahead], changeover())
        .expect("a cancelled future window is not a collision");

    assert_eq!(composed.shorten.window_id, window_id(1));
}
