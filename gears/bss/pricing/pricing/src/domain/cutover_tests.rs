//! Tests for the grandfathering cutover's compose-time refusals.

use chrono::{DateTime, TimeZone, Utc};

use uuid::Uuid;

use super::{CUTOVER_INSTANT_PASSED, check_cutover_instant, compose_cutover_windows};
use crate::domain::error::DomainError;
use crate::domain::supersession::{
    ChangeoverMoment, MAX_BATCHING_DELAY, NamedWindow, SUPERSESSION_INSTANT_PASSED,
    changeover_floor, check_changeover_instant,
};
use crate::domain::window::{WindowInterval, WindowState};

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

// ---------------------------------------------------------------------------
// The three window operations (`inst-co-shorten` / `inst-co-copy` / `inst-co-successor`)
// ---------------------------------------------------------------------------

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2099, 8, 6, hour, 0, 0)
        .single()
        .expect("a fixed future instant")
}

fn window(
    id: u128,
    from: DateTime<Utc>,
    to: Option<DateTime<Utc>>,
    state: WindowState,
) -> NamedWindow {
    NamedWindow {
        window_id: Uuid::from_u128(id),
        interval: WindowInterval::new(from, to, state),
    }
}

#[test]
fn the_three_operations_are_born_of_one_instant() {
    // The gap-freeness `inst-co-atomic` demands is by **construction**: all three
    // instants are the cutover, so no caller can compose a shorten to T1 with
    // schedules from T2 > T1 and leave `[T1, T2)` uncovered. That defect was real
    // on the supersession unit and was found by review, not by a gate — collision
    // is an intersection test and a gap is not an intersection.
    let plane = vec![window(1, at(1), None, WindowState::Active)];

    let composed = compose_cutover_windows(&plane, at(10)).expect("a live key composes");

    assert_eq!(composed.shorten().effective_to, at(10));
    assert_eq!(composed.shorten().window_id, Uuid::from_u128(1));
    assert_eq!(composed.copy().effective_from, at(10));
    assert_eq!(composed.successor().effective_from, at(10));
    assert_eq!(composed.copy().effective_to, None);
    assert_eq!(composed.successor().effective_to, None);
}

#[test]
fn a_dormant_key_is_refused_by_the_cutovers_own_code() {
    // `inst-co-shorten`: the unit presupposes coverage to shorten. Reviving a
    // dormant key is a publish plus a schedule, never a cutover — so the refusal
    // says that rather than inviting a retry.
    let plane = vec![window(1, at(1), Some(at(5)), WindowState::Active)];

    let err = compose_cutover_windows(&plane, at(10)).expect_err("a dormant key must be refused");

    assert!(matches!(err, DomainError::CutoverGap(_)), "{err:?}");
    assert!(
        message(&err).contains("dormant") && message(&err).contains(&at(10).to_rfc3339()),
        "the refusal names the instant with no coverage: {}",
        message(&err)
    );
}

#[test]
fn a_window_beginning_at_the_cutover_is_refused_rather_than_emptied() {
    // The other side of the same absence: shortening a window to its own start
    // leaves `[cutover, cutover)`, so there is no coverage to hand over. On the
    // supersession unit this arm was missing and the key was told it "carries later
    // coverage", naming a sibling that does not exist (review, 2026-08-05).
    let plane = vec![window(7, at(10), None, WindowState::Scheduled)];

    let err = compose_cutover_windows(&plane, at(10)).expect_err("an empty shorten is refused");

    assert!(matches!(err, DomainError::CutoverGap(_)), "{err:?}");
    assert!(
        message(&err).contains(&Uuid::from_u128(7).to_string()),
        "the refusal names the window the operator has to deal with: {}",
        message(&err)
    );
}

#[test]
fn later_coverage_the_cutover_would_not_replace_is_a_collision() {
    let plane = vec![
        window(1, at(1), None, WindowState::Active),
        window(2, at(20), None, WindowState::Scheduled),
    ];

    let err = compose_cutover_windows(&plane, at(10)).expect_err("the later window collides");

    assert!(matches!(err, DomainError::WindowOverlap(_)), "{err:?}");
    assert!(message(&err).contains(&Uuid::from_u128(2).to_string()));
}

#[test]
fn cancelled_and_expired_windows_are_not_coverage() {
    // The occupying set is the shared one, so a cancelled window neither covers the
    // cutover nor collides with the successor.
    let plane = vec![
        window(1, at(1), None, WindowState::Cancelled),
        window(2, at(20), None, WindowState::Cancelled),
    ];

    let err = compose_cutover_windows(&plane, at(10))
        .expect_err("a key whose only windows are cancelled is dormant");

    assert!(matches!(err, DomainError::CutoverGap(_)), "{err:?}");
}
