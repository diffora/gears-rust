//! The resolution order, one case per step and one for the refusal.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeSet;

use super::{is_sold, owed_markets, resolution_order, resolve, resolves_statically};
use crate::domain::currency_binding::Market;
use crate::domain::money::CurrencyCode;
use crate::domain::scope_key::{
    ChargeKind, ChargeLineScopeKey, Cohort, MarketPriceScopeKey, PhaseId, PlanId, PriceEligibility,
    PriceOverlay, Region, SkuId,
};
use uuid::Uuid;

fn charge(sku: u128) -> ChargeLineScopeKey {
    ChargeLineScopeKey::new(
        PlanId::new(Uuid::from_u128(0x11)),
        PhaseId::new(Uuid::from_u128(0x22)),
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
        Cohort::None,
        SkuId::new(Uuid::from_u128(sku)),
    )
    .expect("all_subscriptions pairs with cohort none")
}

fn currency(code: &str) -> CurrencyCode {
    CurrencyCode::new(code).expect("three letters")
}

fn region(value: &str) -> Region {
    Region::new(value).expect("non-blank")
}

fn market(sku: u128, code: &str, value: &str) -> MarketPriceScopeKey {
    MarketPriceScopeKey::new(charge(sku), currency(code), region(value))
}

/// The currency-wide market of one line — the row that states no region.
fn everywhere(sku: u128, code: &str) -> MarketPriceScopeKey {
    MarketPriceScopeKey::currency_wide(charge(sku), currency(code))
}

/// A plan with a currency-wide EUR price and a `DE` override of it.
fn eur_with_a_de_override() -> Vec<MarketPriceScopeKey> {
    vec![everywhere(5, "EUR"), market(5, "EUR", "DE")]
}

fn sold(markets: &[(&str, Option<&str>)]) -> BTreeSet<Market> {
    markets
        .iter()
        .map(|(code, value)| (currency(code), value.map(region)))
        .collect()
}

#[test]
fn the_currency_wide_market_is_the_absent_region_and_not_a_taxonomy_value() {
    // D-381: it has no region at all, so nothing a tenant can declare,
    // deprecate or retire reaches it — the property D-379's reserved `global`
    // value did not have.
    let wide = everywhere(5, "EUR");
    assert!(wide.is_currency_wide());
    assert_eq!(wide.region(), None);
    assert!(!market(5, "EUR", "DE").is_currency_wide());
    // And a region a tenant happens to name `global` is an ordinary region.
    assert!(!market(5, "EUR", "global").is_currency_wide());
    assert_ne!(market(5, "EUR", "global"), wide);
}

#[test]
fn the_region_is_tried_before_the_currency_wide_price_and_each_once() {
    assert_eq!(
        resolution_order(PriceOverlay::Base, Some(&region("DE"))),
        vec![
            (PriceOverlay::Base, Some(region("DE"))),
            (PriceOverlay::Base, None),
        ]
    );
    assert_eq!(
        resolution_order(PriceOverlay::Base, None),
        vec![(PriceOverlay::Base, None)],
        "a buyer with no region tries the currency-wide market alone"
    );
}

#[test]
fn a_region_with_a_price_of_its_own_resolves_it() {
    let keys = eur_with_a_de_override();
    let found = resolve(
        &keys,
        &charge(5),
        &currency("EUR"),
        Some(&region("DE")),
        |_| true,
    );
    assert_eq!(found, Some(&market(5, "EUR", "DE")));
}

#[test]
fn a_region_without_one_resolves_the_currency_wide_price() {
    let keys = eur_with_a_de_override();
    let found = resolve(
        &keys,
        &charge(5),
        &currency("EUR"),
        Some(&region("FR")),
        |_| true,
    );
    assert_eq!(found, Some(&everywhere(5, "EUR")));
}

/// A buyer with no territory is quoted the currency-wide price, and only it.
#[test]
fn a_buyer_with_no_region_resolves_the_currency_wide_price_and_no_override() {
    let keys = eur_with_a_de_override();
    assert_eq!(
        resolve(&keys, &charge(5), &currency("EUR"), None, |_| true),
        Some(&everywhere(5, "EUR"))
    );
    // With only a `DE` override there is nothing such a buyer can be sold.
    let only_de = vec![market(5, "EUR", "DE")];
    assert_eq!(
        resolve(&only_de, &charge(5), &currency("EUR"), None, |_| true),
        None,
        "no region's row serves a buyer who is in no region"
    );
}

