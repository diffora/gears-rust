//! Unit cases for [`super`] — D-51's kept-vs-cancelled split, D-131's per-price-id
//! map, D-182's absent lane, and `inst-re-references`' two weights.

use uuid::Uuid;

use super::{
    BlockingReferenceKind, KeptReason, PlanReference, PresenceMap, ReferenceReport,
    ScheduledWindow, WarningReferenceKind, WindowDisposition, dispose_windows,
};
use crate::domain::error::DomainError;

fn window(price_id: Uuid) -> ScheduledWindow {
    ScheduledWindow {
        window_id: Uuid::now_v7(),
        price_id,
    }
}

fn reference(label: &str) -> PlanReference {
    PlanReference {
        referrer_id: Uuid::now_v7(),
        referrer_label: label.to_owned(),
    }
}

#[test]
fn a_key_with_no_in_flight_subscribers_has_its_scheduled_window_cancelled() {
    let unoccupied = Uuid::now_v7();
    let verdicts = dispose_windows(&[window(unoccupied)], &PresenceMap::resolved([]));

    assert_eq!(verdicts.len(), 1);
    assert_eq!(verdicts[0].disposition, WindowDisposition::Cancelled);
    assert!(verdicts[0].is_cancelled());
}

#[test]
fn a_key_with_in_flight_subscribers_keeps_its_continuing_coverage_window() {
    // D-51: cancelling it opens the trailing void no gap check can see.
    let occupied = Uuid::now_v7();
    let verdicts = dispose_windows(&[window(occupied)], &PresenceMap::resolved([occupied]));

    assert_eq!(
        verdicts[0].disposition,
        WindowDisposition::Kept(KeptReason::InFlightSubscribers)
    );
    assert!(!verdicts[0].is_cancelled());
}

#[test]
fn a_plan_whose_keys_are_mixed_cancels_exactly_the_unoccupied_ones() {
    // D-131's whole argument: a union **count** answers only "does this plan have
    // any subscriber at all", under which this case cancels nothing.
    let occupied = Uuid::now_v7();
    let unoccupied_a = Uuid::now_v7();
    let unoccupied_b = Uuid::now_v7();
    let scheduled = [
        window(occupied),
        window(unoccupied_a),
        window(unoccupied_b),
        // A second window on the occupied key - the map is keyed by price id, so
        // both of its windows are kept.
        window(occupied),
    ];

    let verdicts = dispose_windows(&scheduled, &PresenceMap::resolved([occupied]));

    let cancelled: Vec<_> = verdicts
        .iter()
        .filter(|v| v.is_cancelled())
        .map(|v| v.price_id)
        .collect();
    assert_eq!(cancelled, vec![unoccupied_a, unoccupied_b]);
    assert_eq!(verdicts.iter().filter(|v| !v.is_cancelled()).count(), 2);
}

#[test]
fn an_unanswerable_lane_keeps_every_window_and_says_which_kind_of_kept() {
    // D-131's fail-closed clause, which D-182 makes this system's only case: the
    // D-79 lane has no client, so every key reads occupied and retirement
    // declines to cancel rather than cancelling blind.
    let a = Uuid::now_v7();
    let b = Uuid::now_v7();
    let verdicts = dispose_windows(&[window(a), window(b)], &PresenceMap::fail_closed());

    assert!(
        verdicts
            .iter()
            .all(|v| v.disposition == WindowDisposition::Kept(KeptReason::PresenceUnresolved))
    );
    assert_eq!(verdicts.iter().filter(|v| v.is_cancelled()).count(), 0);
}

#[test]
fn the_fail_closed_map_reports_itself_so_the_preview_can_label_why() {
    // `inst-re-warn`: "kept because nobody could tell us" and "kept because a
    // subscriber is on it" are different facts for the operator confirming.
    assert!(PresenceMap::fail_closed().is_fail_closed());
    assert!(!PresenceMap::resolved([]).is_fail_closed());
    assert!(PresenceMap::fail_closed().is_occupied(Uuid::now_v7()));
    assert!(!PresenceMap::resolved([]).is_occupied(Uuid::now_v7()));
}

#[test]
fn no_scheduled_windows_is_no_verdicts_rather_than_a_refusal() {
    assert!(dispose_windows(&[], &PresenceMap::fail_closed()).is_empty());
}

#[test]
fn a_bundle_component_reference_blocks_the_retirement_and_is_enumerated() {
    let plan_id = Uuid::now_v7();
    let bundle = reference("Starter Suite");
    let report = ReferenceReport {
        blocking: vec![(BlockingReferenceKind::BundleComponent, bundle.clone())],
        warnings: Vec::new(),
    };

    let err = report.ensure_retirable(plan_id).unwrap_err();
    let DomainError::RetirePlanReferenced(detail) = err else {
        panic!("expected RetirePlanReferenced, got {err:?}");
    };
    assert!(detail.contains("bundle component"), "{detail}");
    assert!(detail.contains("Starter Suite"), "{detail}");
    assert!(detail.contains(&bundle.referrer_id.to_string()), "{detail}");
}

#[test]
fn an_add_on_price_override_target_blocks_too() {
    let report = ReferenceReport {
        blocking: vec![(
            BlockingReferenceKind::AddOnPriceOverrideTarget,
            reference("Support Add-on"),
        )],
        warnings: Vec::new(),
    };

    let err = report.ensure_retirable(Uuid::now_v7()).unwrap_err();
    let DomainError::RetirePlanReferenced(detail) = err else {
        panic!("expected RetirePlanReferenced, got {err:?}");
    };
    assert!(detail.contains("add-on price-override target"), "{detail}");
}

#[test]
fn warnings_alone_never_block_a_retirement() {
    // D-24 and D-31 both: the change-target edge goes inert (Subscriptions
    // re-checks lifecycle state at change time) and the overlay stays evaluable
    // for in-flight subscribers, so neither is a reason to refuse.
    let report = ReferenceReport {
        blocking: Vec::new(),
        warnings: vec![
            (WarningReferenceKind::AllowedChangeTarget, reference("Pro")),
            (
                WarningReferenceKind::PriceOverlayTarget,
                reference("EMEA Q3 discount"),
            ),
        ],
    };

    assert!(report.ensure_retirable(Uuid::now_v7()).is_ok());
}

#[test]
fn a_plan_nothing_refers_to_retires() {
    assert!(
        ReferenceReport::default()
            .ensure_retirable(Uuid::now_v7())
            .is_ok()
    );
}

#[test]
fn every_blocking_referrer_is_named_not_just_the_first() {
    // "1 of 3 things is in the way" is not an actionable sentence, and an
    // operator who fixes the named one comes back to the same refusal twice.
    let report = ReferenceReport {
        blocking: vec![
            (BlockingReferenceKind::BundleComponent, reference("Suite A")),
            (BlockingReferenceKind::BundleComponent, reference("Suite B")),
            (
                BlockingReferenceKind::AddOnPriceOverrideTarget,
                reference("Support"),
            ),
        ],
        warnings: Vec::new(),
    };

    let err = report.ensure_retirable(Uuid::now_v7()).unwrap_err();
    let DomainError::RetirePlanReferenced(detail) = err else {
        panic!("expected RetirePlanReferenced, got {err:?}");
    };
    for expected in ["Suite A", "Suite B", "Support"] {
        assert!(
            detail.contains(expected),
            "{expected} missing from {detail}"
        );
    }
}
