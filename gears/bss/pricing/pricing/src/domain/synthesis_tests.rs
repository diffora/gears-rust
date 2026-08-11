//! Unit cases for [`super`] — D-76's tier order, D-81's two instants,
//! `inst-sy-firstrating`'s never-inline rule and clause (3)'s fail-closed refusal.

use chrono::{TimeZone as _, Utc};
use uuid::Uuid;

use super::{
    LiveCandidate, ReferenceCandidate, SelectedRow, SelectionTier, SynthesisOutcome,
    SynthesisTrigger, UnresolvedKey, select_rows,
};
use crate::domain::error::DomainError;

fn at(day: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, day, 0, 0, 0).unwrap()
}

fn live(plan_revision: Option<u64>) -> LiveCandidate {
    LiveCandidate {
        price_id: Uuid::now_v7(),
        plan_revision,
    }
}

fn reference(day: u32) -> ReferenceCandidate {
    ReferenceCandidate {
        historical_price_id: Uuid::now_v7(),
        effective_from: at(day),
    }
}

// ---------------------------------------------------------------------------
// D-76's two tiers.
// ---------------------------------------------------------------------------

#[test]
fn tier_one_resolves_and_the_reference_set_is_not_consulted() {
    // "Reference set **only if** (1) is empty". The two tiers are not equally
    // good evidence: tier 1 *is* what rating resolved at `t`.
    let candidate = live(Some(3));
    let selected = select_rows(&[candidate], &[reference(1)])
        .into_iter()
        .next()
        .expect("tier 1 answers");

    assert_eq!(selected.row_id, candidate.price_id);
    assert_eq!(selected.tier, SelectionTier::LiveHistory);
    assert_eq!(selected.plan_revision, Some(3));
}

#[test]
fn tier_two_resolves_only_when_live_history_is_empty() {
    let row = reference(5);
    let selected = select_rows(&[], &[row])
        .into_iter()
        .next()
        .expect("tier 2 answers");

    assert_eq!(selected.row_id, row.historical_price_id);
    assert_eq!(selected.tier, SelectionTier::HistoricalImport);
    // A reference row belongs to no revision, and that is the fact D-87 makes the
    // payload self-contained *because* of - not a missing lookup.
    assert_eq!(selected.plan_revision, None);
}

#[test]
fn the_reference_set_takes_the_greatest_effective_from_at_or_below_t() {
    // Implemented as stated rather than as an assertion about the store's own
    // interval uniqueness, which would be a second place for that invariant.
    let older = reference(1);
    let newer = reference(9);
    let middle = reference(4);

    let selected = select_rows(&[], &[older, newer, middle])
        .into_iter()
        .next()
        .expect("tier 2 answers");
    assert_eq!(selected.row_id, newer.historical_price_id);

    // Order of presentation must not decide it.
    let reordered = select_rows(&[], &[newer, older, middle])
        .into_iter()
        .next()
        .expect("tier 2 answers");
    assert_eq!(reordered.row_id, newer.historical_price_id);
}

#[test]
fn neither_tier_answering_is_none_and_never_the_current_row() {
    // Clause (3). The whole rule exists for this: the current row is precisely
    // the price the subscriber was **not** paying.
    assert!(select_rows(&[], &[]).is_empty());
}

#[test]
fn the_reference_set_is_empty_in_the_built_system_and_the_seam_still_holds() {
    // `pricing_historical_price` is Slice 5's `inst-bd-store` and is unbuilt, so
    // tier 2 is always handed an empty slice today. This is the shape every call
    // in this system actually makes, and it must fall to clause (3) rather than
    // to anything else.
    assert!(select_rows(&[], &[]).is_empty());
    // ...while tier 1 keeps working, which is what makes the seam a seam.
    let candidate = live(Some(0));
    assert_eq!(
        select_rows(&[candidate], &[]),
        vec![SelectedRow {
            row_id: candidate.price_id,
            tier: SelectionTier::LiveHistory,
            plan_revision: Some(0),
        }]
    );
}

/// **A market with more than one live line freezes all of them.**
///
/// A [`FrozenKey`](crate::infra::synthesis::FrozenKey) is D-76's frozen
/// `(currency, region)` pair — a market, not a canonical scope key — and a market
/// legitimately carries several lines: `inst-cs-hybrid` exists to sanction a
/// recurring row beside a usage row, and an `existing_grandfathered` generation
/// sits beside an `all_subscriptions` row on the same pair.
///
/// This rule answered `Option` until 2026-08-11 and the caller took the first
/// candidate, so every other line on the market was dropped in silence. D-87 makes
/// the frozen payload self-contained — Rating evaluates from it and Billing posts
/// from it, resolving nothing through the read model — so **a dropped line is a
/// charge that never happens**, on a record with a seven-year horizon.
#[test]
fn every_live_line_on_one_market_is_frozen_not_the_first() {
    let recurring = live(Some(0));
    let usage = live(Some(0));

    let selected = select_rows(&[recurring, usage], &[]);

    assert_eq!(
        selected.len(),
        2,
        "both lines of the market are the subscriber's economics, not one of them"
    );
    let frozen: Vec<Uuid> = selected.iter().map(|row| row.row_id).collect();
    assert!(frozen.contains(&recurring.price_id));
    assert!(frozen.contains(&usage.price_id));
    assert!(
        selected
            .iter()
            .all(|row| row.tier == SelectionTier::LiveHistory),
        "tier 1 still short-circuits tier 2 (D-76): the set is one tier's, never a merge"
    );
}

