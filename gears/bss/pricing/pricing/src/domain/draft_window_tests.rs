//! Pure tests for draft-window start resolution and schedule composition.

use super::{
    DraftStart, DraftWindowAction, DraftWindowEntry, ProposedWindow, WindowBaseline,
    compose_windows, project_working_windows, resolve_start,
};
use crate::domain::error::DomainError;
use crate::domain::instant::utc_ymd_hms;
use crate::domain::scope_key::MarketPriceScopeKey;
use crate::domain::window::{WINDOW_OVERLAP, WINDOW_START_ELAPSED};
use std::collections::{BTreeMap, BTreeSet};
use time::OffsetDateTime;
use uuid::Uuid;

fn t(day: i64) -> OffsetDateTime {
    utc_ymd_hms(2026, 9, 1, 0, 0, 0) + time::Duration::days(day)
}

/// One market of one logical line — the full conflict scope a window competes on.
///
/// Currency and region are parameters because they are *inside* the key: two
/// markets of the same line schedule their money independently, and only a test
/// that varies them can tell an independent schedule from a shared one.
fn market(charge_kind: &str, currency: &str, region: &str) -> MarketPriceScopeKey {
    use crate::domain::money::CurrencyCode;
    use crate::domain::scope_key::{
        ChargeKind, ChargeLineScopeKey, Cohort, PhaseId, PlanId, PriceEligibility, Region, SkuId,
    };

    MarketPriceScopeKey::new(
        ChargeLineScopeKey::new(
            PlanId::new(Uuid::from_u128(0x91_a1)),
            PhaseId::new(Uuid::from_u128(0x40_a5)),
            PriceEligibility::AllSubscriptions,
            match charge_kind {
                "recurring" => ChargeKind::Recurring,
                _ => ChargeKind::OneTime,
            },
            Cohort::None,
            SkuId::new(Uuid::from_u128(5)),
        )
        .expect("a valid canonical scope key"),
        CurrencyCode::new(currency).expect("iso currency"),
        Region::new(region).expect("region"),
    )
}

fn key(charge_kind: &str) -> MarketPriceScopeKey {
    market(charge_kind, "USD", "us-east")
}

fn price_id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn window_id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn operation_id(n: u128) -> Uuid {
    Uuid::from_u128(0xE000 + n)
}

fn keys_for(prices: &[(Uuid, &str)]) -> BTreeMap<Uuid, MarketPriceScopeKey> {
    prices.iter().map(|(id, kind)| (*id, key(kind))).collect()
}

/// The candidate set a store would hand composition: one market key per price
/// row, `(price_id, currency, region)`.
fn market_keys_for(prices: &[(Uuid, &str, &str)]) -> BTreeMap<Uuid, MarketPriceScopeKey> {
    prices
        .iter()
        .map(|(id, currency, region)| (*id, market("recurring", currency, region)))
        .collect()
}

/// A `create` whose operation id is derived from its window id, so a case may
/// stage several without colliding on the one-operation-per-window rule.
fn create_on(
    window: Uuid,
    price: Uuid,
    from: OffsetDateTime,
    to: Option<OffsetDateTime>,
) -> DraftWindowEntry {
    DraftWindowEntry {
        operation_id: Uuid::from_u128(0xE100 + window.as_u128()),
        action: DraftWindowAction::Create {
            window_id: window,
            price_id: price,
            start: DraftStart::At(from),
            effective_to: to,
        },
        reason_code: "test".to_owned(),
    }
}

/// Which price rows the composition proposed a window for.
fn proposed_price_ids(proposed: &[ProposedWindow]) -> BTreeSet<Uuid> {
    proposed.iter().map(|window| window.price_id).collect()
}

fn entry(action: DraftWindowAction) -> DraftWindowEntry {
    DraftWindowEntry {
        operation_id: operation_id(1),
        action,
        reason_code: "test".to_owned(),
    }
}

