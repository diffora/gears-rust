//! The resolution order, one case per step and one for the refusal.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::{
    CURRENCY_WIDE_REGION, is_currency_wide, resolution_order, resolve, resolves_statically,
};
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

/// A plan with a currency-wide EUR price and a `DE` override of it.
fn eur_with_a_de_override() -> Vec<MarketPriceScopeKey> {
    vec![
        market(5, "EUR", CURRENCY_WIDE_REGION),
        market(5, "EUR", "DE"),
    ]
}

#[test]
fn the_reserved_region_is_the_one_every_tenant_starts_with() {
    assert_eq!(CURRENCY_WIDE_REGION, crate::domain::taxonomy::SEEDED_REGION);
    assert!(is_currency_wide(&region("global")));
    assert!(!is_currency_wide(&region("DE")));
}

#[test]
fn the_region_is_tried_before_the_currency_wide_price_and_each_once() {
    assert_eq!(
        resolution_order(PriceOverlay::Base, &region("DE")),
        vec![
            (PriceOverlay::Base, region("DE")),
            (PriceOverlay::Base, region("global")),
        ]
    );
    assert_eq!(
        resolution_order(PriceOverlay::Base, &region("global")),
        vec![(PriceOverlay::Base, region("global"))],
        "asking for the currency-wide price tries it once"
    );
}

#[test]
fn a_region_with_a_price_of_its_own_resolves_it() {
    let keys = eur_with_a_de_override();
    let found = resolve(&keys, &charge(5), &currency("EUR"), &region("DE"), |_| true);
    assert_eq!(found, Some(&market(5, "EUR", "DE")));
}

#[test]
fn a_region_without_one_resolves_the_currency_wide_price() {
    let keys = eur_with_a_de_override();
    let found = resolve(&keys, &charge(5), &currency("EUR"), &region("FR"), |_| true);
    assert_eq!(found, Some(&market(5, "EUR", "global")));
}

/// The regional promotion working as intended: an override that does not admit
/// — its window has ended, say — falls back instead of refusing.
#[test]
fn an_override_that_does_not_admit_falls_back_to_the_currency_wide_price() {
    let keys = eur_with_a_de_override();
    let found = resolve(&keys, &charge(5), &currency("EUR"), &region("DE"), |key| {
        is_currency_wide(key.region())
    });
    assert_eq!(found, Some(&market(5, "EUR", "global")));
}

#[test]
fn another_currency_is_refused_there_is_no_fx_and_no_cross_currency_fallback() {
    let keys = eur_with_a_de_override();
    for anywhere in ["DE", "FR", "global"] {
        assert_eq!(
            resolve(
                &keys,
                &charge(5),
                &currency("USD"),
                &region(anywhere),
                |_| true
            ),
            None,
            "USD in {anywhere}"
        );
        assert!(!resolves_statically(
            &keys,
            &charge(5),
            &currency("USD"),
            &region(anywhere)
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
        &region("DE")
    ));
    assert!(!resolves_statically(
        &keys,
        &charge(5),
        &currency("EUR"),
        &region("IT")
    ));
    assert!(
        !resolves_statically(&keys, &charge(5), &currency("EUR"), &region("global")),
        "nothing falls back *up* to a region"
    );
}

/// Resolution is per charge: a sibling line's currency-wide price answers for
/// nobody but itself.
#[test]
fn another_charges_currency_wide_price_does_not_answer_for_this_one() {
    let keys = vec![market(5, "EUR", "DE"), market(6, "EUR", "global")];
    assert!(!resolves_statically(
        &keys,
        &charge(5),
        &currency("EUR"),
        &region("FR")
    ));
    assert!(resolves_statically(
        &keys,
        &charge(6),
        &currency("EUR"),
        &region("FR")
    ));
}
