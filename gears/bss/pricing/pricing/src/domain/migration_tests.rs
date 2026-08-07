//! Unit cases for [`super`] — §4's four edges, D-34's rescoped cancel, D-49's
//! notice floor, `inst-mg-target`'s target predicate and D-39's entry phase.

use chrono::{Duration, TimeZone as _, Utc};
use uuid::Uuid;

use super::{
    MigrationState, NOTICE_FLOOR_DAYS, NoticePeriod, ensure_distinct_plans,
    ensure_target_publishable, entry_phase_index,
};
use crate::domain::error::DomainError;
use crate::domain::lifecycle::LifecycleState;

fn at(year: i32, month: u32, day: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, 0, 0, 0).unwrap()
}

// ---------------------------------------------------------------------------
// §4's state machine.
// ---------------------------------------------------------------------------

#[test]
fn the_four_sanctioned_edges_are_legal_and_nothing_else_is() {
    let legal = [
        (MigrationState::Scheduled, MigrationState::InProgress),
        (MigrationState::Scheduled, MigrationState::Cancelled),
        (MigrationState::InProgress, MigrationState::Completed),
        (MigrationState::InProgress, MigrationState::Cancelled),
    ];
    for (from, to) in legal {
        assert!(from.can_transition(to), "{from} -> {to} should be legal");
        assert!(from.transition(to).is_ok());
    }

    // Every other ordered pair, including all four self-edges.
    for &from in MigrationState::ALL {
        for &to in MigrationState::ALL {
            if legal.contains(&(from, to)) {
                continue;
            }
            assert!(
                !from.can_transition(to),
                "{from} -> {to} should not be legal"
            );
            assert!(from.transition(to).is_err());
        }
    }
}

#[test]
fn a_repeat_start_is_not_a_self_edge_the_machine_accepts() {
    // D-65: a second `start` is a **replay** that returns the persisted exclusion
    // set, and the machine must not quietly accept the flip a second time -
    // otherwise the replay path could be skipped without anything noticing.
    assert!(
        MigrationState::InProgress
            .transition(MigrationState::InProgress)
            .is_err()
    );
}

#[test]
fn cancelling_a_completed_run_answers_migration_completed_and_not_a_bare_lifecycle_refusal() {
    // D-34's one named refusal. The code deliberately is not the pre-D-34
    // `MIGRATION_ALREADY_EFFECTIVE`.
    let err = MigrationState::Completed.ensure_cancellable().unwrap_err();
    assert!(
        matches!(err, DomainError::MigrationCompleted(_)),
        "expected MigrationCompleted, got {err:?}"
    );
}

#[test]
fn an_in_progress_run_is_cancellable_which_is_the_whole_of_what_d34_changed() {
    assert!(MigrationState::InProgress.ensure_cancellable().is_ok());
    assert!(MigrationState::Scheduled.ensure_cancellable().is_ok());
}

#[test]
fn cancelling_an_already_cancelled_run_is_a_lifecycle_refusal_not_a_completion_one() {
    // The two say different things to an operator: one finished doing work, the
    // other was already stopped.
    let err = MigrationState::Cancelled.ensure_cancellable().unwrap_err();
    assert!(
        matches!(err, DomainError::LifecycleForbidden(_)),
        "expected LifecycleForbidden, got {err:?}"
    );
}

#[test]
fn only_the_two_live_states_admit_further_processing() {
    // `inst-mg-cancel`'s state handshake: the predicate Subscriptions asks before
    // execution and per batch.
    assert!(MigrationState::Scheduled.admits_further_processing());
    assert!(MigrationState::InProgress.admits_further_processing());
    assert!(!MigrationState::Completed.admits_further_processing());
    assert!(!MigrationState::Cancelled.admits_further_processing());
}

#[test]
fn terminality_and_the_processing_predicate_agree_on_every_state() {
    for &state in MigrationState::ALL {
        assert_eq!(state.is_terminal(), !state.admits_further_processing());
    }
}

#[test]
fn every_state_round_trips_through_its_token_and_an_unknown_one_does_not_resolve() {
    for &state in MigrationState::ALL {
        assert_eq!(MigrationState::parse(state.as_str()), Some(state));
    }
    // A fifth token is a corrupt row, not a default: resolving it to `scheduled`
    // would hide it behind the one state that still admits every transition.
    assert_eq!(MigrationState::parse("in-progress"), None);
    assert_eq!(MigrationState::parse("effective"), None);
    assert_eq!(MigrationState::parse(""), None);
}

// ---------------------------------------------------------------------------
// D-49's notice period.
// ---------------------------------------------------------------------------