fn code_of(err: &DomainError) -> Option<&'static str> {
    match err {
        DomainError::WindowStartElapsed(_) => Some(WINDOW_START_ELAPSED),
        DomainError::WindowOverlap(_) => Some(WINDOW_OVERLAP),
        _ => None,
    }
}

#[test]
fn symbolic_start_is_resolved_for_each_evaluation() {
    let first = utc_ymd_hms(2027, 1, 1, 0, 0, 0);
    let later = first + time::Duration::hours(2);
    assert_eq!(resolve_start(&DraftStart::AtPublish, first).unwrap(), first);
    assert_eq!(resolve_start(&DraftStart::AtPublish, later).unwrap(), later);
    assert!(resolve_start(&DraftStart::At(first), later).is_err());
}

#[test]
fn adjacent_intervals_on_one_key_compose_without_overlap() {
    let pid = price_id(1);
    let w1 = window_id(1);
    let w2 = window_id(2);
    let submit = t(0);
    let boundary = t(10);
    let keys = keys_for(&[(pid, "recurring")]);
    let entries = vec![
        entry(DraftWindowAction::Create {
            window_id: w1,
            price_id: pid,
            start: DraftStart::At(t(1)),
            effective_to: Some(boundary),
        }),
        entry(DraftWindowAction::Create {
            window_id: w2,
            price_id: pid,
            start: DraftStart::At(boundary),
            effective_to: None,
        }),
    ];

    let composed = compose_windows(&[], &entries, &keys, submit).expect("adjacent pair");
    assert_eq!(composed.len(), 2);
    assert_eq!(composed[0].effective_from, t(1));
    assert_eq!(composed[0].effective_to, Some(boundary));
    assert_eq!(composed[1].effective_from, boundary);
    assert_eq!(composed[1].effective_to, None);
}

#[test]
fn equal_starts_on_one_key_are_refused_as_overlap() {
    let pid = price_id(1);
    let keys = keys_for(&[(pid, "recurring")]);
    let start = t(5);
    let entries = vec![
        entry(DraftWindowAction::Create {
            window_id: window_id(1),
            price_id: pid,
            start: DraftStart::At(start),
            effective_to: Some(t(10)),
        }),
        DraftWindowEntry {
            operation_id: operation_id(2),
            action: DraftWindowAction::Create {
                window_id: window_id(2),
                price_id: pid,
                start: DraftStart::At(start),
                effective_to: Some(t(20)),
            },
            reason_code: "test".to_owned(),
        },
    ];

    let err = compose_windows(&[], &entries, &keys, t(0)).expect_err("equal starts overlap");
    assert_eq!(code_of(&err), Some(WINDOW_OVERLAP));
}

#[test]
fn symbolic_and_exact_overlap_is_refused() {
    let pid = price_id(1);
    let keys = keys_for(&[(pid, "recurring")]);
    let submit = t(0);
    let entries = vec![
        entry(DraftWindowAction::Create {
            window_id: window_id(1),
            price_id: pid,
            start: DraftStart::AtPublish,
            effective_to: Some(t(10)),
        }),
        DraftWindowEntry {
            operation_id: operation_id(2),
            action: DraftWindowAction::Create {
                window_id: window_id(2),
                price_id: pid,
                start: DraftStart::At(t(5)),
                effective_to: None,
            },
            reason_code: "test".to_owned(),
        },
    ];

    let err = compose_windows(&[], &entries, &keys, submit).expect_err("symbolic overlap");
    assert_eq!(code_of(&err), Some(WINDOW_OVERLAP));
}

#[test]
fn missing_baseline_target_is_refused() {
    let pid = price_id(1);
    let keys = keys_for(&[(pid, "recurring")]);
    let entries = vec![entry(DraftWindowAction::AdjustEnd {
        window_id: window_id(99),
        effective_to: Some(t(20)),
    })];

    assert!(matches!(
        compose_windows(&[], &entries, &keys, t(0)),
        Err(DomainError::InvalidRequest(_))
    ));
}