/// The regional promotion working as intended: an override that does not admit
/// — its window has ended, say — falls back instead of refusing.
#[test]
fn an_override_that_does_not_admit_falls_back_to_the_currency_wide_price() {
    let keys = eur_with_a_de_override();
    let found = resolve(
        &keys,
        &charge(5),
        &currency("EUR"),
        Some(&region("DE")),
        MarketPriceScopeKey::is_currency_wide,
    );
    assert_eq!(found, Some(&everywhere(5, "EUR")));
}

#[test]
fn another_currency_is_refused_there_is_no_fx_and_no_cross_currency_fallback() {
    let keys = eur_with_a_de_override();
    for anywhere in [Some("DE"), Some("FR"), None] {
        let asked = anywhere.map(region);
        assert_eq!(
            resolve(&keys, &charge(5), &currency("USD"), asked.as_ref(), |_| {
                true
            }),
            None,
            "USD in {anywhere:?}"
        );
        assert!(!resolves_statically(
            &keys,
            &charge(5),
            &currency("USD"),
            asked.as_ref()
        ));
    }
}

/// No currency-wide price anywhere is today's rule, per pair, unchanged.
#[test]
fn without_a_currency_wide_price_only_an_exact_region_resolves() {
    let keys = vec![market(5, "EUR", "DE"), market(5, "EUR", "FR")];
    assert!(resolves_statically(
        &keys,
        &charge(5),
        &currency("EUR"),
        Some(&region("DE"))
    ));
    assert!(!resolves_statically(
        &keys,
        &charge(5),
        &currency("EUR"),
        Some(&region("IT"))
    ));
    assert!(
        !resolves_statically(&keys, &charge(5), &currency("EUR"), None),
        "nothing falls back *up* from a region to the currency-wide market"
    );
}

/// Resolution is per charge: a sibling line's currency-wide price answers for
/// nobody but itself.
#[test]
fn another_charges_currency_wide_price_does_not_answer_for_this_one() {
    let keys = vec![market(5, "EUR", "DE"), everywhere(6, "EUR")];
    assert!(!resolves_statically(
        &keys,
        &charge(5),
        &currency("EUR"),
        Some(&region("FR"))
    ));
    assert!(resolves_statically(
        &keys,
        &charge(6),
        &currency("EUR"),
        Some(&region("FR"))
    ));
}

/// Completeness per currency: a currency-wide row on any line makes every line
/// owe that market, and an override obliges no sibling.
#[test]
fn a_currency_sold_everywhere_owes_only_the_currency_wide_market() {
    assert_eq!(
        owed_markets(&sold(&[("EUR", None), ("EUR", Some("DE"))])),
        sold(&[("EUR", None)])
    );
}

#[test]
fn a_currency_sold_only_regionally_owes_every_region_it_is_sold_in() {
    let regional = sold(&[("EUR", Some("DE")), ("EUR", Some("FR"))]);
    assert_eq!(owed_markets(&regional), regional);
}

/// The two currencies are judged apart: one sold everywhere does not excuse the
/// other's regional obligations.
#[test]
fn the_two_footings_are_per_currency() {
    assert_eq!(
        owed_markets(&sold(&[
            ("EUR", None),
            ("EUR", Some("DE")),
            ("USD", Some("US")),
            ("USD", Some("CA")),
        ])),
        sold(&[("EUR", None), ("USD", Some("US")), ("USD", Some("CA"))])
    );
}

#[test]
fn a_statement_is_sold_by_its_own_market_or_by_the_currency_wide_one() {
    let only_de = sold(&[("EUR", Some("DE"))]);
    let everywhere_eur = sold(&[("EUR", None)]);

    // A regional statement: its own row, or the currency-wide one behind it.
    assert!(is_sold(&only_de, &currency("EUR"), Some(&region("DE"))));
    assert!(!is_sold(&only_de, &currency("EUR"), Some(&region("FR"))));
    assert!(is_sold(
        &everywhere_eur,
        &currency("EUR"),
        Some(&region("FR"))
    ));

    // **And the fallback runs one way.** A currency-wide statement is every
    // region at once, so a `DE` row does not serve it — the property D-95's
    // `CURRENCY_NOT_COVERED` rests on.
    assert!(!is_sold(&only_de, &currency("EUR"), None));
    assert!(is_sold(&everywhere_eur, &currency("EUR"), None));
    assert!(!is_sold(&only_de, &currency("USD"), None));
}
