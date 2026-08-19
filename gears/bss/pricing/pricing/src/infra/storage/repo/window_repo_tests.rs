//! The half-open interval arithmetic, and the two derivations that hang off the
//! state token.
//!
//! No database here on purpose: [`intersects`] is the whole of §6's non-overlap
//! rule reduced to four instants, and the shapes that matter — adjacency,
//! containment, an open-ended end — are cheaper and clearer to state as
//! arithmetic than to seed as rows. `tests/sqlite_window_repo.rs` is where the
//! rule is proved *through the repository*, against the store that has to enforce
//! it.

use chrono::{DateTime, TimeZone, Utc};

use super::{OCCUPYING_STATES, intersects, pick, render_interval};
use crate::domain::window::WindowState;

/// `2026-08-05T<hour>:00:00Z`. Every instant below is a whole hour of one day, so
/// a reader can see the shape of an interval pair at a glance.
fn t(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 5, hour, 0, 0).unwrap()
}

/// **The rule §9 names by name.** `effectiveTo = next.effectiveFrom` is adjacency
/// and not an overlap, in both directions, because the interval is half-open: the
/// boundary instant belongs to the later window and to nothing else.
///
/// If this ever answers `true`, every coverage test of the slice is wrong with it
/// and so is every supersession and every cutover — both produce exactly this
/// shape.
#[test]
fn two_adjacent_intervals_do_not_intersect() {
    assert!(!intersects(t(1), Some(t(2)), t(2), Some(t(3))));
    assert!(!intersects(t(2), Some(t(3)), t(1), Some(t(2))));
}

/// The statement the guard-by-removal proof uses, as arithmetic: `[t1, t3)` and
/// `[t2, t4)` with `t1 < t2 < t3 < t4`. Fully covered from `t1` to `t4`, no
/// interior gap, and overlapping over `[t2, t3)`.
#[test]
fn a_staggered_pair_intersects_over_the_shared_stretch() {
    assert!(intersects(t(1), Some(t(3)), t(2), Some(t(4))));
    assert!(intersects(t(2), Some(t(4)), t(1), Some(t(3))));
}

#[test]
fn a_contained_interval_intersects_its_container() {
    assert!(intersects(t(1), Some(t(9)), t(3), Some(t(4))));
    assert!(intersects(t(3), Some(t(4)), t(1), Some(t(9))));
}

#[test]
fn an_interval_intersects_itself() {
    assert!(intersects(t(1), Some(t(2)), t(1), Some(t(2))));
    assert!(intersects(t(1), None, t(1), None));
}

/// An open-ended window reaches every later instant, so nothing may be scheduled
/// after it starts — which is why a key with one has no successor to schedule and
/// why `inst-ws-expire` says it never expires.
#[test]
fn an_open_ended_interval_intersects_everything_from_its_start_on() {
    assert!(intersects(t(1), None, t(5), Some(t(6))));
    assert!(intersects(t(5), Some(t(6)), t(1), None));
    assert!(intersects(t(1), None, t(1), Some(t(2))));
}

/// And nothing **before** its start: an open-ended end is not an open-ended
/// beginning. Without this the arithmetic would refuse the one shape a key that
/// has run open-ended forever must still admit — a historical window that closed
/// before it began.
#[test]
fn an_open_ended_interval_does_not_reach_back_before_its_start() {
    assert!(!intersects(t(5), None, t(1), Some(t(5))));
    assert!(!intersects(t(1), Some(t(5)), t(5), None));
}

/// A zero-width probe at a boundary. `chk_pricing_price_window_interval` makes
/// `effective_to = effective_from` unstorable, so this can only arrive from a
/// caller's *candidate* interval — and it must not be read as intersecting the
/// window it abuts.
#[test]
fn an_empty_candidate_interval_intersects_nothing_at_its_own_boundary() {
    assert!(!intersects(t(2), Some(t(2)), t(2), Some(t(3))));
}

/// The occupant set, stated as the property rather than as the constant's own
/// spelling: exactly the two non-terminal states compete for a key.
#[test]
fn only_the_non_terminal_states_occupy_a_key() {
    for state in WindowState::ALL {
        assert_eq!(
            OCCUPYING_STATES.contains(state),
            !state.is_terminal(),
            "{state}"
        );
    }
}

/// An expiry keeps the `activated_at` it was given when the price took effect —
/// `chk_pricing_price_window_activated_at` requires an `expired` window to carry
/// one, so a returned record that recomputed the timestamps from the new state
/// would disagree with the row it just wrote.
#[test]
fn a_flip_stamps_its_own_column_and_carries_the_others_forward() {
    let activated = t(1);
    let expired = t(9);
    assert_eq!(
        pick(
            WindowState::Expired,
            WindowState::Active,
            expired,
            Some(activated)
        ),
        Some(activated)
    );
    assert_eq!(
        pick(WindowState::Expired, WindowState::Expired, expired, None),
        Some(expired)
    );
    assert_eq!(
        pick(WindowState::Active, WindowState::Cancelled, activated, None),
        None
    );
}

/// A rendered interval keeps the half-open brackets and spells an absent end.
/// Both halves are what let an operator reading the refusal see why the window
/// abutting theirs was not the one that collided.
#[test]
fn a_rendered_interval_keeps_its_asymmetry_and_names_an_open_end() {
    let bounded = render_interval(t(1), Some(t(2)));
    assert!(bounded.starts_with('['), "{bounded}");
    assert!(bounded.ends_with(')'), "{bounded}");
    assert!(bounded.contains("2026-08-05T01:00:00"), "{bounded}");
    assert!(bounded.contains("2026-08-05T02:00:00"), "{bounded}");
    assert!(
        render_interval(t(1), None).contains("open-ended"),
        "an absent end is spelled, not dropped"
    );
}