#[test]
fn missing_price_row_in_candidate_set_is_refused() {
    let pid = price_id(1);
    let keys = BTreeMap::new();
    let entries = vec![entry(DraftWindowAction::Create {
        window_id: window_id(1),
        price_id: pid,
        start: DraftStart::AtPublish,
        effective_to: None,
    })];

    assert!(matches!(
        compose_windows(&[], &entries, &keys, t(0)),
        Err(DomainError::InvalidRequest(_))
    ));
}

#[test]
fn immutable_binding_refuses_create_on_existing_baseline_id() {
    let pid = price_id(1);
    let wid = window_id(1);
    let keys = keys_for(&[(pid, "recurring")]);
    let baseline = vec![WindowBaseline {
        window_id: wid,
        price_id: pid,
        mutation_seq: 1,
        effective_from: t(1),
        effective_to: None,
        cancelled: false,
    }];
    let entries = vec![entry(DraftWindowAction::Create {
        window_id: wid,
        price_id: price_id(2),
        start: DraftStart::At(t(20)),
        effective_to: None,
    })];

    assert!(matches!(
        compose_windows(&baseline, &entries, &keys, t(0)),
        Err(DomainError::WindowHistoricalImmutable(_))
    ));
}

#[test]
fn canonical_key_change_is_refused_when_price_row_is_unknown() {
    let pid = price_id(1);
    let wid = window_id(1);
    let keys = BTreeMap::new();
    let baseline = vec![WindowBaseline {
        window_id: wid,
        price_id: pid,
        mutation_seq: 1,
        effective_from: t(1),
        effective_to: None,
        cancelled: false,
    }];
    let entries = vec![entry(DraftWindowAction::AdjustEnd {
        window_id: wid,
        effective_to: Some(t(20)),
    })];

    assert!(matches!(
        compose_windows(&baseline, &entries, &keys, t(0)),
        Err(DomainError::InvalidRequest(_))
    ));
}

#[test]
fn duplicate_operations_on_one_window_are_refused() {
    let pid = price_id(1);
    let wid = window_id(1);
    let keys = keys_for(&[(pid, "recurring")]);
    let baseline = vec![WindowBaseline {
        window_id: wid,
        price_id: pid,
        mutation_seq: 1,
        effective_from: t(1),
        effective_to: None,
        cancelled: false,
    }];
    let entries = vec![
        entry(DraftWindowAction::AdjustEnd {
            window_id: wid,
            effective_to: Some(t(20)),
        }),
        DraftWindowEntry {
            operation_id: operation_id(2),
            action: DraftWindowAction::Cancel { window_id: wid },
            reason_code: "test".to_owned(),
        },
    ];

    assert!(matches!(
        compose_windows(&baseline, &entries, &keys, t(0)),
        Err(DomainError::InvalidRequest(_))
    ));
}

#[test]
fn at_publish_adjacent_to_exact_start_passes_at_submit() {
    let pid = price_id(1);
    let keys = keys_for(&[(pid, "recurring")]);
    let submit = t(0);
    let handoff = t(10);
    let entries = vec![
        entry(DraftWindowAction::Create {
            window_id: window_id(1),
            price_id: pid,
            start: DraftStart::AtPublish,
            effective_to: Some(handoff),
        }),
        DraftWindowEntry {
            operation_id: operation_id(2),
            action: DraftWindowAction::Create {
                window_id: window_id(2),
                price_id: pid,
                start: DraftStart::At(handoff),
                effective_to: None,
            },
            reason_code: "test".to_owned(),
        },
    ];

    let composed = compose_windows(&[], &entries, &keys, submit).expect("submit adjacency");
    assert_eq!(composed.len(), 2);
    assert_eq!(composed[0].effective_from, submit);
    assert_eq!(composed[0].effective_to, Some(handoff));
    assert_eq!(composed[1].effective_from, handoff);
}