/// Tier 2 still answers with **at most one**, and that asymmetry is the rule
/// rather than an oversight: its selection is "the greatest `effective_from <= t`",
/// which names a single row by construction, where tier 1's set is "every row live
/// on the market".
#[test]
fn the_reference_tier_still_answers_with_one_row() {
    let selected = select_rows(&[], &[reference(1), reference(9), reference(5)]);

    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].tier, SelectionTier::HistoricalImport);
}

// ---------------------------------------------------------------------------
// D-81's triggers, and `inst-sy-firstrating`.
// ---------------------------------------------------------------------------

#[test]
fn first_rating_never_runs_inline_and_migration_does() {
    // `inst-sy-firstrating`: the rating line fails closed into the exception
    // path, synthesis runs as a separate audited step, and rating retries.
    assert!(!SynthesisTrigger::FirstRating.runs_inline());
    assert!(SynthesisTrigger::Migration.runs_inline());
}

#[test]
fn every_trigger_round_trips_and_an_unknown_token_does_not_resolve() {
    for &trigger in SynthesisTrigger::ALL {
        assert_eq!(SynthesisTrigger::parse(trigger.as_str()), Some(trigger));
    }
    // The design set's prose spells it `first-rating`; the stored token is
    // `first_rating`, and the hyphenated form must not resolve - a snapshot
    // attributed to the wrong instant rule is worse than one that fails to read.
    assert_eq!(SynthesisTrigger::parse("first-rating"), None);
    assert_eq!(SynthesisTrigger::parse(""), None);
}

#[test]
fn every_selection_tier_round_trips_in_d76s_own_spelling() {
    for &tier in SelectionTier::ALL {
        assert_eq!(SelectionTier::parse(tier.as_str()), Some(tier));
    }
    assert_eq!(SelectionTier::LiveHistory.as_str(), "live_history");
    assert_eq!(
        SelectionTier::HistoricalImport.as_str(),
        "historical_import"
    );
    assert_eq!(SelectionTier::parse("live-history"), None);
}

// ---------------------------------------------------------------------------
// Clause (3) at the outcome level.
// ---------------------------------------------------------------------------

#[test]
fn a_fully_resolved_outcome_freezes() {
    let outcome = SynthesisOutcome {
        selected: vec![SelectedRow {
            row_id: Uuid::now_v7(),
            tier: SelectionTier::LiveHistory,
            plan_revision: Some(0),
        }],
        unresolved: Vec::new(),
    };
    assert!(outcome.ensure_complete(Uuid::now_v7()).is_ok());
}

#[test]
fn one_unresolved_key_refuses_the_whole_snapshot() {
    // **Partial synthesis is the one outcome that must not exist.** A snapshot
    // missing a key is a subscription that will fail to rate on it later, with a
    // frozen record asserting its economics were captured.
    let subscription = Uuid::now_v7();
    let outcome = SynthesisOutcome {
        selected: vec![SelectedRow {
            row_id: Uuid::now_v7(),
            tier: SelectionTier::LiveHistory,
            plan_revision: Some(0),
        }],
        unresolved: vec![UnresolvedKey {
            currency: "JPY".to_owned(),
            region: "APAC".to_owned(),
        }],
    };

    let err = outcome.ensure_complete(subscription).unwrap_err();
    let DomainError::PriceRowAbsent(detail) = err else {
        panic!("expected PriceRowAbsent, got {err:?}");
    };
    assert!(detail.contains("JPY"), "{detail}");
    assert!(detail.contains("APAC"), "{detail}");
    assert!(detail.contains(&subscription.to_string()), "{detail}");
    // The refusal says why it did not simply use the current row.
    assert!(detail.contains("were not paying"), "{detail}");
}

#[test]
fn an_empty_resolution_set_is_refused_even_with_nothing_unresolved() {
    // Belt and braces with `chk_pricing_snapshot_provenance_resolved`: a snapshot
    // resolving nothing is not a snapshot.
    let outcome = SynthesisOutcome {
        selected: Vec::new(),
        unresolved: Vec::new(),
    };
    assert!(outcome.ensure_complete(Uuid::now_v7()).is_err());
}
