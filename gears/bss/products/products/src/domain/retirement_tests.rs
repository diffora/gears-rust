//! Five flip-guard states, replacedBy, EOL, the lead window, and the
//! `effectiveAt` host.

use chrono::{Duration, TimeZone, Utc};
use uuid::Uuid;

use super::{
    FlipPredicate, REPLACEMENT_CHAIN_BROKEN, ReplacementWalk, effective_at, eol_lockout,
    flip_guard, publish_reannounces_retirement, refuse_create_under_retiring_parent,
    replaced_by_must_be_published, replacement_chain_broken_reason, resolve_replacement_chain,
};
use crate::domain::lifecycle::LifecycleRefusal;
use bss_products_sdk::models::LifecycleState;

#[test]
fn flip_guard_passes_only_fresh_zero() {
    flip_guard(FlipPredicate::FreshZero).expect("the one passing state");
}

#[test]
fn flip_guard_defers_all_four_other_states_and_names_producers() {
    for pred in [
        FlipPredicate::FreshPositive,
        FlipPredicate::Stale,
        FlipPredicate::NeverReceived,
        FlipPredicate::NoProducers,
    ] {
        let held = flip_guard(pred).expect_err("defer");
        assert!(!pred.admits_flip());
        if pred == FlipPredicate::NoProducers {
            assert!(
                held.blocking_producers.is_empty(),
                "no-producers names an empty set, not a sentinel"
            );
        } else {
            assert!(
                !held.blocking_producers.is_empty(),
                "{pred:?} must name the blocking producers"
            );
        }
    }
}

#[test]
fn replaced_by_admits_omitted_or_published_and_refuses_the_rest() {
    replaced_by_must_be_published(None).expect("optional");
    replaced_by_must_be_published(Some(LifecycleState::Published)).expect("published successor");
    for bad in [
        LifecycleState::Draft,
        LifecycleState::Deprecated,
        LifecycleState::Retired,
        LifecycleState::Discarded,
    ] {
        let err = replaced_by_must_be_published(Some(bad)).expect_err(bad.as_str());
        assert_eq!(err.code, LifecycleRefusal::REPLACED_BY_NOT_PUBLISHED);
    }
}

#[test]
fn replacement_chain_resolves_skips_retired_and_detects_a_cycle() {
    let a = Uuid::from_u128(0xa);
    let b = Uuid::from_u128(0xb);
    let c = Uuid::from_u128(0xc);
    let chain = [
        (a, Some(b), LifecycleState::Retired),
        (b, Some(c), LifecycleState::Published),
        (c, None, LifecycleState::Published),
    ];
    assert_eq!(
        resolve_replacement_chain(a, &chain, 8),
        ReplacementWalk::Resolved { sku_id: b }
    );

    let looped = [
        (a, Some(b), LifecycleState::Retired),
        (b, Some(a), LifecycleState::Retired),
    ];
    assert!(matches!(
        resolve_replacement_chain(a, &looped, 8),
        ReplacementWalk::Cycle { .. }
    ));
}

#[test]
fn eol_flag_off_refuses_must_migrate_by_and_admits_its_absence() {
    eol_lockout(false, false).expect("no field, flag off");
    let err = eol_lockout(false, true).expect_err("field present");
    assert_eq!(err.code, LifecycleRefusal::EOL_DISABLED);
    eol_lockout(true, true).expect("flag on admits the field");
}

#[test]
fn a_publish_inside_the_window_reannounces_and_one_outside_does_not() {
    let scheduled = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
    let effective = Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap();
    assert!(publish_reannounces_retirement(
        Utc.with_ymd_and_hms(2026, 9, 15, 0, 0, 0).unwrap(),
        scheduled,
        effective
    ));
    assert!(!publish_reannounces_retirement(
        Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap(),
        scheduled,
        effective
    ));
    assert!(!publish_reannounces_retirement(
        Utc.with_ymd_and_hms(2026, 8, 31, 0, 0, 0).unwrap(),
        scheduled,
        effective
    ));
}

#[test]
fn effective_at_computes_the_floor_and_refuses_an_early_operator_instant() {
    let now = Utc.with_ymd_and_hms(2026, 9, 2, 0, 0, 0).unwrap();
    let lead = Duration::days(30);
    assert_eq!(effective_at(now, lead, None).expect("computed"), now + lead);
    assert_eq!(
        effective_at(now, lead, Some(now + lead)).expect("exactly the floor"),
        now + lead
    );
    let err = effective_at(now, lead, Some(now + Duration::days(29))).expect_err("early");
    assert_eq!(err.code, LifecycleRefusal::RETIREMENT_LEAD_TIME);
}

#[test]
fn create_under_a_retiring_parent_is_retirement_pending() {
    let parent = Uuid::from_u128(0x00_dd_00_01);
    refuse_create_under_retiring_parent(parent, false, false).expect("neither fact");
    for (intent, deferral) in [(true, false), (false, true), (true, true)] {
        let err = refuse_create_under_retiring_parent(parent, intent, deferral)
            .expect_err("a retiring parent refuses");
        assert_eq!(err.code, LifecycleRefusal::RETIREMENT_PENDING);
        assert!(err.detail.contains(&parent.to_string()));
    }
}

#[test]
fn replacement_chain_broken_lists_the_pointers() {
    let a = Uuid::from_u128(0xaa);
    let b = Uuid::from_u128(0xbb);
    let reason = replacement_chain_broken_reason(&[a, b]);
    assert!(reason.starts_with(REPLACEMENT_CHAIN_BROKEN));
    assert!(reason.contains(&a.to_string()));
    assert!(reason.contains(&b.to_string()));
}