#[test]
fn at_publish_adjacent_to_exact_start_fails_at_commit_when_handoff_elapsed() {
    let pid = price_id(1);
    let keys = keys_for(&[(pid, "recurring")]);
    let handoff = t(10);
    let commit = handoff;
    let entries = vec![
        entry(DraftWindowAction::Create {
            window_id: window_id(1),
            price_id: pid,
            start: DraftStart::AtPublish,
            effective_to: Some(handoff),
        }),
        DraftWindowEntry {
            operation_id: operation_id(2),
            action: DraftWindowAction::Create {
                window_id: window_id(2),
                price_id: pid,
                start: DraftStart::At(handoff),
                effective_to: None,
            },
            reason_code: "test".to_owned(),
        },
    ];

    assert!(matches!(
        compose_windows(&[], &entries, &keys, commit),
        Err(DomainError::InvalidRequest(_))
    ));
}

#[test]
fn open_ended_at_publish_alone_remains_valid_at_commit() {
    let pid = price_id(1);
    let keys = keys_for(&[(pid, "recurring")]);
    let commit = t(30);
    let entries = vec![entry(DraftWindowAction::Create {
        window_id: window_id(1),
        price_id: pid,
        start: DraftStart::AtPublish,
        effective_to: None,
    })];

    let composed = compose_windows(&[], &entries, &keys, commit).expect("open-ended symbol");
    assert_eq!(
        composed,
        vec![ProposedWindow {
            window_id: window_id(1),
            price_id: pid,
            key: key("recurring"),
            effective_from: commit,
            effective_to: None,
        }]
    );
}

#[test]
fn elapsed_exact_start_at_commit_is_window_start_elapsed() {
    let first = utc_ymd_hms(2027, 1, 1, 0, 0, 0);
    let later = first + time::Duration::hours(2);
    let pid = price_id(1);
    let keys = keys_for(&[(pid, "recurring")]);
    let entries = vec![entry(DraftWindowAction::Create {
        window_id: window_id(1),
        price_id: pid,
        start: DraftStart::At(first),
        effective_to: None,
    })];

    let err = compose_windows(&[], &entries, &keys, later).expect_err("elapsed exact start");
    assert_eq!(code_of(&err), Some(WINDOW_START_ELAPSED));

    let working = project_working_windows(&[], &entries, &keys, later)
        .expect("Working still shows a legally saved elapsed exact start");
    assert_eq!(working.len(), 1);
    assert_eq!(working[0].effective_from, first);
}

#[test]
fn working_projection_keeps_a_cancel_after_the_target_becomes_active() {
    let pid = price_id(1);
    let wid = window_id(1);
    let keys = keys_for(&[(pid, "recurring")]);
    let baseline = vec![WindowBaseline {
        window_id: wid,
        price_id: pid,
        mutation_seq: 1,
        effective_from: t(0),
        effective_to: None,
        cancelled: false,
    }];
    let entries = vec![entry(DraftWindowAction::Cancel { window_id: wid })];

    assert!(matches!(
        compose_windows(&baseline, &entries, &keys, t(1)),
        Err(DomainError::WindowNotCancellable(_))
    ));

    let working = project_working_windows(&baseline, &entries, &keys, t(1))
        .expect("Working still shows the staged cancel of a now-active baseline");
    assert!(working.is_empty());
}

// ---------------------------------------------------------------------------
// Independent monetary windows on the full market key
//
// The conflict scope of a window is `MarketPriceScopeKey` — the logical line
// plus currency and region. These cases vary exactly one axis at a time so a
// grouping that collapsed to the line, to the currency alone, or to the price
// version would fail one of them:
//
// | Input                                     | Assertion                       |
// |-------------------------------------------|---------------------------------|
// | EUR/EU `[t0,t2)`, USD/US `[t0,t3)`        | both survive                    |
// | USD/US `[t0,t2)`, USD/CA `[t0,t3)`        | both survive (equal currency)   |
// | USD/US `[t0,t2)`, USD/US `[t1,t3)`        | overlap despite distinct UUIDs  |
// | USD/US `[t0,t1)`, USD/US `[t1,open)`      | contiguous coverage succeeds    |
// | a priced row with no create intention     | no window is invented           |
//
// The gap case (`A` ends `t1`, `B` starts `t2 > t1`) is a coverage refusal, not
// a composition one — composition permits a hole and `window_coverage_rules`
// refuses it. It lives with the other coverage rules in `coverage_tests.rs`.
// ---------------------------------------------------------------------------