#[test]
fn a_configured_period_below_the_floor_is_raised_to_it_rather_than_honoured() {
    // Fail-closed: lowering is the direction that costs a subscriber their warning.
    assert_eq!(NoticePeriod::resolved(0).days(), NOTICE_FLOOR_DAYS);
    assert_eq!(NoticePeriod::resolved(-5).days(), NOTICE_FLOOR_DAYS);
    assert_eq!(NoticePeriod::resolved(59).days(), NOTICE_FLOOR_DAYS);
    assert_eq!(NoticePeriod::floor().days(), NOTICE_FLOOR_DAYS);
}

#[test]
fn a_tenant_may_raise_the_period_and_the_raised_value_is_what_binds() {
    assert_eq!(NoticePeriod::resolved(90).days(), 90);
    let announced = at(2026, 1, 1);
    assert_eq!(
        NoticePeriod::resolved(90).earliest_effective(announced),
        announced + Duration::days(90)
    );
}

#[test]
fn a_migration_exactly_one_notice_period_out_has_given_the_whole_notice() {
    // The bound is inclusive.
    let announced = at(2026, 1, 1);
    let period = NoticePeriod::floor();
    assert!(
        period
            .ensure_honoured(announced, announced + Duration::days(NOTICE_FLOOR_DAYS))
            .is_ok()
    );
}

#[test]
fn a_migration_inside_the_notice_period_is_refused_naming_the_earliest_instant() {
    let announced = at(2026, 1, 1);
    let period = NoticePeriod::floor();
    let err = period
        .ensure_honoured(announced, announced + Duration::days(NOTICE_FLOOR_DAYS - 1))
        .unwrap_err();

    let DomainError::MigrationNoticeTooShort(detail) = err else {
        panic!("expected MigrationNoticeTooShort, got {err:?}");
    };
    // The operator's next act is to pick a later date or change the policy, and
    // neither is reachable from a refusal that only says "too short".
    assert!(detail.contains("60"), "{detail}");
    assert!(detail.contains("earliest admissible"), "{detail}");
    assert!(detail.contains("no override"), "{detail}");
}

#[test]
fn a_raised_period_refuses_a_date_the_floor_would_have_admitted() {
    // The raise is not decorative: it moves the boundary.
    let announced = at(2026, 1, 1);
    let effective = announced + Duration::days(75);
    assert!(
        NoticePeriod::floor()
            .ensure_honoured(announced, effective)
            .is_ok()
    );
    assert!(
        NoticePeriod::resolved(90)
            .ensure_honoured(announced, effective)
            .is_err()
    );
}

// ---------------------------------------------------------------------------
// `inst-mg-target`.
// ---------------------------------------------------------------------------

#[test]
fn only_a_published_target_admits_a_migration() {
    let target = Uuid::now_v7();
    assert!(ensure_target_publishable(target, LifecycleState::Published).is_ok());

    for state in [
        LifecycleState::Draft,
        LifecycleState::Abandoned,
        LifecycleState::Superseded,
        LifecycleState::Retired,
    ] {
        let err = ensure_target_publishable(target, state).unwrap_err();
        let DomainError::MigrationTargetInvalid(detail) = err else {
            panic!("expected MigrationTargetInvalid for {state}, got {err:?}");
        };
        // The refusal names the state the target is actually in.
        assert!(detail.contains(state.as_str()), "{detail}");
    }
}

#[test]
fn a_migration_onto_the_source_plan_itself_is_refused() {
    let plan = Uuid::now_v7();
    assert!(ensure_distinct_plans(plan, Uuid::now_v7()).is_ok());
    assert!(matches!(
        ensure_distinct_plans(plan, plan).unwrap_err(),
        DomainError::MigrationTargetInvalid(_)
    ));
}

// ---------------------------------------------------------------------------
// D-39.
// ---------------------------------------------------------------------------

#[test]
fn a_migrated_subscription_enters_the_first_non_trial_phase() {
    // D-39, CONFIRMED 2026-08-07: a migration never grants a new `trial`.
    assert_eq!(entry_phase_index(&["trial", "intro", "standard"]), Some(1));
    // ...and **does** grant `intro`, which is the half that is easy to lose.
    assert_eq!(entry_phase_index(&["intro", "standard"]), Some(0));
    assert_eq!(entry_phase_index(&["standard"]), Some(0));
    // Consecutive trials are all skipped, not just the first.
    assert_eq!(entry_phase_index(&["trial", "trial", "standard"]), Some(2));
}

#[test]
fn a_target_that_is_entirely_trial_offers_no_entry_phase() {
    // Not decided here: the caller surfaces it as a boundary delta.
    assert_eq!(entry_phase_index(&["trial"]), None);
    assert_eq!(entry_phase_index(&[]), None);
}