#[test]
fn two_currencies_of_one_line_schedule_independently() {
    let eur = price_id(0x1a);
    let usd = price_id(0x1b);
    let keys = market_keys_for(&[(eur, "EUR", "eu"), (usd, "USD", "us")]);
    let entries = vec![
        create_on(window_id(1), eur, t(1), Some(t(10))),
        create_on(window_id(2), usd, t(1), Some(t(20))),
    ];

    let proposed =
        compose_windows(&[], &entries, &keys, t(0)).expect("two currencies are two scopes");

    assert_eq!(proposed_price_ids(&proposed), BTreeSet::from([eur, usd]));
}

#[test]
fn two_regions_of_one_currency_schedule_independently() {
    let us = price_id(0x2a);
    let ca = price_id(0x2b);
    let keys = market_keys_for(&[(us, "USD", "us"), (ca, "USD", "ca")]);
    let entries = vec![
        create_on(window_id(1), us, t(1), Some(t(10))),
        create_on(window_id(2), ca, t(1), Some(t(20))),
    ];

    let proposed = compose_windows(&[], &entries, &keys, t(0))
        .expect("one currency in two regions is two scopes");

    assert_eq!(proposed_price_ids(&proposed), BTreeSet::from([us, ca]));
}

/// The complement of the two cases above, and without it they prove nothing: a
/// grouping wide enough to keep EUR/EU beside USD/US must still catch two
/// monetary versions of the **same** market. Neither the distinct `price_id`
/// nor the distinct `window_id` widens the scope.
#[test]
fn two_monetary_versions_of_one_market_overlap() {
    let first = price_id(0x3a);
    let second = price_id(0x3b);
    let keys = market_keys_for(&[(first, "USD", "us"), (second, "USD", "us")]);
    let entries = vec![
        create_on(window_id(1), first, t(1), Some(t(10))),
        create_on(window_id(2), second, t(5), Some(t(20))),
    ];

    let err = compose_windows(&[], &entries, &keys, t(0))
        .expect_err("one market cannot hold two prices at once");

    assert_eq!(code_of(&err), Some(WINDOW_OVERLAP));
    let DomainError::WindowOverlap(report) = &err else {
        panic!("the overlap arm carries the report");
    };
    assert!(
        report.contains(&market("recurring", "USD", "us").to_string()),
        "the refusal names the market that is double-booked: {report}"
    );
}

#[test]
fn contiguous_versions_of_one_market_hand_over_at_the_boundary() {
    let first = price_id(0x4a);
    let second = price_id(0x4b);
    let boundary = t(5);
    let keys = market_keys_for(&[(first, "USD", "us"), (second, "USD", "us")]);
    let entries = vec![
        create_on(window_id(1), first, t(1), Some(boundary)),
        create_on(window_id(2), second, boundary, None),
    ];

    let proposed =
        compose_windows(&[], &entries, &keys, t(0)).expect("a half-open hand-over is legal");

    assert_eq!(
        proposed_price_ids(&proposed),
        BTreeSet::from([first, second])
    );
    assert_eq!(proposed[0].effective_to, Some(boundary));
    assert_eq!(proposed[1].effective_from, boundary);
    assert_eq!(proposed[1].effective_to, None);
}

/// D-374: a price row is not a schedule. Composition invents no window for a
/// candidate row nobody authored a `create` for, so a new market arrives
/// uncovered and the coverage rule — not a silent default — is what speaks.
#[test]
fn a_candidate_price_without_a_create_gets_no_window() {
    let keys = market_keys_for(&[(price_id(0x5a), "USD", "us")]);

    let proposed = compose_windows(&[], &[], &keys, t(0)).expect("an empty draft composes");

    assert!(
        proposed.is_empty(),
        "no create intention means no window: {proposed:?}"
    );
}
